//! Go code generation: `//go:generate` directives and the generators they run.
//!
//! **Directives** ([`directives`]): `//go:generate cmd args…` at the start of a
//! line in any `.go` file of a package (test files too), with `-command NAME …`
//! aliases expanded. Each runs in the file's directory with `$GOFILE`,
//! `$GOLINE`, `$GOPACKAGE` and `$DOLLAR` set. The `gogenerate` task source
//! ([`crate::sources::gogenerate`]) makes one task per directive; `go generate
//! [pkgs]` in a script ([`GoGenerate`]) recognizes every directive of the named
//! packages (`./...` walks below, skipping `vendor`, `testdata`, `_*`, `.*` and
//! nested modules) into one task, with the packages' `*.go` files as a
//! membership (a new directive changes the task).
//!
//! **Generators:**
//!
//! | Program | Inputs | Outputs |
//! |---|---|---|
//! | `stringer -type T[,U] [-output f] [dir\|files]` | the package (Go entry, `*.go`) | `-output`, else `<dir>/<t>_string.go` (lowercased first type) |
//! | `enumer -type T [-output f]` | the package | `-output`, else `<dir>/<t>_enumer.go` |
//! | `mockgen -source=f.go [-destination=d] [-aux_files=p=f,…]` | the source file (Go entry), aux files | `-destination` (stdout otherwise) |
//! | `mockgen [-destination=d] importpath Iface,…` (package/reflect mode) | the package if it is in the module, else toolchain | `-destination` |
//! | `oapi-codegen [-config c.yaml] [-o out] spec` | spec (+ `$ref`s), config, `output-options.user-templates` files | `-o`, else config `output` |
//! | `easyjson [-output_filename f] files\|dirs` | the files' packages | `<file>_easyjson.go`, or `<dir>/<dir>_easyjson.go` |
//! | `ent generate [--target d] [--template t] ./schema` | the schema package (Go entry), templates | `--target`, else the schema's parent directory (minus the schema and `generate.go`/`entc.go`) |
//! | `wire [gen] [-output_file_prefix p] [pkgs]` | the packages (Go entries: their imports' providers count) | `<pkg>/<p>wire_gen.go` |
//! | `gqlgen [generate] [--config f]` | `gqlgen.yml` (`.gqlgen.yml`, `gqlgen.yaml` up from the cwd): `schema` globs, `autobind`/`models` packages in the module | `exec`, `model`, `federation`, `resolver` files or directories |
//! | `go-jsonschema [flags] schema.json… -o out.go` | the schemas (+ `$ref`s) | `-o` |
//! | `go run <importpath>[@v] args`, `go tool <name> args` | (dispatched to the tool's recognizer when known: the above, `sqlc`, `buf`, `protoc-gen-*`) | |
//! | `go run ./cmd/gen args`, `go run gen.go args`, `go run <module path>/cmd/gen args` ([`GoRunGenerator`]) | the generator (Go entry), existing files among its arguments (specs with their `$ref`s) | `-o`, `-out`, `-output`, `--output`, `-destination`, `-output-file`, ... |
//!
//! `go run ./cmd/gen` without arguments is handled by the `go` recognizer (an
//! entry).

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use super::{
    config::{self, add_spec, as_str, entries, get, get_path, str_list, unknown},
    finish,
};
use crate::{
    recognizers::{
        run::go_module_configs,
        util::{Args, find_up, opt, positionals},
    },
    registry::{Recognition, RecognizeCx, Recognizer, registry},
    spec::{Command, EntryLang, Membership, TaskSpec, Word},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![
        &GoGenerate,
        &GoRunTool,
        &GoRunGenerator,
        &GoJsonschema,
        &Stringer,
        &Mockgen,
        &OapiCodegen,
        &Easyjson,
        &Ent,
        &Wire,
        &Gqlgen,
    ]
}

/// One `//go:generate` directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    /// 1-based line.
    pub line: u32,
    /// The command line, with `-command` aliases expanded.
    pub command: String,
}

/// The directives of a Go source file (a cheap byte scan: files without
/// `//go:generate` cost one `memmem`).
pub fn directives(src: &[u8]) -> Vec<Directive> {
    const MARK: &[u8] = b"//go:generate";
    let mut out = Vec::new();
    if memchr::memmem::find(src, MARK).is_none() {
        return out;
    }
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    for (i, line) in src.split(|&b| b == b'\n').enumerate() {
        let Some(rest) = line.strip_prefix(MARK) else {
            continue;
        };
        if !matches!(rest.first(), Some(b' ' | b'\t')) {
            continue;
        }
        let text = String::from_utf8_lossy(rest).trim().to_string();
        if text.is_empty() {
            continue;
        }
        if let Some(def) = text.strip_prefix("-command") {
            let def = def.trim_start();
            if let Some((name, cmd)) = def.split_once(char::is_whitespace) {
                aliases.insert(name.to_string(), cmd.trim().to_string());
            }
            continue;
        }
        let first = text.split_whitespace().next().unwrap_or("");
        let command = match aliases.get(first) {
            Some(exp) => format!("{exp}{}", &text[first.len()..]),
            None => text,
        };
        out.push(Directive {
            line: i as u32 + 1,
            command,
        });
    }
    out
}

