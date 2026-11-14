//! Script runners: a file argument becomes an entry traced with the language
//! plugins, so its fs reads and writes are covered by what the plugins already
//! find.
//!
//! - `node`, `tsx`, `ts-node`, `esno`, `babel-node`, `vite-node`, `jiti`,
//!   `sucrase-node`, `node --test`;
//! - `deno run|test|compile|task`;
//! - `python`, `python3`, `python -m mod`;
//! - `go run|build|install|vet|generate` (generate is left to the code
//!   generators when they register it).

use std::path::Path;

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::{Args, add_glob, find_up, opt, positionals},
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, EntryLang, TaskRef, TaskSpec},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&Node, &Deno, &Python, &GoRun]
}

/// Node-compatible runners.
pub struct Node;

const NODE_VALUE_OPTS: &[&str] = &[
    "-r",
    "--require",
    "--import",
    "--loader",
    "--experimental-loader",
    "-e",
    "--eval",
    "-p",
    "--print",
    "--env-file",
    "--env-file-if-exists",
    "--conditions",
    "-C",
    "--input-type",
    "--max-old-space-size",
    "--max-http-header-size",
    "--max-semi-space-size",
    "--stack-trace-limit",
    "--unhandled-rejections",
    "--dns-result-order",
    "--openssl-config",
    "--icu-data-dir",
    "--redirect-warnings",
    "--report-dir",
    "--report-filename",
    "--diagnostic-dir",
    "--secure-heap",
    "--secure-heap-min",
    "--inspect-port",
    "--title",
    "--test-reporter",
    "--test-reporter-destination",
    "--test-name-pattern",
    "--test-concurrency",
    "--watch-path",
    "--stack-size",
    "--tsconfig",
    "--project",
    "-P",
    "--compiler",
    "-O",
    "--compiler-options",
    "--ignore",
    "--extensions",
    "-x",
    "--config-file",
    "--root",
    "--dir",
];

impl Recognizer for Node {
    fn id(&self) -> &'static str {
        "node"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "node",
            "nodejs",
            "tsx",
            "ts-node",
            "ts-node-esm",
            "ts-node-dev",
            "tsnd",
            "esno",
            "esr",
            "babel-node",
            "vite-node",
            "jiti",
            "sucrase-node",
            "swc-node",
            "node-dev",
            "nodemon",
            "ts-node-transpile-only",
        ]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        node_like(cmd, 0, cx, out)
    }
}

/// Recognizes a node-style invocation whose arguments start at `skip` (after
/// the program).
pub fn node_like(
    cmd: &Command,
    skip: usize,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Recognition {
    let prog = cmd.program_name().unwrap_or("node").to_string();
    if !matches!(prog.as_str(), "node" | "nodejs" | "bun") {
        out.tool("npm", npm_package_of(&prog));
    }
    let mut words: Vec<&str> = cmd.args().skip(skip).collect();
    // `tsx watch x`, `nodemon --exec`, `vite-node --watch`
    if matches!(words.first(), Some(&"watch")) && prog == "tsx" {
        words.remove(0);
    }
    let mut result = Recognition::Full;
    let mut i = 0;
    let mut test_mode = false;
    let mut script: Option<&str> = None;
    while i < words.len() {
        let w = words[i];
        i += 1;
        if w == "--" {
            continue;
        }
        if !w.starts_with('-') || w == "-" {
            script = Some(w);
            break;
        }
        let (name, inline) = match w.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (w, None),
        };
        let value = if inline.is_some() {
            inline
        } else if NODE_VALUE_OPTS.contains(&name) {
            i += 1;
            words.get(i - 1).copied()
        } else {
            None
        };
        match name {
            "--test" => test_mode = true,
            "-e" | "--eval" | "-p" | "--print" => {
                out.uncertain(
                    UncertaintyKind::EvalLike,
                    format!("`{prog} {name}` inline code"),
                );
                result = Recognition::Partial;
                // Inline code reads anything: depend on the package.
                cx.whole_package(out, &format!("`{prog} {name}` inline code"));
                return result;
            }
            "-r" | "--require" | "--import" | "--loader" | "--experimental-loader" => {
                if let Some(v) = value {
                    module_entry(cmd, v, out);
                }
            }
            "--env-file" | "--env-file-if-exists" => {
                if let Some(v) = value {
                    out.input(cmd.path(v));
                }
            }
            "--tsconfig" | "--project" | "-P" => {
                if let Some(v) = value {
                    out.config(cmd.path(v));
                }
            }
            _ => {}
        }
    }
    if matches!(
        prog.as_str(),
        "ts-node" | "ts-node-esm" | "ts-node-dev" | "tsnd" | "tsx"
    ) && let Some(t) = find_up(&cmd.cwd, "tsconfig.json", Some(cx.repo_root))
    {
        out.config(t);
    }
    if test_mode {
        // node --test [files|globs]; default: test files under the cwd.
        let rest: Vec<&str> = script
            .into_iter()
            .chain(words[i..].iter().copied())
            .collect();
        if rest.is_empty() {
            add_glob(
                &cmd.cwd,
                "**/{test,test-*,*.test,*-test,*_test}.{js,mjs,cjs,ts,mts,cts}",
                "node --test default pattern",
                out,
            );
            add_glob(
                &cmd.cwd,
                "**/test/**/*.{js,mjs,cjs,ts,mts,cts}",
                "node --test default pattern",
                out,
            );
        }
        for r in rest {
            if !r.starts_with('-') {
                entry_or_glob(cmd, r, out);
            }
        }
        return result;
    }
    let Some(script) = script else {
        // REPL or stdin.
        out.uncertain(
            UncertaintyKind::UnknownCommand,
            format!("`{prog}` without a script"),
        );
        cx.whole_package(out, &format!("`{prog}` without a script"));
        return Recognition::Partial;
    };
    if script == "-" || cmd.argv.iter().any(|w| w.text == script && w.dynamic) {
        out.uncertain(
            UncertaintyKind::ShellUnsupported,
            format!("`{prog}` script `{script}` is computed"),
        );
        cx.whole_package(out, "computed script");
        return Recognition::Partial;
    }
    let p = crate::recognizers::util::resolve_js_file(&cmd.cwd, script);
    out.entry(p, EntryLang::Js);
    // Arguments of the script that name files: inputs (a generator's schema
    // argument).
    crate::recognizers::util::add_existing_files(cmd, &words[i..], out);
    result
}

