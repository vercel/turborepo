//! Tracked runtime APIs as data, for JS / TS.
//!
//! Each [`ApiEntry`] says: calling export `path` of one of `modules` (or the
//! global `path` when `modules` is empty) with argument number `arg` reads a
//! file of kind `kind`, anchored by `base`. The recognizer matches call sites
//! through the import/require bindings of the file (`import fs from 'fs'`,
//! `import * as fs from 'node:fs'`, `const { readFileSync: r } = require('fs'
//! )`, `const fsp = require('fs').promises`, ...) and inline `require('fs').
//! readFileSync(...)`. Receivers that are not bound to a listed module never
//! match.
//!
//! The entry format ([`ApiEntry`], [`BaseRule`], [`CallStyle`]) is shared by
//! all languages ([`turborepo_trace_core::api`]); this module holds the JS
//! table and the JS-specific binding model. The table is `const` data so that
//! user entries from a configuration file could be appended with the same
//! shape.

use std::borrow::Cow;

pub use turborepo_trace_core::api::{ApiEntry, ArgRole, BaseRule, CallStyle};
use turborepo_trace_core::{RefFlags, RefKind, api};

/// `fs` and its drop-in replacements (callback, sync and, for `fs-extra`,
/// promise forms).
pub const FS_MODULES: &[&str] = &["fs", "fs-extra", "fs-extra/esm", "graceful-fs"];
/// `fs/promises`.
pub const FS_PROMISES_MODULES: &[&str] = &["fs/promises"];
const WORKER_MODULES: &[&str] = &["worker_threads"];
const CHILD_PROCESS_MODULES: &[&str] = &["child_process"];

const fn entry(
    modules: &'static [&'static str],
    path: &'static str,
    kind: RefKind,
    flags: RefFlags,
) -> ApiEntry {
    ApiEntry::new(modules, path, kind).flags(flags)
}

const R: RefKind = RefKind::FsRead;
const D: RefKind = RefKind::FsReadDir;
const NONE: RefFlags = RefFlags::empty();
/// A call that tolerates a missing path and may go on to read the file
/// (`open`), or whose result may depend on the contents (`stat`: size and
/// timestamps; see [`is_stat`]).
const PROBE: RefFlags = RefFlags::OPTIONAL;
/// An existence check: it observes whether the path is there, nothing else.
const EXISTS: RefFlags = RefFlags::OPTIONAL.union(RefFlags::NAMES);
/// A directory listing: it observes the entries' names and kinds, not their
/// contents.
const LIST: RefFlags = RefFlags::NAMES;

/// Every tracked API. Order does not matter; lookups are exact.
pub const APIS: &[ApiEntry] = &[
    // fs (callback and sync forms), fs-extra, graceful-fs.
    entry(FS_MODULES, "readFile", R, NONE),
    entry(FS_MODULES, "readFileSync", R, NONE),
    entry(FS_MODULES, "createReadStream", R, NONE),
    entry(FS_MODULES, "readJson", R, NONE),
    entry(FS_MODULES, "readJsonSync", R, NONE),
    entry(FS_MODULES, "readJSON", R, NONE),
    entry(FS_MODULES, "readJSONSync", R, NONE),
    entry(FS_MODULES, "readdir", D, LIST),
    entry(FS_MODULES, "readdirSync", D, LIST),
    entry(FS_MODULES, "opendir", D, LIST),
    entry(FS_MODULES, "opendirSync", D, LIST),
    entry(FS_MODULES, "open", R, PROBE),
    entry(FS_MODULES, "openSync", R, PROBE),
    entry(FS_MODULES, "exists", R, EXISTS),
    entry(FS_MODULES, "existsSync", R, EXISTS),
    entry(FS_MODULES, "pathExists", R, EXISTS),
    entry(FS_MODULES, "pathExistsSync", R, EXISTS),
    entry(FS_MODULES, "stat", R, PROBE),
    entry(FS_MODULES, "statSync", R, PROBE),
    entry(FS_MODULES, "lstat", R, PROBE),
    entry(FS_MODULES, "lstatSync", R, PROBE),
    entry(FS_MODULES, "access", R, EXISTS),
    entry(FS_MODULES, "accessSync", R, EXISTS),
    // fs.promises.*
    // The source of a copy is read: tests copy fixtures into a temporary directory and
    // work on the copy, and reads below the temporary directory are not references.
    entry(FS_MODULES, "cp", R, NONE),
    entry(FS_MODULES, "cpSync", R, NONE),
    entry(FS_MODULES, "copyFile", R, NONE),
    entry(FS_MODULES, "copyFileSync", R, NONE),
    entry(FS_MODULES, "copy", R, NONE),
    entry(FS_MODULES, "copySync", R, NONE),
    entry(FS_MODULES, "promises.cp", R, NONE),
    entry(FS_MODULES, "promises.copyFile", R, NONE),
    entry(FS_PROMISES_MODULES, "cp", R, NONE),
    entry(FS_PROMISES_MODULES, "copyFile", R, NONE),
    entry(FS_MODULES, "promises.readFile", R, NONE),
    entry(FS_MODULES, "promises.readdir", D, LIST),
    entry(FS_MODULES, "promises.opendir", D, LIST),
    entry(FS_MODULES, "promises.open", R, PROBE),
    entry(FS_MODULES, "promises.stat", R, PROBE),
    entry(FS_MODULES, "promises.lstat", R, PROBE),
    entry(FS_MODULES, "promises.access", R, EXISTS),
    // fs/promises
    entry(FS_PROMISES_MODULES, "readFile", R, NONE),
    entry(FS_PROMISES_MODULES, "readdir", D, LIST),
    entry(FS_PROMISES_MODULES, "opendir", D, LIST),
    entry(FS_PROMISES_MODULES, "open", R, PROBE),
    entry(FS_PROMISES_MODULES, "stat", R, PROBE),
    entry(FS_PROMISES_MODULES, "lstat", R, PROBE),
    entry(FS_PROMISES_MODULES, "access", R, EXISTS),
    entry(FS_PROMISES_MODULES, "readJson", R, NONE),
    // Workers and child processes: Node resolves relative paths against the cwd.
    entry(WORKER_MODULES, "Worker", RefKind::Worker, NONE).style(CallStyle::New),
    entry(&[], "Worker", RefKind::Worker, NONE).style(CallStyle::New),
    entry(CHILD_PROCESS_MODULES, "fork", RefKind::ChildProcess, NONE),
];

