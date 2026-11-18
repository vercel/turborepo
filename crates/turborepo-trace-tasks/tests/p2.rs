//! P2 task sources: nx, justfile, Taskfile, tox/nox, cargo
//! aliases/xtask, GitHub Actions (with the paths-filter lint) and Python
//! packaging, each over a fixture in `fixtures/tasks/<source>/`. Paths are
//! shown relative to the fixture root.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fmt::Write,
    path::{Path, PathBuf},
};

use turborepo_trace_tasks::{
    Options, TaskSet, discover,
    sources::{Workspace, gha::lint::lint_paths},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks")
        .join(name)
        .canonicalize()
        .expect("fixture dir")
}

fn rel(s: String, root: &Path) -> String {
    s.replace(&format!("{}/", root.display()), "")
        .replace(&root.display().to_string(), ".")
}

/// Renders the tasks of `set` whose kind is in `kinds`.
fn render(set: &TaskSet, root: &Path, kinds: &[&str]) -> String {
    let mut out = String::new();
    for t in set.tasks.iter().filter(|t| kinds.contains(&t.id.kind())) {
        let v = serde_json::json!({
            "sources": t.sources,
            "files": t.files,
            "deps": t.deps,
            "commands": t.commands.iter().map(|c| format!("{} [{}] {:?}", c.text, c.recognizer.unwrap_or("-"), c.recognition)).collect::<Vec<_>>(),
            "spec": t.spec,
        });
        writeln!(
            out,
            "## {}\n{}\n",
            t.id,
            rel(serde_json::to_string_pretty(&v).unwrap(), root)
        )
        .unwrap();
    }
    out
}

fn ids(set: &TaskSet, kind: &str) -> Vec<String> {
    set.tasks
        .iter()
        .filter(|t| t.id.kind() == kind)
        .map(|t| t.id.to_string())
        .collect()
}

fn deps_of(set: &TaskSet, id: &str) -> Vec<String> {
    set.get(&id.parse().unwrap())
        .unwrap_or_else(|| panic!("no task {id}"))
        .deps
        .iter()
        .map(|d| d.to_string())
        .collect()
}

#[test]
fn nx_workspace() {
    let root = fixture("nx");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    insta::assert_snapshot!("nx", render(&set, &root, &["nx", "task"]));
    assert_eq!(
        ids(&set, "nx"),
        [
            "nx:legacy:build",
            "nx:ui:build",
            "nx:ui:test",
            "nx:web:build",
            "nx:web:deploy",
            "nx:web:e2e",
            "nx:web:lint",
            "nx:web:test",
        ]
    );
    // `^build` reaches the dependency's build; `dependsOn: ["build"]` the own
    // build.
    assert!(deps_of(&set, "nx:web:build").contains(&"nx:ui:build".to_string()));
    assert_eq!(deps_of(&set, "nx:web:deploy"), ["nx:web:build"]);
    // `nx:run-script` depends on the package script.
    assert_eq!(deps_of(&set, "nx:ui:test"), ["task:libs/ui#test"]);
    // The legacy lib (no package.json) depends on every project's build for
    // `^build`.
    assert!(deps_of(&set, "nx:legacy:build").contains(&"nx:ui:build".to_string()));
    // Recognized nx CLI calls in package scripts.
    assert!(deps_of(&set, "task:.#web").contains(&"nx:web:build".to_string()));
    let all = deps_of(&set, "task:.#build:all");
    assert!(all.contains(&"nx:ui:build".to_string()) && all.contains(&"nx:web:build".to_string()));
    // An unknown executor is uncertain.
    let e2e = set.get(&"nx:web:e2e".parse().unwrap()).unwrap();
    assert!(!e2e.spec.uncertainties.is_empty());
    // The vite config is an entry of the vite build; the ui build outputs to
    // dist/libs/ui.
    let web = set.get(&"nx:web:build".parse().unwrap()).unwrap();
    assert!(
        web.spec
            .configs
            .iter()
            .any(|c| c.ends_with("apps/web/vite.config.ts"))
    );
    let lib_out = root.join("dist/libs/ui/index.js");
    assert_eq!(
        set.producers_of(&lib_out)
            .iter()
            .map(|t| t.id.to_string())
            .collect::<Vec<_>>(),
        ["nx:ui:build"]
    );
}

