//! The environment registry (`turborepo_trace_store::compose::env`) on a built
//! graph: the patch it derives, and the cross-file evaluator hook for
//! assignments the per-file facts leave unknown. Shapes from real repositories,
//! names changed.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::Arc};

use turborepo_trace_core::{
    Language,
    env::{EnvAlt, EnvBase, EnvPart, EnvTerm},
};
use turborepo_trace_graph::{GraphView, Origin, Patched};
use turborepo_trace_store::{EnvAnswer, EnvDecl, Graph, GraphOptions, build};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

fn graph(r: &Path) -> Graph {
    let out = r.join(".turbotrace");
    let opts = GraphOptions {
        root: r.to_path_buf(),
        out: Some(out.clone()),
        threads: 2,
        ..Default::default()
    };
    let langs: Vec<Arc<dyn Language>> = vec![Arc::new(turborepo_trace_lang_js::JsLanguage)];
    build(&opts, &langs).unwrap();
    Graph::open(&out).unwrap()
}

/// The reader: a datafile path from the environment, through a const and a
/// guard.
const CLIENT: &str = "import { readFileSync } from 'node:fs';\nconst testDatafilePath = \
                      process.env.FLAGS_TEST_DATAFILE;\nexport const testDatafile = \
                      testDatafilePath\n  ? JSON.parse(readFileSync(testDatafilePath, 'utf8'))\n  \
                      : undefined;\n";

/// A test server passes the variable to the processes it starts; the value is a
/// constructor parameter with a default from an imported helper.
const SERVER: &str = "import { tempFile } from './temp-file';\nexport class FlagsService {\n  \
                      constructor(private readonly flagsFile = tempFile({ prefix: 'flags-' })) \
                      {}\n  async getTestEnv() {\n    return { FLAGS_TEST_DATAFILE: \
                      this.flagsFile };\n  }\n}\n";

fn repo() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(r, ".gitignore", ".turbotrace\n");
    write(r, "package.json", r#"{"name":"root","version":"1.0.0"}"#);
    write(r, "src/client.ts", CLIENT);
    write(r, "fixtures/flags.json", "{}\n");
    t
}

/// Whether `from` depends on the content of `path`: an edge when the path is a
/// graph node, a config input otherwise.
fn depends_on(g: &impl GraphView, from: &str, path: &str) -> bool {
    let n = g.node_by_path(from).unwrap();
    let target = g.node_by_path(path);
    g.inputs(n).any(|i| i == path) || (target.is_some() && g.out_edges(n).any(|e| e.to == target))
}

fn env_records(g: &impl GraphView, path: &str) -> usize {
    let n = g.node_by_path(path).unwrap();
    g.uncertainties(n)
        .filter(|u| u.origin == Origin::Env)
        .count()
}

#[test]
fn assigned_fixture_becomes_an_edge_and_the_record_goes() {
    let t = repo();
    let r = std::fs::canonicalize(t.path()).unwrap();
    write(
        &r,
        "test/setup.ts",
        "import path from 'node:path';\nprocess.env.FLAGS_TEST_DATAFILE = path.join(__dirname, \
         '../fixtures/flags.json');\n",
    );
    let g = graph(&r);
    assert_eq!(env_records(&g, "src/client.ts"), 1);
    let (patch, stats) = g.env_patch(&[]);
    assert_eq!((stats.uses, stats.resolved), (1, 1), "{stats:?}");
    let p = Patched::new(&g, patch);
    assert_eq!(env_records(&p, "src/client.ts"), 0);
    assert!(depends_on(&p, "src/client.ts", "fixtures/flags.json"));
    assert!(!depends_on(&g, "src/client.ts", "fixtures/flags.json"));
    // The assigning file is a config input: its content, not its imports.
    let client = p.node_by_path("src/client.ts").unwrap();
    assert!(p.inputs(client).any(|i| i == "test/setup.ts"));
}

