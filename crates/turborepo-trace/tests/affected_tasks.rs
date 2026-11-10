//! `turbotrace affected` and build tasks end to end: which tests a change
//! selects through the tasks that produce files the tests load (generated-file
//! linking).
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn copy_dir(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dst).unwrap();
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn turbotrace(root: &Path, args: &[&str]) -> String {
    // These tests are about how open uncertainty travels; their fixtures use a read
    // with a computed path as the uncertain site. Such reads are bounded by
    // names by default (tests/affected_names.rs), so that layer is off here.
    let o = Command::new(support::harness())
        .args(args)
        .env("TURBO_TRACE_DEBUG_NAMES", "0")
        .current_dir(root)
        .output()
        .expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

/// The jest tests `changed` selects.
fn affected(root: &Path, changed: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest"];
    for c in changed {
        args.push("--changed");
        args.push(c);
    }
    turbotrace(root, &args).lines().map(str::to_owned).collect()
}

/// A pnpm-style workspace link: `<from>/node_modules/<name>` → `<to>`.
#[cfg(unix)]
fn link(root: &Path, from: &str, name: &str, to: &str) {
    let l = root.join(from).join("node_modules").join(name);
    std::fs::create_dir_all(l.parent().unwrap()).unwrap();
    turborepo_trace_core::os::symlink(root.join(to), l).unwrap();
}

/// Fixture `fixtures/tasks/tsc-solution/` as an installed, unbuilt checkout
/// (workspace links, no `dist/`): what CI history replays and fresh clones look
/// like.
#[cfg(unix)]
fn tsc_solution() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/tasks/tsc-solution"),
        r,
    );
    link(r, "apps/web", "@acme/icons", "packages/icons");
    link(r, "apps/web", "@acme/patched", "packages/patched");
    link(r, "apps/web", "@acme/ui", "packages/ui");
    link(r, "apps/docs", "@acme/ui", "packages/ui");
    link(r, "packages/ui", "@acme/icons", "packages/icons");
    d
}

