//! Cargo aliases (`.cargo/config.toml` `[alias]`) and xtask crates.
//!
//! Every alias is a task `cargo:<dir>:<alias>` (`<dir>`: the directory holding
//! `.cargo/`, repo-relative) whose command line is the expansion (`xtask = "run
//! -p xtask --"` → `cargo run -p xtask --`), recognized like any cargo command.
//! For `run`/`build` of a named package (`-p X`, `--package X`,
//! `--manifest-path X/Cargo.toml`), the crate's roots (`src/main.rs`,
//! `src/lib.rs`, `src/bin/*.rs`, `build.rs`) are Rust entries, traced
//! with lang-rust, so an xtask's own sources and the files it reads count; the
//! crate manifest is a config and `rust-toolchain.toml` an input.
//!
//! The [`CargoAlias`] recognizer expands aliases used in other tasks (`cargo
//! xtask dist` in a CI step) and adds a dependency on the alias task. Unknown
//! cargo subcommands (plugins) are uncertain.

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::find_up,
    registry::{Recognition, RecognizeCx, Recognizer, registry},
    sources::{DiscoveredTask, SourceCx, TaskSource, taskfile::repo_rel},
    spec::{Command, EntryLang, TaskId, TaskRef, TaskSpec, Word},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&CargoAliases]
}

/// Recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&CargoAlias]
}

/// Built-in subcommands without file inputs worth recording.
const BUILTIN_NOOP: &[&str] = &[
    "new",
    "init",
    "add",
    "remove",
    "rm",
    "update",
    "search",
    "login",
    "logout",
    "owner",
    "yank",
    "help",
    "version",
    "clean",
    "vendor",
    "verify-project",
    "locate-project",
    "pkgid",
    "generate-lockfile",
    "report",
    "config",
    "uninstall",
    "info",
];

/// Aliases with their expansion words.
pub type Aliases = Vec<(String, Vec<String>)>;

/// The `[alias]` table of the cargo config in `dir/.cargo/`: (config file,
/// aliases).
pub fn aliases(dir: &Path) -> Option<(PathBuf, Aliases)> {
    let file = fsx::first_existing(&dir.join(".cargo"), &["config.toml", "config"])?;
    let src = fsx::read_to_string(&file).ok()?;
    let t: toml::Table = toml::from_str(&src).ok()?;
    let mut out = Vec::new();
    if let Some(a) = t.get("alias").and_then(|a| a.as_table()) {
        for (k, v) in a {
            let words = match v {
                toml::Value::String(s) => s.split_whitespace().map(str::to_string).collect(),
                toml::Value::Array(a) => a
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect(),
                _ => continue,
            };
            out.push((k.clone(), words));
        }
    }
    Some((file, out))
}

/// Quotes a word for a command line.
fn quote(a: &str) -> String {
    if !a.is_empty()
        && a.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b))
    {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

/// `.cargo/config.toml` `[alias]`.
pub struct CargoAliases;

impl TaskSource for CargoAliases {
    fn id(&self) -> &'static str {
        "cargo"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["config.toml", "config"]
    }

    /// The entries of an alias are the targets of the crate it runs, which
    /// discovery finds by listing (`src/bin`, the workspace members a glob
    /// names) in that crate, maybe another package: what it asked of the
    /// tree is each task's [`DiscoveredTask::decided_by`]
    /// ([`crate::sources::with_decided_by`]), so that a binary that comes
    /// or goes makes a store analyze the package with the alias again.
    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        crate::sources::with_decided_by(|| self.discover_aliases(cx))
    }
}