#[test]
fn justfile() {
    let root = fixture("just");
    let ws = Workspace::from_roots(&root, Vec::<String>::new());
    let set = discover(&ws, &Options::default());
    insta::assert_snapshot!("just", render(&set, &root, &["just"]));
    assert!(ids(&set, "just").contains(&"just:.:docs::build".to_string()));
    assert!(ids(&set, "just").contains(&"just:.:lint".to_string()));
    assert_eq!(deps_of(&set, "just:.:build"), ["just:.:gen"]);
    assert_eq!(deps_of(&set, "just:.:b"), ["just:.:build"]);
    // A parameter makes the recipe uncertain; the release script is inlined (it
    // sources lib.sh and runs gen.js).
    let release = set.get(&"just:.:release".parse().unwrap()).unwrap();
    assert!(!release.spec.uncertainties.is_empty());
    assert!(
        release
            .spec
            .entries
            .iter()
            .any(|e| e.path.ends_with("scripts/lib.sh"))
    );
    let perl = set.get(&"just:.:perl-thing".parse().unwrap()).unwrap();
    assert!(
        perl.spec
            .uncertainties
            .iter()
            .any(|u| u.detail.contains("perl"))
    );
}

#[test]
fn taskfile() {
    let root = fixture("taskfile");
    let ws = Workspace::from_roots(&root, Vec::<String>::new());
    let set = discover(&ws, &Options::default());
    insta::assert_snapshot!("taskfile", render(&set, &root, &["taskfile"]));
    assert_eq!(
        deps_of(&set, "taskfile:.:build"),
        ["taskfile:.:gen", "taskfile:.:lib:compile"]
    );
    // `sources`/`generates` are declared inputs and outputs: the generated client
    // links back.
    let generator = set.get(&"taskfile:.:gen".parse().unwrap()).unwrap();
    assert!(
        generator
            .spec
            .memberships
            .iter()
            .any(|m| m.dir == root.join("schema")
                && m.pattern == "*.json"
                && m.exclude == ["draft.json"])
    );
    assert_eq!(
        set.producers_of(&root.join("src/generated/client.js"))
            .iter()
            .map(|t| t.id.to_string())
            .collect::<Vec<_>>(),
        ["taskfile:.:gen"]
    );
    // The included task runs in its `dir`.
    let compile = set.get(&"taskfile:.:lib:compile".parse().unwrap()).unwrap();
    assert!(
        compile
            .spec
            .configs
            .iter()
            .any(|c| c == &root.join("lib/tsconfig.json"))
    );
}

#[test]
fn tox_and_nox() {
    let root = fixture("tox");
    let ws = Workspace::from_roots(&root, Vec::<String>::new());
    let set = discover(&ws, &Options::default());
    insta::assert_snapshot!("tox", render(&set, &root, &["tox", "nox"]));
    assert_eq!(
        ids(&set, "tox"),
        ["tox:.:lint", "tox:.:py311", "tox:.:py312"]
    );
    assert_eq!(
        ids(&set, "nox"),
        ["nox:.:docs", "nox:.:lint", "nox:.:tests"]
    );
    assert_eq!(deps_of(&set, "nox:.:tests"), ["nox:.:lint"]);
    let py = set.get(&"tox:.:py311".parse().unwrap()).unwrap();
    assert!(
        py.spec
            .inputs
            .iter()
            .any(|i| i == &root.join("requirements-test.txt"))
    );
    let docs = set.get(&"nox:.:docs".parse().unwrap()).unwrap();
    assert!(!docs.spec.uncertainties.is_empty());
}

#[test]
fn cargo_aliases() {
    let root = fixture("cargo");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    insta::assert_snapshot!("cargo", render(&set, &root, &["cargo", "task"]));
    let x = set.get(&"cargo:.:xtask".parse().unwrap()).unwrap();
    assert!(
        x.spec
            .entries
            .iter()
            .any(|e| e.path == root.join("xtask/src/main.rs"))
    );
    assert!(x.spec.inputs.contains(&root.join("rust-toolchain.toml")));
    // `cargo xtask codegen` in a package script expands the alias.
    let cg = set.get(&"task:.#codegen".parse().unwrap()).unwrap();
    assert!(cg.deps.iter().any(|d| d.to_string() == "cargo:.:xtask"));
    assert!(
        cg.spec
            .entries
            .iter()
            .any(|e| e.path == root.join("xtask/src/main.rs"))
    );
    // An alias of an alias.
    assert!(deps_of(&set, "cargo:.:gen").contains(&"cargo:.:xtask".to_string()));
}