/// The npm package that provides a binary.
pub fn npm_package_of(bin: &str) -> &str {
    match bin {
        "ts-node-esm" | "ts-node-transpile-only" => "ts-node",
        "tsnd" => "ts-node-dev",
        "esno" | "esr" => bin,
        "babel-node" => "@babel/node",
        "swc-node" => "@swc-node/register",
        "tsc" => "typescript",
        "vite-node" => "vite-node",
        "next" => "next",
        "run-s" | "run-p" => "npm-run-all",
        "eslint" => "eslint",
        _ => bin,
    }
}

/// `-r x` / `--import x`: a local file is an entry, a package is a toolchain
/// package.
fn module_entry(cmd: &Command, v: &str, out: &mut TaskSpec) {
    if v.starts_with('.') || v.starts_with('/') {
        out.entry(
            crate::recognizers::util::resolve_js_file(&cmd.cwd, v),
            EntryLang::Js,
        );
    } else {
        let name = package_name(v);
        out.tool("npm", name);
    }
}

/// `@scope/name/sub` → `@scope/name`; `name/sub` → `name`.
pub fn package_name(spec: &str) -> &str {
    let mut it = spec.match_indices('/');
    let cut = if spec.starts_with('@') {
        it.nth(1).map(|(i, _)| i)
    } else {
        it.next().map(|(i, _)| i)
    };
    cut.map_or(spec, |i| &spec[..i])
}

fn entry_or_glob(cmd: &Command, a: &str, out: &mut TaskSpec) {
    if crate::glob::is_glob(a) {
        add_glob(&cmd.cwd, a, "test files", out);
    } else {
        let p = cmd.path(a);
        if fsx::is_dir(&p) {
            out.membership(p, "**", "test files");
        } else {
            out.entry(p.clone(), EntryLang::of(&p));
        }
    }
}

/// `deno run|test|compile|bench|check|task|fmt|lint`.
pub struct Deno;

