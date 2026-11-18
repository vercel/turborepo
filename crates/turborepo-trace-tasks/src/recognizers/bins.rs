//! Package binaries: commands whose program no recognizer knows are resolved as
//! npm package binaries or Python console scripts before the unknown-command
//! fallback.
//!
//! [`resolve_unknown`] is called by the registry's unknown-program hook (after
//! wrapper stripping, so `npx x`, `pnpm exec x`, `bunx x`, `yarn x` arrive as
//! `x`; an explicit `node_modules/.bin/x` path is handled too). Resolution
//! ([`npm`], [`python`]):
//!
//! 1. installed npm binaries: `node_modules/.bin/<name>` from the cwd up to the
//!    repo root (symlinks and shell shims), workspace packages' `bin` fields
//!    (works without `node_modules`), installed dependencies' `bin` fields;
//! 2. first-party Python console scripts (`[project.scripts]`);
//! 3. a declared npm dependency named like the program (no install).
//!
//! What it adds:
//!
//! | Found | Spec | Recognition |
//! |---|---|---|
//! | first-party JS bin | the target file as a JS `entry` (its closure and fs reads are traced), existing-file arguments as inputs, the bin package's `package.json` as config | `Full` (`Partial` plus a cwd membership if the bin reads relative to the cwd) |
//! | first-party bin wrapping a known tool ([`wrapper`]) | the above, plus the remaining argv forwarded to the tool's recognizer (`jest-dd -c x` → jest with config `x`) and an `unknown_command` uncertainty naming the inference | `Partial` |
//! | first-party Python bin (shebang) / shell bin | Python entry / `Auto` entry plus uncertainty | `Full` / `Partial` |
//! | bin target missing (a build output) | the entry as a probe, the bin package as a membership, a `generated_file` uncertainty | `Partial` |
//! | third-party bin | `tool` (`npm:<package>`, lockfile-diffed), existing-file arguments, uncertainty (inputs not modelled) | `Partial` |
//! | third-party bin of a known tool package under another name (`tsc6` from `@typescript/typescript6`) | `tool`, plus the argv forwarded to the tool's recognizer | the tool recognizer's |
//! | Python console script | the module file as a Python entry, the `pyproject.toml` as config | `Full` (`Partial` with an uncertainty if the module file is not found) |
//!
//! The registry hook adds the package-directory membership for every
//! uncertainty, as for any recognizer. [`go::GoTool`] (`go tool <x>`) is a
//! regular recognizer.

pub mod go;
pub mod npm;
pub mod python;
pub mod wrapper;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

use turborepo_trace_core::{DirCache, UncertaintyKind};
use turborepo_trace_frameworks::fsx;

use self::npm::Found;
use crate::{
    recognizers::util::add_existing_files,
    registry::{Recognition, RecognizeCx, Recognizer, registry},
    spec::{Command, EntryLang, TaskSpec, Word},
};

/// The recognizers of this module (registered before
/// [`crate::recognizers::run`], whose `go` recognizer would otherwise take `go
/// tool`).
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&go::GoTool]
}

/// Recognizer ids reported for resolved binaries.
pub mod ids {
    /// A first-party npm binary traced as an entry.
    pub const BIN: &str = "bin";
    /// A first-party binary that wraps a known tool.
    pub const WRAPPER: &str = "bin-wrapper";
    /// A third-party npm binary.
    pub const PACKAGE: &str = "bin-package";
    /// A Python console script.
    pub const CONSOLE_SCRIPT: &str = "console-script";
}

/// The result of [`resolve_unknown`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// One of [`ids`].
    pub recognizer: &'static str,
    /// How well.
    pub recognition: Recognition,
}

/// The dir cache for link resolution and memo keys: the active shared cache
/// (inside [`fsx::with_fs`], e.g. store builds), else one process-wide cache.
/// Outside a shared cache `fsx::cache()` hands out a fresh cache per call,
/// which would re-list every directory on each canonicalization; callers there
/// (the `tasks` CLI, tests) are one-shot.
pub(crate) fn link_cache() -> (Arc<DirCache>, bool) {
    static PROCESS: OnceLock<Arc<DirCache>> = OnceLock::new();
    // `fsx::cache()` returns the active shared cache, or a fresh one each call.
    let (a, b) = (fsx::cache(), fsx::cache());
    if Arc::ptr_eq(&a, &b) {
        (a, true)
    } else {
        (
            Arc::clone(PROCESS.get_or_init(|| Arc::new(DirCache::new()))),
            false,
        )
    }
}

