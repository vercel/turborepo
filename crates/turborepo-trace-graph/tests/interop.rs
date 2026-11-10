//! Output formats, JSON import/export and SQLite export.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::graph;
use turborepo_trace_graph::{
    Engine, GraphView, MemGraph,
    json::JsonError,
    output::{self, Format, OutputOptions},
};

fn render(g: &MemGraph, src: &str, format: Format, edges: bool) -> String {
    let e = Engine::new(g);
    let r = e.query(src).unwrap();
    let mut out = Vec::new();
    output::write(&e, &r, &OutputOptions { format, edges }, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn list_is_sorted() {
    let g = graph();
    assert_eq!(
        render(&g, "deps(src/b.ts)", Format::List, false),
        "node_modules/lodash/index.js\nsrc/b.ts\nsrc/shared.ts\nsrc/util.ts\nsrc/x.ts\nsrc/y.ts\n"
    );
}

#[test]
fn tree_marks_cycles_and_repeats() {
    let g = graph();
    assert_eq!(
        render(&g, "deps(test/b.test.ts)", Format::Tree, false),
        "\
test/b.test.ts
└── src/b.ts
    ├── src/shared.ts
    │   └── src/util.ts
    │       └── node_modules/lodash/index.js
    └── src/x.ts
        └── src/y.ts
            └── src/x.ts (cycle)
"
    );
    assert_eq!(
        render(&g, "deps(d/1.ts)", Format::Tree, false),
        "\
d/1.ts
├── d/2.ts
│   └── d/4.ts
└── d/3.ts
    └── d/4.ts (*)
"
    );
    // A pure cycle has no root; the smallest path starts it.
    assert_eq!(
        render(&g, "src/x.ts + src/y.ts", Format::Tree, false),
        "src/x.ts\n└── src/y.ts\n    └── src/x.ts (cycle)\n"
    );
}

#[test]
fn json_nodes_and_edges() {
    let g = graph();
    let s = render(&g, "deps(test/a.test.ts, 2)", Format::Json, true);
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["count"], 4);
    assert_eq!(v["nodes"][0]["path"], "src/a.ts");
    assert_eq!(v["nodes"][0]["package"], ".");
    assert_eq!(v["nodes"][0]["version"], "workspace");
    assert_eq!(v["nodes"][0]["lang"], "js");
    let edges = v["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3);
    assert_eq!(edges[0]["from"], "src/a.ts");
    assert_eq!(edges[0]["to"], "src/shared.ts");
    assert_eq!(edges[1]["flags"][0], "type_only");
    let s = render(
        &g,
        "somepath(test/a.test.ts, src/util.ts)",
        Format::Json,
        false,
    );
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(
        v["path"],
        serde_json::json!(["test/a.test.ts", "src/a.ts", "src/shared.ts", "src/util.ts"])
    );
    assert!(v.get("edges").is_none());
}

#[test]
fn dot_output() {
    let g = graph();
    let s = render(&g, "deps(test/a.test.ts, 2)", Format::Dot, false);
    assert!(s.starts_with("digraph deps {\n"));
    let a = g.node_by_path("src/a.ts").unwrap().0;
    let t = g.node_by_path("src/types.ts").unwrap().0;
    assert!(s.contains(&format!("  {a} [label=\"src/a.ts\"];")), "{s}");
    assert!(
        s.contains(&format!("  {a} -> {t} [label=import, style=dotted];")),
        "{s}"
    );
    assert!(s.ends_with("}\n"));
}

#[test]
fn affected_json_and_explain() {
    let g = graph();
    let src = "affected(modified(src/util.ts) + added(locales/fr.json) + modified(nope.md), \
               test/**, unknown)";
    let v: serde_json::Value = serde_json::from_str(&render(&g, src, Format::Json, false)).unwrap();
    assert_eq!(v["policy"], "unknown");
    assert_eq!(v["entries"], 4);
    assert_eq!(v["affected"], 3);
    assert_eq!(v["unknown"], 1);
    assert_eq!(v["unaffected"], 0);
    assert_eq!(
        v["unmatched"],
        serde_json::json!([{"path": "nope.md", "kind": "modified"}])
    );
    let r = &v["results"];
    assert_eq!(r[0]["entry"], "test/a.test.ts");
    assert_eq!(r[0]["verdict"], "affected");
    assert_eq!(
        r[0]["witness"],
        serde_json::json!(["src/util.ts", "src/shared.ts", "src/a.ts", "test/a.test.ts"])
    );
    assert_eq!(
        r[0]["cause"],
        serde_json::json!({"type": "modified", "path": "src/util.ts"})
    );
    assert_eq!(
        r[2]["cause"],
        serde_json::json!({"type": "membership", "path": "locales/fr.json", "change": "added",
                           "dir": "locales", "pattern": "*.json"})
    );
    assert_eq!(r[3]["verdict"], "unknown");
    assert_eq!(
        r[3]["uncertain"],
        serde_json::json!({"file": "src/dyn.ts", "reasons": [
            {"kind": "computed_specifier", "line": 3, "detail": "require(name)"}]})
    );
    assert_eq!(
        render(&g, src, Format::List, false),
        "test/a.test.ts\ntest/b.test.ts\ntest/c.test.ts\n"
    );
    let explain = render(&g, src, Format::Tree, false);
    assert_eq!(
        explain.lines().next().unwrap(),
        "affected test/a.test.ts: modified src/util.ts → src/shared.ts → src/a.ts → test/a.test.ts"
    );
    assert!(explain.contains(
        "affected test/c.test.ts: added locales/fr.json (matches locales/*.json) at src/c.ts → \
         test/c.test.ts"
    ));
    assert!(explain.contains(
        "unknown test/d.test.ts: uncertain (computed_specifier) src/dyn.ts → test/d.test.ts"
    ));
}

#[test]
fn json_round_trip() {
    let g = graph();
    let s = g.to_json_string();
    let g2 = MemGraph::from_json_str(&s).unwrap();
    assert_eq!(g2.to_json_string(), s);
    assert_eq!(g2.node_count(), g.node_count());
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["schema"], "turbotrace-graph");
    assert_eq!(v["version"], 1);
    assert_eq!(v["packages"][1]["version"], "4.17.21");
    // Same query answers on the reloaded graph.
    let a = Engine::new(&g).query("rdeps(src/util.ts)").unwrap().nodes;
    let b = Engine::new(&g2).query("rdeps(src/util.ts)").unwrap().nodes;
    assert_eq!(a, b);
}

#[test]
fn json_minimal_and_defaults() {
    let g = MemGraph::from_json_str(
        r#"{"schema": "turbotrace-graph", "version": 1,
            "packages": [{"name": "app", "version": "workspace"}],
            "files": [{"path": "a.go", "package": 0, "lang": "go"}, {"path": "b.go", "package": 0}],
            "edges": [{"from": 0, "to": 1, "kind": "package"},
                      {"from": 0, "kind": "import", "specifier": "fmt", "line": 3}]}"#,
    )
    .unwrap();
    let edges: Vec<_> = g.out_edges(turborepo_trace_graph::NodeId(0)).collect();
    assert_eq!(edges[0].phase(), turborepo_trace_graph::Phase::Build);
    assert_eq!(edges[1].to, None);
    assert_eq!(edges[1].specifier, Some("fmt"));
}