/// Whether `e` is `fs.stat` or `fs.lstat` in any form. Their result carries the
/// size and the timestamps, which change with the contents, so they are content
/// reads unless the call site only asks for the kind of the entry
/// (`super::statuse`).
pub fn is_stat(e: &ApiEntry) -> bool {
    matches!(
        e.path.rsplit('.').next(),
        Some("stat" | "statSync" | "lstat" | "lstatSync")
    )
}

/// Loaders whose wrappers are tracked ([`ArgRole::LoaderWrapper`]; the deep
/// pass finds `const load = n => require(n)`).
pub const LOADERS: &[ApiEntry] = &[ApiEntry::new(&[], "require", RefKind::Require)
    .base(BaseRule::Module)
    .role(ArgRole::LoaderWrapper)];

/// The loader name of [`LOADERS`].
pub const LOADER: &str = LOADERS[0].path;

/// String registries ([`CallStyle::Property`] + [`ArgRole::ModuleString`]): in
/// a file importing one of `modules`, the property value names a module loaded
/// later.
pub const REGISTRIES: &[ApiEntry] = &[
    // pino transports run `target` in a worker thread (also through fastify's `logger`).
    ApiEntry::new(&["pino", "fastify"], "target", RefKind::Require)
        .base(BaseRule::Module)
        .flags(RefFlags::OPTIONAL)
        .style(CallStyle::Property)
        .role(ArgRole::ModuleString),
];

/// Whether importing `spec` activates a [`REGISTRIES`] entry.
pub fn registry_module(spec: &str) -> bool {
    api::table_has_module(REGISTRIES, normalize_module(spec))
}

/// Whether `key` is a property key of a [`REGISTRIES`] entry.
pub fn is_registry_key(key: &str) -> bool {
    REGISTRIES
        .iter()
        .any(|e| e.style == CallStyle::Property && e.path == key)
}

/// Normalizes a module specifier for table lookups: strips `node:`.
pub fn normalize_module(spec: &str) -> &str {
    spec.strip_prefix("node:").unwrap_or(spec)
}

/// Finds the entry for `path` exported by `module` (`None` = a global).
#[inline]
pub fn lookup(module: Option<&str>, path: &str, style: CallStyle) -> Option<&'static ApiEntry> {
    api::lookup(APIS, module, path, style)
}

/// Modules whose bindings the recognizer tracks: every module in [`APIS`], plus
/// the ones the evaluator understands (`path`, `url`, `module`, `process`,
/// `resolve-from`).
pub fn is_tracked_module(module: &str) -> bool {
    matches!(
        module,
        "path" | "path/posix" | "url" | "module" | "process" | "resolve-from" | "vm"
    ) || api::table_has_module(APIS, module)
}

