//! tox environments (`tox.ini`, `tox.toml`, `pyproject.toml` `[tool.tox]`) and
//! nox sessions (`noxfile.py`).
//!
//! Every tox environment is `tox:<dir>:<env>`, every nox session
//! `nox:<dir>:<session>`.
//!
//! tox:
//!
//! | Setting | Becomes |
//! |---|---|
//! | `envlist` / `env_list` (brace and factor expansion) plus `[testenv:x]` sections | the environments |
//! | `commands_pre`, `commands`, `commands_post` (factor-conditional lines, `{toxinidir}`, `{envname}`, `{env:X:default}`, `{[section]key}` substituted) | command lines; `{posargs}` becomes its default (or nothing) |
//! | `deps` | `-r`/`-c` requirement files are inputs, package names toolchain inputs |
//! | `changedir` | the working directory |
//! | `setenv` | environment for the commands (`file|path` entries are inputs) |
//! | `passenv` | env names |
//! | package install (unless `skip_install`) | `pyproject.toml`/`setup.py`/`setup.cfg` configs |
//!
//! Native TOML configuration (`env_run_base`, `env.<name>`, commands as
//! argument arrays) is read the same way, and `legacy_tox_ini` is parsed as
//! INI.
//!
//! nox: `@nox.session` functions are scanned for `session.run(...)` /
//! `session.run_always(...)` / `session.install(...)` / `session.chdir(...)` /
//! `session.notify(...)` calls with string-literal arguments
//! (`*session.posargs` expands to nothing; other expressions make the word
//! computed). Calls of other functions with the session are uncertain.
//! `noxfile.py` is a config and a Python entry.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::{Args, opt, opts},
    registry::{Recognition, RecognizeCx, Recognizer},
    sources::{DiscoveredTask, SourceCx, TaskSource, taskfile::repo_rel},
    spec::{Command, EntryLang, TaskId, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&Tox, &Nox]
}

/// Recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&ToxCli, &NoxCli]
}

/// An unknown substitution: an unknown shell variable, so the word is computed.
const UNKNOWN: &str = "${__TOX_UNKNOWN}";

/// A parsed INI file: sections in order, each with keys in order (values with
/// continuation lines joined by `\n`).
pub type Ini = Vec<(String, Vec<(String, String)>)>;

/// Parses INI (`[section]`, `key = value`, indented continuation lines, `#`/`;`
/// comments).
pub fn parse_ini(src: &str) -> Ini {
    let mut out: Ini = Vec::new();
    let mut cur_key: Option<(usize, usize)> = None;
    for raw in src.lines() {
        let line = raw.trim_end_matches('\r');
        let t = line.trim();
        if t.starts_with('#') || t.starts_with(';') {
            continue;
        }
        if t.is_empty() {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented && let Some((s, k)) = cur_key {
            let v = &mut out[s].1[k].1;
            if !v.is_empty() {
                v.push('\n');
            }
            v.push_str(t);
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            out.push((t[1..t.len() - 1].trim().to_string(), Vec::new()));
            cur_key = None;
            continue;
        }
        let Some(sec) = out.len().checked_sub(1) else {
            continue;
        };
        let (k, v) = match (t.find('='), t.find(':')) {
            (Some(e), Some(c)) if c < e => t.split_at(c),
            (Some(e), _) => t.split_at(e),
            (None, Some(c)) => t.split_at(c),
            (None, None) => continue,
        };
        out[sec]
            .1
            .push((k.trim().to_string(), v[1..].trim().to_string()));
        cur_key = Some((sec, out[sec].1.len() - 1));
    }
    out
}

pub(crate) fn ini_get<'a>(ini: &'a Ini, section: &str, key: &str) -> Option<&'a str> {
    ini.iter()
        .filter(|(s, _)| s == section)
        .flat_map(|(_, kv)| kv.iter())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Splits `py38-django` into factors.
fn factors(env: &str) -> Vec<&str> {
    env.split('-').collect()
}

/// Applies factor conditions: keeps lines whose `f1,f2-f3:` prefix matches
/// `env` (and unconditional lines), with the prefix removed.
fn factor_filter(value: &str, env: &str) -> Vec<String> {
    let fs = factors(env);
    let mut out = Vec::new();
    for line in value.lines() {
        let t = line.trim();
        if let Some((cond, rest)) = t.split_once(':')
            && !cond.is_empty()
            && cond
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'!' | b',' | b'-'))
            && (rest.is_empty() || rest.starts_with(' '))
        {
            let ok = cond.split(',').any(|alt| {
                alt.split('-').all(|f| match f.strip_prefix('!') {
                    Some(n) => !fs.contains(&n),
                    None => fs.contains(&f),
                })
            });
            if ok {
                out.push(rest.trim().to_string());
            }
            continue;
        }
        out.push(t.to_string());
    }
    out
}