#[test]
fn json_errors() {
    let msg = |s: &str| MemGraph::from_json_str(s).unwrap_err().to_string();
    assert_eq!(
        msg(r#"{"files": []}"#),
        "not a turbo trace graph (expected \"schema\": \"turbotrace-graph\")"
    );
    assert_eq!(
        msg(r#"{"schema": "turbotrace-graph", "version": 2, "files": [], "nodes": 1}"#),
        "unsupported graph schema version 2 (this build reads version 1)"
    );
    let e = msg(
        r#"{"schema": "turbotrace-graph", "version": 1, "packages": [{"name":"a","version":"workspace"}],
            "files": [{"path": "a", "package": 0}], "edges": [{"from": 0, "to": 5, "kind": "import"}]}"#,
    );
    assert!(
        e.contains("edge target refers to file 5, but there are 1 files"),
        "{e}"
    );
    let e = msg(
        r#"{"schema": "turbotrace-graph", "version": 1, "packages": [{"name":"a","version":"workspace"}],
            "files": [{"path": "a", "package": 0}], "edges": [{"from": 0, "to": 0, "kind": "imprt"}]}"#,
    );
    assert!(e.contains("unknown variant `imprt`"), "{e}");
    let e = msg(
        r#"{"schema": "turbotrace-graph", "version": 1, "packages": [],
            "files": [{"path": "a", "package": 0}]}"#,
    );
    assert!(
        e.contains("refers to package 0, but there are 0 packages"),
        "{e}"
    );
    assert!(matches!(
        MemGraph::from_json_str("{"),
        Err(JsonError::Syntax(_))
    ));
}

#[test]
fn ndjson_lines_match_json_records() {
    let mut b = turborepo_trace_graph::MemGraph::builder();
    let a = b.file("src/a.ts");
    let c = b.file("src/b.ts");
    b.edge(a, c, turborepo_trace_graph::RefKind::Import)
        .probe(a, "src/x.ts");
    let g = b.build().unwrap();
    let mut out = Vec::new();
    turborepo_trace_graph::json::write_ndjson(&g, Some("/r"), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["schema"], "turbotrace-graph-ndjson");
    let types: Vec<&str> = lines[1..]
        .iter()
        .map(|l| l["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["package", "file", "file", "edge", "probe"]);
    assert_eq!(lines[4]["from"], 0);
    assert_eq!(lines[4]["to"], 1);
    assert_eq!(lines[5]["path"], "src/x.ts");
}