const ICON_BUTTON: &str = "apps/web/lib/icon-button.test.ts";
const MATH: &str = "apps/web/lib/math.test.ts";
const VERSION: &str = "apps/web/lib/version.test.ts";
const CARD: &str = "apps/docs/lib/card.test.ts";
const SLUG: &str = "apps/docs/lib/slug.test.ts";

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        // Independent of the developer's signing setup.
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// The jest tests selected for the working tree against `HEAD`.
fn since_head(root: &Path) -> Vec<String> {
    turbotrace(root, &["affected", "--tests", "jest", "--since", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn edit(root: &Path, rel: &str, from: &str, to: &str) {
    let p = root.join(rel);
    let s = std::fs::read_to_string(&p).unwrap();
    assert!(s.contains(from), "{rel} has no {from:?}");
    std::fs::write(p, s.replace(from, to)).unwrap();
}

/// Regression for build-task hubs of a large monorepo. Every `tsc -b` whose
/// reference closure contains a package "produced" that package's `dist/`: the
/// root solution build (every source of the repository) and every app's
/// `type-check`. So an edit of any page of one app selected every test that
/// loads a built workspace package, in any app, and every test of an app whose
/// jest config resolves one (`require.resolve('@acme/ui')` in a module mapper).
/// The built files of a project now depend on that project only.
#[cfg(unix)]
#[test]
fn tsc_build_outputs_link_to_their_project() {
    let d = tsc_solution();
    let r = d.path();
    // Unrelated edits select nothing: a page of an app, a README. (Both reached
    // every consumer of a `dist/` through `task:.#build:ts-debug`.)
    assert!(affected(r, &["apps/web/app/page.tsx"]).is_empty());
    assert!(affected(r, &["README.md"]).is_empty());
    // A source of one app selects its own test only (`apps/docs#type-check`
    // references `packages/ui`, which `apps/web` tests don't even load).
    assert_eq!(affected(r, &["apps/docs/lib/slug.ts"]), [SLUG]);
    assert_eq!(affected(r, &["apps/web/lib/math.ts"]), [MATH]);

    // Soundness: the consumers of a built package still depend on what produces it.
    // `icons` is loaded by `icon-button` directly, by `card` through the built
    // `ui`, and by every test of `docs` through its jest config; the `patched`
    // project references it.
    assert_eq!(
        affected(r, &["packages/icons/src/paths.ts"]),
        [CARD, SLUG, ICON_BUTTON, VERSION]
    );
    assert_eq!(affected(r, &["packages/ui/src/index.ts"]), [CARD, SLUG]);
    // The project's configuration, and the shared base it extends: an edit of an
    // option selects the consumers of what the project builds. That is decided
    // against the base revision; named as changed without one, nothing says
    // which files the project included before, and every file that consulted it
    // as a referenced project counts (`crates/turborepo-trace/src/tsgovern.rs`).
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    let all = [CARD, SLUG, ICON_BUTTON, MATH, VERSION];
    for (config, from, to, consumers) in [
        (
            "packages/icons/tsconfig.json",
            "\"rootDir\": \"src\"",
            "\"rootDir\": \"src\", \"strict\": true",
            &[CARD, SLUG, ICON_BUTTON, VERSION][..],
        ),
        (
            "tooling/tsconfig/built.json",
            "\"declaration\": true,",
            "\"declaration\": true, \"strict\": true,",
            &[CARD, SLUG, ICON_BUTTON, VERSION][..],
        ),
        (
            "tooling/tsconfig/bundler.json",
            "\"declaration\": true,",
            "\"declaration\": true, \"strict\": true,",
            &[CARD, SLUG][..],
        ),
    ] {
        edit(r, config, from, to);
        assert_eq!(since_head(r), consumers, "{config}");
        edit(r, config, to, from);
        assert_eq!(affected(r, &[config]), all, "{config} without a base");
    }
    // A new source file of the project (the include listing).
    std::fs::write(
        r.join("packages/icons/src/extra.ts"),
        "export const x = 1;\n",
    )
    .unwrap();
    assert_eq!(
        affected(r, &["packages/icons/src/extra.ts"]),
        [CARD, SLUG, ICON_BUTTON, VERSION]
    );
    // `tsc -b && node scripts/patch-dist.js` in `packages/patched`: the script
    // rewrites the files emitted for its own project, so their importers depend
    // on it (those outputs stay outputs of the whole task). The `icons` project
    // it references is not its to rewrite: importers of `icons/dist` don't
    // depend on the script.
    assert_eq!(
        affected(r, &["packages/patched/scripts/patch-dist.js"]),
        [VERSION]
    );
    assert_eq!(affected(r, &["packages/patched/src/index.ts"]), [VERSION]);

    // The project node: the importer depends on it, not on the solution task.
    let deps = turbotrace(r, &["deps", "apps/web/lib/icon-button.ts"]);
    assert!(
        deps.lines()
            .any(|l| l == "tsproject:packages/icons/tsconfig.json"),
        "{deps}"
    );
    assert!(
        !deps.lines().any(|l| l == "task:.#build:ts-debug"),
        "{deps}"
    );
    assert!(!deps.lines().any(|l| l.starts_with("apps/docs/")), "{deps}");
    // The solution task still depends on everything (a valid `--tasks` entry).
    let tasks = turbotrace(
        r,
        &[
            "affected",
            "--tasks",
            "task:.#build:ts-debug",
            "--changed",
            "apps/web/app/page.tsx",
        ],
    );
    assert_eq!(tasks.trim(), "task:.#build:ts-debug");
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// What a task only reads does not floor the consumers of its outputs; what it
/// may run or compile does. The same package, two commands for the task that
/// declares `src/gen/**`:
///
/// - `frobnicate`, an unknown command (the audit's N3 fixture). Its package
///   directory is the conservative fallback for whatever it does, and that
///   includes running the script (a wrapper around a first-party entry, a tool
///   that evaluates a config module) and compiling, bundling or copying it into
///   `src/gen/`, which the consumer then runs. The consumer stays on the floor
///   until a recognizer says what the command does.
/// - `tsc -p tsconfig.json` over a `noEmit` project: the script with a computed
///   read is type-checked, not run, and no output is code made from it. The
///   consumer of the task's output is not on the floor.
///
/// Where the consumer runs copies of the members by construction (a `tsc`
/// project that emits JavaScript), their uncertainty is the consumer's in both
/// cases. Changes of a member propagate in every case.
#[test]
fn only_what_a_task_may_run_or_compile_floors_its_consumers() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["pkg","lib","app"]}"#,
    );
    write(
        r,
        "turbo.json",
        r#"{"tasks":{"codegen":{"outputs":["src/gen/**"]},"build":{"outputs":["dist/**"]}}}"#,
    );
    // An unknown command: every file of the package is its input.
    write(
        r,
        "pkg/package.json",
        r#"{"name":"pkg","version":"1.0.0","scripts":{"codegen":"frobnicate"}}"#,
    );
    write(
        r,
        "pkg/scripts/other.js",
        "const fs = require('fs');\nmodule.exports = function load(p) {\n  return \
         fs.readFileSync(p, 'utf8');\n};\n",
    );
    // A project that emits JavaScript from a source with a computed load.
    write(
        r,
        "lib/package.json",
        r#"{"name":"lib","version":"1.0.0","main":"dist/index.js","scripts":{"build":"tsc -p tsconfig.json"}}"#,
    );
    write(
        r,
        "lib/tsconfig.json",
        r#"{"compilerOptions":{"outDir":"dist","rootDir":"src"},"include":["src"]}"#,
    );
    write(
        r,
        "lib/src/index.ts",
        "export const load = (name: string) => require(name);\n",
    );
    write(r, "app/package.json", r#"{"name":"app","version":"1.0.0"}"#);
    write(r, "app/jest.config.js", "module.exports = {};\n");
    write(
        r,
        "app/gen.test.js",
        "const types = require('../pkg/src/gen/types.js');\ntest('t', () => \
         expect(types).toBeDefined());\n",
    );
    write(
        r,
        "app/lib.test.js",
        "const { load } = require('../lib/dist/index.js');\ntest('l', () => \
         expect(load).toBeDefined());\n",
    );
    write(r, "README.md", "x\n");
    // A null change with the unknown command: both consumers are on the floor.
    assert_eq!(
        affected(r, &["README.md"]),
        ["app/gen.test.js", "app/lib.test.js"]
    );
    // A command that only reads the script: the consumer of the generated file is
    // not floored by it; the consumer of the emitted JavaScript still is by its
    // source.
    write(
        r,
        "pkg/package.json",
        r#"{"name":"pkg","version":"1.0.0","scripts":{"codegen":"tsc -p tsconfig.json"}}"#,
    );
    write(
        r,
        "pkg/tsconfig.json",
        r#"{"compilerOptions":{"noEmit":true,"allowJs":true},"include":["scripts"]}"#,
    );
    assert_eq!(affected(r, &["README.md"]), ["app/lib.test.js"]);
    // Changes still propagate through the hub.
    assert_eq!(
        affected(r, &["pkg/scripts/other.js"]),
        ["app/gen.test.js", "app/lib.test.js"]
    );
}

/// Fixture `fixtures/tasks/data-inputs/` as an installed, unbuilt checkout.
#[cfg(unix)]
fn data_inputs() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/tasks/data-inputs"),
        r,
    );
    for p in ["analytics", "assets", "gen", "lib", "tool", "typed"] {
        link(
            r,
            "apps/web",
            &format!("@acme/{p}"),
            &format!("packages/{p}"),
        );
    }
    d
}

