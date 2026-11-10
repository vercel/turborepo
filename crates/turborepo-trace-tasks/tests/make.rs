//! Makefile task source fixtures (`fixtures/tasks/make/<case>`): every `make:`
//! task (and every task whose commands the make recognizers handled) with
//! resolved deps, expanded recipe lines, recognition and spec. Paths are shown
//! relative to the fixture root. Also a no-panic test of the parser over random
//! input.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fmt::Write,
    path::{Path, PathBuf},
};

use turborepo_trace_tasks::{
    Options, TaskSet, discover,
    sources::{
        Workspace,
        make::{self, parse},
    },
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/make")
        .join(name)
        .canonicalize()
        .expect("fixture dir")
}

fn rel(s: String, root: &Path) -> String {
    s.replace(&format!("{}/", root.display()), "")
        .replace(&root.display().to_string(), ".")
}

fn render(set: &TaskSet, root: &Path) -> String {
    let mut out = String::new();
    for t in &set.tasks {
        let ours = matches!(t.id.kind(), "make" | "docker")
            || t.commands
                .iter()
                .any(|c| matches!(c.recognizer, Some("make" | "cc" | "ar" | "docker")));
        if !ours {
            continue;
        }
        let v = serde_json::json!({
            "sources": t.sources,
            "deps": t.deps,
            "lines": t.lines,
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

fn run(name: &str) -> (TaskSet, String) {
    let root = fixture(name);
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    let s = render(&set, &root);
    (set, s)
}

fn deps(set: &TaskSet, id: &str) -> Vec<String> {
    set.get(&id.parse().unwrap())
        .unwrap_or_else(|| panic!("missing task {id}"))
        .deps
        .iter()
        .map(|d| d.to_string())
        .collect()
}

#[test]
fn go_project() {
    let (set, s) = run("go-project");
    insta::assert_snapshot!("go_project", s);
    // Phony aggregation and the default goal's chain.
    assert_eq!(
        deps(&set, "make:.:all"),
        ["make:.:build", "make:.:generate", "make:.:test"]
    );
    assert_eq!(deps(&set, "make:.:build"), ["make:.:bin/server"]);
    assert_eq!(deps(&set, "make:.:test"), ["make:.:generate"]);
    // Recipe commands linked to other task sources.
    assert_eq!(deps(&set, "make:.:assets"), ["task:.#build"]);
    assert!(deps(&set, "make:.:image").contains(&"docker:Dockerfile".to_string()));
    let bin = set.get(&"make:.:bin/server".parse().unwrap()).unwrap();
    // `$(wildcard)` became memberships, and the version from `$(shell)` did not
    // make the build uncertain.
    assert!(bin.spec.memberships.iter().any(|m| m.pattern == "*/*.go"));
    assert!(
        bin.spec.uncertainties.is_empty(),
        "{:?}",
        bin.spec.uncertainties
    );
    let gen_ = set
        .get(&"make:.:internal/api/gen.go".parse().unwrap())
        .unwrap();
    assert!(
        gen_.spec
            .outputs
            .iter()
            .any(|o| o.path.ends_with("internal/api/gen.go"))
    );
    // The lint flags depend on an unknown conditional.
    let lint = set.get(&"make:.:lint".parse().unwrap()).unwrap();
    assert!(!lint.spec.uncertainties.is_empty());
}

#[test]
fn c_project() {
    let (set, s) = run("c-project");
    insta::assert_snapshot!("c_project", s);
    // Pattern rule applied to the objects `$(wildcard)` + `patsubst` produced.
    assert_eq!(
        deps(&set, "make:.:app"),
        ["make:.:build/main.o", "make:.:build/util.o"]
    );
    assert_eq!(deps(&set, "make:.:build/util.o"), ["make:.:build"]);
    let util = set.get(&"make:.:build/util.o".parse().unwrap()).unwrap();
    let ins: Vec<String> = util
        .spec
        .inputs
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    // The #include closure through -Iinclude and the source directory.
    for h in ["util.c", "util.h", "config.h", "local.h"] {
        assert!(ins.contains(&h.to_string()), "{h}: {ins:?}");
    }
    // Old-style suffix rule.
    assert_eq!(deps(&set, "make:.:docs"), ["make:.:notes.html"]);
}

#[test]
fn recursive() {
    let (set, s) = run("recursive");
    insta::assert_snapshot!("recursive", s);
    assert_eq!(deps(&set, "make:.:lib"), ["make:lib:all"]);
    assert_eq!(deps(&set, "make:.:app"), ["make:.:lib", "make:app:all"]);
    assert_eq!(deps(&set, "make:.:test"), ["make:.:all", "make:app:test"]);
    let lib = set.get(&"make:lib:liblib.a".parse().unwrap()).unwrap();
    assert!(lib.files.iter().any(|f| f.ends_with("mk/common.mk")));
    assert!(lib.spec.configs.iter().any(|f| f.ends_with("lib/local.mk")));
}

/// Deterministic xorshift.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const MAKE_TOKENS: &[&str] = &[
    "$(",
    "${",
    ")",
    "}",
    ":",
    "::",
    "=",
    ":=",
    "?=",
    "+=",
    "!=",
    "\t",
    "\n",
    "\\\n",
    "%",
    "|",
    ";",
    "#",
    "$$",
    "$@",
    "$<",
    "$^",
    "$*",
    "$(@D)",
    "ifeq",
    "ifneq",
    "ifdef",
    "else",
    "endif",
    "define",
    "endef",
    "include",
    "-include",
    "export",
    "override",
    ".PHONY",
    "$(wildcard *.c)",
    "$(patsubst %.c,%.o,$(S))",
    "$(foreach x,a b,$(x))",
    "$(call f,1,2)",
    "$(eval x)",
    "$(shell ls)",
    "$(S:.c=.o)",
    "(a,b)",
    " ",
    "a",
    "b.c",
    "src/",
    ".c.o",
    "%.o: %.c",
    "$(MAKE) -C d",
];

#[test]
fn parser_never_panics() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let path = Path::new("/nonexistent-fuzz/Makefile");
    for round in 0..3000 {
        let mut s = String::new();
        let n = rng.below(60);
        for _ in 0..n {
            if round % 3 == 0 {
                // Raw bytes, lossily decoded.
                let b: Vec<u8> = (0..rng.below(8)).map(|_| rng.next() as u8).collect();
                s.push_str(&String::from_utf8_lossy(&b));
            } else {
                s.push_str(MAKE_TOKENS[rng.below(MAKE_TOKENS.len())]);
            }
        }
        let db = parse::parse_str(path, &s);
        let a = make::resolve(db);
        for t in a.targets.values() {
            let _ = a.recipe_lines(t);
        }
        let _ = a.db.default_goal();
        let _ = a.exported_env();
    }
}