/// The package clause name of a Go file (`package foo` → `foo`).
pub fn package_clause(src: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(&src[..src.len().min(64 * 1024)]).ok()?;
    let mut in_block = false;
    for line in text.lines() {
        let l = line.trim();
        if in_block {
            if let Some(e) = l.find("*/") {
                in_block = false;
                let rest = l[e + 2..].trim();
                if let Some(p) = rest.strip_prefix("package ") {
                    return Some(p.split_whitespace().next()?.to_string());
                }
            }
            continue;
        }
        if l.starts_with("//") || l.is_empty() {
            continue;
        }
        if l.starts_with("/*") {
            if !l.contains("*/") {
                in_block = true;
            }
            continue;
        }
        if let Some(p) = l.strip_prefix("package ") {
            return Some(
                p.split_whitespace()
                    .next()?
                    .trim_end_matches(';')
                    .to_string(),
            );
        }
        return None;
    }
    None
}

/// The environment `go generate` sets for a directive.
pub fn directive_env(file: &Path, line: u32, src: &[u8]) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if let Some(n) = file.file_name() {
        env.insert("GOFILE".into(), n.to_string_lossy().into_owned());
    }
    env.insert("GOLINE".into(), line.to_string());
    if let Some(p) = package_clause(src) {
        env.insert("GOPACKAGE".into(), p);
    }
    env.insert("DOLLAR".into(), "$".into());
    env
}

/// Directories Go tooling skips when walking `./...`.
pub fn skip_go_dir(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with('_')
        || name == "testdata"
        || name == "vendor"
        || fsx::skip_dir(name)
}

/// `go generate [flags] [packages|files]`.
pub struct GoGenerate;

impl Recognizer for GoGenerate {
    fn id(&self) -> &'static str {
        "go-generate"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["go"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        cmd.program_name() == Some("go") && cmd.args().next() == Some("generate")
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("system", "go");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args = Args::new(
            cmd,
            1,
            &["-run", "-skip", "-tags", "-mod", "-modfile", "-C"],
        )
        .collect_all();
        let base = match opt(&args, &["-C"]) {
            Some(d) => cmd.path(d),
            None => cmd.cwd.clone(),
        };
        let mut pos = positionals(&args);
        if pos.is_empty() {
            pos.push(".");
        }
        let mut files: Vec<PathBuf> = Vec::new();
        for p in pos {
            if p.ends_with(".go") {
                files.push(fsx::join(&base, p));
            } else if let Some(d) = p.strip_suffix("/...").or((p == "...").then_some(".")) {
                let d = fsx::join(&base, d);
                out.membership(d.clone(), "**/*.go", "go generate packages");
                walk_go_files(&d, true, &mut files);
            } else if p == "." || p.starts_with("./") || p.starts_with("../") || p.starts_with('/')
            {
                let d = fsx::join(&base, p);
                out.membership(d.clone(), "*.go", "go generate package");
                walk_go_files(&d, false, &mut files);
            } else {
                match local_package_dir(&base, cx.repo_root, p) {
                    Some(d) => {
                        out.membership(d.clone(), "*.go", "go generate package");
                        walk_go_files(&d, false, &mut files);
                    }
                    None => unknown(
                        out,
                        &base,
                        UncertaintyKind::UnresolvedSpecifier,
                        format!("go generate package `{p}`"),
                    ),
                }
            }
        }
        for f in files {
            let Ok(src) = fsx::read(&f) else { continue };
            let ds = directives(&src);
            if ds.is_empty() {
                continue;
            }
            out.input(f.clone());
            let dir = config::dir_of(&f);
            for d in ds {
                let mut env = cx.env.clone();
                env.extend(directive_env(&f, d.line, &src));
                let parsed = crate::shell::parse(&d.command, &dir, &env);
                out.uncertainties.extend(parsed.uncertainties);
                let rcx = RecognizeCx { env: &env, ..*cx };
                for c in &parsed.commands {
                    registry().recognize(c, &rcx, out);
                }
            }
        }
        finish(out, before)
    }
}