/// The files a task only reads don't lend their uncertainty to the consumers of
/// its outputs, whether they reach the task through a membership or as a named
/// input file.
///
/// Seen in a large monorepo: a package's tsconfig lists its build script next
/// to `src` (`"include": ["src", "build.ts"]`), and the script loads a module
/// with computed file reads. Every `tsc -b` type-check that reaches the project
/// took the script as a plain input file, so the tests that import anything
/// those type-checks declare were selected on every change. The project only
/// emits declarations: `tsc` reads the script, nothing runs it and no output is
/// code made from it. Likewise for the files turbo hashes (`inputs: ["
/// $TURBO_DEFAULT$", "$TURBO_ROOT$/tsconfig.json"]`).
///
/// What keeps flowing: a script the task runs (`gen`), sources a compiler emits
/// as JavaScript that the consumer then runs (`lib`), and the package of an
/// unknown command (`tool`).
#[cfg(unix)]
#[test]
fn files_a_task_only_reads_do_not_lend_their_uncertainty() {
    const ICON: &str = "apps/web/lib/icon.test.ts";
    const GEN: &str = "apps/web/lib/gen.test.ts";
    const LIB: &str = "apps/web/lib/lib.test.ts";
    const TOOL: &str = "apps/web/lib/tool.test.ts";
    const TYPED: &str = "apps/web/lib/typed.test.ts";
    let d = data_inputs();
    let r = d.path();
    let selected = |changed: &str| affected(r, &[changed]);

    // A change that touches nothing selects the tests on the floor:
    // - `gen`: its `build` task runs a script with a computed read (the output
    //   depends on files the graph does not know);
    // - `lib`: `tsc` emits JavaScript, so the test runs a copy of `src/index.ts`,
    //   which loads a module it computes;
    // - `tool`: the same source behind an unknown compiler (`frobnicate src --out
    //   dist`). Its package is all the graph knows about the command, and the test
    //   runs whatever it makes of `src/index.ts`;
    // - `typed`: `pnpm typegen && tsc -b`. The package's own `typegen` runs a
    //   script with computed reads and may generate the sources `tsc` compiles.
    // Not `icon`: the three uncertain files of `packages/assets` (a script and a
    // test file inside the turbo inputs and the type-checked project, and the
    // file the project's `include` names) are only hashed and type-checked, and
    // the `build` task compiles none of them.
    // Not `analytics` either: its `tsc --build` depends on the upstream build of
    // `packages/gen` (turbo `^build`), which runs the uncertain script, but a task
    // of another package doesn't write into this one, and the sources import
    // nothing of it. The importer is linked to the project, not to the task and
    // its dependency chain.
    assert_eq!(selected("README.md"), [GEN, LIB, TOOL, TYPED]);

    // Changes still propagate along data edges. The importer looks for the
    // package's declarations too (`exports.types`: `dist/index.d.ts`), which
    // the type-checked project emits: an edit of a file the project reads
    // selects it, like an edit of a source the `build` compiles.
    for file in [
        "packages/assets/src/names.ts",
        "packages/assets/src/names.test.ts",
        "packages/assets/tools/upload.ts",
    ] {
        assert_eq!(selected(file), [GEN, ICON, LIB, TOOL, TYPED], "{file}");
    }
    // The script is only one of the `type-check` task's turbo inputs. The task
    // writes what its project writes (its declared `dist` is a cache list), so
    // the script is nothing to the importers of the package. The task itself
    // still depends on it.
    let script = "packages/assets/scripts/visual/extract.mjs";
    assert_eq!(selected(script), [GEN, LIB, TOOL, TYPED]);
    let tasks = turbotrace(
        r,
        &[
            "affected",
            "--tasks",
            "task:packages/assets#type-check",
            "--changed",
            script,
        ],
    );
    assert_eq!(tasks.trim(), "task:packages/assets#type-check");
    // What a project emits still depends on its sources and its configuration.
    const ANALYTICS: &str = "apps/web/lib/analytics.test.ts";
    for file in [
        "packages/analytics/src/index.ts",
        "packages/analytics/tsconfig.json",
    ] {
        assert_eq!(selected(file), [ANALYTICS, GEN, LIB, TOOL, TYPED], "{file}");
    }
}

