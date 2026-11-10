//! Test runners: the real config path from the command line goes to the
//! existing framework adapters ([`turborepo_trace_frameworks`]), whose inputs
//! become the task's inputs.
//!
//! | Program | Config | Adapter |
//! |---|---|---|
//! | `jest` | `-c/--config`, else `jest.config.*` / `package.json#jest` up from the cwd | [`turborepo_trace_frameworks::jest`] |
//! | `vitest` | `-c/--config`, `--root`, else `vitest.config.*` / `vite.config.*` | [`turborepo_trace_frameworks::vitest`] |
//! | `pytest`, `py.test` | `-c`, else the nearest `pytest.ini` / `pyproject.toml` / `tox.ini` / `setup.cfg` | [`turborepo_trace_frameworks::pytest`] |
//! | `playwright test` | `-c/--config`, else `playwright.config.*` | [`turborepo_trace_frameworks::playwright`] |
//! | `go test` | packages (`./...`), `testdata/**` | none (package directories) |
//! | `cargo test/build/check/clippy/run/bench/doc/nextest` | `--manifest-path`, else the workspace | none (workspace sources) |
//! | `mocha`, `ava`, `tap`, `uvu`, `karma`, `cypress run` | config files, positional globs | none |
//!
//! The test files themselves (and through the graph, their closures) are
//! memberships of the adapter's discovery roots. `c8`, `nyc` and `istanbul
//! cover` wrap another command.

use std::path::Path;

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::{Framework, FrameworkInputs, Target as FwTarget, fsx};

use crate::{
    recognizers::{
        run::{go_module_configs, go_package_arg, package_name},
        util::{
            Args, add_code_config, add_glob, find_config, find_up, opt, positionals,
            resolve_js_file,
        },
    },
    registry::{Recognition, RecognizeCx, Recognizer, registry},
    spec::{Command, EntryLang, TaskSpec},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![
        &Jest,
        &Vitest,
        &Pytest,
        &Playwright,
        &GoTest,
        &Cargo,
        &GenericTest,
        &Coverage,
    ]
}

/// Adds a framework adapter's inputs to `out`; returns false if it had
/// uncertainties.
pub fn add_framework_inputs(fi: &FrameworkInputs, out: &mut TaskSpec) -> bool {
    let mut complete = true;
    if let Some(c) = &fi.config_file {
        add_code_config(c, out);
        // The runner's config defines how the task runs its tests ([`crate::runs`]): a
        // config named with `-c` is not the one the tests' own discovery finds.
        out.defines(c.clone());
    }
    for u in &fi.uncertainties {
        complete = false;
        out.uncertain(u.kind, format!("{}: {}", fi.framework, u.detail));
    }
    if fi.projects.is_empty()
        && let Some(dir) = fi.config_file.as_deref().and_then(Path::parent)
    {
        // An unreadable config: its directory at least.
        out.test_run(fi.framework, dir);
    }
    for p in &fi.projects {
        // The tests this run executes ([`crate::runs`]): everything below the discovery
        // roots (filters such as `--shard` or positional patterns only narrow it).
        if p.discovery.roots.is_empty() {
            out.test_run(fi.framework, p.root_dir.clone());
        }
        for r in &p.discovery.roots {
            out.test_run(fi.framework, r.clone());
        }
        if let Some(c) = &p.config_file {
            add_code_config(c, out);
            out.defines(c.clone());
        }
        for c in &p.global.config_files {
            out.config(c.clone());
        }
        for e in &p.global.js_entries {
            out.entry(e.clone(), EntryLang::Js);
        }
        for e in &p.global.entries {
            match &e.target {
                FwTarget::Path(p) => out.entry(p.clone(), EntryLang::of(p)),
                FwTarget::Module { request, from_dir } => {
                    if turborepo_trace_frameworks::common::is_path_request(request) {
                        out.entry(resolve_js_file(from_dir, request), EntryLang::Js);
                    } else {
                        out.tool("npm", package_name(request));
                    }
                }
            }
        }
        for pr in &p.global.probes {
            out.input(pr.clone());
        }
        for m in &p.global.memberships {
            out.membership(m.dir.clone(), m.pattern.clone(), m.reason.clone());
        }
        let exts = &p.discovery.extensions;
        // Globs relative to a base (vitest): one membership per include, when our glob
        // syntax covers them (no extglobs).
        if let Some(base) = &p.discovery.glob_base
            && !p.discovery.include.is_empty()
            && p.discovery.include.iter().all(
                |t| matches!(t, turborepo_trace_frameworks::TestPattern::Glob(g) if !is_extglob(g)),
            )
        {
            for t in &p.discovery.include {
                if let turborepo_trace_frameworks::TestPattern::Glob(g) = t {
                    add_glob(base, g, &format!("{} test files", fi.framework), out);
                }
            }
            for u in &p.uncertainties {
                complete = false;
                out.uncertain(u.kind, format!("{}: {}", fi.framework, u.detail));
            }
            continue;
        }
        let pattern = if exts.is_empty() {
            "**".to_string()
        } else if exts.len() == 1 {
            format!("**/*.{}", exts[0])
        } else {
            format!("**/*.{{{}}}", exts.join(","))
        };
        let roots = if p.discovery.roots.is_empty() {
            vec![p.root_dir.clone()]
        } else {
            p.discovery.roots.clone()
        };
        for r in roots {
            if !r.as_os_str().is_empty() {
                out.membership(r, pattern.clone(), format!("{} test files", fi.framework));
            }
        }
        for u in &p.uncertainties {
            complete = false;
            out.uncertain(u.kind, format!("{}: {}", fi.framework, u.detail));
        }
    }
    complete
}

