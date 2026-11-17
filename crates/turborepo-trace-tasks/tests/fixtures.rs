//! Recognizer fixtures (`fixtures/tasks/recognizers/<family>/commands.txt`: one
//! command per line → the recognized `TaskSpec`), and the turbo monorepo
//! fixture (`discover` over a whole workspace). Paths are shown relative to the
//! fixture root.

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
        .join("../turborepo-trace-testkit/fixtures/tasks")
        .canonicalize()
        .expect("fixtures dir")
}

fn rel(s: String, root: &Path) -> String {
    s.replace(&format!("{}/", root.display()), "")
        .replace(&root.display().to_string(), ".")
}

fn family(name: &str) -> String {
    let root = fixtures().join("recognizers").join(name);
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

family_test!(ts);
family_test!(bundlers);
family_test!(run);
family_test!(testrunners);
family_test!(fs);

#[test]
fn turbo_monorepo() {
    let root = fixtures().join("turbo-monorepo");
    let ws = Workspace::discover(&root);
    assert_eq!(
        ws.packages
            .iter()
            .map(|p| p.rel.as_str())
            .collect::<Vec<_>>(),
        [".", "apps/web", "packages/db", "packages/ui"]
    );
    let set = discover(&ws, &Options::default());
    let mut out = String::new();
    for t in &set.tasks {
        let v = serde_json::json!({
            "sources": t.sources,
            "deps": t.deps,
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
    insta::assert_snapshot!("turbo_monorepo", out);
    // Task deps resolve through turbo `^build`, `dependsOn`, pre hooks and `turbo
    // run`.
    let get = |id: &str| {
        set.get(&id.parse().unwrap())
            .unwrap_or_else(|| panic!("{id}"))
    };
    let web = get("task:apps/web#build");
    assert!(
        web.deps
            .iter()
            .any(|d| d.to_string() == "task:packages/ui#build"),
        "{:?}",
        web.deps
    );
    assert!(
        web.deps
            .iter()
            .any(|d| d.to_string() == "task:packages/db#build"),
        "{:?}",
        web.deps
    );
    let db = get("task:packages/db#build");
    assert!(
        db.deps
            .iter()
            .any(|d| d.to_string() == "task:packages/db#prebuild")
    );
    assert!(
        db.deps
            .iter()
            .any(|d| d.to_string() == "task:packages/db#codegen")
    );
    let root_build = get("task:.#build");
    assert_eq!(root_build.deps.len(), 3, "{:?}", root_build.deps);
    let closure: Vec<String> = set
        .closure(&web.id)
        .iter()
        .map(|t| t.id.to_string())
        .collect();
    assert!(
        closure.contains(&"task:packages/db#codegen".to_string()),
        "{closure:?}"
    );
    // The generated file links back to its producer.
    let generated = root.join("packages/db/src/generated/client.ts");
    let producers: Vec<String> = set
        .producers_of(&generated)
        .iter()
        .map(|t| t.id.to_string())
        .collect();
    assert_eq!(producers, ["task:packages/db#codegen"]);
}
