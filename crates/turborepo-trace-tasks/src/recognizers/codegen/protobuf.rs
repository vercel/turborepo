//! Protocol Buffers: `buf`, `protoc`, `python -m grpc_tools.protoc`,
//! `grpc_tools_node_protoc`, and `protoc-gen-*` plugins run directly.
//!
//! **`buf generate [input] [--template t] [-o dir]`**: the template is
//! `--template` (a file, or inline YAML/JSON), else `buf.gen.yaml` in the cwd.
//! Every plugin's `out` (relative to `-o`, default the cwd) is an output. The
//! input (positional, v2 template `inputs`, default `.`) is a directory whose
//! `buf.yaml` (v2 `modules[].path`/`excludes`, v1 `build.roots`/ `excludes`) or
//! `buf.work.yaml` (`directories`) names the module roots; every `*.proto`
//! below them is an input, plus `buf.yaml` and `buf.lock`. Remote inputs
//! (`buf.build/…`, git, tarballs) and remote plugins are network: uncertain /
//! toolchain. `buf build|lint|breaking|format|export|ls-files` read the same
//! modules (`build -o`, `format -w`, `export -o` write).
//!
//! **`protoc`**: `-I`/`--proto_path` include dirs (default the cwd); proto file
//! arguments and their `import`s, resolved through the include dirs
//! transitively (unresolved imports other than `google/protobuf/*` are probes);
//! globs and directories make every `*.proto` below the include dirs an input.
//! `--<plugin>_out=[opts:]DIR` outputs: known plugins produce a file pattern
//! below `DIR` (`*.pb.go`, `*_pb2.py`, ...); unknown plugins produce
//! the files named after the inputs (`DIR/**/<stem>*`), or the whole `DIR` when
//! it is dedicated. `.zip`/`.jar` outputs, `--descriptor_set_out`/`-o` and
//! `--dependency_out` are files. `--plugin=protoc-gen-x=PATH` adds the plugin
//! (a `node_modules/.bin` plugin is an npm toolchain package). `@argsfile` is
//! uncertain.

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;
use turborepo_trace_lang_js::extract::config::ConfigVal;

use super::{
    config::{self, as_str, get, output_pattern, str_list, unknown},
    finish,
};
use crate::{
    recognizers::util::{Args, flag, opt, positionals},
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, Membership, OutputKind, TaskSpec},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&Buf, &Protoc]
}

/// `buf …`.
pub struct Buf;