/// Whether a glob uses extglob syntax (`?(x)`, `+(x)`, `@(x)`, `!(x)`, `*(x)`).
fn is_extglob(g: &str) -> bool {
    g.contains("?(") || g.contains("+(") || g.contains("@(") || g.contains("!(") || g.contains("*(")
}

fn up_to_package<'a>(cmd: &'a Command, cx: &'a RecognizeCx<'_>) -> impl Iterator<Item = &'a Path> {
    let stop = if cmd.cwd.starts_with(cx.package_root) {
        cx.package_root
    } else {
        cx.repo_root
    };
    cmd.cwd.ancestors().take_while(move |d| d.starts_with(stop))
}

/// `jest`.
pub struct Jest;

impl Recognizer for Jest {
    fn id(&self) -> &'static str {
        "jest"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["jest", "craco-test"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "jest");
        let args = Args::new(
            cmd,
            0,
            &[
                "-c",
                "--config",
                "--rootDir",
                "--roots",
                "--testPathPattern",
                "--testPathPatterns",
                "-t",
                "--testNamePattern",
                "--maxWorkers",
                "-w",
                "--shard",
                "--outputFile",
                "--reporters",
                "--coverageDirectory",
                "--selectProjects",
                "--testTimeout",
                "--env",
                "--testEnvironment",
                "--setupFiles",
                "--setupFilesAfterEnv",
                "--coverageReporters",
            ],
        )
        .collect_all();
        if let Some(o) = opt(&args, &["--outputFile"]) {
            out.output(cmd.path(o));
        }
        if let Some(o) = opt(&args, &["--coverageDirectory"]) {
            out.output(cmd.path(o));
        }
        let config = match opt(&args, &["-c", "--config"]) {
            Some(c) if c.trim_start().starts_with('{') => {
                out.uncertain(
                    UncertaintyKind::ParseIncomplete,
                    "jest --config with inline JSON",
                );
                out.test_run("jest", cx.repo_root);
                cx.whole_package(out, "jest inline config");
                return Recognition::Partial;
            }
            Some(c) => Some(cmd.path(c)),
            None => {
                up_to_package(cmd, cx).find_map(turborepo_trace_frameworks::jest::config_in_dir)
            }
        };
        match config {
            Some(c) => {
                let fi = turborepo_trace_frameworks::jest::Jest.load(&c);
                if add_framework_inputs(&fi, out) {
                    Recognition::Full
                } else {
                    Recognition::Partial
                }
            }
            None => {
                // Defaults: rootDir is the cwd.
                let root = opt(&args, &["--rootDir"])
                    .map(|r| cmd.path(r))
                    .unwrap_or_else(|| cmd.cwd.clone());
                out.test_run("jest", root.clone());
                out.membership(
                    root.clone(),
                    "**/*.{js,jsx,ts,tsx,mjs,cjs,json,snap}",
                    "jest default test files",
                );
                out.input(root.join("package.json"));
                for n in turborepo_trace_frameworks::jest::Jest.config_file_names() {
                    out.input(root.join(n));
                }
                if let Some(b) = find_up(&root, "babel.config.js", Some(cx.repo_root)) {
                    add_code_config(&b, out);
                }
                Recognition::Full
            }
        }
    }
}