impl CargoAliases {
    fn discover_aliases(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let dir = &cx.package.dir;
        let Some((file, list)) = aliases(dir) else {
            return Vec::new();
        };
        let rel = repo_rel(&cx.workspace.root, dir);
        let mut out = Vec::new();
        for (name, words) in list {
            let mut t = DiscoveredTask::new(
                TaskId::other("cargo", format!("{rel}:{name}")),
                name.clone(),
                dir.clone(),
            );
            t.files.push(file.clone());
            let line: Vec<String> = std::iter::once("cargo".to_string())
                .chain(words.iter().map(|w| quote(w)))
                .collect();
            t.lines.push(line.join(" "));
            crate_entries(&words, dir, &cx.workspace.root, &mut t.spec);
            out.push(t);
        }
        crate::sources::gha::script::sanitize_lines(&mut out);
        out
    }
}

/// The cargo workspace root above `dir` (the topmost `Cargo.toml` with
/// `[workspace]` below the repo root), else the nearest `Cargo.toml`'s
/// directory.
pub fn workspace_root(dir: &Path, repo_root: &Path) -> Option<PathBuf> {
    let nearest = find_up(dir, "Cargo.toml", Some(repo_root))?;
    let mut root = nearest.parent()?.to_path_buf();
    for x in dir.ancestors() {
        if !x.starts_with(repo_root) {
            break;
        }
        if fsx::read_to_string(x.join("Cargo.toml")).is_ok_and(|s| s.contains("[workspace]")) {
            root = x.to_path_buf();
        }
    }
    Some(root)
}

/// The directory of the workspace member named `name` (or whose directory is
/// named so).
pub fn find_crate(ws_root: &Path, name: &str) -> Option<PathBuf> {
    let src = fsx::read_to_string(ws_root.join("Cargo.toml")).ok()?;
    let t: toml::Table = toml::from_str(&src).ok()?;
    let pkg_name = |dir: &Path| -> Option<String> {
        let s = fsx::read_to_string(dir.join("Cargo.toml")).ok()?;
        let t: toml::Table = toml::from_str(&s).ok()?;
        t.get("package")?.get("name")?.as_str().map(str::to_string)
    };
    if pkg_name(ws_root).as_deref() == Some(name) {
        return Some(ws_root.to_path_buf());
    }
    let members: Vec<String> = t
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut dirs = Vec::new();
    for m in members {
        if crate::glob::is_glob(&m) {
            let (prefix, rest) = crate::glob::split_static_prefix(&m);
            let base = fsx::join(ws_root, prefix);
            if let Some(entries) = fsx::read_dir(&base) {
                for (p, is_dir) in entries {
                    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if is_dir && crate::glob::matches(rest, n) {
                        dirs.push(p);
                    }
                }
            }
        } else {
            dirs.push(fsx::join(ws_root, &m));
        }
    }
    dirs.sort();
    dirs.iter()
        .find(|d| pkg_name(d).as_deref() == Some(name))
        .or_else(|| {
            dirs.iter().find(|d| {
                d.file_name().and_then(|n| n.to_str()) == Some(name)
                    && fsx::is_file(&d.join("Cargo.toml"))
            })
        })
        .cloned()
}