impl Recognizer for Buf {
    fn id(&self) -> &'static str {
        "buf"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["buf"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("system", "buf");
        let args = Args::new(
            cmd,
            0,
            &[
                "--template",
                "-o",
                "--output",
                "--path",
                "--exclude-path",
                "--config",
                "--type",
                "--error-format",
                "--log-format",
                "--timeout",
                "--against",
                "--against-config",
                "--as",
                "--from",
                "--to",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        let mut sub = pos.first().copied().unwrap_or("");
        let mut rest = &pos[pos.len().min(1)..];
        // `buf beta …`, `buf alpha …`, `buf mod …`, `buf dep …`
        if matches!(
            sub,
            "beta" | "alpha" | "mod" | "dep" | "registry" | "config"
        ) {
            let s2 = pos.get(1).copied().unwrap_or("");
            sub = match (sub, s2) {
                ("mod" | "dep", "update" | "prune") => "dep-update",
                (_, "init") => "ignored",
                _ => s2,
            };
            rest = &pos[pos.len().min(2)..];
        }
        let input = rest.first().copied();
        let config_override = opt(&args, &["--config"]);
        match sub {
            "generate" => {
                let base = match opt(&args, &["-o", "--output"]) {
                    Some(o) => cmd.path(o),
                    None => cmd.cwd.clone(),
                };
                let tpl = opt(&args, &["--template"]);
                let (tpl_val, tpl_dir) = match tpl {
                    Some(t) if t.trim_start().starts_with('{') || t.contains('\n') => {
                        (config::parse_yaml(t), cmd.cwd.clone())
                    }
                    Some(t) => {
                        let p = cmd.path(t);
                        (config::load(&p, out), config::dir_of(&p))
                    }
                    None => match fsx::first_existing(&cmd.cwd, &["buf.gen.yaml", "buf.gen.yml"]) {
                        Some(p) => (config::load(&p, out), cmd.cwd.clone()),
                        None => {
                            out.config(cmd.cwd.join("buf.gen.yaml"));
                            (None, cmd.cwd.clone())
                        }
                    },
                };
                let Some(tv) = tpl_val else {
                    unknown(
                        out,
                        &tpl_dir,
                        UncertaintyKind::ParseIncomplete,
                        "buf generate template not readable",
                    );
                    return finish(out, before);
                };
                let module_dirs = match input {
                    Some(i) => buf_input(cmd, i, config_override, &tpl_dir, out),
                    None => {
                        let mut dirs = Vec::new();
                        let tin = get(&tv, "inputs");
                        match tin {
                            Some(ConfigVal::Arr(items)) if !items.is_empty() => {
                                for it in items {
                                    dirs.extend(template_input(cmd, &it, &tpl_dir, out));
                                }
                            }
                            _ => dirs.extend(buf_input(cmd, ".", config_override, &tpl_dir, out)),
                        }
                        dirs
                    }
                };
                plugins(cmd, &tv, &base, &module_dirs, &tpl_dir, out);
            }
            "build" | "lint" | "breaking" | "format" | "export" | "ls-files" | "convert"
            | "stats" => {
                let dirs = buf_input(cmd, input.unwrap_or("."), config_override, &cmd.cwd, out);
                if sub == "breaking"
                    && let Some(a) = opt(&args, &["--against"])
                {
                    if a.contains(".git") || a.contains("://") || a.starts_with("buf.build/") {
                        out.uncertain(
                            UncertaintyKind::EnvDependent,
                            format!("buf breaking against `{a}`"),
                        );
                    } else {
                        buf_input(cmd, a, None, &cmd.cwd, out);
                    }
                }
                if let Some(o) = opt(&args, &["-o", "--output"])
                    && o != "-"
                {
                    out.output(cmd.path(o));
                }
                if sub == "format" && (flag(&args, &["-w", "--write"])) {
                    for d in dirs {
                        output_pattern(out, d, "**/*.proto", &[]);
                        // A formatter rewrites sources in place.
                        if let Some(o) = out.outputs.last_mut() {
                            o.kind = OutputKind::InPlace;
                        }
                    }
                }
            }
            "dep-update" => {
                let d = input
                    .map(|i| cmd.path(i))
                    .unwrap_or_else(|| cmd.cwd.clone());
                out.config(d.join("buf.yaml"));
                out.output(d.join("buf.lock"));
                out.uncertain(
                    UncertaintyKind::EnvDependent,
                    "buf dependency update resolves remote modules",
                );
            }
            "" | "ignored" | "--version" | "help" | "push" | "registry" | "curl" | "login"
            | "logout" | "studio-agent" | "completion" => return Recognition::Ignored,
            other => unknown(
                out,
                cx.package_root,
                UncertaintyKind::UnknownCommand,
                format!("buf {other}"),
            ),
        }
        finish(out, before)
    }
}

/// A v2 template `inputs` item.
fn template_input(
    cmd: &Command,
    it: &ConfigVal,
    tpl_dir: &Path,
    out: &mut TaskSpec,
) -> Vec<PathBuf> {
    if let Some(d) = get(it, "directory").as_ref().and_then(as_str) {
        return buf_input(cmd, &d, None, tpl_dir, out);
    }
    if let Some(f) = get(it, "proto_file").as_ref().and_then(as_str) {
        return buf_input(cmd, &f, None, tpl_dir, out);
    }
    for k in [
        "module",
        "git_repo",
        "tarball",
        "zip_archive",
        "binary_image",
    ] {
        if let Some(v) = get(it, k) {
            let s = as_str(&v).unwrap_or_default();
            if matches!(k, "binary_image") && !config::is_url(&s) && !s.is_empty() {
                out.input(fsx::join(&cmd.cwd, &s));
                return Vec::new();
            }
            unknown(
                out,
                tpl_dir,
                UncertaintyKind::EnvDependent,
                format!("buf remote input `{k}: {s}`"),
            );
            return Vec::new();
        }
    }
    unknown(
        out,
        tpl_dir,
        UncertaintyKind::ParseIncomplete,
        "buf template input not understood",
    );
    Vec::new()
}

/// A buf input: a local directory (module roots from
/// `buf.yaml`/`buf.work.yaml`), a proto file, or a remote reference
/// (uncertain). Returns the module roots.
fn buf_input(
    cmd: &Command,
    input: &str,
    config_override: Option<&str>,
    cfg_dir: &Path,
    out: &mut TaskSpec,
) -> Vec<PathBuf> {
    let looks_remote = input.starts_with("buf.build/")
        || input.contains("://")
        || input.contains(".git")
        || input.ends_with(".tar.gz")
        || input.ends_with(".zip")
        || input.contains("#");
    if looks_remote {
        unknown(
            out,
            cfg_dir,
            UncertaintyKind::EnvDependent,
            format!("buf remote input `{input}`"),
        );
        return Vec::new();
    }
    let p = cmd.path(input);
    if input.ends_with(".proto") {
        let includes = vec![config::dir_of(&p)];
        proto_closure(&[p], &includes, out);
        return Vec::new();
    }
    if input.ends_with(".binpb") || input.ends_with(".json") || input.ends_with(".bin") {
        out.input(p);
        return Vec::new();
    }
    let mut roots: Vec<Membership> = Vec::new();
    let buf_yaml = match config_override {
        Some(c) if !c.trim_start().starts_with('{') => Some(cmd.path(c)),
        _ => fsx::first_existing(&p, &["buf.yaml", "buf.yml"]),
    };
    let work = fsx::first_existing(&p, &["buf.work.yaml", "buf.work.yml"]);
    out.input(p.join("buf.lock"));
    if let Some(w) = &work
        && let Some(v) = config::load(w, out)
        && let Some(ds) = get(&v, "directories")
    {
        for d in str_list(&ds).0 {
            let dir = p.join(&d);
            let by = fsx::first_existing(&dir, &["buf.yaml", "buf.yml"]);
            if by.is_none() {
                out.config(dir.join("buf.yaml"));
            }
            roots.extend(module_roots(&dir, by, out));
            out.input(dir.join("buf.lock"));
        }
    } else {
        if buf_yaml.is_none() {
            out.config(p.join("buf.yaml"));
        }
        roots.extend(module_roots(&p, buf_yaml, out));
    }
    let dirs = roots.iter().map(|m| m.dir.clone()).collect();
    out.memberships.extend(roots);
    dirs
}

/// Module roots of a directory from its `buf.yaml` (v2 `modules`, v1
/// `build.roots`).
fn module_roots(dir: &Path, buf_yaml: Option<PathBuf>, out: &mut TaskSpec) -> Vec<Membership> {
    let mut res = Vec::new();
    let v = buf_yaml.as_ref().and_then(|b| config::load(b, out));
    let mk = |d: PathBuf, excl: Vec<String>, root: &Path| {
        let rel_ex: Vec<String> = excl
            .iter()
            .filter_map(|e| {
                let ep = fsx::join(root, e);
                ep.strip_prefix(&d)
                    .ok()
                    .map(|r| format!("{}/**", r.to_string_lossy()))
            })
            .collect();
        Membership {
            dir: d,
            pattern: "**/*.proto".into(),
            exclude: rel_ex,
            reason: "buf module".into(),
            data: false,
            own: false,
        }
    };
    if let Some(v) = &v {
        if let Some(ConfigVal::Arr(mods)) = get(v, "modules") {
            for m in mods {
                let path = get(&m, "path")
                    .as_ref()
                    .and_then(as_str)
                    .unwrap_or(".".into());
                let excl = get(&m, "excludes")
                    .map(|e| str_list(&e).0)
                    .unwrap_or_default();
                res.push(mk(fsx::join(dir, &path), excl, dir));
            }
            return res;
        }
        let build = get(v, "build");
        let roots = build
            .as_ref()
            .and_then(|b| get(b, "roots"))
            .map(|r| str_list(&r).0)
            .unwrap_or_default();
        let excl = build
            .as_ref()
            .and_then(|b| get(b, "excludes"))
            .or_else(|| get(v, "excludes"))
            .map(|e| str_list(&e).0)
            .unwrap_or_default();
        if !roots.is_empty() {
            for r in roots {
                res.push(mk(fsx::join(dir, &r), excl.clone(), dir));
            }
            return res;
        }
        res.push(mk(dir.to_path_buf(), excl, dir));
        return res;
    }
    res.push(mk(dir.to_path_buf(), Vec::new(), dir));
    res
}

/// buf.gen.yaml `plugins`: tools and outputs.
fn plugins(
    cmd: &Command,
    tv: &ConfigVal,
    base: &Path,
    module_dirs: &[PathBuf],
    tpl_dir: &Path,
    out: &mut TaskSpec,
) {
    let list = match get(tv, "plugins") {
        Some(ConfigVal::Arr(a)) => a,
        Some(other) => {
            unknown(
                out,
                tpl_dir,
                UncertaintyKind::ParseIncomplete,
                format!("buf plugins: {}", config::describe(&other)),
            );
            return;
        }
        None => {
            unknown(
                out,
                tpl_dir,
                UncertaintyKind::ParseIncomplete,
                "buf.gen.yaml without plugins",
            );
            return;
        }
    };
    for p in list {
        let mut name = String::new();
        for k in ["local", "name", "plugin", "remote", "protoc_builtin"] {
            if let Some(v) = get(&p, k) {
                let (ss, _) = str_list(&v);
                let Some(first) = ss.first() else { continue };
                name = match k {
                    "remote" | "plugin" if first.contains('/') => {
                        out.tool("buf", first.split(':').next().unwrap_or(first));
                        first.rsplit('/').next().unwrap_or(first).to_string()
                    }
                    "local" | "name" | "plugin" => {
                        plugin_binary(cmd, first, out);
                        first
                            .rsplit('/')
                            .next()
                            .unwrap_or(first)
                            .trim_start_matches("protoc-gen-")
                            .to_string()
                    }
                    _ => first.clone(),
                };
                break;
            }
        }
        let Some(o) = get(&p, "out").as_ref().and_then(as_str) else {
            unknown(
                out,
                tpl_dir,
                UncertaintyKind::ParseIncomplete,
                format!("buf plugin `{name}` without `out`"),
            );
            continue;
        };
        let dir = fsx::join(base, &o);
        add_plugin_output(cmd, &name, dir, module_dirs, &[], out);
    }
}

/// A local plugin binary: `node_modules/.bin/x` is an npm package, a path an
/// input, a bare name a system tool.
fn plugin_binary(cmd: &Command, bin: &str, out: &mut TaskSpec) {
    if let Some((_, b)) = bin.split_once("node_modules/.bin/") {
        out.tool("npm", npm_plugin_package(b));
    } else if bin.contains('/') {
        let p = cmd.path(bin);
        out.input(p);
    } else {
        let full = if bin.starts_with("protoc-gen-") {
            bin.to_string()
        } else {
            format!("protoc-gen-{bin}")
        };
        out.tool("system", full);
    }
}

/// The npm package that provides a protoc plugin binary.
fn npm_plugin_package(bin: &str) -> &str {
    match bin {
        "protoc-gen-ts_proto" => "ts-proto",
        "protoc-gen-es" => "@bufbuild/protoc-gen-es",
        "protoc-gen-connect-es" => "@connectrpc/protoc-gen-connect-es",
        "protoc-gen-ts" => "protoc-gen-ts",
        "protoc-gen-grpc" | "grpc_tools_node_protoc_plugin" => "grpc-tools",
        _ => bin,
    }
}

/// Output file patterns of well-known plugins.
fn plugin_pattern(name: &str) -> Option<&'static str> {
    Some(match name {
        "go" => "**/*.pb.go",
        "go-grpc" | "go_grpc" => "**/*_grpc.pb.go",
        "grpc-gateway" => "**/*.pb.gw.go",
        "openapiv2" => "**/*.swagger.json",
        "validate" | "validate-go" => "**/*.pb.validate.go",
        "connect-go" => "**/*connect/*.connect.go",
        "twirp" => "**/*.twirp.go",
        "gogo" | "gogofast" | "gogofaster" | "gogoslick" => "**/*.pb.go",
        "python" => "**/*_pb2.py",
        "pyi" | "mypy" => "**/*_pb2.pyi",
        "grpc_python" | "grpc-python" => "**/*_pb2_grpc.py",
        "mypy_grpc" => "**/*_pb2_grpc.pyi",
        "betterproto" | "python_betterproto" => "**/*.py",
        "js" => "**/*_pb.js",
        "grpc-web" => "**/*_grpc_web_pb.{js,d.ts}",
        "es" => "**/*_pb.{js,ts,d.ts}",
        "connect-es" => "**/*_connect.{js,ts,d.ts}",
        "connect-query" => "**/*_connectquery.{js,ts,d.ts}",
        "cpp" => "**/*.pb.{h,cc}",
        "grpc_cpp" | "grpc-cpp" => "**/*.grpc.pb.{h,cc}",
        "java" | "grpc-java" => "**/*.java",
        "kotlin" => "**/*.kt",
        "csharp" | "grpc_csharp" => "**/*.cs",
        "ruby" => "**/*_pb.rb",
        "grpc_ruby" => "**/*_services_pb.rb",
        "php" => "**/*.php",
        "swift" => "**/*.pb.swift",
        "grpc-swift" => "**/*.grpc.swift",
        "dart" => "**/*.pb*.dart",
        "rust" | "prost" | "tonic" => "**/*.rs",
        "objc" => "**/*.pbobjc.{h,m}",
        _ => return None,
    })
}

/// Adds a plugin's output directory: a known plugin's pattern, else files named
/// after the inputs (`stems`), else the whole directory when it holds no
/// inputs.
fn add_plugin_output(
    cmd: &Command,
    plugin: &str,
    dir: PathBuf,
    input_dirs: &[PathBuf],
    stems: &[String],
    out: &mut TaskSpec,
) {
    let name = plugin.trim_start_matches("protoc-gen-");
    let ext = Path::new(&dir).extension().and_then(|e| e.to_str());
    if matches!(ext, Some("zip" | "jar" | "srcjar")) {
        out.output(dir);
        return;
    }
    if let Some(p) = plugin_pattern(name) {
        output_pattern(out, dir, p, &[]);
        return;
    }
    let shared = dir == cmd.cwd || input_dirs.iter().any(|i| i.starts_with(&dir));
    if !shared {
        out.output(dir);
    } else if !stems.is_empty() {
        let pat = if stems.len() == 1 {
            format!("**/{}*", stems[0])
        } else {
            format!("**/{{{}}}*", stems.join(","))
        };
        output_pattern(out, dir, &pat, &["**/*.proto".to_string()]);
    } else {
        output_pattern(out, dir, "**", &["**/*.proto".to_string()]);
    }
}

/// `protoc …` and wrappers.
pub struct Protoc;

impl Recognizer for Protoc {
    fn id(&self) -> &'static str {
        "protoc"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "protoc",
            "grpc_tools.protoc",
            "grpc_tools_node_protoc",
            "grpc_tools_ruby_protoc",
        ]
    }
    fn matches(&self, cmd: &Command) -> bool {
        cmd.program_name()
            .is_some_and(|p| self.programs().contains(&p) || p.starts_with("protoc-gen-"))
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        let prog = cmd.program_name().unwrap_or("protoc");
        match prog {
            "grpc_tools.protoc" => out.tool("pypi", "grpcio-tools"),
            "grpc_tools_node_protoc" => out.tool("npm", "grpc-tools"),
            "grpc_tools_ruby_protoc" => out.tool("gem", "grpc-tools"),
            "protoc" => out.tool("system", "protoc"),
            p => {
                // A plugin run directly reads a CodeGeneratorRequest from stdin.
                plugin_binary(cmd, p, out);
                unknown(
                    out,
                    cx.package_root,
                    UncertaintyKind::UnknownCommand,
                    format!("protoc plugin `{p}` run directly"),
                );
                return finish(out, before);
            }
        }
        let mut includes: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        let mut broad = false;
        let mut outs: Vec<(String, String)> = Vec::new();
        let words: Vec<&crate::spec::Word> = cmd.arg_words().iter().collect();
        let mut i = 0;
        while i < words.len() {
            let w = &words[i].text;
            i += 1;
            let next = |i: &mut usize| -> Option<String> {
                let v = words.get(*i).map(|w| w.text.clone());
                *i += 1;
                v
            };
            if let Some(v) = w.strip_prefix("--proto_path=") {
                includes.push(cmd.path(v));
            } else if w == "-I" || w == "--proto_path" {
                if let Some(v) = next(&mut i) {
                    includes.push(cmd.path(&v));
                }
            } else if let Some(v) = w.strip_prefix("-I") {
                includes.push(cmd.path(v.trim_start_matches('=')));
            } else if let Some(v) = w.strip_prefix("--plugin=") {
                let (_, path) = v.split_once('=').unwrap_or(("", v));
                plugin_binary(cmd, path, out);
            } else if let Some(v) = w
                .strip_prefix("--descriptor_set_out=")
                .or_else(|| w.strip_prefix("--dependency_out="))
                .or_else(|| w.strip_prefix("-o"))
            {
                if !v.is_empty() {
                    out.output(cmd.path(v.trim_start_matches('=')));
                }
            } else if let Some(v) = w.strip_prefix("--descriptor_set_in=") {
                for f in v.split(':') {
                    out.input(cmd.path(f));
                }
            } else if let Some(v) = w.strip_prefix('@') {
                out.input(cmd.path(v));
                unknown(
                    out,
                    &cmd.cwd,
                    UncertaintyKind::ShellUnsupported,
                    format!("protoc arguments file `@{v}`"),
                );
            } else if w.starts_with("--") && w.contains("_out") {
                let body = &w[2..];
                let (k, v) = match body.split_once('=') {
                    Some((k, v)) => (k.to_string(), v.to_string()),
                    None => (body.to_string(), next(&mut i).unwrap_or_default()),
                };
                let plugin = k.trim_end_matches("_out").to_string();
                // `--go_out=plugins=grpc:dir`: the directory is after the last `:`.
                let dir = v.rsplit(':').next().unwrap_or(&v).to_string();
                outs.push((plugin, dir));
            } else if w.starts_with('-') {
                // `--go_opt=…`, `--include_imports`,
                // `--experimental_allow_proto3_optional`.
            } else {
                let word = words[i - 1];
                if word.glob || crate::glob::is_glob(w) {
                    broad = true;
                    crate::recognizers::util::add_glob(&cmd.cwd, w, "protoc inputs", out);
                } else {
                    let p = cmd.path(w);
                    if fsx::is_dir(&p) {
                        broad = true;
                        out.membership(p, "**/*.proto", "protoc inputs");
                    } else {
                        files.push(p);
                    }
                }
            }
        }
        if includes.is_empty() {
            includes.push(cmd.cwd.clone());
        }
        if broad {
            for d in &includes {
                out.membership(d.clone(), "**/*.proto", "protoc include path");
            }
        }
        proto_closure(&files, &includes, out);
        let stems: Vec<String> = files
            .iter()
            .filter_map(|f| f.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        let mut input_dirs: Vec<PathBuf> = files.iter().map(|f| config::dir_of(f)).collect();
        input_dirs.extend(includes.iter().cloned());
        if outs.is_empty() && files.is_empty() && !broad && out.uncertainties.len() == before {
            // `protoc --version`, `protoc --help`.
            return Recognition::Ignored;
        }
        for (plugin, dir) in outs {
            let d = cmd.path(&dir);
            if plugin_pattern(&plugin).is_none() && !matches!(plugin.as_str(), "") {
                out.tool("system", format!("protoc-gen-{plugin}"));
            }
            add_plugin_output(cmd, &plugin, d, &input_dirs, &stems, out);
        }
        finish(out, before)
    }
}