impl Recognizer for Deno {
    fn id(&self) -> &'static str {
        "deno"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["deno"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "deno");
        for n in ["deno.json", "deno.jsonc"] {
            if let Some(p) = find_up(&cmd.cwd, n, Some(cx.repo_root)) {
                out.config(p);
            }
        }
        let args = Args::new(
            cmd,
            1,
            &[
                "--config",
                "-c",
                "--import-map",
                "--lock",
                "-o",
                "--output",
                "--location",
                "--cert",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        if let Some(c) = opt(&args, &["--config", "-c"]) {
            out.config(cmd.path(c));
        }
        if let Some(m) = opt(&args, &["--import-map"]) {
            out.config(cmd.path(m));
        }
        match cmd.args().next() {
            Some("run" | "compile" | "bench" | "check" | "bundle" | "install") => {
                if let Some(f) = pos.first() {
                    if f.contains("://") || f.starts_with("npm:") || f.starts_with("jsr:") {
                        out.tool("deno", *f);
                        return Recognition::Partial;
                    }
                    out.entry(cmd.path(f), EntryLang::Js);
                }
                if let Some(o) = opt(&args, &["-o", "--output"]) {
                    out.output(cmd.path(o));
                }
                Recognition::Full
            }
            Some("task") => match pos.first() {
                Some(t) => {
                    out.dep(
                        TaskRef::Local {
                            name: t.to_string(),
                        },
                        "deno task",
                    );
                    Recognition::Full
                }
                None => Recognition::Ignored,
            },
            Some("test" | "fmt" | "lint") => {
                if pos.is_empty() {
                    out.membership(
                        &cmd.cwd,
                        "**/*.{ts,tsx,js,jsx,mjs,mts,json,jsonc,md}",
                        "deno default files",
                    );
                }
                for p in pos {
                    entry_or_glob(cmd, p, out);
                }
                Recognition::Full
            }
            _ => {
                // `deno file.ts` (implicit run)
                match cmd.args().next() {
                    Some(f) if crate::recognizers::util::is_pathish(f) => {
                        out.entry(cmd.path(f), EntryLang::Js);
                        Recognition::Full
                    }
                    _ => {
                        out.uncertain(UncertaintyKind::UnknownCommand, "deno subcommand");
                        cx.whole_package(out, "deno subcommand");
                        Recognition::Partial
                    }
                }
            }
        }
    }
}

/// `python file.py`, `python -m module`, `python -c` (uncertain).
pub struct Python;

impl Recognizer for Python {
    fn id(&self) -> &'static str {
        "python"
    }
    // Versioned interpreters are listed explicitly (the default `matches` and
    // dispatch indexing use this list); an unlisted `python3.N` falls back
    // soundly as an unknown command.
    fn programs(&self) -> &'static [&'static str] {
        &[
            "python",
            "python3",
            "python2",
            "py",
            "pypy3",
            "python3.6",
            "python3.7",
            "python3.8",
            "python3.9",
            "python3.10",
            "python3.11",
            "python3.12",
            "python3.13",
            "python3.14",
            "python3.15",
        ]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "python");
        let words: Vec<&str> = cmd.args().collect();
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            i += 1;
            match w {
                "-c" => {
                    out.uncertain(UncertaintyKind::EvalLike, "`python -c` inline code");
                    cx.whole_package(out, "`python -c` inline code");
                    return Recognition::Partial;
                }
                "-m" => {
                    let Some(m) = words.get(i) else {
                        return Recognition::Partial;
                    };
                    let rest = cmd.shifted(i + 2);
                    return python_module(cmd, m, &rest, cx, out);
                }
                // `python - args <<'EOF'`: the program is read from standard input; the words
                // after `-` are its arguments, not a script.
                "-" => {
                    out.uncertain(
                        UncertaintyKind::EvalLike,
                        "`python -` program from standard input",
                    );
                    cx.whole_package(out, "`python -` program from standard input");
                    return Recognition::Partial;
                }
                "-W" | "-X" | "--check-hash-based-pycs" => i += 1,
                w if w.starts_with('-') => {}
                f => {
                    let p = cmd.path(f);
                    out.entry(p, EntryLang::Python);
                    crate::recognizers::util::add_existing_files(cmd, &words[i..], out);
                    return Recognition::Full;
                }
            }
        }
        out.uncertain(UncertaintyKind::UnknownCommand, "`python` without a script");
        cx.whole_package(out, "`python` without a script");
        Recognition::Partial
    }
}