/// A memo of pure functions of the filesystem, per shared dir cache (only
/// inside [`fsx::with_fs`]; elsewhere [`RecMemo::get_or`] just computes). The
/// first computation runs under [`turborepo_trace_core::record::capture`];
/// every use, the first included, replays what it observed into the caller's
/// frame, so callers record exactly what recomputing would. Computations run
/// outside the lock (concurrent misses may compute twice; the first
/// result is kept).
pub(crate) struct RecMemo<K, V> {
    map: Mutex<RecMemoMap<K, V>>,
}

type RecMemoMap<K, V> = HashMap<
    (usize, K),
    (
        Weak<DirCache>,
        V,
        Arc<turborepo_trace_core::record::Observations>,
    ),
>;

impl<K: std::hash::Hash + Eq, V: Clone> RecMemo<K, V> {
    pub(crate) fn new() -> Self {
        RecMemo {
            map: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn get_or(&self, key: K, f: impl FnOnce() -> V) -> V {
        use turborepo_trace_core::record;
        let (cache, shared) = link_cache();
        if !shared {
            return f();
        }
        let k = (Arc::as_ptr(&cache) as usize, key);
        {
            let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((w, v, obs)) = map.get(&k)
                && w.upgrade().is_some_and(|c| Arc::ptr_eq(&c, &cache))
            {
                let (v, obs) = (v.clone(), Arc::clone(obs));
                drop(map);
                record::replay(&obs);
                return v;
            }
        }
        let (v, obs) = record::capture(f);
        let obs = Arc::new(obs);
        let v = {
            let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
            if map.len() > 65536 {
                map.retain(|_, (w, _, _)| w.strong_count() > 0);
            }
            match map.get(&k) {
                Some((w, v, _)) if w.upgrade().is_some_and(|c| Arc::ptr_eq(&c, &cache)) => {
                    v.clone()
                }
                _ => {
                    map.insert(k, (Arc::downgrade(&cache), v.clone(), Arc::clone(&obs)));
                    v
                }
            }
        };
        record::replay(&obs);
        v
    }
}

/// Memo key: the dir cache's address and a path.
type MemoKey = (usize, PathBuf);

/// Values computed once per dir cache (see [`link_cache`]) and path.
pub(crate) struct Memo<V> {
    map: Mutex<HashMap<MemoKey, (Weak<DirCache>, V)>>,
}

impl<V: Clone> Memo<V> {
    pub(crate) fn new() -> Self {
        Memo {
            map: Mutex::new(HashMap::new()),
        }
    }

    /// The memoized value for `key`, or `f()` (run under the lock). The flag
    /// tells whether the value was memoized.
    pub(crate) fn get_or(&self, key: &Path, f: impl FnOnce() -> V) -> (V, bool) {
        let (cache, _) = link_cache();
        let k = (Arc::as_ptr(&cache) as usize, key.to_path_buf());
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((w, v)) = map.get(&k)
            && w.upgrade().is_some_and(|c| Arc::ptr_eq(&c, &cache))
        {
            return (v.clone(), true);
        }
        if map.len() > 4096 {
            map.retain(|_, (w, _)| w.strong_count() > 0);
        }
        let v = f();
        map.insert(k, (Arc::downgrade(&cache), v.clone()));
        (v, false)
    }
}

/// [`wrapper::scan`], memoized. Inside a shared cache every task that runs the
/// bin records what the scan observed (its spec depends on it): the files it
/// read and the paths its import resolution looked for. With a plain memo the
/// probes landed on whichever task scanned first, which depends on thread
/// scheduling.
fn scan_memo(file: &Path, pkg_dir: &Path) -> wrapper::Scan {
    if link_cache().1 {
        static R: OnceLock<RecMemo<PathBuf, wrapper::Scan>> = OnceLock::new();
        return R
            .get_or_init(RecMemo::new)
            .get_or(file.to_path_buf(), || wrapper::scan(file, pkg_dir));
    }
    static M: OnceLock<Memo<wrapper::Scan>> = OnceLock::new();
    M.get_or_init(Memo::new)
        .get_or(file, || wrapper::scan(file, pkg_dir))
        .0
}

/// Whether `prog` (as written) is an explicit `node_modules/.bin/x` path.
fn is_dot_bin(prog: &str) -> bool {
    prog.starts_with("node_modules/.bin/") || prog.contains("/node_modules/.bin/")
}

/// Resolves an unknown program as a package binary. Returns `None` (and leaves
/// `out` untouched) when it is not one; the caller then applies the existing
/// fallbacks.
pub fn resolve_unknown(
    cmd: &Command,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Option<Resolved> {
    // Forwarding re-enters the registry; bins wrapping each other must not recurse
    // forever.
    thread_local! {
        static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            DEPTH.with(|d| d.set(d.get() - 1));
        }
    }
    if DEPTH.with(|d| d.get()) >= 4 {
        return None;
    }
    DEPTH.with(|d| d.set(d.get() + 1));
    let _guard = Guard;
    let w0 = cmd.argv.first()?;
    if w0.dynamic || w0.glob {
        return None;
    }
    let prog = w0.text.as_str();
    let name = cmd.program_name()?;
    if name.is_empty() || name.starts_with('-') {
        return None;
    }
    let explicit = if prog.contains('/') {
        if !is_dot_bin(prog) {
            return None;
        }
        Some(cmd.path(prog))
    } else {
        None
    };
    let mut spec = TaskSpec::default();
    let found = npm::find_installed(name, explicit.as_deref(), cmd, cx);
    let r = match found {
        Some(Found::FirstParty { file, pkg_dir, via }) => {
            first_party(name, &file, &pkg_dir, via, cmd, cx, &mut spec)
        }
        Some(Found::ThirdParty { package, file }) => {
            third_party(name, &package, file.as_deref(), cmd, cx, &mut spec)
        }
        None if explicit.is_none() => {
            if let Some(cs) = python::find(name, cmd, cx) {
                console_script(name, &cs, cmd, &mut spec)
            } else if let Some(Found::ThirdParty { package, file }) =
                npm::declared_dependency(name, cx)
            {
                third_party(name, &package, file.as_deref(), cmd, cx, &mut spec)
            } else {
                return None;
            }
        }
        None => return None,
    };
    out.merge(spec);
    Some(r)
}

/// The language of a bin file: by extension, else by shebang.
fn bin_lang(file: &Path) -> EntryLang {
    let l = EntryLang::of(file);
    if l != EntryLang::Auto {
        return l;
    }
    let Ok(b) = fsx::read(file) else {
        return EntryLang::Auto;
    };
    let first = b.split(|&c| c == b'\n').next().unwrap_or(&[]);
    let first = String::from_utf8_lossy(first);
    let Some(sb) = first.strip_prefix("#!") else {
        return EntryLang::Auto;
    };
    // `#!/usr/bin/env -S node --flags`, `#!/usr/bin/python3`.
    let interp = sb
        .split_whitespace()
        .map(|w| w.rsplit('/').next().unwrap_or(w))
        .find(|w| *w != "env" && !w.starts_with('-'))
        .unwrap_or("");
    match interp {
        "node" | "nodejs" | "bun" | "tsx" | "ts-node" | "deno" | "esno" | "jiti" => EntryLang::Js,
        i if i.starts_with("python") || i == "pypy3" => EntryLang::Python,
        _ => EntryLang::Auto,
    }
}

fn rel<'a>(p: &'a Path, cx: &RecognizeCx<'_>) -> std::borrow::Cow<'a, str> {
    p.strip_prefix(cx.repo_root).unwrap_or(p).to_string_lossy()
}

#[allow(clippy::too_many_arguments)]
fn first_party(
    name: &str,
    file: &Path,
    pkg_dir: &Path,
    via: &str,
    cmd: &Command,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Resolved {
    out.config(pkg_dir.join("package.json"));
    let args: Vec<&str> = cmd.args().collect();
    if !fsx::is_file(file) {
        out.entry(file.to_path_buf(), EntryLang::of(file));
        out.membership(pkg_dir, "**", format!("bin `{name}` target not built"));
        out.uncertain(
            UncertaintyKind::GeneratedFile,
            format!("bin `{name}` ({via}) → `{}` does not exist", rel(file, cx)),
        );
        add_existing_files(cmd, &args, out);
        return Resolved {
            recognizer: ids::BIN,
            recognition: Recognition::Partial,
        };
    }
    match bin_lang(file) {
        EntryLang::Js => {
            out.entry(file.to_path_buf(), EntryLang::Js);
            let scan = scan_memo(file, pkg_dir);
            if !scan.tools.is_empty() {
                for t in &scan.tools {
                    let mut fwd = cmd.clone();
                    fwd.argv[0] = Word::lit(*t);
                    fwd.redirects.clear();
                    registry().recognize(&fwd, cx, out);
                    out.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!(
                            "`{name}` runs `{t}` (inferred from `{}`, {} evidence)",
                            rel(file, cx),
                            if scan.weak { "weak" } else { "strong" }
                        ),
                    );
                }
                return Resolved {
                    recognizer: ids::WRAPPER,
                    recognition: Recognition::Partial,
                };
            }
            add_existing_files(cmd, &args, out);
            if scan.cwd {
                out.membership(
                    cmd.cwd.clone(),
                    "**",
                    format!("bin `{name}` reads paths relative to the cwd"),
                );
                return Resolved {
                    recognizer: ids::BIN,
                    recognition: Recognition::Partial,
                };
            }
            Resolved {
                recognizer: ids::BIN,
                recognition: Recognition::Full,
            }
        }
        EntryLang::Python => {
            out.entry(file.to_path_buf(), EntryLang::Python);
            add_existing_files(cmd, &args, out);
            Resolved {
                recognizer: ids::BIN,
                recognition: python_cwd(file, cmd, name, out),
            }
        }
        lang => {
            // A shell script or native binary: its contents are an input, its effects
            // unknown.
            out.entry(file.to_path_buf(), lang);
            out.uncertain(
                UncertaintyKind::UnknownCommand,
                format!("bin `{name}` is a script `{}`", rel(file, cx)),
            );
            Resolved {
                recognizer: ids::BIN,
                recognition: Recognition::Partial,
            }
        }
    }
}

/// `Partial` plus a cwd membership if the Python file reads relative to the
/// cwd.
fn python_cwd(file: &Path, cmd: &Command, name: &str, out: &mut TaskSpec) -> Recognition {
    let src = fsx::read(file).unwrap_or_default();
    let src = String::from_utf8_lossy(&src);
    if src.contains("os.getcwd") || src.contains("Path.cwd") || src.contains("Path('.')") {
        out.membership(
            cmd.cwd.clone(),
            "**",
            format!("`{name}` reads paths relative to the cwd"),
        );
        Recognition::Partial
    } else {
        Recognition::Full
    }
}

fn third_party(
    name: &str,
    package: &str,
    file: Option<&Path>,
    cmd: &Command,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Resolved {
    out.tool("npm", package);
    // The package is a known tool under another binary name (`tsc6` from
    // `@typescript/typescript6`): the tool's recognizer handles the argv.
    if let Some(t) = wrapper::tool_of_package(package)
        && t != name
    {
        let mut fwd = cmd.clone();
        fwd.argv[0] = Word::lit(t);
        fwd.redirects.clear();
        if registry().find(&fwd).is_some() {
            let rep = registry().recognize(&fwd, cx, out);
            return Resolved {
                recognizer: ids::PACKAGE,
                recognition: rep.recognition,
            };
        }
    }
    let args: Vec<&str> = cmd.args().collect();
    add_existing_files(cmd, &args, out);
    out.uncertain(
        UncertaintyKind::UnknownCommand,
        format!(
            "`{name}` from third-party package `{package}`{}: inputs not modelled",
            if file.is_none() {
                " (not installed)"
            } else {
                ""
            }
        ),
    );
    Resolved {
        recognizer: ids::PACKAGE,
        recognition: Recognition::Partial,
    }
}

fn console_script(
    name: &str,
    cs: &python::ConsoleScript,
    cmd: &Command,
    out: &mut TaskSpec,
) -> Resolved {
    out.config(cs.pyproject.clone());
    let args: Vec<&str> = cmd.args().collect();
    add_existing_files(cmd, &args, out);
    let recognition = match &cs.module_file {
        Some(f) => {
            out.entry(f.clone(), EntryLang::Python);
            python_cwd(f, cmd, name, out)
        }
        None => {
            out.entry(cs.probe.clone(), EntryLang::Python);
            out.uncertain(
                UncertaintyKind::UnresolvedSpecifier,
                format!(
                    "console script `{name}` = `{}`: module file not found",
                    cs.target
                ),
            );
            Recognition::Partial
        }
    };
    Resolved {
        recognizer: ids::CONSOLE_SCRIPT,
        recognition,
    }
}