/// One environment's settings.
#[derive(Clone, Debug, Default)]
struct EnvCfg {
    commands: Vec<String>,
    deps: Vec<String>,
    changedir: Option<String>,
    setenv: Vec<(String, String)>,
    passenv: Vec<String>,
    skip_install: bool,
}

struct IniCx<'a> {
    ini: &'a Ini,
    dir: &'a Path,
    env: &'a str,
    setenv: BTreeMap<String, String>,
}

impl IniCx<'_> {
    /// The value of `key` for this environment (env section, then `[testenv]`).
    fn get(&self, key: &str) -> Option<String> {
        let alt = key.replace('_', "");
        for sec in env_sections(self.ini, self.env)
            .into_iter()
            .chain(["testenv".to_string()])
        {
            for k in [key, alt.as_str()] {
                if let Some(v) = ini_get(self.ini, &sec, k) {
                    return Some(v.to_string());
                }
            }
        }
        None
    }

    fn subst(&self, s: &str, depth: usize) -> String {
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            // Nested braces (`{env:A:{env:B}}`) are rare: take up to the matching `}`.
            let mut d = 0;
            let mut end = None;
            for (j, c) in after.char_indices() {
                match c {
                    '{' => d += 1,
                    '}' if d == 0 => {
                        end = Some(j);
                        break;
                    }
                    '}' => d -= 1,
                    _ => {}
                }
            }
            let Some(j) = end else {
                out.push_str(&rest[i..]);
                return out;
            };
            out.push_str(&self.eval(&after[..j], depth));
            rest = &after[j + 1..];
        }
        out.push_str(rest);
        out
    }

    fn eval(&self, e: &str, depth: usize) -> String {
        let dir = self.dir.to_string_lossy();
        let work = format!("{dir}/.tox");
        match e {
            "toxinidir" | "tox_root" => return dir.into_owned(),
            "envname" | "env_name" => return self.env.to_string(),
            "toxworkdir" | "work_dir" => return work,
            "envdir" | "env_dir" => return format!("{work}/{}", self.env),
            "envtmpdir" | "env_tmp_dir" => return format!("{work}/{}/tmp", self.env),
            "envlogdir" | "env_log_dir" => return format!("{work}/{}/log", self.env),
            "envbindir" | "env_bin_dir" => return format!("{work}/{}/bin", self.env),
            "posargs" => return String::new(),
            "/" => return "/".into(),
            ":" => return ":".into(),
            _ => {}
        }
        if let Some(d) = e.strip_prefix("posargs:") {
            return self.subst(d, depth + 1);
        }
        if let Some(rest) = e.strip_prefix("env:") {
            let (var, def) = match rest.split_once(':') {
                Some((v, d)) => (v, Some(d)),
                None => (rest, None),
            };
            return match (self.setenv.get(var), def) {
                (Some(v), _) => v.clone(),
                (None, Some(d)) => self.subst(d, depth + 1),
                (None, None) => UNKNOWN.into(),
            };
        }
        if let Some(rest) = e.strip_prefix('[')
            && let Some((sec, key)) = rest.split_once(']')
            && depth < 8
        {
            let sec = if sec.is_empty() { "testenv" } else { sec };
            if let Some(v) = ini_get(self.ini, sec, key) {
                return self.subst(v, depth + 1);
            }
        }
        UNKNOWN.into()
    }

    fn cfg(mut self) -> EnvCfg {
        let mut cfg = EnvCfg::default();
        for l in factor_filter(&self.get("setenv").unwrap_or_default(), self.env) {
            if let Some((k, v)) = l.split_once('=') {
                let v = self.subst(v.trim(), 0);
                self.setenv.insert(k.trim().to_string(), v.clone());
                cfg.setenv.push((k.trim().to_string(), v));
            } else if let Some(f) = l.strip_prefix("file|") {
                cfg.setenv.push(("file|".into(), self.subst(f, 0)));
            }
        }
        for key in ["commands_pre", "commands", "commands_post"] {
            let v = self.get(key).unwrap_or_default();
            let mut joined: Vec<String> = Vec::new();
            for l in factor_filter(&v, self.env) {
                match joined.last_mut() {
                    Some(prev) if prev.ends_with('\\') => {
                        prev.pop();
                        prev.push(' ');
                        prev.push_str(&l);
                    }
                    _ => joined.push(l),
                }
            }
            for l in joined {
                let l = l.trim_start_matches(['-', '!']).trim();
                if !l.is_empty() {
                    cfg.commands.push(self.subst(l, 0));
                }
            }
        }
        for l in factor_filter(&self.get("deps").unwrap_or_default(), self.env) {
            if !l.is_empty() {
                cfg.deps.push(self.subst(&l, 0));
            }
        }
        cfg.changedir = self
            .get("changedir")
            .or_else(|| self.get("change_dir"))
            .map(|c| self.subst(&c, 0));
        for l in factor_filter(&self.get("passenv").unwrap_or_default(), self.env) {
            cfg.passenv.extend(
                l.split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            );
        }
        cfg.skip_install =
            matches!(
                self.get("skip_install").as_deref().map(str::trim),
                Some("true" | "True" | "1")
            ) || matches!(
                ini_get(self.ini, "tox", "skipsdist").map(str::trim),
                Some("true" | "True" | "1")
            ) || matches!(self.get("package").as_deref().map(str::trim), Some("skip"));
        cfg
    }
}