/// `python -m <module> …`: a local module is an entry; known tools delegate.
fn python_module(
    cmd: &Command,
    m: &str,
    rest: &Command,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Recognition {
    // Delegate known tool modules to their recognizers: `python -m pytest`.
    let mut as_cmd = rest.clone();
    as_cmd.argv.insert(0, crate::spec::Word::lit(m));
    if let Some(r) = crate::registry::registry().find(&as_cmd)
        && r.id() != "python"
    {
        return r.recognize(&as_cmd, cx, out);
    }
    let rel = m.replace('.', "/");
    for cand in [format!("{rel}.py"), format!("{rel}/__main__.py")] {
        let p = cmd.path(&cand);
        if fsx::is_file(&p) {
            out.entry(p, EntryLang::Python);
            return Recognition::Full;
        }
    }
    // Also look under src/ layouts.
    for cand in [format!("src/{rel}.py"), format!("src/{rel}/__main__.py")] {
        let p = cx.package_root.join(&cand);
        if fsx::is_file(&p) {
            out.entry(p, EntryLang::Python);
            return Recognition::Full;
        }
    }
    match m {
        "pip" | "venv" | "ensurepip" | "http.server" | "json.tool" | "build" | "twine" => {
            out.tool("pypi", m);
            if m == "build" {
                cx.whole_package(out, "python -m build");
                return Recognition::Partial;
            }
            Recognition::Ignored
        }
        _ => {
            out.tool("pypi", m.split('.').next().unwrap_or(m));
            out.uncertain(UncertaintyKind::UnknownCommand, format!("python -m {m}"));
            cx.whole_package(out, &format!("python -m {m}"));
            Recognition::Partial
        }
    }
}

/// `go run|build|install|vet|fmt` (go test is in testrunners).
pub struct GoRun;

impl Recognizer for GoRun {
    fn id(&self) -> &'static str {
        "go"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["go"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        cmd.program_name() == Some("go")
            && matches!(
                cmd.args().next(),
                Some(
                    "run"
                        | "build"
                        | "install"
                        | "vet"
                        | "fmt"
                        | "mod"
                        | "work"
                        | "get"
                        | "env"
                        | "version"
                        | "list"
                        | "tool"
                )
            )
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "go");
        let sub = cmd.args().next().unwrap_or("");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        match sub {
            "run" | "build" | "install" | "vet" | "fmt" | "list" => {
                let args = Args::new(
                    cmd,
                    1,
                    &[
                        "-o", "-tags", "-ldflags", "-gcflags", "-mod", "-modfile", "-C", "-p",
                        "-exec", "-overlay", "-pgo",
                    ],
                )
                .collect_all();
                let base = match opt(&args, &["-C"]) {
                    Some(d) => cmd.path(d),
                    None => cmd.cwd.clone(),
                };
                if let Some(o) = opt(&args, &["-o"]) {
                    out.output(cmd.path(o));
                }
                if let Some(o) = opt(&args, &["-overlay", "-modfile", "-pgo"]) {
                    out.input(cmd.path(o));
                }
                let mut pos = positionals(&args);
                if sub == "run" {
                    // Only the package/files before the program's own arguments.
                    let n = pos.iter().take_while(|p| p.ends_with(".go")).count().max(1);
                    pos.truncate(n);
                }
                if pos.is_empty() {
                    pos.push(".");
                }
                let mut partial = false;
                for p in pos {
                    partial |= go_package_arg(&base, p, out);
                }
                if partial {
                    Recognition::Partial
                } else {
                    Recognition::Full
                }
            }
            "mod" | "work" | "get" => Recognition::Ignored,
            _ => Recognition::Ignored,
        }
    }
}

/// `go.mod`, `go.sum`, `go.work` above `dir`.
pub fn go_module_configs(dir: &Path, repo_root: &Path, out: &mut TaskSpec) {
    if let Some(m) = find_up(dir, "go.mod", Some(repo_root)) {
        let d = m.parent().unwrap_or(dir).to_path_buf();
        out.config(m);
        out.config(d.join("go.sum"));
    }
    if let Some(w) = find_up(dir, "go.work", Some(repo_root)) {
        out.config(w);
    }
}

/// A Go package argument: `./...` (all packages below), `./cmd/x`, `main.go`,
/// or an import path (toolchain). Returns true when not fully modelled.
pub fn go_package_arg(base: &Path, p: &str, out: &mut TaskSpec) -> bool {
    if let Some(dir) = p.strip_suffix("/...") {
        let d = fsx::join(base, dir);
        out.membership(d.clone(), "**/*.go", "go packages (./...)");
        out.membership(
            d,
            "**/*.{c,h,s,S,cc,cpp,hpp,m,swig,syso}",
            "go package sources",
        );
        false
    } else if p.ends_with(".go") {
        let f = fsx::join(base, p);
        out.entry(f, EntryLang::Go);
        false
    } else if p == "." || p.starts_with("./") || p.starts_with("../") || p.starts_with('/') {
        let d = fsx::join(base, p);
        out.entry(d.clone(), EntryLang::Go);
        out.membership(d, "*.go", "go package directory");
        false
    } else {
        // An import path: local module packages are handled by the Go plugin when
        // traced as a dependency; remote ones are toolchain.
        out.tool("go", p.split('@').next().unwrap_or(p));
        p.contains('@')
    }
}