/// For `run`/`build` expansions naming a package: the crate's roots as Rust
/// entries, its manifest as a config, the toolchain file as an input.
fn crate_entries(words: &[String], cwd: &Path, repo_root: &Path, spec: &mut TaskSpec) {
    let sub = words
        .iter()
        .find(|w| !w.starts_with('-') && !w.starts_with('+'))
        .map(String::as_str);
    if !matches!(sub, Some("run" | "r" | "build" | "b")) {
        return;
    }
    let Some(ws) = workspace_root(cwd, repo_root) else {
        return;
    };
    spec.tool("system", "cargo");
    for n in ["rust-toolchain.toml", "rust-toolchain"] {
        spec.input(ws.join(n));
    }
    let mut crates = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i].as_str();
        i += 1;
        if w == "--" {
            break;
        }
        let val = |i: &mut usize| -> Option<String> {
            *i += 1;
            words.get(*i - 1).cloned()
        };
        match w {
            "-p" | "--package" => {
                if let Some(v) = val(&mut i) {
                    crates.push(find_crate(&ws, &v));
                }
            }
            "--manifest-path" => {
                if let Some(v) = val(&mut i) {
                    crates.push(fsx::join(cwd, &v).parent().map(Path::to_path_buf));
                }
            }
            _ => {
                if let Some(v) = w
                    .strip_prefix("--package=")
                    .or_else(|| w.strip_prefix("-p"))
                {
                    if !v.is_empty() {
                        crates.push(find_crate(&ws, v));
                    }
                } else if let Some(v) = w.strip_prefix("--manifest-path=") {
                    crates.push(fsx::join(cwd, v).parent().map(Path::to_path_buf));
                }
            }
        }
    }
    for c in crates.into_iter().flatten() {
        spec.config(c.join("Cargo.toml"));
        for r in ["src/main.rs", "src/lib.rs", "build.rs"] {
            let p = c.join(r);
            if fsx::is_file(&p) {
                spec.entry(p, EntryLang::Rust);
            }
        }
        if let Some(bins) = fsx::read_dir(&c.join("src/bin")) {
            for (p, is_dir) in bins {
                let main = if is_dir { p.join("main.rs") } else { p };
                if main.extension().is_some_and(|e| e == "rs") && fsx::is_file(&main) {
                    spec.entry(main, EntryLang::Rust);
                }
            }
        }
    }
}

thread_local! {
    static EXPANDING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `cargo <alias> …` (an alias from `.cargo/config.toml`, `cargo xtask …`) and
/// other cargo subcommands the cargo recognizer does not handle.
pub struct CargoAlias;

impl Recognizer for CargoAlias {
    fn id(&self) -> &'static str {
        "cargo-alias"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["cargo"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "cargo");
        let Some(pos) = cmd
            .arg_words()
            .iter()
            .position(|w| !w.text.starts_with('-') && !w.text.starts_with('+'))
        else {
            return Recognition::Ignored;
        };
        let sub = cmd.arg_words()[pos].text.clone();
        if BUILTIN_NOOP.contains(&sub.as_str()) {
            return Recognition::Ignored;
        }
        // The nearest `.cargo/config.toml` defining the alias, up to the repo root.
        let mut found = None;
        for d in cmd.cwd.ancestors() {
            if let Some((file, list)) = aliases(d)
                && let Some((_, words)) = list.into_iter().find(|(n, _)| *n == sub)
            {
                found = Some((d.to_path_buf(), file, words));
                break;
            }
            if d == cx.repo_root {
                break;
            }
        }
        let Some((dir, file, words)) = found else {
            out.uncertain(UncertaintyKind::UnknownCommand, format!("cargo {sub}"));
            cx.whole_package(out, &format!("unknown command `cargo {sub}`"));
            return Recognition::Unknown;
        };
        out.config(file);
        out.dep(
            TaskRef::Id {
                id: TaskId::other("cargo", format!("{}:{sub}", repo_rel(cx.repo_root, &dir))),
            },
            "cargo alias",
        );
        crate_entries(&words, &cmd.cwd, cx.repo_root, out);
        if EXPANDING.with(|e| e.get()) >= 4 {
            return Recognition::Full;
        }
        // Expand: `cargo <opts> <expansion…> <rest…>`.
        let mut c = cmd.clone();
        let rest: Vec<Word> = cmd.arg_words()[pos + 1..].to_vec();
        let mut argv: Vec<Word> = vec![cmd.argv[0].clone()];
        argv.extend(cmd.arg_words()[..pos].iter().cloned());
        argv.extend(words.iter().map(|w| Word::lit(w.clone())));
        argv.extend(rest);
        c.argv = argv;
        if !words.is_empty() {
            EXPANDING.with(|e| e.set(e.get() + 1));
            let r = registry().recognize(&c, cx, out).recognition;
            EXPANDING.with(|e| e.set(e.get() - 1));
            return r;
        }
        Recognition::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("run"), "run");
        assert_eq!(quote("a b"), "'a b'");
    }
}