#[test]
fn github_actions() {
    let root = fixture("gha");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    insta::assert_snapshot!("gha", render(&set, &root, &["gha"]));
    let ci = ".github/workflows/ci.yml";
    assert_eq!(
        deps_of(&set, &format!("gha:{ci}#build")),
        ["task:.#build".to_string(), format!("gha:{ci}#setup")]
    );
    assert_eq!(
        deps_of(&set, &format!("gha:{ci}#reuse")),
        ["gha:.github/workflows/release.yml#publish"]
    );
    // The composite action's step and the JS action's entry.
    let setup = set
        .get(&format!("gha:{ci}#setup").parse().unwrap())
        .unwrap();
    assert!(
        setup
            .spec
            .entries
            .iter()
            .any(|e| e.path == root.join("tools/install.js"))
    );
    assert!(
        setup
            .spec
            .entries
            .iter()
            .any(|e| e.path == root.join(".github/actions/js-action/dist/index.js"))
    );
    assert!(setup.spec.inputs.contains(&root.join(".nvmrc")));
    // The script run by the test job is inlined: it sources lib.sh.
    let test = set.get(&format!("gha:{ci}#test").parse().unwrap()).unwrap();
    assert!(
        test.spec
            .entries
            .iter()
            .any(|e| e.path == root.join("tools/lib.sh"))
    );
    // On push (paths: src/**, package.json) the filter misses lib/ (tsc include,
    // via `turbo run build`) and tools/; on pull_request (paths-ignore:
    // docs/**) nothing.
    let lint = lint_paths(&set);
    let mut out = String::new();
    for f in &lint {
        writeln!(
            out,
            "{} on {}: paths {:?} paths-ignore {:?}",
            f.task, f.event, f.paths, f.paths_ignore
        )
        .unwrap();
        for m in &f.missing {
            writeln!(out, "  missing {m}").unwrap();
        }
    }
    insta::assert_snapshot!("gha_lint", out);
    let build = lint
        .iter()
        .find(|f| f.task.to_string() == format!("gha:{ci}#build") && f.event == "push")
        .expect("build finding");
    assert!(
        build.missing.iter().any(|m| m.starts_with("lib")),
        "{:?}",
        build.missing
    );
    assert!(
        build.missing.iter().any(|m| m == "tools/scripts/check.js"),
        "{:?}",
        build.missing
    );
    assert!(
        !build.missing.iter().any(|m| m.starts_with("src")),
        "{:?}",
        build.missing
    );
    assert!(!lint.iter().any(|f| f.event == "pull_request"), "{lint:?}");
}

#[test]
fn python_packaging() {
    let mut out = String::new();
    for p in ["hatch", "setuptools", "poetry"] {
        let root = fixture(&format!("pypackaging/{p}"));
        let ws = Workspace::from_roots(&root, Vec::<String>::new());
        let set = discover(&ws, &Options::default());
        writeln!(out, "# {p}\n{}", render(&set, &root, &["py-dist"])).unwrap();
        assert_eq!(ids(&set, "py-dist"), ["py-dist:."], "{p}");
    }
    insta::assert_snapshot!("pypackaging", out);
    let root = fixture("pypackaging/setuptools");
    let ws = Workspace::from_roots(&root, Vec::<String>::new());
    let set = discover(&ws, &Options::default());
    let t = set.get(&"py-dist:.".parse().unwrap()).unwrap();
    // package_data, data_files, py_modules, MANIFEST.in; a computed `package_data=`
    // argument (a computed `version=` does not change what ships).
    assert!(
        t.spec
            .memberships
            .iter()
            .any(|m| m.dir == root.join("src/stpkg/data") && m.pattern == "*.json")
    );
    assert!(t.spec.inputs.contains(&root.join("conf/default.cfg")));
    assert!(t.spec.inputs.contains(&root.join("src/standalone.py")));
    assert!(t.spec.memberships.iter().any(|m| m.dir == root.join("docs")
        && m.pattern == "**/*.rst"
        && m.exclude.iter().any(|e| e.starts_with("_build"))));
    assert!(
        t.spec
            .uncertainties
            .iter()
            .any(|u| u.detail.contains("package_data"))
    );
    assert!(
        !t.spec
            .uncertainties
            .iter()
            .any(|u| u.detail.contains("version"))
    );
}
