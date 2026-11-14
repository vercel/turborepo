//! Code generator recognizer fixtures
//! (`fixtures/tasks/codegen/<family>/commands.txt`: one command per line → the
//! recognized `TaskSpec`), the `//go:generate` task source, and generated-file
//! linking over discovered tasks. Paths are shown relative to the fixture root.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeMap,
    fmt::Write,
    path::{Path, PathBuf},
};

use turborepo_trace_tasks::{
    Options, discover,
    registry::{RecognizeCx, registry},
    shell,
    sources::Workspace,
    spec::TaskSpec,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/codegen")
        .canonicalize()
        .expect("fixtures dir")
}

fn rel(s: String, root: &Path) -> String {
    s.replace(&format!("{}/", root.display()), "")
        .replace(&root.display().to_string(), ".")
}

fn family(name: &str) -> String {
    let root = fixtures().join(name);
    let ws = Workspace::from_roots(&root, Vec::<String>::new());
    let pkg = &ws.packages[0];
    let scripts: Vec<String> =
        turborepo_trace_tasks::sources::read_json(&root.join("package.json"))
            .and_then(|m| {
                m.get("scripts")
                    .and_then(|s| s.as_object())
                    .map(|o| o.keys().cloned().collect())
            })
            .unwrap_or_default();
    let commands = std::fs::read_to_string(root.join("commands.txt")).expect("commands.txt");
    let env = BTreeMap::new();
    let mut out = String::new();
    for line in commands
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let parsed = shell::parse(line, &root, &env);
        let mut spec = TaskSpec::default();
        spec.uncertainties.extend(parsed.uncertainties.clone());
        let cx = RecognizeCx {
            repo_root: &root,
            package_root: &pkg.dir,
            env: &env,
            task_name: None,
            scripts: &scripts,
        };
        let mut recs = Vec::new();
        for c in &parsed.commands {
            let r = registry().recognize(c, &cx, &mut spec);
            recs.push(format!(
                "{} [{}] {:?}",
                r.program.unwrap_or_default(),
                r.recognizer.unwrap_or("-"),
                r.recognition
            ));
        }
        spec.normalize();
        let json = serde_json::to_string_pretty(&spec).expect("json");
        writeln!(
            out,
            "$ {line}\n# {}\n{}\n",
            recs.join(" | "),
            rel(json, &root)
        )
        .unwrap();
    }
    out
}

macro_rules! family_test {
    ($name:ident) => {
        #[test]
        fn $name() {
            insta::assert_snapshot!(stringify!($name), family(stringify!($name)));
        }
    };
}

family_test!(prisma);
family_test!(drizzle);
family_test!(graphql);
family_test!(protobuf);
family_test!(openapi);
family_test!(sqlc);
family_test!(golang);
family_test!(python);

/// Every `//go:generate` directive of the Go module fixture becomes a task
/// (`testdata/` is skipped), and generated files link back to the directive
/// that writes them.
#[test]
fn gogenerate_source() {
    let root = fixtures().join("golang");
    let ws = Workspace::from_roots(&root, Vec::<String>::new());
    let set = discover(&ws, &Options::default());
    let mut out = String::new();
    for t in set.tasks.iter().filter(|t| t.id.kind() == "gogenerate") {
        let v = serde_json::json!({
            "name": t.name,
            "lines": t.lines,
            "commands": t.commands.iter().map(|c| format!("{} [{}] {:?}", c.text, c.recognizer.unwrap_or("-"), c.recognition)).collect::<Vec<_>>(),
            "spec": t.spec,
        });
        writeln!(
            out,
            "## {}\n{}\n",
            t.id,
            rel(serde_json::to_string_pretty(&v).unwrap(), &root)
        )
        .unwrap();
    }
    insta::assert_snapshot!("gogenerate_source", out);
    assert!(
        set.tasks
            .iter()
            .all(|t| !t.id.to_string().contains("testdata")),
        "testdata is skipped"
    );
    let producers = |rel: &str| -> Vec<String> {
        set.producers_of(&root.join(rel))
            .iter()
            .map(|t| t.id.to_string())
            .collect()
    };
    assert_eq!(
        producers("color/color_string.go"),
        ["gogenerate:color/color.go:3"]
    );
    assert_eq!(
        producers("mocks/mock_store.go"),
        ["gogenerate:store/store.go:4"]
    );
    assert_eq!(producers("api/api.gen.go"), ["gogenerate:api/api.go:3"]);
    assert_eq!(producers("ent/user.go"), ["gogenerate:ent/generate.go:3"]);
    assert!(producers("ent/schema/user.go").is_empty());
    assert!(producers("ent/generate.go").is_empty());
    assert_eq!(producers("di/wire_gen.go"), ["gogenerate:di/wire.go:5"]);
    assert_eq!(
        producers("graph/resolvers/schema.resolvers.go"),
        ["gogenerate:graph/resolver.go:3"]
    );
    assert_eq!(
        producers("ej/model_easyjson.go"),
        ["gogenerate:ej/model.go:3"]
    );
    // The stringer task reads its package, not its own output.
    let stringer = set
        .get(&"gogenerate:color/color.go:3".parse().unwrap())
        .unwrap();
    let m = stringer
        .spec
        .memberships
        .iter()
        .find(|m| m.reason == "go package")
        .unwrap();
    assert_eq!(m.exclude, ["color_string.go"]);
}

/// The end-to-end fixtures' tasks, as discovered (the graph-level checks are in
/// the CLI's `codegen_e2e` test).
#[test]
fn e2e_tasks() {
    let mut out = String::new();
    for name in ["e2e-js", "e2e-go", "e2e-python"] {
        let root = fixtures().join(name);
        let ws = Workspace::discover(&root);
        let set = discover(&ws, &Options::default());
        // Only script and directive tasks: other task sources are not under test here.
        for t in set
            .tasks
            .iter()
            .filter(|t| matches!(t.id.kind(), "task" | "gogenerate"))
        {
            let mut v = serde_json::json!({
                "commands": t.commands.iter().map(|c| format!("{} [{}] {:?}", c.text, c.recognizer.unwrap_or("-"), c.recognition)).collect::<Vec<_>>(),
                "inputs": t.spec.inputs,
                "outputs": t.spec.outputs,
                "uncertainties": t.spec.uncertainties,
            });
            if !t.spec.output_packages.is_empty() {
                v["output_packages"] = serde_json::json!(t.spec.output_packages);
            }
            writeln!(
                out,
                "## {name}: {}\n{}\n",
                t.id,
                rel(serde_json::to_string_pretty(&v).unwrap(), &root)
            )
            .unwrap();
        }
    }
    insta::assert_snapshot!("e2e_tasks", out);
}
