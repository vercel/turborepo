use std::{fmt::Write, path::PathBuf, sync::Arc};

use swc_core::{
    common::SourceMap,
    ecma::{
        ast::{EsVersion, Id},
        codegen::text_writer::JsWriter,
        parser::parse_file_as_module,
    },
    testing::{self, fixture, NormalizedOutput},
};

use super::{
    graph::{DepGraph, ItemId, ItemIdKind},
    Analyzer,
};

#[fixture("tests/tree-shaker/analyzer/**/input.js")]
fn test_fixture(input: PathBuf) {
    run(input);
}

fn run(input: PathBuf) {
    testing::run_test(false, |cm, handler| {
        let fm = cm.load_file(&input).unwrap();

        let module = parse_file_as_module(
            &fm,
            Default::default(),
            EsVersion::latest(),
            None,
            &mut vec![],
        )
        .unwrap();

        let mut g = DepGraph::default();
        let (item_ids, mut items) = g.init(&module);

        let mut s = String::new();

        writeln!(s, "# Items\n").unwrap();
        writeln!(s, "Count: {}", item_ids.len()).unwrap();
        writeln!(s).unwrap();

        for (i, id) in item_ids.iter().enumerate() {
            let item = &items[id];

            if id.index == usize::MAX {
                continue;
            }

            writeln!(s, "## Item {}: Stmt {}, `{:?}`", i + 1, id.index, id.kind).unwrap();
            writeln!(
                s,
                "\n```js\n{}\n```\n",
                print(&cm, &[&module.body[id.index]])
            )
            .unwrap();

            if item.is_hoisted {
                writeln!(s, "- Hoisted").unwrap();
            }

            if item.side_effects {
                writeln!(s, "- Side effects").unwrap();
            }

            let f = |ids: &[Id]| {
                let mut s = String::new();
                for (i, id) in ids.iter().enumerate() {
                    if i == 0 {
                        write!(s, "`{}`", id.0).unwrap();
                    } else {
                        write!(s, ", `{}`", id.0).unwrap();
                    }
                }
                s
            };

            if !item.var_decls.is_empty() {
                writeln!(s, "- Declares: {:?}", f(&item.var_decls)).unwrap();
            }

            if !item.read_vars.is_empty() {
                writeln!(s, "- Reads: {:?}", f(&item.read_vars)).unwrap();
            }

            if !item.eventual_read_vars.is_empty() {
                writeln!(s, "- Reads (eventual): {:?}", f(&item.eventual_read_vars)).unwrap();
            }

            if !item.write_vars.is_empty() {
                writeln!(s, "- Write: {:?}", f(&item.write_vars)).unwrap();
            }

            if !item.eventual_write_vars.is_empty() {
                writeln!(s, "- Write (eventual): {:?}", f(&item.eventual_write_vars)).unwrap();
            }

            writeln!(s).unwrap();
        }

        let mut analyzer = Analyzer {
            g: &mut g,
            item_ids: &item_ids,
            items: &mut items,
            last_side_effect: Default::default(),
            vars: Default::default(),
        };

        let eventual_ids = analyzer.hoist_vars_and_bindings(&module);

        writeln!(s, "# Phase 1").unwrap();
        writeln!(
            s,
            "```mermaid\n{}```",
            render_graph(&item_ids, &mut analyzer.g)
        )
        .unwrap();

        analyzer.evaluate_immediate(&module, &eventual_ids);

        writeln!(s, "# Phase 2").unwrap();
        writeln!(
            s,
            "```mermaid\n{}```",
            render_graph(&item_ids, &mut analyzer.g)
        )
        .unwrap();

        analyzer.evaluate_eventual(&module);

        writeln!(s, "# Phase 3").unwrap();
        writeln!(
            s,
            "```mermaid\n{}```",
            render_graph(&item_ids, &mut analyzer.g)
        )
        .unwrap();

        analyzer.handle_exports(&module);

        writeln!(s, "# Phase 4").unwrap();
        writeln!(
            s,
            "```mermaid\n{}```",
            render_graph(&item_ids, &mut analyzer.g)
        )
        .unwrap();

        let condensed = analyzer.g.finalize();
        let condensed = condensed.map(
            |ix, indexes| {
                let mut buf = vec![];
                for index in indexes {
                    let item_id = analyzer.g.g.graph_ix.get_index(*index as _).unwrap();

                    let rendered = render_item_id(&item_id.kind)
                        .unwrap_or_else(|| print(&cm, &[&module.body[item_id.index]]));
                    buf.push(rendered);
                }

                buf
            },
            |ix, edge| *edge,
        );

        let dot = petgraph::dot::Dot::with_config(&condensed, &[]);
        println!("DOT!\n{:?}", dot);

        NormalizedOutput::from(s)
            .compare_to_file(input.with_file_name("output.md"))
            .unwrap();

        Ok(())
    })
    .unwrap();
}

fn print<N: swc_core::ecma::codegen::Node>(cm: &Arc<SourceMap>, nodes: &[&N]) -> String {
    let mut buf = vec![];

    {
        let mut emitter = swc_core::ecma::codegen::Emitter {
            cfg: Default::default(),
            cm: cm.clone(),
            comments: None,
            wr: Box::new(JsWriter::new(cm.clone(), "\n", &mut buf, None)),
        };

        for n in nodes {
            n.emit_with(&mut emitter).unwrap();
        }
    }

    String::from_utf8(buf).unwrap()
}

fn render_graph(item_ids: &[ItemId], g: &mut DepGraph) -> String {
    let mut mermaid = String::from("graph TD\n");

    for (i, id) in item_ids.iter().enumerate() {
        let i = g.node(id);

        writeln!(mermaid, "    Item{};", i + 1).unwrap();

        if let Some(item_id) = render_item_id(&id.kind) {
            writeln!(mermaid, "    Item{}[\"{}\"];", i + 1, item_id).unwrap();
        }
    }

    for (from, to, strong) in g.inner.all_edges() {
        writeln!(
            mermaid,
            "    Item{} -{}-> Item{};",
            from + 1,
            if *strong { "" } else { "." },
            to + 1,
        )
        .unwrap();
    }

    mermaid
}

fn render_item_id(id: &ItemIdKind) -> Option<String> {
    match id {
        ItemIdKind::ModuleEvaluation => Some("ModuleEvaluation".into()),
        ItemIdKind::Export(id) => Some(format!("export {}", id.0)),
        _ => None,
    }
}