/// Sections that configure `env` (`testenv:env`, or a section whose brace
/// expansion includes it).
fn env_sections(ini: &Ini, env: &str) -> Vec<String> {
    ini.iter()
        .filter_map(|(s, _)| {
            let name = s.strip_prefix("testenv:")?;
            expand_env_names(name)
                .iter()
                .any(|n| n == env)
                .then(|| s.clone())
        })
        .collect()
}

/// Expands `py{38,39}-{a,b}` and splits `a, b` / newline lists.
fn expand_env_names(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in split_env_list(s) {
        for e in crate::glob::expand_braces(&part) {
            let e = e.trim().to_string();
            if !e.is_empty() && !out.contains(&e) {
                out.push(e);
            }
        }
    }
    out
}

/// Splits on commas and newlines outside braces.
fn split_env_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut d = 0i32;
    for c in s.chars() {
        match c {
            '{' => {
                d += 1;
                cur.push(c);
            }
            '}' => {
                d -= 1;
                cur.push(c);
            }
            ',' | '\n' if d <= 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// A tox configuration: the file and every environment's settings.
struct ToxConfig {
    file: PathBuf,
    envs: Vec<(String, EnvCfg)>,
}

fn ini_config(file: PathBuf, src: &str, dir: &Path) -> ToxConfig {
    let ini = parse_ini(src);
    let mut names: Vec<String> = Vec::new();
    for key in ["envlist", "env_list"] {
        if let Some(v) = ini_get(&ini, "tox", key) {
            names.extend(expand_env_names(v));
        }
    }
    for (s, _) in &ini {
        if let Some(n) = s.strip_prefix("testenv:") {
            names.extend(expand_env_names(n));
        }
    }
    let mut seen = Vec::new();
    names.retain(|n| {
        let fresh = !seen.contains(n);
        seen.push(n.clone());
        fresh
    });
    let envs = names
        .into_iter()
        .map(|n| {
            let cfg = IniCx {
                ini: &ini,
                dir,
                env: &n,
                setenv: BTreeMap::new(),
            }
            .cfg();
            (n, cfg)
        })
        .collect();
    ToxConfig { file, envs }
}

/// Quotes one argument for a command line.
fn quote(a: &str) -> String {
    if !a.is_empty()
        && a.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%^".contains(&b))
    {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

fn toml_config(file: PathBuf, t: &toml::Table, dir: &Path) -> ToxConfig {
    let mut names: Vec<String> = Vec::new();
    for key in ["env_list", "envlist"] {
        if let Some(a) = t.get(key).and_then(|v| v.as_array()) {
            names.extend(a.iter().filter_map(|v| v.as_str().map(str::to_string)));
        }
    }
    let envs_t = t.get("env").and_then(|v| v.as_table());
    if let Some(e) = envs_t {
        for k in e.keys() {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    let base = t.get("env_run_base").and_then(|v| v.as_table());
    let mut envs = Vec::new();
    for n in names {
        let own = envs_t.and_then(|e| e.get(&n)).and_then(|v| v.as_table());
        let get = |k: &str| {
            own.and_then(|o| o.get(k))
                .or_else(|| base.and_then(|b| b.get(k)))
        };
        let sub = |s: &str| {
            let ini = Ini::new();
            IniCx {
                ini: &ini,
                dir,
                env: &n,
                setenv: BTreeMap::new(),
            }
            .subst(s, 0)
        };
        let mut cfg = EnvCfg::default();
        for key in ["commands_pre", "commands", "commands_post"] {
            if let Some(a) = get(key).and_then(|v| v.as_array()) {
                for c in a {
                    let Some(args) = c.as_array() else { continue };
                    let mut words = Vec::new();
                    for a in args {
                        match a {
                            toml::Value::String(s) => words.push(quote(&sub(s))),
                            toml::Value::Table(o)
                                if o.get("replace").and_then(|r| r.as_str()) == Some("posargs") =>
                            {
                                if let Some(d) = o.get("default").and_then(|d| d.as_array()) {
                                    words.extend(
                                        d.iter().filter_map(|x| x.as_str()).map(|s| quote(&sub(s))),
                                    );
                                }
                            }
                            _ => words.push(UNKNOWN.into()),
                        }
                    }
                    if !words.is_empty() {
                        cfg.commands.push(words.join(" "));
                    }
                }
            }
        }
        if let Some(a) = get("deps").and_then(|v| v.as_array()) {
            cfg.deps
                .extend(a.iter().filter_map(|v| v.as_str()).map(sub));
        }
        cfg.changedir = get("change_dir").and_then(|v| v.as_str()).map(sub);
        if let Some(o) = get("set_env").and_then(|v| v.as_table()) {
            for (k, v) in o {
                if let Some(s) = v.as_str() {
                    cfg.setenv.push((k.clone(), sub(s)));
                }
            }
        }
        if let Some(a) = get("pass_env").and_then(|v| v.as_array()) {
            cfg.passenv
                .extend(a.iter().filter_map(|v| v.as_str().map(str::to_string)));
        }
        cfg.skip_install = get("skip_install").and_then(|v| v.as_bool()) == Some(true)
            || get("package").and_then(|v| v.as_str()) == Some("skip");
        envs.push((n, cfg));
    }
    ToxConfig { file, envs }
}

/// The tox configuration of `dir`: `tox.ini`, else `tox.toml`, else
/// `pyproject.toml` `[tool.tox]` (native or `legacy_tox_ini`).
fn tox_config(dir: &Path) -> Option<ToxConfig> {
    let ini = dir.join("tox.ini");
    if let Ok(s) = fsx::read_to_string(&ini) {
        return Some(ini_config(ini, &s, dir));
    }
    let tt = dir.join("tox.toml");
    if let Ok(s) = fsx::read_to_string(&tt) {
        let t: toml::Table = toml::from_str(&s).unwrap_or_default();
        return Some(toml_config(tt, &t, dir));
    }
    let pp = dir.join("pyproject.toml");
    let s = fsx::read_to_string(&pp).ok()?;
    let t: toml::Table = toml::from_str(&s).ok()?;
    let tox = t.get("tool")?.get("tox")?.as_table()?;
    if let Some(legacy) = tox.get("legacy_tox_ini").and_then(|v| v.as_str()) {
        return Some(ini_config(pp, legacy, dir));
    }
    Some(toml_config(pp, tox, dir))
}

/// tox environments.
pub struct Tox;

impl TaskSource for Tox {
    fn id(&self) -> &'static str {
        "tox"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["tox.ini", "tox.toml", "pyproject.toml"]
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let dir = &cx.package.dir;
        let Some(cfg) = tox_config(dir) else {
            return Vec::new();
        };
        let rel = repo_rel(&cx.workspace.root, dir);
        let mut out = Vec::new();
        for (name, e) in cfg.envs {
            let cwd = e
                .changedir
                .as_deref()
                .map_or_else(|| dir.clone(), |c| fsx::join(dir, c));
            let mut t = DiscoveredTask::new(
                TaskId::other("tox", format!("{rel}:{name}")),
                name.clone(),
                cwd,
            );
            t.files.push(cfg.file.clone());
            t.lines = e.commands.clone();
            t.spec.tool("pypi", "tox");
            for (k, v) in &e.setenv {
                if k == "file|" {
                    t.spec.input(fsx::join(dir, v));
                } else if !v.contains(UNKNOWN) {
                    t.env.insert(k.clone(), v.clone());
                }
            }
            t.spec.env.extend(e.passenv.iter().cloned());
            requirements(&e.deps, dir, &mut t.spec);
            if !e.skip_install {
                package_configs(dir, &mut t.spec);
            }
            out.push(t);
        }
        crate::sources::gha::script::sanitize_lines(&mut out);
        out
    }
}

/// `deps` / `session.install` arguments: requirement and constraint files are
/// inputs, local paths (`.`, `-e .`) the project configs, names toolchain
/// packages.
fn requirements(deps: &[String], dir: &Path, spec: &mut TaskSpec) {
    let mut words: Vec<String> = Vec::new();
    for d in deps {
        words.extend(d.split_whitespace().map(str::to_string));
    }
    let mut i = 0;
    while i < words.len() {
        let w = words[i].as_str();
        i += 1;
        let file = if matches!(w, "-r" | "-c" | "--requirement" | "--constraint") {
            i += 1;
            words.get(i - 1).cloned()
        } else {
            w.strip_prefix("-r")
                .or_else(|| w.strip_prefix("-c"))
                .or_else(|| w.strip_prefix("--requirement="))
                .or_else(|| w.strip_prefix("--constraint="))
                .map(str::to_string)
        };
        if let Some(f) = file {
            if !f.contains(UNKNOWN) && !f.contains("${") {
                spec.input(fsx::join(dir, &f));
            }
            continue;
        }
        if w == "-e" || w == "--editable" {
            continue;
        }
        if w.starts_with('-') || w.contains(UNKNOWN) {
            continue;
        }
        if w == "." || w.starts_with("./") || w.starts_with("../") || w.starts_with('/') {
            let p = fsx::join(dir, w.split('[').next().unwrap_or(w));
            package_configs(&p, spec);
            continue;
        }
        let name: String = w
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        if !name.is_empty() {
            spec.tool("pypi", name);
        }
    }
}

fn package_configs(dir: &Path, spec: &mut TaskSpec) {
    for n in ["pyproject.toml", "setup.py", "setup.cfg"] {
        let p = dir.join(n);
        if fsx::is_file(&p) {
            spec.config(p);
        }
    }
}

/// `tox [-e a,b] [-c file] [-- args]`.
pub struct ToxCli;

impl Recognizer for ToxCli {
    fn id(&self) -> &'static str {
        "tox"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["tox"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("pypi", "tox");
        let args = Args::new(
            cmd,
            0,
            &[
                "-e",
                "-c",
                "--conf",
                "-p",
                "--parallel",
                "-m",
                "-f",
                "--root",
                "--workdir",
                "-x",
                "--override",
            ],
        )
        .collect_all();
        let dir = match opt(&args, &["-c", "--conf"]).or_else(|| opt(&args, &["--root"])) {
            Some(c) => {
                let p = cmd.path(c);
                if fsx::is_dir(&p) {
                    p
                } else {
                    p.parent().unwrap_or(&cmd.cwd).to_path_buf()
                }
            }
            None => cmd.cwd.clone(),
        };
        let rel = repo_rel(cx.repo_root, &dir);
        let mut envs: Vec<String> = opts(&args, &["-e"])
            .iter()
            .flat_map(|e| e.split(','))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if envs.is_empty()
            || envs.iter().any(|e| e == "ALL")
            || !opts(&args, &["-m", "-f"]).is_empty()
        {
            envs = tox_config(&dir)
                .map(|c| c.envs.into_iter().map(|(n, _)| n).collect())
                .unwrap_or_default();
        }
        for e in envs {
            out.dep(
                TaskRef::Id {
                    id: TaskId::other("tox", format!("{rel}:{e}")),
                },
                "tox",
            );
        }
        Recognition::Full
    }
}

/// A nox session found in a noxfile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoxSession {
    /// Name.
    pub name: String,
    /// The calls on the session, in order: (method, positional args, all
    /// literal).
    pub calls: Vec<NoxCall>,
    /// Calls of other functions with the session (`_run_tests(session)`).
    pub helper_calls: Vec<String>,
}

/// One `session.<method>(...)` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoxCall {
    /// `run`, `install`, `chdir`, `notify`, ...
    pub method: String,
    /// Positional arguments: `Some(literal)` or `None` (an expression).
    pub args: Vec<Option<String>>,
}

/// Scans a noxfile for sessions. Never panics.
pub fn scan_noxfile(src: &str) -> Vec<NoxSession> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if !(t.starts_with("@nox.session") || t.starts_with("@session")) {
            i += 1;
            continue;
        }
        // The decorator, possibly spanning lines.
        let mut deco = t.to_string();
        i += 1;
        while paren_depth(&deco) > 0 && i < lines.len() {
            deco.push('\n');
            deco.push_str(lines[i]);
            i += 1;
        }
        // Further decorators, then `def name(`.
        while i < lines.len() && !lines[i].trim_start().starts_with("def ") {
            if !lines[i].trim_start().starts_with('@') && !lines[i].trim().is_empty() {
                break;
            }
            i += 1;
        }
        let Some(def) = lines.get(i) else { break };
        let indent = def.len() - def.trim_start().len();
        let Some(fname) = def
            .trim_start()
            .strip_prefix("def ")
            .and_then(|r| r.split('(').next())
        else {
            continue;
        };
        let name = kwarg_str(&deco, "name").unwrap_or_else(|| fname.trim().to_string());
        i += 1;
        let mut body = String::new();
        while i < lines.len() {
            let l = lines[i];
            let lt = l.trim();
            let ind = l.len() - l.trim_start().len();
            if !lt.is_empty() && !lt.starts_with('#') && ind <= indent {
                break;
            }
            body.push_str(l);
            body.push('\n');
            i += 1;
        }
        out.push(scan_body(&name, &body));
    }
    out
}

pub(crate) fn paren_depth(s: &str) -> i32 {
    let mut d = 0;
    let mut q: Option<char> = None;
    for c in s.chars() {
        match q {
            Some(x) if c == x => q = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => q = Some(c),
                '(' | '[' | '{' => d += 1,
                ')' | ']' | '}' => d -= 1,
                _ => {}
            },
        }
    }
    d
}