/// `.go` files in `dir` (and below with `recursive`, Go's `./...` rules).
pub fn walk_go_files(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Some(entries) = fsx::read_dir(&d) else {
            continue;
        };
        let mut subdirs = Vec::new();
        for (p, is_dir) in entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_dir {
                if recursive && depth < 32 && !skip_go_dir(name) {
                    subdirs.push(p);
                }
            } else if name.ends_with(".go") && !name.starts_with('.') && !name.starts_with('_') {
                out.push(p);
            }
        }
        for s in subdirs {
            // A nested module is not part of `./...`.
            if !fsx::is_file(&s.join("go.mod")) {
                stack.push((s, depth + 1));
            }
        }
    }
    out.sort();
}

/// The module directory of an import path inside the current module (`go.mod`
/// above `cwd`).
pub fn local_package_dir(cwd: &Path, repo_root: &Path, path: &str) -> Option<PathBuf> {
    if path == "." || path.starts_with("./") || path.starts_with("../") {
        return Some(fsx::join(cwd, path));
    }
    let gomod = find_up(cwd, "go.mod", Some(repo_root))?;
    let text = fsx::read_to_string(&gomod).ok()?;
    let module = text.lines().find_map(|l| {
        l.trim()
            .strip_prefix("module ")
            .map(|m| m.trim().trim_matches('"').to_string())
    })?;
    let root = config::dir_of(&gomod);
    if path == module {
        Some(root)
    } else {
        path.strip_prefix(&format!("{module}/"))
            .map(|rest| root.join(rest))
    }
}

/// A Go package directory as an input: a Go entry (its imports count) and its
/// `*.go` files, minus `exclude` (generated outputs in the same directory).
fn go_package(dir: &Path, exclude: &[String], out: &mut TaskSpec) {
    out.entry(dir.to_path_buf(), EntryLang::Go);
    out.memberships.push(Membership {
        dir: dir.to_path_buf(),
        pattern: "*.go".into(),
        exclude: exclude.to_vec(),
        reason: "go package".into(),
        data: false,
        own: false,
    });
}

/// Rel name of `p` inside `dir`, for membership excludes.
fn rel_in(dir: &Path, p: &Path) -> Option<String> {
    p.strip_prefix(dir)
        .ok()
        .map(|r| r.to_string_lossy().replace('\\', "/"))
}

/// The known code generator program of a Go import path
/// (`golang.org/x/tools/cmd/stringer` → `stringer`, `entgo.io/ent/cmd/ent` →
/// `ent`, `github.com/99designs/gqlgen` → `gqlgen`).
pub fn go_tool_program(path: &str) -> Option<&'static str> {
    let path = path.split('@').next().unwrap_or(path);
    let mut segs: Vec<&str> = path.split('/').collect();
    if segs.last().is_some_and(|s| {
        s.len() > 1 && s.starts_with('v') && s[1..].bytes().all(|b| b.is_ascii_digit())
    }) {
        segs.pop();
    }
    let last = *segs.last()?;
    const KNOWN: &[&str] = &[
        "stringer",
        "enumer",
        "mockgen",
        "oapi-codegen",
        "easyjson",
        "ent",
        "entc",
        "wire",
        "gqlgen",
        "sqlc",
        "buf",
        "protoc",
        "go-enum",
        "mockery",
        "moq",
        "counterfeiter",
        "ifacemaker",
        "genny",
        "templ",
        "goa",
        "swag",
    ];
    KNOWN.iter().copied().find(|k| *k == last)
}

/// `go run <importpath>[@v] args` and `go tool <name> args` for known
/// generators.
pub struct GoRunTool;

impl GoRunTool {
    /// The import path / tool name and its index in argv.
    fn split(cmd: &Command) -> Option<(String, usize)> {
        let words: Vec<&str> = cmd.args().collect();
        match words.first() {
            Some(&"tool") => {
                let name = words.get(1)?;
                Some((name.to_string(), 2))
            }
            Some(&"run") => {
                let mut i = 1;
                while let Some(w) = words.get(i) {
                    if !w.starts_with('-') {
                        break;
                    }
                    // Flags with a separate value.
                    if matches!(
                        *w,
                        "-tags"
                            | "-mod"
                            | "-modfile"
                            | "-C"
                            | "-exec"
                            | "-ldflags"
                            | "-gcflags"
                            | "-p"
                            | "-overlay"
                            | "-pgo"
                    ) {
                        i += 1;
                    }
                    i += 1;
                }
                let pkg = words.get(i)?;
                if pkg.ends_with(".go")
                    || pkg.starts_with('.')
                    || pkg.starts_with('/')
                    || !pkg.contains('.')
                {
                    return None;
                }
                Some((pkg.to_string(), i + 1))
            }
            _ => None,
        }
    }
}