const VITEST_CONFIGS: &[&str] = &[
    "vitest.config.ts",
    "vitest.config.mts",
    "vitest.config.cts",
    "vitest.config.js",
    "vitest.config.mjs",
    "vitest.config.cjs",
    "vitest.workspace.ts",
    "vitest.workspace.js",
    "vitest.workspace.json",
    "vite.config.ts",
    "vite.config.mts",
    "vite.config.cts",
    "vite.config.js",
    "vite.config.mjs",
    "vite.config.cjs",
];

/// `vitest`.
pub struct Vitest;

impl Recognizer for Vitest {
    fn id(&self) -> &'static str {
        "vitest"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["vitest"]
    }
    fn recognize(&self, cmd: &Command, _cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "vitest");
        let args = Args::new(
            cmd,
            0,
            &[
                "-c",
                "--config",
                "-r",
                "--root",
                "--dir",
                "--project",
                "--workspace",
                "--outputFile",
                "--reporter",
                "--environment",
                "--pool",
                "--shard",
                "-t",
                "--testNamePattern",
                "--mode",
                "--coverage.reportsDirectory",
            ],
        )
        .collect_all();
        if let Some(o) = opt(&args, &["--outputFile"]) {
            out.output(cmd.path(o));
        }
        let root = opt(&args, &["-r", "--root"])
            .map(|r| cmd.path(r))
            .unwrap_or_else(|| cmd.cwd.clone());
        let config = match opt(&args, &["-c", "--config"]) {
            Some(c) => Some(cmd.path(c)),
            None => find_config(&root, VITEST_CONFIGS),
        };
        for n in VITEST_CONFIGS {
            out.input(root.join(n));
        }
        match config {
            Some(c) => {
                let fi = turborepo_trace_frameworks::vitest::Vitest.load(&c);
                let complete = add_framework_inputs(&fi, out);
                if let Some(d) = opt(&args, &["--dir"]) {
                    out.test_run("vitest", cmd.path(d));
                    out.membership(cmd.path(d), "**", "vitest --dir");
                }
                if complete {
                    Recognition::Full
                } else {
                    Recognition::Partial
                }
            }
            None => {
                out.test_run("vitest", root.clone());
                out.membership(
                    root,
                    "**/*.{test,spec}.{js,jsx,ts,tsx,mjs,cjs,mts,cts}",
                    "vitest default test files",
                );
                out.membership(
                    cmd.cwd.clone(),
                    "**/*.{js,jsx,ts,tsx,mjs,cjs,mts,cts,snap}",
                    "vitest default sources",
                );
                Recognition::Full
            }
        }
    }
}

/// `pytest`, `py.test`.
pub struct Pytest;

impl Recognizer for Pytest {
    fn id(&self) -> &'static str {
        "pytest"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["pytest", "py.test"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("pypi", "pytest");
        let args = Args::new(
            cmd,
            0,
            &[
                "-c",
                "--config-file",
                "--rootdir",
                "-p",
                "--confcutdir",
                "-k",
                "-m",
                "--junitxml",
                "--junit-xml",
                "--cov",
                "--cov-report",
                "--cov-config",
                "-n",
                "--numprocesses",
                "--basetemp",
                "--ignore",
                "--deselect",
                "--durations",
                "--maxfail",
                "-o",
                "--override-ini",
                "--import-mode",
                "--tb",
                "-W",
                "--log-file",
                "--html",
            ],
        )
        .collect_all();
        for o in [
            opt(&args, &["--junitxml", "--junit-xml"]),
            opt(&args, &["--html"]),
            opt(&args, &["--log-file"]),
        ]
        .into_iter()
        .flatten()
        {
            out.output(cmd.path(o));
        }
        if let Some(c) = opt(&args, &["--cov-config"]) {
            out.config(cmd.path(c));
        }
        let config = match opt(&args, &["-c", "--config-file"]) {
            Some(c) => Some(cmd.path(c)),
            None => turborepo_trace_frameworks::pytest_config_for(
                &cmd.cwd.join("__turbotrace_probe__.py"),
            ),
        };
        // Without the pytest adapter (feature `python`), what the run reads is unknown.
        let mut complete = turborepo_trace_frameworks::PYTHON;
        if let Some(c) = &config
            && let Some(fi) = turborepo_trace_frameworks::pytest_inputs(c)
        {
            complete = add_framework_inputs(&fi, out);
        }
        for p in [
            "pytest.ini",
            "pyproject.toml",
            "tox.ini",
            "setup.cfg",
            ".pytest.ini",
        ] {
            out.input(cmd.cwd.join(p));
        }
        let pos = positionals(&args);
        if pos.is_empty() && config.is_none() {
            out.test_run("pytest", cmd.cwd.clone());
            out.membership(cmd.cwd.clone(), "**/*.py", "pytest default discovery");
        }
        for p in pos {
            let path_part = p.split("::").next().unwrap_or(p);
            let (stat, _) = crate::glob::split_static_prefix(path_part);
            out.test_run("pytest", cmd.path(stat.trim_end_matches('/')));
            if crate::glob::is_glob(path_part) {
                add_glob(&cmd.cwd, path_part, "pytest paths", out);
                continue;
            }
            let abs = cmd.path(path_part);
            if fsx::is_dir(&abs) {
                out.membership(abs, "**/*.py", "pytest paths");
            } else {
                out.entry(abs, EntryLang::Python);
            }
        }
        for plugin in crate::recognizers::util::opts(&args, &["-p"]) {
            if !plugin.starts_with("no:") {
                out.tool("pypi", plugin);
            }
        }
        let _ = cx;
        if complete {
            Recognition::Full
        } else {
            Recognition::Partial
        }
    }
}