/// `name="x"` in a decorator.
fn kwarg_str(deco: &str, key: &str) -> Option<String> {
    let i = deco.find(&format!("{key}="))?;
    if deco[..i]
        .chars()
        .last()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    py_string(deco[i + key.len() + 1..].trim_start()).map(|(s, _)| s)
}

/// A Python string literal at the start of `s` (with `r`/`b`/`u`/`f` prefixes):
/// its value (`None` for an f-string with substitutions) and its length.
pub(crate) fn py_string(s: &str) -> Option<(String, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut fstr = false;
    let mut raw = false;
    while i < b.len() && i < 3 && b[i].is_ascii_alphabetic() {
        match b[i].to_ascii_lowercase() {
            b'f' => fstr = true,
            b'r' => raw = true,
            b'b' | b'u' => {}
            _ => return None,
        }
        i += 1;
    }
    let q = *b.get(i)?;
    if q != b'\'' && q != b'"' {
        return None;
    }
    let triple = b.get(i + 1) == Some(&q) && b.get(i + 2) == Some(&q);
    let start = if triple { i + 3 } else { i + 1 };
    let mut j = start;
    let mut v = String::new();
    while j < b.len() {
        if triple {
            if b[j] == q && b.get(j + 1) == Some(&q) && b.get(j + 2) == Some(&q) {
                let lit = (!fstr || !v.contains('{')).then_some(v);
                return lit.map(|v| (v, j + 3));
            }
        } else if b[j] == q {
            let lit = (!fstr || !v.contains('{')).then_some(v);
            return lit.map(|v| (v, j + 1));
        } else if b[j] == b'\n' {
            return None;
        }
        if b[j] == b'\\' && !raw && j + 1 < b.len() {
            j += 1;
        }
        let ch_len = s[j..].chars().next().map_or(1, char::len_utf8);
        v.push_str(&s[j..j + ch_len]);
        j += ch_len;
    }
    None
}