#[test]
fn unevaluable_assignment_keeps_the_record_until_an_evaluator_answers() {
    let t = repo();
    let r = std::fs::canonicalize(t.path()).unwrap();
    write(&r, "server/flags.ts", SERVER);
    write(
        &r,
        "server/temp-file.ts",
        "import { randomUUID } from 'node:crypto';\nimport path from 'node:path';\nexport \
         function tempFile({ prefix }: { prefix?: string } = {}): string {\n  return \
         path.resolve(__dirname, '../.tmp', `${prefix ?? ''}${randomUUID()}`);\n}\n",
    );
    let g = graph(&r);
    // The class is exported: its constructor argument is bound by callers the file
    // does not see, so the file's own facts cannot evaluate the assignment.
    let (patch, stats) = g.env_patch(&[]);
    assert_eq!((stats.uses, stats.resolved), (1, 0), "{stats:?}");
    let p = Patched::new(&g, patch);
    assert_eq!(env_records(&p, "src/client.ts"), 1);
    // The assigning file is an input of the reader either way.
    let client = p.node_by_path("src/client.ts").unwrap();
    assert!(p.inputs(client).any(|i| i == "server/flags.ts"));

    // A cross-file evaluator is asked for the assigned expression, by file and
    // span.
    let server = g.node_by_path("server/flags.ts").unwrap();
    let helper = g.node_by_path("server/temp-file.ts").unwrap();
    let text = std::fs::read_to_string(r.join("server/flags.ts")).unwrap();
    let asked = std::cell::RefCell::new(Vec::new());
    let oracle = |n, span: [u32; 2]| {
        asked
            .borrow_mut()
            .push((n, text[span[0] as usize..span[1] as usize].to_owned()));
        // `<root>/.tmp/flags-<unknown>`, or below a temporary directory that a variable
        // names.
        Some(EnvAnswer {
            alts: vec![
                EnvAlt::Term(EnvTerm {
                    base: EnvBase::Root,
                    parts: vec![EnvPart::Lit(".tmp/flags-".to_owned()), EnvPart::Wild],
                }),
                EnvAlt::Term(EnvTerm::str("/tmp/flags")),
            ],
            files: vec![helper],
        })
    };
    let (patch, stats) = g.env_patch_with(&[], &oracle);
    assert_eq!(stats.resolved, 1, "{stats:?}");
    assert_eq!(
        *asked.borrow(),
        [(server, "this.flagsFile".to_owned())],
        "asked for the assigned expression"
    );
    let p = Patched::new(&g, patch);
    assert_eq!(env_records(&p, "src/client.ts"), 0);
    // The file the evaluator read is an input of the reader too.
    assert!(p.inputs(client).any(|i| i == "server/temp-file.ts"));
    let members: Vec<(String, String)> = p
        .memberships(client)
        .map(|m| (m.dir.to_owned(), m.pattern.to_owned()))
        .collect();
    // The file itself, or anything below a directory of that name.
    assert!(
        members.contains(&(".tmp".to_owned(), "flags-*{,/**}".to_owned())),
        "{members:?}"
    );
}

#[test]
fn declared_values_and_known_variables() {
    let t = repo();
    let r = std::fs::canonicalize(t.path()).unwrap();
    write(
        &r,
        "scripts/report.ts",
        "import fs from 'node:fs';\nimport path from 'node:path';\nexport const a = \
         fs.readFileSync(path.join(process.env.GITHUB_WORKSPACE!, \
         'fixtures/flags.json'));\nexport const b = fs.readFileSync(path.join(process.env.HOME!, \
         '.config/tool.json'));\n",
    );
    let g = graph(&r);
    let decls = [EnvDecl {
        name: "FLAGS_TEST_DATAFILE".to_owned(),
        values: vec!["fixtures/*.json".to_owned()],
    }];
    let (patch, stats) = g.env_patch(&decls);
    assert_eq!((stats.uses, stats.resolved), (3, 3), "{stats:?}");
    let p = Patched::new(&g, patch);
    let client = p.node_by_path("src/client.ts").unwrap();
    assert!(depends_on(&p, "src/client.ts", "fixtures/flags.json"));
    assert!(
        p.memberships(client)
            .any(|m| m.dir == "fixtures" && m.pattern == "*.json")
    );
    // `GITHUB_WORKSPACE` is the repository root; `HOME` is outside the repository.
    assert!(depends_on(&p, "scripts/report.ts", "fixtures/flags.json"));
    assert!(!depends_on(&g, "scripts/report.ts", "fixtures/flags.json"));
    let report = p.node_by_path("scripts/report.ts").unwrap();
    assert_eq!(
        p.memberships(report).count(),
        g.memberships(report).count(),
        "nothing for the path below HOME"
    );
    assert_eq!(p.probes(report).count(), g.probes(report).count());
}