const PLAYWRIGHT_CONFIGS: &[&str] = &[
    "playwright.config.ts",
    "playwright.config.js",
    "playwright.config.mts",
    "playwright.config.mjs",
    "playwright.config.cts",
    "playwright.config.cjs",
];

/// `playwright test`.
pub struct Playwright;

impl Recognizer for Playwright {
    fn id(&self) -> &'static str {
        "playwright"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["playwright"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "@playwright/test");
        let args = Args::new(
            cmd,
            0,
            &[
                "-c",
                "--config",
                "--project",
                "-g",
                "--grep",
                "--output",
                "--reporter",
                "-j",
                "--workers",
                "--shard",
                "--retries",
                "--timeout",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        if pos.first() != Some(&"test") {
            // `playwright install`, `show-report`, `codegen`.
            return Recognition::Ignored;
        }
        let config = match opt(&args, &["-c", "--config"]) {
            Some(c) => Some(cmd.path(c)),
            None => find_config(&cmd.cwd, PLAYWRIGHT_CONFIGS),
        };
        let mut complete = true;
        match &config {
            Some(c) => {
                let fi = turborepo_trace_frameworks::playwright::Playwright.load(c);
                complete = add_framework_inputs(&fi, out);
            }
            None => {
                out.test_run("playwright", cmd.cwd.clone());
                out.membership(
                    cmd.cwd.clone(),
                    "**/*.{spec,test}.{js,ts,mjs,mts}",
                    "playwright default tests",
                );
            }
        }
        // End-to-end tests exercise the app through URLs, not imports.
        out.uncertain(
            UncertaintyKind::UnknownCommand,
            "playwright tests reach the app over HTTP",
        );
        cx.whole_package(out, "playwright tests reach the app over HTTP");
        let _ = complete;
        Recognition::Partial
    }
}

/// `go test`.
pub struct GoTest;

impl Recognizer for GoTest {
    fn id(&self) -> &'static str {
        "gotest"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["go", "gotestsum"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        match cmd.program_name() {
            Some("go") => cmd.args().next() == Some("test"),
            Some("gotestsum") => true,
            _ => false,
        }
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "go");
        // `gotestsum [opts] -- [go test args]`
        let cmd = if cmd.program_name() == Some("gotestsum") {
            match cmd.argv.iter().position(|w| w.text == "--") {
                Some(i) => {
                    let mut c = cmd.shifted(i);
                    c.argv[0] = crate::spec::Word::lit("go");
                    c.argv.insert(1, crate::spec::Word::lit("test"));
                    c
                }
                None => {
                    let mut c = cmd.clone();
                    c.argv = vec![crate::spec::Word::lit("go"), crate::spec::Word::lit("test")];
                    c
                }
            }
        } else {
            cmd.clone()
        };
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let words: Vec<&str> = cmd.args().skip(1).collect();
        let stop = words
            .iter()
            .position(|w| *w == "-args")
            .unwrap_or(words.len());
        let sub = cmd.shifted(0);
        let args = Args::new(
            &sub,
            1,
            &[
                "-run",
                "-tags",
                "-count",
                "-timeout",
                "-p",
                "-parallel",
                "-coverprofile",
                "-cpuprofile",
                "-memprofile",
                "-bench",
                "-benchtime",
                "-covermode",
                "-coverpkg",
                "-exec",
                "-ldflags",
                "-gcflags",
                "-mod",
                "-o",
                "-skip",
                "-C",
                "-shuffle",
                "-outputdir",
                "-json",
                "-vet",
                "-failfast",
            ],
        )
        .collect_all();
        let mut pos: Vec<&str> = positionals(&args)
            .into_iter()
            .filter(|p| words[..stop].contains(p))
            .collect();
        for o in [
            opt(&args, &["-coverprofile"]),
            opt(&args, &["-cpuprofile"]),
            opt(&args, &["-memprofile"]),
            opt(&args, &["-o"]),
        ]
        .into_iter()
        .flatten()
        {
            out.output(cmd.path(o));
        }
        let base = opt(&args, &["-C"])
            .map(|d| cmd.path(d))
            .unwrap_or_else(|| cmd.cwd.clone());
        // A computed package argument (`go test "$1" "$@"`) names packages the command
        // line does not say: the one of the working directory or any below it.
        let computed = |p: &str| cmd.argv.iter().any(|w| w.text == p && w.dynamic);
        if pos.iter().any(|p| computed(p)) {
            pos.retain(|p| !computed(p));
            for all in [".", "./..."] {
                if !pos.contains(&all) {
                    pos.push(all);
                }
            }
        }
        if pos.is_empty() {
            pos.push(".");
        }
        let mut partial = false;
        for p in pos {
            partial |= go_package_arg(&base, p, out);
            // The tests this run executes ([`crate::runs`]): the package directories; an
            // import path could be any package of the repository.
            let run_dir = p.strip_suffix("...").unwrap_or(p).trim_end_matches('/');
            let relative = run_dir.is_empty()
                || run_dir == "."
                || run_dir.starts_with("./")
                || (run_dir.starts_with("../") && !words.iter().any(|w| w.contains('$')));
            if relative {
                out.test_run("gotest", fsx::join(&base, run_dir));
            } else {
                out.test_run("gotest", cx.repo_root);
            }
            if let Some(dir) = p.strip_suffix("/...") {
                out.membership(fsx::join(&base, dir), "**/testdata/**", "go test testdata");
            } else if p == "." || p.starts_with("./") || p.starts_with("../") {
                out.membership(
                    fsx::join(&base, p).join("testdata"),
                    "**",
                    "go test testdata",
                );
            }
        }
        if partial {
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// `cargo test|build|check|clippy|run|bench|doc|nextest|fmt|install`.
pub struct Cargo;

impl Recognizer for Cargo {
    fn id(&self) -> &'static str {
        "cargo"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["cargo"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        cmd.program_name() == Some("cargo")
            && matches!(
                cmd.args()
                    .find(|a| !a.starts_with('+') && !a.starts_with('-')),
                Some(
                    "test"
                        | "t"
                        | "build"
                        | "b"
                        | "check"
                        | "c"
                        | "clippy"
                        | "run"
                        | "r"
                        | "bench"
                        | "doc"
                        | "d"
                        | "nextest"
                        | "fmt"
                        | "install"
                        | "fetch"
                        | "metadata"
                        | "tree"
                        | "publish"
                        | "package"
                        | "llvm-cov"
                        | "insta"
                        | "deny"
                        | "audit"
                        | "machete"
                )
            )
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "cargo");
        let args = Args::new(
            cmd,
            0,
            &[
                "-p",
                "--package",
                "--manifest-path",
                "--target",
                "--target-dir",
                "--features",
                "-F",
                "--bin",
                "--example",
                "--test",
                "--bench",
                "-j",
                "--jobs",
                "--profile",
                "--exclude",
                "--config",
                "-Z",
                "--color",
                "--message-format",
            ],
        )
        .collect_all();
        let manifest = match opt(&args, &["--manifest-path"]) {
            Some(m) => Some(cmd.path(m)),
            None => find_up(&cmd.cwd, "Cargo.toml", Some(cx.repo_root)),
        };
        let Some(manifest) = manifest else {
            out.uncertain(
                UncertaintyKind::UnknownCommand,
                "cargo without a Cargo.toml",
            );
            cx.whole_package(out, "cargo without a Cargo.toml");
            return Recognition::Partial;
        };
        // The workspace root: the topmost Cargo.toml with [workspace] up to the repo
        // root.
        let crate_dir = manifest.parent().unwrap_or(&cmd.cwd).to_path_buf();
        let mut root = crate_dir.clone();
        for x in crate_dir.ancestors().skip(1) {
            if !x.starts_with(cx.repo_root) {
                break;
            }
            if fsx::read_to_string(x.join("Cargo.toml")).is_ok_and(|s| s.contains("[workspace]")) {
                root = x.to_path_buf();
            }
        }
        let sub = positionals(&args).first().copied().unwrap_or("");
        if matches!(
            sub,
            "fetch" | "metadata" | "tree" | "install" | "audit" | "deny" | "machete"
        ) {
            out.config(root.join("Cargo.lock"));
            return Recognition::Ignored;
        }
        out.config(manifest.clone());
        out.config(root.join("Cargo.lock"));
        out.config(root.join("Cargo.toml"));
        for n in [
            ".cargo/config.toml",
            ".cargo/config",
            "rust-toolchain.toml",
            "rust-toolchain",
            "clippy.toml",
            ".clippy.toml",
            "rustfmt.toml",
            ".rustfmt.toml",
        ] {
            out.input(root.join(n));
        }
        out.membership(root.clone(), "**/*.rs", "cargo workspace sources");
        out.membership(root.clone(), "**/Cargo.toml", "cargo workspace manifests");
        if sub != "fmt" {
            // Build scripts and tests read arbitrary files (include_str!, fixtures).
            out.membership(
                root.clone(),
                "**/{build.rs,*.proto,*.c,*.h}",
                "cargo build inputs",
            );
            if matches!(
                sub,
                "test" | "t" | "nextest" | "bench" | "insta" | "llvm-cov"
            ) {
                out.test_run("cargo-test", root.clone());
                out.membership(
                    root.clone(),
                    "**/{tests,benches,fixtures,testdata,snapshots}/**",
                    "cargo test data",
                );
            }
        }
        if matches!(sub, "build" | "b" | "run" | "r" | "doc" | "d" | "package") {
            out.output(
                opt(&args, &["--target-dir"])
                    .map(|t| cmd.path(t))
                    .unwrap_or_else(|| root.join("target")),
            );
        }
        Recognition::Full
    }
}

/// `mocha`, `ava`, `tap`, `uvu`, `karma`, `cypress run`, `jasmine`, `bun
/// test`-style runners without an adapter: config files plus positional test
/// globs (default: `test/`).
pub struct GenericTest;

impl Recognizer for GenericTest {
    fn id(&self) -> &'static str {
        "test-runner"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "mocha",
            "ava",
            "tap",
            "uvu",
            "karma",
            "cypress",
            "jasmine",
            "tape",
            "lab",
            "node-tap",
            "wdio",
            "storybook",
            "test-storybook",
            "start-storybook",
            "build-storybook",
        ]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("");
        out.tool("npm", prog);
        let args = Args::new(
            cmd,
            0,
            &[
                "-r",
                "--require",
                "--config",
                "-c",
                "--reporter",
                "-R",
                "--timeout",
                "-t",
                "--spec",
                "-s",
                "--browser",
                "-b",
                "--project",
                "-P",
                "--env",
                "-e",
                "--ignore",
                "--extension",
                "--file",
                "-g",
                "--grep",
                "--port",
                "-p",
                "-o",
                "--output-dir",
                "--config-dir",
                "--static-dir",
            ],
        )
        .collect_all();
        let configs: &[&str] = match prog {
            "mocha" => &[
                ".mocharc.js",
                ".mocharc.cjs",
                ".mocharc.json",
                ".mocharc.jsonc",
                ".mocharc.yml",
                ".mocharc.yaml",
            ],
            "ava" => &["ava.config.js", "ava.config.cjs", "ava.config.mjs"],
            "karma" => &["karma.conf.js", "karma.conf.ts", "karma.conf.cjs"],
            "cypress" => &[
                "cypress.config.ts",
                "cypress.config.js",
                "cypress.config.mjs",
                "cypress.config.cjs",
                "cypress.json",
            ],
            "tap" | "node-tap" => &[".taprc", ".taprc.yml"],
            "jasmine" => &["spec/support/jasmine.json", "jasmine.json"],
            "wdio" => &["wdio.conf.ts", "wdio.conf.js"],
            _ => &[],
        };
        for c in configs {
            let p = cmd.cwd.join(c);
            if fsx::is_file(&p) {
                add_code_config(&p, out);
            }
        }
        if let Some(c) = opt(&args, &["--config", "-c", "--config-dir"]) {
            let p = cmd.path(c);
            if fsx::is_dir(&p) {
                out.membership(p, "**", format!("{prog} config dir"));
            } else {
                add_code_config(&p, out);
            }
        }
        for r in crate::recognizers::util::opts(&args, &["-r", "--require", "--file"]) {
            if r.starts_with('.') || r.starts_with('/') {
                out.entry(resolve_js_file(&cmd.cwd, r), EntryLang::Js);
            } else {
                out.tool("npm", package_name(r));
            }
        }
        if let Some(o) = opt(&args, &["-o", "--output-dir"]) {
            out.output(cmd.path(o));
        }
        let mut pos: Vec<&str> = positionals(&args);
        if prog == "cypress" || prog == "storybook" {
            if prog == "storybook" && pos.first() == Some(&"build") {
                out.output(cmd.path("storybook-static"));
            }
            pos.retain(|p| !matches!(*p, "run" | "open" | "dev" | "build" | "component" | "e2e"));
        }
        pos.extend(crate::recognizers::util::opts(&args, &["--spec", "-s"]));
        if !prog.contains("storybook") || prog == "test-storybook" {
            out.test_run(prog, cmd.cwd.clone());
        }
        if pos.is_empty() {
            match prog {
                "cypress" => out.membership(cmd.cwd.join("cypress"), "**", "cypress default dir"),
                "storybook" | "start-storybook" | "build-storybook" | "test-storybook" => {
                    out.membership(cmd.cwd.join(".storybook"), "**", "storybook config");
                    cx.whole_package(out, "storybook stories");
                }
                _ => {
                    out.membership(
                        cmd.cwd.join("test"),
                        "**",
                        format!("{prog} default test dir"),
                    );
                    out.membership(
                        cmd.cwd.join("spec"),
                        "**",
                        format!("{prog} default test dir"),
                    );
                }
            }
        }
        for p in pos {
            if crate::glob::is_glob(p) {
                add_glob(&cmd.cwd, p, &format!("{prog} test files"), out);
            } else {
                let abs = cmd.path(p);
                if fsx::is_dir(&abs) {
                    out.membership(abs, "**", format!("{prog} test files"));
                } else {
                    out.entry(abs.clone(), EntryLang::of(&abs));
                }
            }
        }
        Recognition::Full
    }
}

