//! Package binaries (`recognizers::bins`): workspaces under
//! `fixtures/tasks/bins/` whose scripts run internal, wrapper, third-party and
//! Python console-script binaries, with (`linked`) and without (`unlinked`)
//! `node_modules`. Paths are shown relative to the fixture root.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fmt::Write,
    path::{Path, PathBuf},
};

use turborepo_trace_tasks::{
    Options, Task, TaskSet, discover, registry::Recognition, sources::Workspace, spec::EntryLang,
};

fn root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/bins")
        .join(name)
        .canonicalize()
        .expect("fixture dir")
}

fn rel(s: String, root: &Path) -> String {
    s.replace(&format!("{}/", root.display()), "")
        .replace(&root.display().to_string(), ".")
}

/// Package-script tasks only: other task sources (Python packaging, go
/// generate, ...) may find tasks in the same fixtures, which are not what this
/// test is about.
fn dump(root: &Path, set: &TaskSet) -> String {
    let mut out = String::new();
    for t in set.tasks.iter().filter(|t| t.id.kind() == "task") {
        let v = serde_json::json!({
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

fn get<'a>(set: &'a TaskSet, id: &str) -> &'a Task {
    set.get(&id.parse().unwrap())
        .unwrap_or_else(|| panic!("{id}"))
}

fn has_entry(t: &Task, root: &Path, p: &str) -> bool {
    t.spec.entries.iter().any(|e| e.path == root.join(p))
}

fn has_membership_in(t: &Task, root: &Path, dir: &str) -> bool {
    t.spec.memberships.iter().any(|m| m.dir == root.join(dir))
}

#[test]
fn linked() {
    let root = root("linked");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    insta::assert_snapshot!("linked", dump(&root, &set));

    // An internal bin through a pnpm shim: its source is an entry, the schema an
    // input, the redirect an output; the bin's package is not a membership.
    let t = get(&set, "task:packages/app#codegen");
    assert_eq!(t.commands[0].recognizer, Some("bin"));
    assert_eq!(t.commands[0].recognition, Recognition::Full);
    assert!(has_entry(t, &root, "tools/gen/bin/gen-client.js"));
    assert!(
        t.spec
            .inputs
            .contains(&root.join("packages/app/schema.json"))
    );
    assert!(!has_membership_in(t, &root, "tools/gen"));
    assert!(!has_membership_in(t, &root, "packages/app"));
    // The same through `pnpm exec`, `npx`, `yarn` and an explicit
    // `node_modules/.bin` path.
    for s in [
        "codegen:pnpm",
        "codegen:npx",
        "codegen:yarn",
        "codegen:explicit",
    ] {
        let t = get(&set, &format!("task:packages/app#{s}"));
        assert_eq!(t.commands[0].recognizer, Some("bin"), "{s}");
        assert!(has_entry(t, &root, "tools/gen/bin/gen-client.js"), "{s}");
    }
    // A wrapper (symlinked `.bin`, weak evidence): jest with the config from the
    // argv.
    let t = get(&set, "task:packages/app#test");
    assert_eq!(t.commands[0].recognizer, Some("bin-wrapper"));
    assert_eq!(t.commands[0].recognition, Recognition::Partial);
    assert!(
        t.spec
            .configs
            .contains(&root.join("packages/app/jest.unit.config.js"))
    );
    assert!(has_entry(t, &root, "tools/test-dd/bin/jest-dd.js"));
    assert!(t.spec.toolchain.iter().any(|tc| tc.name == "jest"));
    // Only through the workspace `bin` field (no `.bin` entry): vitest; strong
    // evidence: tsc.
    let t = get(&set, "task:packages/app#test:vitest");
    assert!(t.spec.toolchain.iter().any(|tc| tc.name == "vitest"));
    assert!(
        t.spec
            .configs
            .contains(&root.join("packages/app/vitest.config.ts"))
    );
    let t = get(&set, "task:packages/app#build");
    assert_eq!(t.commands[0].recognizer, Some("bin-wrapper"));
    assert!(t.spec.toolchain.iter().any(|tc| tc.name == "typescript"));
    // Shebang-detected Python bin.
    let t = get(&set, "task:packages/app#py");
    assert!(
        t.spec
            .entries
            .iter()
            .any(|e| e.path == root.join("tools/gen/bin/gen-py") && e.lang == EntryLang::Python)
    );
    // Third-party bins: a toolchain package, conservatively partial.
    let t = get(&set, "task:packages/app#tp");
    assert_eq!(t.commands[0].recognizer, Some("bin-package"));
    assert!(t.spec.toolchain.iter().any(|tc| tc.name == "tp-tool"));
    let t = get(&set, "task:packages/app#od");
    assert!(t.spec.toolchain.iter().any(|tc| tc.name == "other-dep"));
    // A bin that reads relative to the cwd covers the cwd.
    let t = get(&set, "task:packages/app#cwd");
    assert_eq!(t.commands[0].recognition, Recognition::Partial);
    assert!(has_membership_in(t, &root, "packages/app"));
    // A stale `.bin` shim (target gone) falls back to the workspace `bin` field.
    let t = get(&set, "task:packages/app#test:vitest");
    assert_eq!(t.commands[0].recognizer, Some("bin-wrapper"));
    assert!(has_entry(t, &root, "tools/test-dd/bin/vitest-dd.js"));
    // A known tool package under another bin name: forwarded to the tool's
    // recognizer.
    let t = get(&set, "task:packages/app#typecheck");
    assert_eq!(t.commands[0].recognizer, Some("bin-package"));
    assert_eq!(t.commands[0].recognition, Recognition::Full);
    assert!(
        t.spec
            .configs
            .contains(&root.join("packages/app/tsconfig.build.json"))
    );
    assert!(
        t.spec
            .toolchain
            .iter()
            .any(|tc| tc.name == "@typescript/typescript6")
    );
    // Unknown stays unknown.
    let t = get(&set, "task:packages/app#unknown");
    assert_eq!(t.commands[0].recognition, Recognition::Unknown);
    // go tool: the tool declared in go.mod.
    let t = get(&set, "task:packages/gosvc#lint");
    assert_eq!(t.commands[0].recognizer, Some("go-tool"));
    assert!(t.spec.toolchain.iter().any(|tc| tc.ecosystem == "go"
        && tc.name == "github.com/golangci/golangci-lint/v2/cmd/golangci-lint"));
}

#[test]
fn unlinked() {
    let root = root("unlinked");
    assert!(!root.join("node_modules").exists());
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    insta::assert_snapshot!("unlinked", dump(&root, &set));

    // Workspace `bin` (string form) without node_modules.
    let t = get(&set, "task:packages/web#codegen");
    assert_eq!(t.commands[0].recognizer, Some("bin"));
    assert_eq!(t.commands[0].recognition, Recognition::Full);
    assert!(has_entry(t, &root, "tools/gen/bin/gen.js"));
    // Strong evidence (`require('jest').run`) with the config from the argv.
    let t = get(&set, "task:packages/web#test");
    assert_eq!(t.commands[0].recognizer, Some("bin-wrapper"));
    assert!(
        t.spec
            .configs
            .contains(&root.join("packages/web/jest.unit.config.js"))
    );
    // A bin whose target is a build output: probe plus the bin package.
    let t = get(&set, "task:packages/web#built");
    assert!(has_entry(t, &root, "tools/built/dist/cli.js"));
    assert!(has_membership_in(t, &root, "tools/built"));
    // A declared, uninstalled third-party dependency.
    let t = get(&set, "task:packages/web#tp");
    assert!(t.spec.toolchain.iter().any(|tc| tc.name == "tp-only"));
    // Python console scripts.
    let t = get(&set, "task:packages/web#py");
    assert_eq!(t.commands[0].recognizer, Some("console-script"));
    assert_eq!(t.commands[0].recognition, Recognition::Full);
    assert!(has_entry(t, &root, "packages/web/src/webgen/cli.py"));
    let t = get(&set, "task:packages/web#py:missing");
    assert_eq!(t.commands[0].recognition, Recognition::Partial);
    assert!(has_entry(t, &root, "packages/web/webgen/missing.py"));
}