/// The text of a call's arguments starting after `(` at `s[0..]`: up to the
/// matching `)`.
pub(crate) fn call_args(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    let mut d = 0i32;
    let mut j = 0;
    while j < b.len() {
        match b[j] {
            b'\'' | b'"' => {
                // Skip the string.
                let rest = &s[j..];
                match py_string(rest) {
                    Some((_, n)) => {
                        j += n;
                        continue;
                    }
                    None => {
                        // An f-string with substitutions or an unterminated string: skip to
                        // the closing quote on this line.
                        let q = b[j];
                        j += 1;
                        while j < b.len() && b[j] != q && b[j] != b'\n' {
                            j += 1;
                        }
                    }
                }
            }
            b'(' | b'[' | b'{' => d += 1,
            b')' | b']' | b'}' => {
                if d == 0 {
                    return s.get(..j);
                }
                d -= 1;
            }
            b'#' => {
                while j < b.len() && b[j] != b'\n' {
                    j += 1;
                }
                continue;
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Splits call arguments on top-level commas.
pub(crate) fn split_args(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut d = 0i32;
    let mut start = 0;
    let mut j = 0;
    while j < b.len() {
        match b[j] {
            b'\'' | b'"' => {
                if let Some((_, n)) = py_string(&s[j..]) {
                    j += n;
                    continue;
                }
                let q = b[j];
                j += 1;
                while j < b.len() && b[j] != q {
                    j += 1;
                }
            }
            b'(' | b'[' | b'{' => d += 1,
            b')' | b']' | b'}' => d -= 1,
            b',' if d == 0 => {
                out.push(&s[start..j]);
                start = j + 1;
            }
            b'#' => {
                // A comment: drop it from the current argument.
                let end = s[j..].find('\n').map_or(s.len(), |e| j + e);
                if d == 0 && s[start..j].trim().is_empty() {
                    start = end;
                }
                j = end;
                continue;
            }
            _ => {}
        }
        j += 1;
    }
    out.push(&s[start.min(s.len())..]);
    out.into_iter()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .collect()
}

fn scan_body(name: &str, body: &str) -> NoxSession {
    let mut s = NoxSession {
        name: name.to_string(),
        ..Default::default()
    };
    let methods = [
        "run_always",
        "run_install",
        "run",
        "install",
        "conda_install",
        "chdir",
        "cd",
        "notify",
    ];
    let mut i = 0;
    let b = body.as_bytes();
    while i < b.len() {
        // `session.<method>(`
        if body[i..].starts_with("session.")
            && (i == 0
                || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'.'))
        {
            let after = &body[i + "session.".len()..];
            if let Some(m) = methods.iter().find(|m| after.starts_with(&format!("{m}("))) {
                let argstart = i + "session.".len() + m.len() + 1;
                if let Some(args) = call_args(&body[argstart..]) {
                    let mut pos = Vec::new();
                    for a in split_args(args) {
                        if is_kwarg(a) {
                            continue;
                        }
                        if a.starts_with('*') && a.contains("posargs") {
                            continue;
                        }
                        pos.push(py_string(a).filter(|(_, n)| *n == a.len()).map(|(v, _)| v));
                    }
                    s.calls.push(NoxCall {
                        method: m.to_string(),
                        args: pos,
                    });
                    i = argstart + args.len();
                    continue;
                }
            }
        }
        // `helper(session` (not a method call on the session itself).
        if body[i..].starts_with("(session")
            && body[i + "(session".len()..]
                .chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        {
            let head = &body[..i];
            let fname: String = head
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if !fname.is_empty() && !fname.starts_with("session.") {
                s.helper_calls.push(fname);
            }
        }
        i += body[i..].chars().next().map_or(1, char::len_utf8);
    }
    s
}

pub(crate) fn is_kwarg(a: &str) -> bool {
    let n: String = a
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    !n.is_empty()
        && a[n.len()..].trim_start().starts_with('=')
        && !a[n.len()..].trim_start().starts_with("==")
}

/// nox sessions.
pub struct Nox;

impl TaskSource for Nox {
    fn id(&self) -> &'static str {
        "nox"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["noxfile.py"]
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let dir = &cx.package.dir;
        let file = dir.join("noxfile.py");
        let Ok(src) = fsx::read_to_string(&file) else {
            return Vec::new();
        };
        let rel = repo_rel(&cx.workspace.root, dir);
        let id = |n: &str| TaskId::other("nox", format!("{rel}:{n}"));
        let mut out = Vec::new();
        for s in scan_noxfile(&src) {
            let mut base = DiscoveredTask::new(id(&s.name), s.name.clone(), dir.clone());
            base.files.push(file.clone());
            base.spec.entry(file.clone(), EntryLang::Python);
            base.spec.tool("pypi", "nox");
            for h in &s.helper_calls {
                base.spec.uncertain(
                    UncertaintyKind::UnknownCommand,
                    format!("nox session `{}` calls `{h}(session, …)`", s.name),
                );
            }
            let mut cwd = dir.clone();
            let mut runs: Vec<DiscoveredTask> = Vec::new();
            for c in &s.calls {
                match c.method.as_str() {
                    "run" | "run_always" | "run_install" => {
                        let words: Vec<String> = c
                            .args
                            .iter()
                            .map(|a| match a {
                                Some(v) => quote(v),
                                None => "${__NOX_DYNAMIC}".to_string(),
                            })
                            .collect();
                        if words.is_empty() {
                            continue;
                        }
                        let mut t = DiscoveredTask::new(id(&s.name), s.name.clone(), cwd.clone());
                        t.files.push(file.clone());
                        t.lines.push(words.join(" "));
                        runs.push(t);
                    }
                    "install" | "conda_install" => {
                        let lits: Vec<String> = c.args.iter().flatten().cloned().collect();
                        requirements(&[lits.join(" ")], &cwd, &mut base.spec);
                        if c.args.iter().any(Option::is_none) {
                            base.spec.tool("pypi", "<computed>");
                        }
                    }
                    "chdir" | "cd" => match c.args.first() {
                        Some(Some(d)) => cwd = fsx::join(&cwd, d),
                        _ => base.spec.uncertain(
                            UncertaintyKind::CwdDependent,
                            format!("nox session `{}` changes to a computed directory", s.name),
                        ),
                    },
                    "notify" => match c.args.first() {
                        Some(Some(n)) => base.spec.dep(TaskRef::Id { id: id(n) }, "nox notify"),
                        _ => base.spec.uncertain(
                            UncertaintyKind::UnknownCommand,
                            format!("nox session `{}` notifies a computed session", s.name),
                        ),
                    },
                    _ => {}
                }
            }
            out.push(base);
            out.extend(runs);
        }
        crate::sources::gha::script::sanitize_lines(&mut out);
        out
    }
}

/// `nox [-s a b] [-f noxfile] [-- args]`.
pub struct NoxCli;

impl Recognizer for NoxCli {
    fn id(&self) -> &'static str {
        "nox"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["nox"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("pypi", "nox");
        let args = Args::new(
            cmd,
            0,
            &[
                "-f",
                "--noxfile",
                "-p",
                "--python",
                "-k",
                "-t",
                "--tags",
                "--envdir",
                "--default-venv-backend",
                "-db",
            ],
        )
        .collect_all();
        let file = opt(&args, &["-f", "--noxfile"])
            .map(|f| cmd.path(f))
            .unwrap_or_else(|| cmd.cwd.join("noxfile.py"));
        out.config(file.clone());
        let dir = file.parent().unwrap_or(&cmd.cwd).to_path_buf();
        let rel = repo_rel(cx.repo_root, &dir);
        // Sessions: `-s a b`, `--session a`, `-e a` (positionals after -s up to the
        // next option).
        let words: Vec<&str> = cmd.args().take_while(|w| *w != "--").collect();
        let mut names = Vec::new();
        let mut in_s = false;
        for w in words {
            if matches!(w, "-s" | "--session" | "--sessions" | "-e") {
                in_s = true;
                continue;
            }
            if w.starts_with('-') {
                in_s = false;
                continue;
            }
            if in_s {
                names.push(w.to_string());
            }
        }
        if names.is_empty() {
            names = fsx::read_to_string(&file)
                .map(|s| scan_noxfile(&s).into_iter().map(|s| s.name).collect())
                .unwrap_or_default();
        }
        for n in names {
            let base = n.split('(').next().unwrap_or(&n);
            let base = base
                .rsplit_once('-')
                .filter(|(_, v)| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
                .map_or(base, |(b, _)| b);
            out.dep(
                TaskRef::Id {
                    id: TaskId::other("nox", format!("{rel}:{base}")),
                },
                "nox",
            );
        }
        Recognition::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_and_factors() {
        let src = "[tox]\nenvlist = py{38,39}-{a,b}, lint\n\n[testenv]\ndeps =\n    \
                   -rrequirements.txt\n    pytest\ncommands =\n    py38: python -c 1\n    !py38: \
                   python -c 2\n    pytest {posargs:tests} {env:X:dflt}\nsetenv =\n    X = \
                   y\n\n[testenv:lint]\ncommands = ruff check {toxinidir}/src\n";
        let cfg = ini_config(PathBuf::from("/r/tox.ini"), src, Path::new("/r"));
        let names: Vec<&str> = cfg.envs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["py38-a", "py38-b", "py39-a", "py39-b", "lint"]);
        let py38 = &cfg.envs[0].1;
        assert_eq!(py38.commands, ["python -c 1", "pytest tests y"]);
        let py39 = &cfg.envs[2].1;
        assert_eq!(py39.commands[0], "python -c 2");
        assert_eq!(cfg.envs[4].1.commands, ["ruff check /r/src"]);
        assert_eq!(py38.deps, ["-rrequirements.txt", "pytest"]);
    }

    #[test]
    fn noxfile_scan() {
        let src = r#"import nox

@nox.session(python=["3.11"])
def tests(session):
    session.install("-r", "requirements-dev.txt")
    session.run(
        "pytest",
        "tests/unit",  # comment
        *session.posargs,
        env={"A": "b"},
    )
    session.run("python", f"{x}.py")
    session.notify("lint")
    _helper(session)

@nox.session(name="lint")
def lint_session(session):
    session.chdir("src")
    session.run("ruff", 'check', ".")
"#;
        let s = scan_noxfile(src);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "tests");
        assert_eq!(
            s[0].calls[1].args,
            [Some("pytest".into()), Some("tests/unit".into())]
        );
        assert_eq!(s[0].calls[2].args, [Some("python".into()), None]);
        assert_eq!(s[0].helper_calls, ["_helper"]);
        assert_eq!(s[1].name, "lint");
        assert_eq!(s[1].calls[0].method, "chdir");
    }

    #[test]
    fn parsers_never_panic() {
        let alphabet = b"[]=:{}, \n\t#;-!py38envlist@session.run(\"'f*)def x";
        let mut x: u64 = 0xda942042e4dd58b5;
        for _ in 0..3000 {
            let mut s = String::new();
            for _ in 0..64 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push(alphabet[(x % alphabet.len() as u64) as usize] as char);
            }
            let ini = parse_ini(&s);
            let _ = ini_config(PathBuf::from("/r/tox.ini"), &s, Path::new("/r"));
            let _ = ini;
            scan_noxfile(&s);
            let _ = py_string(&s);
            let _ = call_args(&s);
        }
        for s in [
            "@nox.session(",
            "@session\ndef",
            "session.run(\"",
            "f'{",
            "r'''",
        ] {
            scan_noxfile(s);
            py_string(s);
        }
    }
}