/// A committed file that a task writes is just a file when nothing in the
/// repository runs the task (and that automation can be read: a workflow runs
/// the tests): the tests read the committed bytes. (Seen in a large monorepo: a
/// generator that fetches content types from a CMS and renders 482 checked-in
/// source files is run by hand; its renderer has computed reads, and every test
/// that imports one of those files was selected on every change.) The link to
/// the producer stays whenever something regenerates the file (a test script,
/// an install script or a CI job depends on the generator or runs it, directly
/// or through other tasks), and for a file that is not in the checkout.
#[test]
fn committed_outputs_of_a_task_nothing_runs_are_just_files() {
    const TEST: &str = "apps/web/lib/entry.test.ts";
    const GENERATOR: &str = "task:packages/content#generate-types";
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*","apps/*"]}"#,
    );
    write(
        r,
        "turbo.json",
        r#"{"tasks":{"generate-types":{"outputs":["src/types/**"]},"test":{}}}"#,
    );
    write(
        r,
        "packages/content/package.json",
        r#"{"name":"@acme/content","version":"1.0.0","scripts":{"generate-types":"node scripts/generate/index.js"}}"#,
    );
    // The generator loads a renderer it computes: open uncertainty for whoever
    // depends on what it writes.
    write(
        r,
        "packages/content/scripts/generate/index.js",
        "const fs = require('node:fs');\nconst { render } = \
         require(process.env.CONTENT_RENDERER);\nfs.writeFileSync('src/types/entry.ts', \
         render());\n",
    );
    // Its output, checked in.
    let output = "packages/content/src/types/entry.ts";
    write(r, output, "export const entryTypes = ['post'];\n");
    let web_manifest =
        |test: &str| format!(r#"{{"name":"web","private":true,"scripts":{{"test":"{test}"}}}}"#);
    write(r, "apps/web/package.json", &web_manifest("jest"));
    write(r, "apps/web/jest.config.js", "module.exports = {};\n");
    write(
        r,
        TEST,
        "import { entryTypes } from '../../../packages/content/src/types/entry';\ntest('e', () => \
         expect(entryTypes).toContain('post'));\n",
    );
    write(r, "README.md", "x\n");
    let none: [&str; 0] = [];
    let workflow = |steps: &[&str]| {
        let steps: String = steps
            .iter()
            .map(|s| format!("      - run: {s}\n"))
            .collect();
        write(
            r,
            ".github/workflows/ci.yml",
            &format!("on: push\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n{steps}"),
        );
    };

    // A workflow runs the tests, and nothing runs the generator: neither its
    // uncertainty nor an edit of its script reaches the test. An edit of the
    // committed file does.
    workflow(&["pnpm --filter web test"]);
    assert_eq!(affected(r, &["README.md"]), none);
    assert_eq!(
        affected(r, &["packages/content/scripts/generate/index.js"]),
        none
    );
    assert_eq!(affected(r, &[output]), [TEST]);
    let deps = turbotrace(r, &["deps", TEST]);
    assert!(!deps.lines().any(|l| l == GENERATOR), "{deps}");
    // The task itself still depends on its script.
    let tasks = turbotrace(
        r,
        &[
            "affected",
            "--tasks",
            GENERATOR,
            "--changed",
            "packages/content/scripts/generate/index.js",
        ],
    );
    assert_eq!(tasks.trim(), GENERATOR);

    // Something runs it (the test script regenerates before it starts jest): the
    // test depends on the generator again, uncertainty included.
    write(
        r,
        "apps/web/package.json",
        &web_manifest("pnpm --filter @acme/content generate-types && jest"),
    );
    assert_eq!(affected(r, &["README.md"]), [TEST]);
    let deps = turbotrace(r, &["deps", TEST]);
    assert!(deps.lines().any(|l| l == GENERATOR), "{deps}");
    write(r, "apps/web/package.json", &web_manifest("jest"));
    assert_eq!(affected(r, &["README.md"]), none);

    // A script that could run it but that nothing runs itself (a launcher run by
    // hand) changes nothing.
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*","apps/*"],"scripts":{"regen":"turbo run generate-types"}}"#,
    );
    assert_eq!(affected(r, &["README.md"]), none);

    // A workflow step runs it.
    workflow(&[
        "pnpm --filter @acme/content generate-types",
        "pnpm --filter web test",
    ]);
    assert_eq!(affected(r, &["README.md"]), [TEST]);
    workflow(&["pnpm --filter web test"]);
    assert_eq!(affected(r, &["README.md"]), none);

    // Not in the checkout (a gitignored output): somebody has to run the generator
    // for the file to exist, whatever the repository's automation says.
    std::fs::remove_file(r.join(output)).unwrap();
    assert_eq!(affected(r, &["README.md"]), [TEST]);

    // No CI that turbotrace reads (a fresh graph of the same repository without the
    // workflow): where the tests run and what runs before them is not known, so the
    // committed file stays linked to its generator.
    write(r, output, "export const entryTypes = ['post'];\n");
    std::fs::remove_dir_all(r.join(".github")).unwrap();
    std::fs::remove_dir_all(r.join(".turbo/trace")).unwrap();
    assert_eq!(affected(r, &["README.md"]), [TEST]);
}