/// Adds `files` and every proto they import (transitively, through `includes`)
/// as inputs.
pub fn proto_closure(files: &[PathBuf], includes: &[PathBuf], out: &mut TaskSpec) {
    let mut todo: Vec<PathBuf> = files.to_vec();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(f) = todo.pop() {
        if !seen.insert(f.clone()) || seen.len() > 4096 {
            continue;
        }
        out.input(f.clone());
        let Ok(text) = fsx::read_to_string(&f) else {
            continue;
        };
        for imp in proto_imports(&text) {
            let found = includes
                .iter()
                .map(|d| fsx::join(d, &imp))
                .find(|p| fsx::is_file(p));
            match found {
                Some(p) => todo.push(p),
                None if imp.starts_with("google/protobuf/") => {}
                None => {
                    // Probes: the import resolves once one of these exists.
                    for d in includes {
                        out.input(fsx::join(d, &imp));
                    }
                }
            }
        }
    }
}

/// The `import "x.proto";` paths of a proto file (`public`/`weak` too).
pub fn proto_imports(text: &str) -> Vec<String> {
    let mut res = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let mut l = line.trim();
        if in_block {
            match l.find("*/") {
                Some(e) => {
                    in_block = false;
                    l = l[e + 2..].trim();
                }
                None => continue,
            }
        }
        if let Some(s) = l.find("/*")
            && !l[s..].contains("*/")
        {
            in_block = true;
            l = l[..s].trim();
        }
        let Some(rest) = l.strip_prefix("import") else {
            continue;
        };
        if !rest.starts_with([' ', '\t', '"', '\'']) {
            continue;
        }
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix("public")
            .or_else(|| rest.strip_prefix("weak"))
            .unwrap_or(rest)
            .trim_start();
        let q = rest.chars().next();
        if let Some(q @ ('"' | '\'')) = q
            && let Some(end) = rest[1..].find(q)
        {
            res.push(rest[1..1 + end].to_string());
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports() {
        let t = "syntax = \"proto3\";\nimport \"a/b.proto\";\nimport public 'c.proto';\n// import \
                 \"x.proto\";\n/* import \"y.proto\"; */\nimport weak \"d.proto\";\n";
        assert_eq!(proto_imports(t), ["a/b.proto", "c.proto", "d.proto"]);
    }

    #[test]
    fn entries_helper() {
        let v = config::parse_yaml("a: 1\nb: 2\n").unwrap();
        assert_eq!(config::entries(&v).unwrap().len(), 2);
    }
}