impl Recognizer for GoRunTool {
    fn id(&self) -> &'static str {
        "go-run-tool"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["go"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        cmd.program_name() == Some("go")
            && Self::split(cmd).is_some_and(|(p, _)| {
                go_tool_program(&p).is_some()
                    || p.rsplit('/')
                        .next()
                        .is_some_and(|l| l.starts_with("protoc-gen-"))
            })
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let Some((path, at)) = Self::split(cmd) else {
            return Recognition::Partial;
        };
        out.tool("system", "go");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        if path.contains('/') {
            out.tool("go", path.split('@').next().unwrap_or(&path));
        }
        let prog = go_tool_program(&path)
            .map(str::to_string)
            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(&path).to_string());
        // Rewrite as `<prog> args…` (argv[0] replaced by the program, the go words
        // dropped).
        let mut c = cmd.shifted(at);
        c.argv[0] = Word::lit(prog.clone());
        match registry().find(&c) {
            Some(r) if r.id() != self.id() => r.recognize(&c, cx, out),
            _ => {
                out.uncertain(
                    UncertaintyKind::UnknownCommand,
                    format!("go generator `{prog}`"),
                );
                cx.whole_package(out, &format!("unknown go generator `{prog}`"));
                Recognition::Partial
            }
        }
    }
}

/// Output flags of ad-hoc generators run with `go run`.
const GEN_OUTPUT_FLAGS: &[&str] = &[
    "-o",
    "--o",
    "-out",
    "--out",
    "-output",
    "--output",
    "-destination",
    "--destination",
    "-dest",
    "--dest",
    "-outfile",
    "--outfile",
    "-output-file",
    "--output-file",
    "-output_file",
    "--output_file",
    "-out-file",
    "--out-file",
];

/// `go run <local generator> args…` (a package or file of this module, with
/// arguments): the generator is a Go entry, output flags are outputs, and
/// existing files among the arguments are inputs (`.json`/`.yaml` specs with
/// their `$ref`s).
pub struct GoRunGenerator;

impl GoRunGenerator {
    /// The argv index of the first generator argument and the package words.
    fn split(cmd: &Command) -> Option<(usize, Vec<String>)> {
        let words: Vec<&str> = cmd.args().collect();
        if words.first() != Some(&"run") {
            return None;
        }
        let mut i = 1;
        while let Some(w) = words.get(i) {
            if !w.starts_with('-') {
                break;
            }
            if matches!(
                *w,
                "-tags"
                    | "-mod"
                    | "-modfile"
                    | "-C"
                    | "-exec"
                    | "-ldflags"
                    | "-gcflags"
                    | "-p"
                    | "-overlay"
                    | "-pgo"
            ) {
                i += 1;
            }
            i += 1;
        }
        let first = *words.get(i)?;
        let pkgs: Vec<String> = if first.ends_with(".go") {
            words[i..]
                .iter()
                .take_while(|w| w.ends_with(".go"))
                .map(|w| w.to_string())
                .collect()
        } else {
            vec![first.to_string()]
        };
        let next = i + pkgs.len();
        // argv index = words index + 1 (argv[0] is `go`).
        (next < words.len()).then_some((next + 1, pkgs))
    }

    fn is_local(cmd: &Command, pkg: &str) -> bool {
        pkg.ends_with(".go")
            || pkg == "."
            || pkg.starts_with("./")
            || pkg.starts_with("../")
            || local_package_dir(&cmd.cwd, Path::new("/"), pkg).is_some()
    }
}

impl Recognizer for GoRunGenerator {
    fn id(&self) -> &'static str {
        "go-run-generator"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["go"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        cmd.program_name() == Some("go")
            && Self::split(cmd).is_some_and(|(_, pkgs)| Self::is_local(cmd, &pkgs[0]))
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        let Some((at, pkgs)) = Self::split(cmd) else {
            return Recognition::Partial;
        };
        out.tool("system", "go");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        for p in &pkgs {
            if p.ends_with(".go") {
                out.entry(cmd.path(p), EntryLang::Go);
            } else {
                match local_package_dir(&cmd.cwd, cx.repo_root, p) {
                    Some(d) => go_package(&d, &[], out),
                    None => out.tool("go", p.as_str()),
                }
            }
        }
        let words: Vec<&str> = cmd.argv[at..].iter().map(|w| w.text.as_str()).collect();
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            i += 1;
            if w.starts_with('-') {
                let (name, inline) = match w.split_once('=') {
                    Some((n, v)) => (n, Some(v)),
                    None => (w, None),
                };
                if GEN_OUTPUT_FLAGS.contains(&name) {
                    let v = inline.or_else(|| {
                        i += 1;
                        words.get(i - 1).copied()
                    });
                    if let Some(v) = v
                        && v != "-"
                    {
                        out.output(cmd.path(v));
                    }
                    continue;
                }
                if let Some(v) = inline {
                    gen_arg_input(cmd, v, out);
                }
                continue;
            }
            gen_arg_input(cmd, w, out);
        }
        finish(out, before)
    }
}