/// `c8 …`, `nyc …`, `istanbul cover …`: coverage wrappers around another
/// command.
pub struct Coverage;

impl Recognizer for Coverage {
    fn id(&self) -> &'static str {
        "coverage"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["c8", "nyc", "istanbul"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("");
        out.tool("npm", prog);
        for n in [".c8rc", ".c8rc.json", ".nycrc", ".nycrc.json", ".nycrc.yml"] {
            out.input(cmd.cwd.join(n));
        }
        // Skip options (values attached with `=` or known value options).
        const VALUE: &[&str] = &[
            "-r",
            "--reporter",
            "--reports-dir",
            "-o",
            "--temp-directory",
            "--include",
            "-n",
            "--exclude",
            "-x",
            "--extension",
            "-e",
            "--src",
            "--report-dir",
            "--cwd",
            "--check-coverage",
            "--lines",
            "--branches",
            "--functions",
            "--statements",
        ];
        let mut i = 1;
        let words: Vec<&str> = cmd.argv.iter().map(|w| w.text.as_str()).collect();
        if prog == "istanbul" && words.get(1) == Some(&"cover") {
            i = 2;
        }
        while let Some(w) = words.get(i) {
            if *w == "--" {
                i += 1;
                break;
            }
            if !w.starts_with('-') {
                break;
            }
            i += if VALUE.contains(w) { 2 } else { 1 };
        }
        if matches!(
            words.get(i),
            Some(&("report" | "check-coverage" | "merge" | "instrument"))
        ) || i >= words.len()
        {
            return Recognition::Ignored;
        }
        out.output(cmd.cwd.join("coverage"));
        out.output(cmd.cwd.join(".nyc_output"));
        let inner = cmd.shifted(i);
        let rep = registry().recognize(&inner, cx, out);
        rep.recognition
    }
}