/// Whether `spec` names a builtin module of Node among the tracked ones (`fs`,
/// `node:path`, ...) or by its `node:` prefix: the same module for every
/// `require` function, wherever that function searches from.
pub fn is_builtin_module(spec: &str) -> bool {
    spec.starts_with("node:")
        || (is_tracked_module(spec)
            && !matches!(
                spec,
                "fs-extra" | "fs-extra/esm" | "graceful-fs" | "resolve-from"
            ))
}

/// Whether export `path` of `vm` runs its first argument as source text
/// (`vm.runInNewContext(code)`, `new vm.Script(code)`): code evaluation like
/// `eval` and `new Function`, with no file to load.
pub fn vm_evaluates(path: &str, style: CallStyle) -> bool {
    match style {
        CallStyle::Call => matches!(
            path,
            "runInNewContext" | "runInContext" | "runInThisContext" | "compileFunction"
        ),
        CallStyle::New => matches!(path, "Script" | "SourceTextModule"),
        _ => false,
    }
}

/// A local name bound to a tracked module (or one of its exports), at any
/// depth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiBinding<'s> {
    /// The local name.
    pub local: &'s str,
    /// The normalized module specifier ([`normalize_module`]).
    pub module: Cow<'s, str>,
    /// `None` for the whole module (namespace, default import, `require()`
    /// result), `Some(name)` for one export.
    pub member: Option<Cow<'s, str>>,
    /// The context depth of the declaration; the binding ends with that scope.
    pub depth: u32,
}

impl ApiBinding<'_> {
    /// The dotted export path of `binding.chain...`.
    pub fn export_path(&self, chain: &[&str]) -> String {
        let mut s = String::new();
        if let Some(m) = &self.member {
            s.push_str(m);
        }
        for c in chain {
            if !s.is_empty() {
                s.push('.');
            }
            s.push_str(c);
        }
        s
    }
}

/// A local name with `require` semantics: an alias (`const r = require`) or the
/// result of `createRequire(...)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loader<'s> {
    /// The local name.
    pub local: &'s str,
    /// An alias of `require` (reported as `AliasedLoader`); false for
    /// `createRequire`.
    pub aliased: bool,
    /// Made by `createRequire(a)` where `a` is not literally the file
    /// (`import.meta.url`, `__filename`): a `require` that may search from
    /// another place. The value-flow engine decides, call by call, from
    /// where; the token evaluator knows nothing about it.
    pub elsewhere: bool,
    /// The context depth of the declaration.
    pub depth: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        let e = lookup(Some("fs"), "readFileSync", CallStyle::Call).unwrap();
        assert_eq!(e.kind, RefKind::FsRead);
        assert!(e.flags.is_empty());
        let e = lookup(Some("graceful-fs"), "existsSync", CallStyle::Call).unwrap();
        assert!(e.flags.contains(RefFlags::OPTIONAL));
        let e = lookup(Some("fs/promises"), "readdir", CallStyle::Call).unwrap();
        assert_eq!(e.kind, RefKind::FsReadDir);
        assert!(lookup(Some("fs"), "promises.readFile", CallStyle::Call).is_some());
        assert!(lookup(Some("fs"), "writeFileSync", CallStyle::Call).is_none());
        assert!(lookup(Some("fs"), "readFileSync", CallStyle::New).is_none());
        assert!(lookup(Some("lodash"), "readFileSync", CallStyle::Call).is_none());
        let w = lookup(None, "Worker", CallStyle::New).unwrap();
        assert_eq!(w.kind, RefKind::Worker);
        assert!(lookup(None, "readFileSync", CallStyle::Call).is_none());
        let f = lookup(Some("child_process"), "fork", CallStyle::Call).unwrap();
        assert_eq!(f.kind, RefKind::ChildProcess);
    }

    #[test]
    fn tracked_modules() {
        for m in [
            "fs",
            "fs/promises",
            "fs-extra",
            "path",
            "url",
            "worker_threads",
        ] {
            assert!(is_tracked_module(m), "{m}");
        }
        assert!(!is_tracked_module("lodash"));
        assert_eq!(normalize_module("node:fs"), "fs");
        let b = ApiBinding {
            local: "fsp",
            module: "fs".into(),
            member: Some("promises".into()),
            depth: 0,
        };
        assert_eq!(b.export_path(&["readFile"]), "promises.readFile");
    }
}