/// A generator argument that names an existing file: an input (specs with their
/// `$ref`s).
fn gen_arg_input(cmd: &Command, v: &str, out: &mut TaskSpec) {
    let p = cmd.path(v);
    if v.is_empty() || !fsx::is_file(&p) {
        return;
    }
    if matches!(
        p.extension().and_then(|e| e.to_str()),
        Some("json" | "yaml" | "yml" | "graphql" | "graphqls")
    ) {
        add_spec(&cmd.cwd, v, &cmd.cwd, out);
    } else {
        out.input(p);
    }
}

/// `go-jsonschema` (github.com/omissis/go-jsonschema).
pub struct GoJsonschema;

impl Recognizer for GoJsonschema {
    fn id(&self) -> &'static str {
        "go-jsonschema"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["go-jsonschema", "gojsonschema"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("go", "github.com/omissis/go-jsonschema");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args = Args::new(
            cmd,
            0,
            &[
                "-o",
                "--output",
                "-p",
                "--package",
                "--schema-output",
                "--schema-package",
                "--schema-root-type",
                "--tags",
                "--capitalization",
                "--resolve-extension",
                "--struct-name-from-title",
                "--yaml-extension",
                "--min-sized-ints",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        if pos.is_empty() {
            unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "go-jsonschema without schemas",
            );
        }
        for p in pos {
            add_spec(&cmd.cwd, p, &cmd.cwd, out);
        }
        for o in crate::recognizers::util::opts(&args, &["--schema-output"]) {
            // `--schema-output=<schema id>=<file>`
            if let Some((_, f)) = o.rsplit_once('=') {
                out.output(cmd.path(f));
            }
        }
        match opt(&args, &["-o", "--output"]) {
            Some(o) if o != "-" => out.output(cmd.path(o)),
            _ => {}
        }
        finish(out, before)
    }
}

/// The directory a stringer-like tool works on, and whether files were named.
fn target_dir(cmd: &Command, pos: &[&str]) -> PathBuf {
    match pos.first() {
        None => cmd.cwd.clone(),
        Some(p) if p.ends_with(".go") => config::dir_of(&cmd.path(p)),
        Some(p) => cmd.path(p),
    }
}

/// `stringer` and `enumer`.
pub struct Stringer;

impl Recognizer for Stringer {
    fn id(&self) -> &'static str {
        "stringer"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["stringer", "enumer"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        let prog = cmd.program_name().unwrap_or("stringer");
        let (tool, suffix) = if prog == "enumer" {
            ("github.com/dmarkham/enumer", "_enumer.go")
        } else {
            ("golang.org/x/tools/cmd/stringer", "_string.go")
        };
        out.tool("go", tool);
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args = Args::new(
            cmd,
            0,
            &[
                "-type",
                "-output",
                "-trimprefix",
                "-tags",
                "-transform",
                "-addprefix",
                "-comment",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        let dir = target_dir(cmd, &pos);
        let output = match opt(&args, &["-output"]) {
            Some(o) => cmd.path(o),
            None => match opt(&args, &["-type"]).and_then(|t| t.split(',').next()) {
                Some(t) if !t.is_empty() => dir.join(format!("{}{suffix}", t.to_lowercase())),
                _ => {
                    unknown(
                        out,
                        &dir,
                        UncertaintyKind::ParseIncomplete,
                        format!("`{prog}` without -type"),
                    );
                    return finish(out, before);
                }
            },
        };
        let exclude: Vec<String> = rel_in(&dir, &output).into_iter().collect();
        go_package(&dir, &exclude, out);
        for p in pos.iter().filter(|p| p.ends_with(".go")) {
            out.input(cmd.path(p));
        }
        out.output(output);
        finish(out, before)
    }
}

/// `mockgen`.
pub struct Mockgen;

impl Recognizer for Mockgen {
    fn id(&self) -> &'static str {
        "mockgen"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["mockgen"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("go", "go.uber.org/mock/mockgen");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args = Args::new(
            cmd,
            0,
            &[
                "-source",
                "-destination",
                "-package",
                "-imports",
                "-aux_files",
                "-build_flags",
                "-mock_names",
                "-self_package",
                "-copyright_file",
                "-exclude_interfaces",
                "-exec_only",
                "-prog_only",
                "-model_gob",
            ],
        )
        .collect_all();
        let dest = opt(&args, &["-destination"]).map(|d| cmd.path(d));
        let mut exclude = Vec::new();
        if let Some(src) = opt(&args, &["-source"]) {
            let f = cmd.path(src);
            out.entry(f.clone(), EntryLang::Go);
            out.input(f);
        } else {
            let pos = positionals(&args);
            match pos.first() {
                Some(pkg) => match local_package_dir(&cmd.cwd, cx.repo_root, pkg) {
                    Some(d) => {
                        if let Some(dst) = &dest {
                            exclude.extend(rel_in(&d, dst));
                        }
                        go_package(&d, &exclude, out);
                    }
                    None => out.tool("go", *pkg),
                },
                None => unknown(
                    out,
                    &cmd.cwd,
                    UncertaintyKind::ParseIncomplete,
                    "mockgen without -source or a package",
                ),
            }
        }
        if let Some(aux) = opt(&args, &["-aux_files"]) {
            for a in aux.split(',') {
                let f = a.split_once('=').map_or(a, |(_, f)| f);
                if !f.is_empty() {
                    out.input(cmd.path(f));
                }
            }
        }
        if let Some(c) = opt(&args, &["-copyright_file"]) {
            out.input(cmd.path(c));
        }
        if let Some(g) = opt(&args, &["-model_gob"]) {
            out.input(cmd.path(g));
        }
        if let Some(d) = dest {
            out.output(d);
        }
        finish(out, before)
    }
}

/// `oapi-codegen`.
pub struct OapiCodegen;

impl Recognizer for OapiCodegen {
    fn id(&self) -> &'static str {
        "oapi-codegen"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["oapi-codegen"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool(
            "go",
            "github.com/oapi-codegen/oapi-codegen/v2/cmd/oapi-codegen",
        );
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args = Args::new(
            cmd,
            0,
            &[
                "-config",
                "--config",
                "-o",
                "-generate",
                "-package",
                "-templates",
                "-include-tags",
                "-exclude-tags",
                "-import-mapping",
                "-exclude-schemas",
                "-response-type-suffix",
                "-alias-types",
                "-initialism-overrides",
            ],
        )
        .collect_all();
        let mut output = opt(&args, &["-o"]).map(|o| cmd.path(o));
        if let Some(c) = opt(&args, &["-config", "--config"]) {
            let p = cmd.path(c);
            let dir = config::dir_of(&p);
            match config::load(&p, out) {
                Some(v) => {
                    if output.is_none()
                        && let Some(o) = get(&v, "output").as_ref().and_then(as_str)
                    {
                        output = Some(cmd.path(&o));
                    }
                    if let Some(t) = get_path(&v, &["output-options", "user-templates"]) {
                        // Values are file paths, URLs or inline templates.
                        if let Ok(es) = entries(&t) {
                            for (_, x) in es {
                                if let Some(s) = as_str(&x)
                                    && !s.contains('\n')
                                    && !s.contains("{{")
                                {
                                    if config::is_url(&s) {
                                        unknown(
                                            out,
                                            &dir,
                                            UncertaintyKind::EnvDependent,
                                            format!("oapi-codegen remote template `{s}`"),
                                        );
                                    } else {
                                        out.input(cmd.path(&s));
                                    }
                                }
                            }
                        }
                    }
                }
                None => unknown(
                    out,
                    &dir,
                    UncertaintyKind::ParseIncomplete,
                    "oapi-codegen config not readable",
                ),
            }
        }
        if let Some(t) = opt(&args, &["-templates"]) {
            out.membership(cmd.path(t), "**", "oapi-codegen templates");
        }
        match positionals(&args).first() {
            Some(spec) => {
                add_spec(&cmd.cwd, spec, &cmd.cwd, out);
            }
            None => unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "oapi-codegen without a spec",
            ),
        }
        if let Some(o) = output {
            out.output(o);
        }
        finish(out, before)
    }
}

/// `easyjson`.
pub struct Easyjson;

impl Recognizer for Easyjson {
    fn id(&self) -> &'static str {
        "easyjson"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["easyjson"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("go", "github.com/mailru/easyjson/easyjson");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args = Args::new(
            cmd,
            0,
            &["-output_filename", "-build_tags", "-gen_build_flags"],
        )
        .collect_all();
        let named = opt(&args, &["-output_filename"]);
        let pos = positionals(&args);
        if pos.is_empty() {
            unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "easyjson without inputs",
            );
        }
        for p in pos {
            let f = cmd.path(p);
            let (dir, outp) = if fsx::is_dir(&f) || !p.ends_with(".go") {
                let name = f
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (f.clone(), f.join(format!("{name}_easyjson.go")))
            } else {
                let s = f.to_string_lossy();
                (
                    config::dir_of(&f),
                    PathBuf::from(format!("{}_easyjson.go", s.trim_end_matches(".go"))),
                )
            };
            let outp = named.map(|n| cmd.path(n)).unwrap_or(outp);
            let exclude: Vec<String> = rel_in(&dir, &outp).into_iter().collect();
            go_package(&dir, &exclude, out);
            out.output(outp);
        }
        finish(out, before)
    }
}

/// `ent` / `entc`.
pub struct Ent;

impl Recognizer for Ent {
    fn id(&self) -> &'static str {
        "ent"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["ent", "entc"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("go", "entgo.io/ent/cmd/ent");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args = Args::new(
            cmd,
            0,
            &[
                "--target",
                "--template",
                "--feature",
                "--header",
                "--storage",
                "--idtype",
                "--target-package",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        match pos.first().copied() {
            Some("generate") => {}
            Some("describe") | Some("schema") => {
                if let Some(s) = pos.get(1) {
                    go_package(&cmd.path(s), &[], out);
                }
                return finish(out, before);
            }
            _ => return Recognition::Ignored,
        }
        let Some(schema) = pos.get(1) else {
            unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "ent generate without a schema path",
            );
            return finish(out, before);
        };
        let schema_dir = match local_package_dir(&cmd.cwd, cx.repo_root, schema) {
            Some(d) => d,
            None => {
                out.tool("go", *schema);
                return finish(out, before);
            }
        };
        go_package(&schema_dir, &[], out);
        for t in crate::recognizers::util::opts(&args, &["--template"]) {
            let t = t
                .strip_prefix("dir=")
                .or_else(|| t.strip_prefix("file="))
                .unwrap_or(t);
            if let Some(g) = t.strip_prefix("glob=") {
                crate::recognizers::util::add_glob(&cmd.cwd, g, "ent templates", out);
            } else {
                crate::recognizers::util::add_path_input(cmd, t, "ent templates", out);
            }
        }
        if let Some(h) = opt(&args, &["--header"])
            && fsx::is_file(&cmd.path(h))
        {
            out.input(cmd.path(h));
        }
        let target = match opt(&args, &["--target"]) {
            Some(t) => cmd.path(t),
            None => config::dir_of(&schema_dir),
        };
        let mut exclude = vec!["generate.go".to_string(), "entc.go".to_string()];
        if let Some(r) = rel_in(&target, &schema_dir) {
            exclude.push(format!("{r}/**"));
        }
        config::output_pattern(out, target, "**", &exclude);
        finish(out, before)
    }
}

/// `wire`.
pub struct Wire;

impl Recognizer for Wire {
    fn id(&self) -> &'static str {
        "wire"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["wire"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("go", "github.com/google/wire/cmd/wire");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args =
            Args::new(cmd, 0, &["-output_file_prefix", "-tags", "-header_file"]).collect_all();
        let mut pos = positionals(&args);
        let writes = match pos.first().copied() {
            Some("gen") => {
                pos.remove(0);
                true
            }
            Some("check" | "show" | "diff") => {
                pos.remove(0);
                false
            }
            Some("help" | "flags" | "commands") => return Recognition::Ignored,
            _ => true,
        };
        if pos.is_empty() {
            pos.push(".");
        }
        let prefix = opt(&args, &["-output_file_prefix"]).unwrap_or("");
        if let Some(h) = opt(&args, &["-header_file"]) {
            out.input(cmd.path(h));
        }
        for p in pos {
            let Some(d) = local_package_dir(&cmd.cwd, cx.repo_root, p) else {
                unknown(
                    out,
                    &cmd.cwd,
                    UncertaintyKind::UnresolvedSpecifier,
                    format!("wire package `{p}`"),
                );
                continue;
            };
            let name = format!("{prefix}wire_gen.go");
            go_package(&d, std::slice::from_ref(&name), out);
            if writes {
                out.output(d.join(name));
            }
        }
        finish(out, before)
    }
}

/// `gqlgen`.
pub struct Gqlgen;

impl Recognizer for Gqlgen {
    fn id(&self) -> &'static str {
        "gqlgen"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["gqlgen"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("go", "github.com/99designs/gqlgen");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let args = Args::new(cmd, 0, &["--config", "-c"]).collect_all();
        match positionals(&args).first().copied() {
            None | Some("generate") => {}
            Some("init" | "version" | "help") => return Recognition::Ignored,
            Some(other) => {
                unknown(
                    out,
                    cx.package_root,
                    UncertaintyKind::UnknownCommand,
                    format!("gqlgen {other}"),
                );
                return finish(out, before);
            }
        }
        let cfg = match opt(&args, &["--config", "-c"]) {
            Some(c) => Some(cmd.path(c)),
            None => {
                let mut found = None;
                let mut d = Some(cmd.cwd.as_path());
                while let Some(x) = d {
                    if let Some(p) =
                        fsx::first_existing(x, &[".gqlgen.yml", "gqlgen.yml", "gqlgen.yaml"])
                    {
                        found = Some(p);
                        break;
                    }
                    out.config(x.join("gqlgen.yml"));
                    if x == cx.repo_root {
                        break;
                    }
                    d = x.parent();
                }
                found
            }
        };
        let Some(cfg) = cfg else {
            unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "gqlgen without gqlgen.yml",
            );
            return finish(out, before);
        };
        let dir = config::dir_of(&cfg);
        let Some(v) = config::load(&cfg, out) else {
            unknown(
                out,
                &dir,
                UncertaintyKind::ParseIncomplete,
                "gqlgen.yml not readable",
            );
            return finish(out, before);
        };
        let schemas = get(&v, "schema")
            .map(|s| str_list(&s).0)
            .unwrap_or_else(|| vec!["graph/*.graphqls".into()]);
        for s in schemas {
            config::add_location(
                &dir,
                &s,
                config::PathRole::Data,
                "**/*.{graphql,graphqls}",
                "gqlgen schema",
                &dir,
                out,
            );
        }
        // Generated files.
        let file_or_dir = |key: &str, def: Option<&str>, out: &mut TaskSpec| {
            let sec = get(&v, key);
            let layout = sec
                .as_ref()
                .and_then(|s| get(s, "layout"))
                .as_ref()
                .and_then(as_str);
            let d = sec
                .as_ref()
                .and_then(|s| get(s, "dir"))
                .as_ref()
                .and_then(as_str);
            let f = sec
                .as_ref()
                .and_then(|s| get(s, "filename"))
                .as_ref()
                .and_then(as_str)
                .or_else(|| def.map(str::to_string));
            match (layout.as_deref(), d, f) {
                (Some("follow-schema"), Some(d), _) => {
                    let pattern = sec
                        .as_ref()
                        .and_then(|s| get(s, "filename_template"))
                        .as_ref()
                        .and_then(as_str)
                        .map(|t| t.replace("{name}", "*"))
                        .unwrap_or_else(|| {
                            if key == "resolver" {
                                "*.resolvers.go".into()
                            } else {
                                "*.generated.go".into()
                            }
                        });
                    config::output_pattern(out, fsx::join(&dir, &d), &pattern, &[]);
                    if key == "exec" {
                        out.output(fsx::join(&dir, &d).join("root_.generated.go"));
                    }
                }
                (_, _, Some(f)) if sec.is_some() || def.is_some() => {
                    out.output(fsx::join(&dir, &f));
                }
                _ => {}
            }
        };
        file_or_dir("exec", Some("generated.go"), out);
        file_or_dir("model", Some("models_gen.go"), out);
        if get(&v, "federation").is_some() {
            file_or_dir("federation", Some("federation.go"), out);
        }
        if get(&v, "resolver").is_some() {
            file_or_dir("resolver", None, out);
        }
        // Go packages whose types are bound into the generated code.
        let mut pkgs: Vec<String> = get(&v, "autobind")
            .map(|a| str_list(&a).0)
            .unwrap_or_default();
        if let Some(ms) = get(&v, "models")
            && let Ok(es) = entries(&ms)
        {
            for (_, m) in es {
                for t in get(&m, "model").map(|x| str_list(&x).0).unwrap_or_default() {
                    if let Some((p, _)) = t.rsplit_once('.') {
                        pkgs.push(p.to_string());
                    }
                }
            }
        }
        pkgs.sort();
        pkgs.dedup();
        for p in pkgs {
            match local_package_dir(&dir, cx.repo_root, &p) {
                Some(d) => go_package(&d, &[], out),
                None => out.tool("go", p),
            }
        }
        finish(out, before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_directives() {
        let src = b"// Package x.\npackage x\n\n//go:generate stringer -type=Color\n// go:generate no\n //go:generate no\n//go:generate -command mg go run go.uber.org/mock/mockgen\n//go:generate mg -source=$GOFILE -destination=mocks/x.go\n//go:generatex no\n";
        let d = directives(src);
        assert_eq!(
            d,
            [
                Directive {
                    line: 4,
                    command: "stringer -type=Color".into()
                },
                Directive {
                    line: 8,
                    command: "go run go.uber.org/mock/mockgen -source=$GOFILE \
                              -destination=mocks/x.go"
                        .into()
                }
            ]
        );
        assert_eq!(package_clause(src).as_deref(), Some("x"));
        assert_eq!(
            package_clause(b"/* a\n b */\npackage y_test\n").as_deref(),
            Some("y_test")
        );
    }

    #[test]
    fn tool_programs() {
        assert_eq!(
            go_tool_program("golang.org/x/tools/cmd/stringer@v0.20.0"),
            Some("stringer")
        );
        assert_eq!(go_tool_program("entgo.io/ent/cmd/ent"), Some("ent"));
        assert_eq!(
            go_tool_program("github.com/99designs/gqlgen"),
            Some("gqlgen")
        );
        assert_eq!(
            go_tool_program("github.com/oapi-codegen/oapi-codegen/v2/cmd/oapi-codegen"),
            Some("oapi-codegen")
        );
        assert_eq!(go_tool_program("github.com/x/y"), None);
    }
}
