//! Static evaluation of configuration modules (`jest.config.js`,
//! `vite.config.ts`, ...).
//!
//! Test-runner and bundler configs are ordinary JS/TS modules that build one
//! object. This module evaluates such a module without running it and produces
//! a [`ConfigModule`]: the CommonJS `module.exports` value, the ESM default
//! export and named exports, as [`ConfigVal`] trees, plus [`ConfigNote`]s for
//! statements it could not model.
//!
//! Understood (everything else becomes [`ConfigVal::Unknown`] with the source
//! text):
//!
//! - object literals (keys: identifiers, strings, numbers, computed string
//!   keys; shorthand properties; methods and getters become
//!   [`ConfigVal::Function`]), arrays, holes, `...spread` of known objects and
//!   arrays (unknown spreads mark an object [open](ConfigObj::open) or add an
//!   unknown array element), regex literals, `true`, `false`, `null`,
//!   `undefined`, numbers;
//! - string leaves through the shared partial evaluator ([`super::evaluator`]):
//!   concatenation, templates, `path.join/resolve/dirname/...`, `__dirname`,
//!   `__filename`, `import.meta.url`, `import.meta.dirname`, `fileURLToPath`,
//!   `new URL(x, import.meta.url)`, `process.cwd()`. Paths anchored at the file
//!   are made absolute against the module's directory, so values that cross
//!   modules (a package config spreading the root config) stay correct;
//! - `require.resolve('x')` as [`ConfigVal::Resolve`] (resolved later by the
//!   caller), from the directories of `{ paths: [...] }` when given;
//! - `Object.keys/values/entries` of known objects and `Object.fromEntries` of
//!   known pairs; `xs.map(f)`, `xs.filter(f)`, `xs.join(sep)` over known
//!   arrays; `replace`, `replaceAll`, `split`, `trim`, `toLowerCase`,
//!   `toUpperCase` on known strings (a regex pattern only when it matches one
//!   literal string); `new RegExp(source, flags)` with a known source;
//! - `fs.realpathSync(p)` of a known path through [`ConfigHost::realpath`],
//!   also inside `path.resolve(...)` / `path.join(...)`;
//! - `a ?? b`, `a || b` (the left side if it is known, the right side if the
//!   left side is missing, otherwise both as [`ConfigVal::Alts`]), `a && b`, `c
//!   ? a : b` (alternatives);
//! - top-level and function-local `const`/`let`/`var` (with object and array
//!   destructuring), reassignment, member assignment (`config.x = ...`,
//!   `config['x'] = ...`), `delete`, `arr.push(...)`, `Object.assign(target,
//!   ...)`, `arr.concat(...)`;
//! - `module.exports = ...`, `module.exports.x = ...`, `exports.x = ...`,
//!   `export default`, `export const`, `export function`, `export { a as b }`,
//!   TS `export =`;
//! - `import` declarations and `require(...)`: relative and absolute specifiers
//!   are loaded through [`ConfigHost::load`] (the host decides how deep to
//!   follow); other modules become [`ConfigVal::External`] references, whose
//!   members the host may know ([`ConfigHost::external`]), and
//!   `path`/`url`/`process` bindings feed the leaf evaluator;
//! - `defineConfig(x)`, `defineProject(x)`, `defineWorkspace(x)` (identity,
//!   when the callee is imported from a package the host does not follow) and
//!   `mergeConfig(a, b)` (deep merge, [`merge_deep`]);
//! - config factories, for a host that evaluates them
//!   ([`ConfigHost::factories`]): a bare specifier the host follows as source
//!   ([`ConfigHost::load_package`], a workspace package) is a module like a
//!   local one, a call of a function another module exports
//!   (`defineConfig(__dirname)`) is evaluated in that module
//!   ([`ConfigHost::call_export`]), and `table[key]` with a key that is not
//!   statically known is any value of the table. Without such a host a function
//!   of another module cannot be called (unknown);
//! - `next/jest`: `nextJest(options)(custom)` is an async function resolving to
//!   `custom` (or what `custom()` returns) with next's `testPathIgnorePatterns`
//!   ([`next_jest_config`]);
//! - functions: arrow functions, function expressions and declarations
//!   (hoisted), called with known arguments up to a small depth; `return`
//!   inside `if` collects alternatives; an exported function (`module.exports =
//!   async () => ({...})`) is called with no arguments, with a note;
//! - `if`/`else`, `try`, loops: their bodies are evaluated as *conditional*: an
//!   assignment there yields the alternatives of the old and the new value,
//!   which over-approximates;
//! - TypeScript: type annotations, `as`, `satisfies`, non-null `!`, `import
//!   type`, `type`, `interface`, `declare`, `enum` are skipped.
//!
//! Besides the values, the module reports what callers need to reason about a
//! config as source: the ranges it could not follow and the strings visible
//! there ([`ConfigModule::opaque`], [`ConfigModule::escaped`]), the functions
//! with the shapes it recognizes ([`FunctionShape`]), where property values are
//! written and where import bindings are used ([`ConfigModule::properties`],
//! [`ConfigModule::imports`]).
//!
//! Evaluation is bounded (token count, steps, call depth) and never panics.

use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use turborepo_trace_core::{
    eval::{Value, path},
    lexkit::{EscapeRules, decode},
};

use super::{
    api::{self, ApiBinding},
    evaluator::{self, Anchor, JsVal, Scope},
};
use crate::lexer::{Dialect, Kw, Lexer, NEWLINE_BEFORE, TokKind, Token, UNTERMINATED};

/// Maximum number of tokens evaluated; longer files are truncated (with a
/// note).
pub const MAX_TOKENS: usize = 200_000;
/// Maximum evaluation steps.
const MAX_STEPS: u32 = 400_000;
/// Maximum function call depth.
const MAX_CALL_DEPTH: u32 = 8;
/// Maximum nesting of values while parsing.
const MAX_NEST: u32 = 128;
/// Maximum length of an unknown-value reason.
const MAX_REASON: usize = 160;

static INSTANCE: AtomicU64 = AtomicU64::new(1);

/// A statically evaluated config value.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigVal {
    /// `null` or `undefined` (also a missing value in alternatives).
    Null,
    /// A boolean.
    Bool(bool),
    /// A number, as written.
    Num(String),
    /// A string. Paths anchored at a module (`path.resolve(__dirname, 'x')`)
    /// are absolute.
    Str(Value),
    /// A path relative to the process working directory (`path.resolve('x')`,
    /// `process.cwd()`); the value is the relative part.
    CwdPath(Value),
    /// A regex literal.
    Regex {
        /// The pattern between the slashes.
        source: String,
        /// The flags.
        flags: String,
    },
    /// `require.resolve(spec)` evaluated in a module in directory `from_dir`.
    Resolve {
        /// The specifier.
        spec: String,
        /// The absolute directory of the module that called `require.resolve`.
        from_dir: String,
    },
    /// An array.
    Arr(Vec<ConfigVal>),
    /// An object.
    Obj(ConfigObj),
    /// One of several values (conditionals, `??`, conditional assignment).
    Alts(Vec<ConfigVal>),
    /// A function (methods, arrows, `function`): opaque to the caller.
    Function(FuncVal),
    /// A reference into a module that was not evaluated (a package import or
    /// require): `module` and the member path (`import { defineConfig }
    /// from 'vitest/config'` is `External { module: "vitest/config", path:
    /// ["defineConfig"] }`).
    External {
        /// The module specifier.
        module: String,
        /// Member path inside the module (empty for the module itself or its
        /// default).
        path: Vec<String>,
    },
    /// Nothing is known; the source text or an explanation.
    Unknown(Box<str>),
}

/// A function value. Only the module that created it can call it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuncVal {
    /// 1-based line of the definition.
    pub line: u32,
    id: (u64, u32),
}

/// An object: ordered entries, plus whether unknown keys may exist.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigObj {
    /// Entries in insertion order (later assignments replace in place, like
    /// JS).
    pub entries: Vec<(String, ConfigVal)>,
    /// Why other keys may exist (an unknown spread or computed key); `None` if
    /// the key set is exact.
    pub open: Option<Box<str>>,
}

impl ConfigObj {
    /// The value of `key`, if present.
    pub fn get(&self, key: &str) -> Option<&ConfigVal> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut ConfigVal> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Sets `key` (replacing in place).
    pub fn set(&mut self, key: impl Into<String>, v: ConfigVal) {
        let key = key.into();
        match self.get_mut(&key) {
            Some(slot) => *slot = v,
            None => self.entries.push((key, v)),
        }
    }

    /// Removes `key`.
    pub fn remove(&mut self, key: &str) {
        self.entries.retain(|(k, _)| k != key);
    }
}

impl ConfigVal {
    /// A string value.
    pub fn str(s: impl Into<String>) -> ConfigVal {
        ConfigVal::Str(Value::Str(s.into()))
    }

    fn unknown(r: impl Into<Box<str>>) -> ConfigVal {
        ConfigVal::Unknown(r.into())
    }

    /// The known string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ConfigVal::Str(v) => v.as_str(),
            _ => None,
        }
    }

    /// Whether this is [`ConfigVal::Null`].
    pub fn is_null(&self) -> bool {
        matches!(self, ConfigVal::Null)
    }

    /// The alternatives of this value, flattened (the value itself if not
    /// [`ConfigVal::Alts`]).
    pub fn alternatives(&self) -> Vec<&ConfigVal> {
        let mut out = Vec::new();
        fn go<'a>(v: &'a ConfigVal, out: &mut Vec<&'a ConfigVal>) {
            match v {
                ConfigVal::Alts(vs) => vs.iter().for_each(|x| go(x, out)),
                v => out.push(v),
            }
        }
        go(self, &mut out);
        out
    }

    /// Builds alternatives, flattening and deduplicating; one value is returned
    /// as is.
    pub fn alts(vals: impl IntoIterator<Item = ConfigVal>) -> ConfigVal {
        let mut out: Vec<ConfigVal> = Vec::new();
        for v in vals {
            match v {
                ConfigVal::Alts(inner) => {
                    for x in inner {
                        if !out.contains(&x) {
                            out.push(x);
                        }
                    }
                }
                v => {
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
        }
        match out.len() {
            0 => ConfigVal::Null,
            1 => out.pop().unwrap_or(ConfigVal::Null),
            _ => ConfigVal::Alts(out),
        }
    }

    /// The value of property `key`: `None` if the object certainly lacks it, an
    /// unknown value if the object is open or unknown, alternatives over
    /// alternatives (a missing key in one alternative contributes
    /// [`ConfigVal::Null`]).
    pub fn field(&self, key: &str) -> Option<ConfigVal> {
        match self {
            ConfigVal::Obj(o) => match o.get(key) {
                Some(v) => Some(v.clone()),
                // The reason is the unknown part itself, so that every field of the same
                // open object reports the same construct.
                None => o.open.as_ref().map(|r| ConfigVal::Unknown(r.clone())),
            },
            ConfigVal::Alts(vs) => {
                let fs: Vec<Option<ConfigVal>> = vs.iter().map(|v| v.field(key)).collect();
                if fs.iter().all(Option::is_none) {
                    return None;
                }
                Some(ConfigVal::alts(
                    fs.into_iter().map(|f| f.unwrap_or(ConfigVal::Null)),
                ))
            }
            ConfigVal::Unknown(r) => Some(ConfigVal::Unknown(format!("{r}.{key}").into())),
            ConfigVal::External { module, path } => {
                let mut p = path.clone();
                p.push(key.to_string());
                Some(ConfigVal::Unknown(
                    format!("{module}:{} (package value)", p.join(".")).into(),
                ))
            }
            ConfigVal::Function(_) => Some(ConfigVal::unknown(format!("function.{key}"))),
            _ => None,
        }
    }

    /// Whether any part of the value is unknown (an unknown leaf, an open
    /// object, a partially known string, a function, or a package
    /// reference).
    pub fn has_unknown(&self) -> bool {
        match self {
            ConfigVal::Str(v) | ConfigVal::CwdPath(v) => !v.is_known(),
            ConfigVal::Arr(xs) | ConfigVal::Alts(xs) => xs.iter().any(ConfigVal::has_unknown),
            ConfigVal::Obj(o) => o.open.is_some() || o.entries.iter().any(|(_, v)| v.has_unknown()),
            ConfigVal::Unknown(_) | ConfigVal::Function(_) | ConfigVal::External { .. } => true,
            _ => false,
        }
    }

    /// Reasons of every unknown part, in order (see
    /// [`ConfigVal::has_unknown`]).
    pub fn unknown_reasons(&self, out: &mut Vec<String>) {
        match self {
            ConfigVal::Str(v) | ConfigVal::CwdPath(v) if !v.is_known() => {
                let mut rs = Vec::new();
                evaluator::wild_reasons(v, &mut rs);
                if rs.is_empty() {
                    out.push("partially known string".into());
                }
                out.extend(rs.into_iter().map(|r| r.to_string()));
            }
            ConfigVal::Arr(xs) | ConfigVal::Alts(xs) => {
                xs.iter().for_each(|x| x.unknown_reasons(out))
            }
            ConfigVal::Obj(o) => {
                if let Some(r) = &o.open {
                    out.push(format!("...{r}"));
                }
                o.entries.iter().for_each(|(_, v)| v.unknown_reasons(out));
            }
            ConfigVal::Unknown(r) => out.push(r.to_string()),
            ConfigVal::Function(f) => out.push(format!("function (line {})", f.line)),
            ConfigVal::External { module, path } => {
                out.push(if path.is_empty() {
                    format!("value from package '{module}'")
                } else {
                    format!("value from package '{module}': {}", path.join("."))
                });
            }
            _ => {}
        }
    }
}

/// Deep merge with Vite `mergeConfig` semantics: objects merge recursively,
/// arrays concatenate, `null`/missing on either side keeps the other side,
/// anything else is replaced by `b`. Alternatives on either side distribute.
pub fn merge_deep(a: ConfigVal, b: ConfigVal) -> ConfigVal {
    match (a, b) {
        (a, ConfigVal::Null) => a,
        (ConfigVal::Null, b) => b,
        (ConfigVal::Obj(mut x), ConfigVal::Obj(y)) => {
            for (k, v) in y.entries {
                let merged = match x.get(&k) {
                    Some(old) => merge_deep(old.clone(), v),
                    None => v,
                };
                x.set(k, merged);
            }
            if x.open.is_none() {
                x.open = y.open;
            }
            ConfigVal::Obj(x)
        }
        (ConfigVal::Arr(mut x), ConfigVal::Arr(y)) => {
            x.extend(y);
            ConfigVal::Arr(x)
        }
        (ConfigVal::Arr(mut x), b @ ConfigVal::Str(_)) => {
            x.push(b);
            ConfigVal::Arr(x)
        }
        (ConfigVal::Alts(xs), b) => {
            ConfigVal::alts(xs.into_iter().map(|x| merge_deep(x, b.clone())))
        }
        (a, ConfigVal::Alts(ys)) => {
            ConfigVal::alts(ys.into_iter().map(|y| merge_deep(a.clone(), y)))
        }
        (ConfigVal::Obj(x), u @ ConfigVal::Unknown(_)) => {
            let mut x = x;
            if let ConfigVal::Unknown(r) = &u {
                x.open = Some(r.clone());
            }
            ConfigVal::alts([ConfigVal::Obj(x), u])
        }
        (_, b) => b,
    }
}

/// What kind of statement-level problem a [`ConfigNote`] describes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NoteKind {
    /// A construct that is deliberately not modelled; the affected value is
    /// unknown.
    Unsupported,
    /// A value assigned under a condition (alternatives were kept).
    Conditional,
    /// A local module was not followed (depth limit, not found, unreadable).
    NotFollowed,
    /// A value depends on the environment (`process.env`).
    EnvDependent,
    /// The exported config is a function that was called with no arguments.
    FunctionConfig,
    /// The file was too large or evaluation hit a limit.
    Truncated,
    /// The lexer reported an error.
    ParseIncomplete,
}

impl NoteKind {
    /// Stable snake_case name.
    pub const fn as_str(self) -> &'static str {
        match self {
            NoteKind::Unsupported => "unsupported",
            NoteKind::Conditional => "conditional",
            NoteKind::NotFollowed => "not_followed",
            NoteKind::EnvDependent => "env_dependent",
            NoteKind::FunctionConfig => "function_config",
            NoteKind::Truncated => "truncated",
            NoteKind::ParseIncomplete => "parse_incomplete",
        }
    }
}

/// A statement-level remark about the evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigNote {
    /// 1-based line.
    pub line: u32,
    /// Category.
    pub kind: NoteKind,
    /// Source text or explanation.
    pub detail: String,
}

/// The evaluated module.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigModule {
    /// `module.exports` (or TS `export =`), if assigned.
    pub cjs: Option<ConfigVal>,
    /// `export default`, if present.
    pub esm_default: Option<ConfigVal>,
    /// Named exports (`export const`, `export function`, `export {}`,
    /// `exports.x = ...`).
    pub named: Vec<(String, ConfigVal)>,
    /// Statement-level notes.
    pub notes: Vec<ConfigNote>,
    /// Local module specifiers loaded through [`ConfigHost::load`], in order.
    pub loaded: Vec<String>,
    /// Byte ranges of the source the evaluator could not follow: expressions
    /// whose value became unknown (a call into a package, an untracked
    /// helper) and bodies of functions it never called. A value used inside
    /// such a range may be used in ways the evaluator did not see.
    pub opaque: Vec<(u32, u32)>,
    /// Known strings that a binding made visible inside an
    /// [opaque](ConfigModule::opaque) range (`collect(SRC_DIR)`): the
    /// evaluator cannot say what happens to them.
    pub escaped: Vec<String>,
    /// The functions defined in the module.
    pub functions: Vec<FunctionInfo>,
    /// The function the module exports as its default export or
    /// `module.exports`, if it exports one (a function config, a jest
    /// resolver).
    pub exported_function: Option<FunctionInfo>,
    /// Where property values are written: the name and the byte range of the
    /// value, for object literal properties (`reporters: [...]`) and member
    /// assignments (`config.reporters = [...]`), at any depth.
    pub properties: Vec<(String, (u32, u32))>,
    /// The `import` declarations with bindings.
    pub imports: Vec<ImportInfo>,
    /// The function the module exports as its default export or
    /// `module.exports`, as a value: [`ConfigModule::esm_default`] and
    /// [`ConfigModule::cjs`] hold what it returned when called with no
    /// arguments. A host that evaluates factories
    /// ([`ConfigHost::factories`]) gives it to the importer, which calls it.
    pub default_function: Option<FuncVal>,
}

/// An `import` declaration of a config module and where its bindings are used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportInfo {
    /// Byte range of the declaration.
    pub span: (u32, u32),
    /// The module specifier.
    pub spec: String,
    /// The local names it binds.
    pub locals: Vec<String>,
    /// Byte offsets of the uses of those names outside the declaration.
    pub uses: Vec<u32>,
}

/// A function defined in a config module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionInfo {
    id: (u64, u32),
    /// Byte range of the definition.
    pub span: (u32, u32),
    /// What the function does, when it is one of the shapes the evaluator
    /// recognizes.
    pub shape: Option<FunctionShape>,
}

impl FunctionInfo {
    /// Whether this is the definition of the function value `f`.
    pub fn defines(&self, f: &FuncVal) -> bool {
        self.id == f.id
    }
}

/// A recognized function body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FunctionShape {
    /// `(id) => require.resolve(id)`: Node resolution of the first argument.
    RequireResolve,
    /// A jest resolver that is the default resolver with other export
    /// conditions: `(request, options) => options.defaultResolver(request,
    /// { ...options, conditions })`. `Some(requests)`: `conditions` differs
    /// from `options.conditions` only for these requests (`request === 'x'
    /// ? [...] : options.conditions`); `None`: for any request.
    DefaultResolverConditions(Option<Vec<String>>),
}

impl ConfigModule {
    /// The definition of `f`, if this module defined it.
    pub fn function(&self, f: &FuncVal) -> Option<&FunctionInfo> {
        self.functions.iter().find(|x| x.id == f.id)
    }

    /// The config object: the default export, else `module.exports`, else an
    /// object of the named exports (`None` if the module exports nothing).
    pub fn config(&self) -> Option<ConfigVal> {
        if let Some(v) = &self.esm_default {
            return Some(v.clone());
        }
        if let Some(v) = &self.cjs {
            return Some(v.clone());
        }
        (!self.named.is_empty()).then(|| self.named_obj())
    }

    fn named_obj(&self) -> ConfigVal {
        ConfigVal::Obj(ConfigObj {
            entries: self.named.clone(),
            open: None,
        })
    }

    /// What `require()` of this module returns: `module.exports`, or the ESM
    /// namespace.
    pub fn require_value(&self) -> ConfigVal {
        if let Some(v) = &self.cjs {
            return v.clone();
        }
        self.namespace()
    }

    /// `import * as ns`: named exports plus `default`.
    pub fn namespace(&self) -> ConfigVal {
        let mut o = ConfigObj {
            entries: self.named.clone(),
            open: None,
        };
        if let Some(d) = self.esm_default.clone().or_else(|| self.cjs.clone()) {
            o.set("default", d);
        }
        ConfigVal::Obj(o)
    }

    /// `import d from`: the default export, or `module.exports` (interop).
    pub fn import_default(&self) -> ConfigVal {
        self.esm_default
            .clone()
            .or_else(|| self.cjs.clone())
            .unwrap_or_else(|| self.named_obj())
    }

    /// `import { name } from`: the named export, or a property of
    /// `module.exports`.
    pub fn import_named(&self, name: &str) -> ConfigVal {
        if name == "default" {
            return self.import_default();
        }
        if let Some((_, v)) = self.named.iter().find(|(k, _)| k == name) {
            return v.clone();
        }
        match &self.cjs {
            Some(c) => c.field(name).unwrap_or(ConfigVal::Null),
            None => ConfigVal::Null,
        }
    }
}

/// The environment of an evaluation.
pub trait ConfigHost {
    /// Loads the local module `spec` (relative or absolute), as imported or
    /// required by the module being evaluated. `Err` explains why it was
    /// not followed.
    fn load(&mut self, spec: &str) -> Result<ConfigModule, String>;

    /// The value of member `path` of package `module`, if the host knows it
    /// (for example `configDefaults.exclude` of `vitest/config`).
    fn external(&mut self, _module: &str, _path: &[String]) -> Option<ConfigVal> {
        None
    }

    /// The real path of the absolute path `path` (`fs.realpathSync`): every
    /// symlinked component resolved. `None` if the path does not exist or
    /// the host has no file system; the evaluator then keeps the path as
    /// written.
    fn realpath(&mut self, _path: &str) -> Option<String> {
        None
    }

    /// Loads the module of the bare specifier `spec`
    /// (`@acme/playwright/config`) when the host follows that package as
    /// source: a first-party workspace package. `None` (the default): the
    /// package is not followed and the import is a [`ConfigVal::External`]
    /// reference, as for every installed package. `Some(Err)`: a package the
    /// host follows but could not load; the import is unknown, with a note.
    fn load_package(&mut self, _spec: &str) -> Option<Result<ConfigModule, String>> {
        None
    }

    /// Calls the function that the module `spec` exports as `export` (a named
    /// export, or `default`) with `args`, where `spec` is a specifier the
    /// module being evaluated loaded through [`ConfigHost::load`] or
    /// [`ConfigHost::load_package`]: a config factory
    /// (`defineConfig(__dirname)` of a shared module). A host does this with
    /// [`eval_export_call`]. `None` (the default): the host does not call
    /// across modules, and the call is unknown.
    fn call_export(
        &mut self,
        _spec: &str,
        _export: &str,
        _args: &[ConfigVal],
    ) -> Option<ConfigVal> {
        None
    }

    /// Whether the host evaluates config factories (it implements
    /// [`ConfigHost::call_export`]). Modules are then evaluated for a caller
    /// that calls into them, which changes two things. A function that a
    /// module exports as its default export or `module.exports` stays a
    /// function for the importer, instead of the value of calling it with
    /// no arguments. And what a factory chooses by the environment is kept as
    /// alternatives: `table[key]` with a key that is not statically known
    /// (`patterns[process.env.KIND ?? 'e2e']`) over an object literal is one of
    /// the table's values. `false` (the default): the default export is the
    /// called value, and such a lookup is unknown.
    fn factories(&mut self) -> bool {
        false
    }
}

/// A host that follows nothing.
#[derive(Copy, Clone, Debug, Default)]
pub struct NoLoad;

impl ConfigHost for NoLoad {
    fn load(&mut self, spec: &str) -> Result<ConfigModule, String> {
        Err(format!("'{spec}' not followed"))
    }
}

/// Evaluates the config module `src`, located at the absolute path `file`
/// (`/`-separated).
pub fn eval_config(
    src: &[u8],
    dialect: Dialect,
    file: &str,
    host: &mut dyn ConfigHost,
) -> ConfigModule {
    eval_module(src, dialect, file, host, None).0
}

/// Evaluates the module `src` like [`eval_config`], then calls the function it
/// exports as `export` (a named export, or `default`) with `args`: what a host
/// does for [`ConfigHost::call_export`]. Returns the module (for its notes) and
/// the value of the call, which is unknown when that export is not a function.
pub fn eval_export_call(
    src: &[u8],
    dialect: Dialect,
    file: &str,
    host: &mut dyn ConfigHost,
    export: &str,
    args: Vec<ConfigVal>,
) -> (ConfigModule, ConfigVal) {
    let (m, v) = eval_module(src, dialect, file, host, Some((export, args)));
    (m, v.unwrap_or(ConfigVal::Null))
}

fn eval_module(
    src: &[u8],
    dialect: Dialect,
    file: &str,
    host: &mut dyn ConfigHost,
    call: Option<(&str, Vec<ConfigVal>)>,
) -> (ConfigModule, Option<ConfigVal>) {
    let (dir, name) = match file.rfind('/') {
        Some(0) => ("/".to_string(), &file[1..]),
        Some(i) => (file[..i].to_string(), &file[i + 1..]),
        None => (".".to_string(), file),
    };
    let mut toks = Vec::new();
    let mut lx = Lexer::new(src, dialect);
    let mut truncated = false;
    loop {
        let t = lx.next_token();
        match t.kind {
            TokKind::Eof => break,
            TokKind::TripleSlash => continue,
            _ => {}
        }
        if toks.len() >= MAX_TOKENS {
            truncated = true;
            break;
        }
        toks.push(t);
    }
    let lex_error = lx.take_error();
    let close = match_brackets(&toks);
    let mut line_starts = vec![0u32];
    for (i, b) in src.iter().enumerate() {
        if *b == b'\n' {
            line_starts.push(i as u32 + 1);
        }
    }
    let mut ev = Ev {
        src,
        toks,
        close,
        line_starts,
        dir,
        file: file.to_string(),
        file_name: name.to_string(),
        scopes: vec![HashMap::new()],
        apis: Vec::new(),
        funcs: Vec::new(),
        fn_meta: Vec::new(),
        opaque: RefCell::default(),
        escaped: RefCell::default(),
        frames: Vec::new(),
        host,
        out: ConfigModule::default(),
        loaded: HashMap::new(),
        packages: HashMap::new(),
        foreign: HashMap::new(),
        instance: INSTANCE.fetch_add(1, Ordering::Relaxed),
        steps: 0,
        cond: 0,
        nest: 0,
        exhausted: false,
    };
    if truncated {
        ev.note(
            0,
            NoteKind::Truncated,
            format!("more than {MAX_TOKENS} tokens"),
        );
    }
    if let Some(u) = lex_error {
        ev.out.notes.push(ConfigNote {
            line: 0,
            kind: NoteKind::ParseIncomplete,
            detail: u.detail.to_string(),
        });
    }
    let n = ev.toks.len();
    ev.hoist(0, n);
    ev.block(0, n);
    // A call of one export (a factory called by the module that imports it).
    let wanted = call.is_some();
    let called = call.map(|(export, args)| {
        let f = if export == "default" {
            ev.out.import_default()
        } else {
            ev.out.import_named(export)
        };
        match f {
            ConfigVal::Function(f) => ev.invoke(&f, args, "call"),
            other => ConfigVal::unknown(format!(
                "call of export '{export}' of {}: not a function ({})",
                ev.file_name,
                short(&other)
            )),
        }
    });
    // A config exported as a function is called with no arguments (not when the
    // caller wanted one export called with its arguments).
    let mut exported: Option<(u64, u32)> = None;
    for slot in 0..if wanted { 0 } else { 2 } {
        let v = if slot == 0 {
            ev.out.esm_default.clone()
        } else {
            ev.out.cjs.clone()
        };
        if let Some(ConfigVal::Function(f)) = v {
            exported.get_or_insert(f.id);
            // What `import d from` gives: the default export, else `module.exports`.
            if slot == 0 || ev.out.esm_default.is_none() {
                ev.out.default_function = Some(f.clone());
            }
            ev.note_line(
                f.line,
                NoteKind::FunctionConfig,
                "config is a function; called with no arguments".into(),
            );
            let r = ev.invoke(&f, Vec::new(), "exported config function");
            if slot == 0 {
                ev.out.esm_default = Some(r);
            } else {
                ev.out.cjs = Some(r);
            }
        }
    }
    if ev.exhausted {
        ev.note(
            0,
            NoteKind::Truncated,
            "evaluation step limit reached".into(),
        );
    }
    // A function nobody called is as opaque as an unknown value: what its body does
    // with the bindings it sees is not known.
    let mut opaque = ev.opaque.take();
    let mut escaped = ev.escaped.take();
    for m in std::mem::take(&mut ev.fn_meta) {
        if !m.invoked && m.shape.is_none() {
            opaque.push(m.span);
            escaped.extend(m.captured);
        }
        ev.out.functions.push(FunctionInfo {
            id: (ev.instance, m.index),
            span: m.span,
            shape: m.shape,
        });
    }
    ev.out.exported_function = exported
        .and_then(|id| ev.out.functions.iter().find(|f| f.id == id))
        .cloned();
    opaque.sort_unstable();
    opaque.dedup();
    escaped.sort_unstable();
    escaped.dedup();
    // Where the bindings of each import declaration are used.
    if !ev.out.imports.is_empty() {
        let mut imports = std::mem::take(&mut ev.out.imports);
        for (k, t) in ev.toks.iter().enumerate() {
            if t.kind != TokKind::Ident
                || (k > 0 && matches!(ev.toks[k - 1].kind, TokKind::Dot | TokKind::OptChain))
            {
                continue;
            }
            let name = &ev.src[t.start as usize..t.end as usize];
            for imp in &mut imports {
                if !(imp.span.0 <= t.start && t.end <= imp.span.1)
                    && imp.locals.iter().any(|l| l.as_bytes() == name)
                {
                    imp.uses.push(t.start);
                }
            }
        }
        ev.out.imports = imports;
    }
    let mut out = ev.out;
    out.opaque = opaque;
    out.escaped = escaped;
    out.notes.sort_by_key(|n| n.line);
    (out, called)
}

/// The closer index for each opener (`(`, `[`, `{`, template head), `u32::MAX`
/// otherwise.
fn match_brackets(toks: &[Token]) -> Vec<u32> {
    let mut close = vec![u32::MAX; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                stack.push(i)
            }
            TokKind::RParen | TokKind::RBracket | TokKind::RBrace | TokKind::TemplateTail => {
                if let Some(o) = stack.pop() {
                    close[o] = i as u32;
                }
            }
            _ => {}
        }
    }
    close
}

#[derive(Clone, Debug)]
enum Body {
    Expr(usize, usize),
    Block(usize, usize),
    /// A synthetic function returning a fixed value (`createJestConfig(x)` is
    /// an async function resolving to the merged config).
    Const(ConfigVal),
    /// `next/jest`'s `createJestConfig`, the result of `nextJest(options)`:
    /// called with the custom config (an object or a function returning
    /// one), it returns a [`Body::Const`] function resolving to
    /// [`next_jest_config`] of it.
    NextJestFactory,
}

/// What `next/jest` resolves a custom config `x` to, for the fields the tracer
/// uses: `x` itself, with `testPathIgnorePatterns` prefixed by `/node_modules/`
/// and `/.next/` (next's own entries come first), and with next's SWC
/// `transform` entry before the custom ones: `{ '^.+\\.(js|jsx|ts|tsx|mjs)$':
/// [require.resolve('next/dist/build/swc/jest-transformer'), options] }`
/// (resolved from `dir`, the config's directory, so that it equals what the
/// config itself gets from `require.resolve('next/dist/build/swc/
/// jest-transformer')`). Configs rewrite that entry
/// (`Object.entries(config.transform).map(...)`, to wrap or replace next's
/// transformer with their own file), so it has to be there to be rewritten.
/// next also adds `moduleNameMapper` entries (style and asset imports,
/// `next/font`, `server-only`) that point into the `next` package (opaque in
/// query mode). They are left out: without them the tracer follows the real
/// imported asset file instead of next's stub, which only adds inputs.
fn next_jest_config(x: ConfigVal, dir: &str) -> ConfigVal {
    let ConfigVal::Obj(mut o) = x else {
        return match x {
            ConfigVal::Alts(xs) => {
                ConfigVal::alts(xs.into_iter().map(|x| next_jest_config(x, dir)))
            }
            other => other,
        };
    };
    let mut transform = ConfigObj::default();
    transform.set(
        NEXT_JEST_TRANSFORM_PATTERN,
        ConfigVal::Arr(vec![
            ConfigVal::Resolve {
                spec: NEXT_JEST_TRANSFORMER.to_string(),
                from_dir: dir.to_string(),
            },
            // The transformer's options (SWC settings): not paths.
            ConfigVal::Obj(ConfigObj::default()),
        ]),
    );
    if let Some(custom) = o.get("transform").cloned() {
        spread_into(&mut transform, custom, false);
    }
    o.set("transform", ConfigVal::Obj(transform));
    let mut ignore = vec![ConfigVal::str("/node_modules/"), ConfigVal::str("/.next/")];
    match o.get("testPathIgnorePatterns").cloned() {
        None | Some(ConfigVal::Null) => {}
        Some(s @ ConfigVal::Str(_)) => ignore.push(s),
        Some(v) => spread_array(&mut ignore, v),
    }
    o.set("testPathIgnorePatterns", ConfigVal::Arr(ignore));
    ConfigVal::Obj(o)
}

/// The file pattern of the `transform` entry `next/jest` adds.
const NEXT_JEST_TRANSFORM_PATTERN: &str = "^.+\\.(js|jsx|ts|tsx|mjs)$";
/// The transformer module of that entry.
const NEXT_JEST_TRANSFORMER: &str = "next/dist/build/swc/jest-transformer";

/// `Object.keys(x)`, `Object.values(x)` or `Object.entries(x)` (`which`) of a
/// known object: an array in insertion order (entries are `[key, value]`
/// arrays). An open object (an unknown spread) adds one unknown element, so
/// what is built from the list stays a superset. `None` when `x` is not an
/// object.
fn object_list(which: &str, x: ConfigVal, why: &str) -> Option<ConfigVal> {
    match x {
        ConfigVal::Obj(o) => {
            let mut out: Vec<ConfigVal> = o
                .entries
                .into_iter()
                .map(|(k, v)| match which {
                    "keys" => ConfigVal::str(k),
                    "values" => v,
                    _ => ConfigVal::Arr(vec![ConfigVal::str(k), v]),
                })
                .collect();
            if let Some(open) = o.open {
                out.push(ConfigVal::unknown(format!("{why}: {open}")));
            }
            Some(ConfigVal::Arr(out))
        }
        ConfigVal::Alts(xs) => xs
            .into_iter()
            .map(|x| object_list(which, x, why))
            .collect::<Option<Vec<_>>>()
            .map(ConfigVal::alts),
        _ => None,
    }
}

/// `Object.fromEntries(x)` of a known array of `[key, value]` arrays with known
/// string keys. Any other element (an unknown one, a computed key) leaves the
/// object [open](ConfigObj::open). `None` when `x` is not an array.
fn object_from_entries(x: ConfigVal, why: &str) -> Option<ConfigVal> {
    match x {
        ConfigVal::Arr(xs) => {
            let mut o = ConfigObj::default();
            for x in xs {
                // One entry, or the alternatives of one (a callback with several `return`s):
                // `[key, value]` arrays that agree on a known key.
                let pairs: Vec<(String, ConfigVal)> = x
                    .alternatives()
                    .into_iter()
                    .filter_map(|p| match p {
                        ConfigVal::Arr(kv) if kv.len() == 2 => {
                            Some((kv[0].as_str()?.to_string(), kv[1].clone()))
                        }
                        _ => None,
                    })
                    .collect();
                let same_key = pairs.first().is_some_and(|(k, _)| {
                    pairs.len() == x.alternatives().len() && pairs.iter().all(|(k2, _)| k2 == k)
                });
                if same_key {
                    let key = pairs[0].0.clone();
                    o.set(key, ConfigVal::alts(pairs.into_iter().map(|(_, v)| v)));
                } else {
                    o.open = Some(format!("{why}: an entry is not statically known").into());
                }
            }
            Some(ConfigVal::Obj(o))
        }
        ConfigVal::Alts(xs) => xs
            .into_iter()
            .map(|x| object_from_entries(x, why))
            .collect::<Option<Vec<_>>>()
            .map(ConfigVal::alts),
        _ => None,
    }
}

/// String methods evaluated on a known string ([`string_method`]).
fn is_string_method(name: &str) -> bool {
    matches!(
        name,
        "replace" | "replaceAll" | "split" | "trim" | "toLowerCase" | "toUpperCase"
    )
}

/// `s.<name>(args)` for a known string and known arguments; `None` when an
/// argument is not known or the call needs a regex engine.
/// `replace`/`replaceAll` take a string pattern or a regex that matches one
/// literal string (`/\./g`, `/\//`: [`literal_regex`]), which covers
/// the escaping and separator rewrites configs do; the replacement may use `$$`
/// and `$&`.
fn string_method(s: &str, name: &str, args: &[ConfigVal]) -> Option<ConfigVal> {
    match name {
        "trim" if args.is_empty() => Some(ConfigVal::str(s.trim())),
        "toLowerCase" if args.is_empty() => Some(ConfigVal::str(s.to_lowercase())),
        "toUpperCase" if args.is_empty() => Some(ConfigVal::str(s.to_uppercase())),
        "split" if args.len() == 1 => {
            let sep = args[0].as_str()?;
            let parts: Vec<ConfigVal> = if sep.is_empty() {
                s.chars().map(|c| ConfigVal::str(c.to_string())).collect()
            } else {
                s.split(sep).map(ConfigVal::str).collect()
            };
            Some(ConfigVal::Arr(parts))
        }
        "replace" | "replaceAll" if args.len() == 2 => {
            let (pat, all) = match &args[0] {
                ConfigVal::Str(Value::Str(p)) => (p.clone(), name == "replaceAll"),
                ConfigVal::Regex { source, flags } => {
                    let all = flags.contains('g');
                    if flags.chars().any(|f| !matches!(f, 'g' | 'u' | 's' | 'm'))
                        || (name == "replaceAll" && !all)
                    {
                        return None;
                    }
                    (literal_regex(source)?, all)
                }
                _ => return None,
            };
            if pat.is_empty() {
                return None;
            }
            let rep = args[1].as_str()?;
            let mut with = String::with_capacity(rep.len());
            let mut it = rep.chars().peekable();
            while let Some(c) = it.next() {
                if c != '$' {
                    with.push(c);
                    continue;
                }
                match it.peek() {
                    Some('$') => {
                        it.next();
                        with.push('$');
                    }
                    Some('&') => {
                        it.next();
                        with.push_str(&pat);
                    }
                    // The text before or after the match, a named group: not modelled.
                    Some('`' | '\'' | '<') => return None,
                    // The pattern has no capture group: `$1` is literal text.
                    _ => with.push('$'),
                }
            }
            Some(ConfigVal::str(if all {
                s.replace(&pat, &with)
            } else {
                s.replacen(&pat, &with, 1)
            }))
        }
        _ => None,
    }
}

/// The one string a regex source matches, when it is a sequence of literal
/// characters (plain characters and escaped punctuation; no anchors, classes,
/// groups or quantifiers).
fn literal_regex(source: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len());
    let mut it = source.chars();
    while let Some(c) = it.next() {
        match c {
            // An escaped punctuation character stands for itself; a letter or digit after a
            // backslash is a class, an anchor or a back-reference.
            '\\' => match it.next()? {
                e if e.is_ascii_punctuation() => out.push(e),
                _ => return None,
            },
            '.' | '*' | '+' | '?' | '^' | '$' | '|' | '(' | ')' | '[' | ']' | '{' | '}' => {
                return None;
            }
            c => out.push(c),
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Whether `module`/`path` names `next/jest`'s default export (`nextJest`).
fn is_next_jest(module: &str, path: &[String]) -> bool {
    matches!(module, "next/jest" | "next/jest.js")
        && (path.is_empty() || (path.len() == 1 && path[0] == "default"))
}

#[derive(Clone, Debug)]
struct FnDef {
    /// Parameter tokens (inside the parentheses, or the single identifier).
    params: (usize, usize),
    body: Body,
}

/// What [`Ev::make_func`] records about a function of the source.
struct FnMeta {
    /// Index into `Ev::funcs`.
    index: u32,
    /// Byte range of the definition.
    span: (u32, u32),
    shape: Option<FunctionShape>,
    /// Whether the evaluator called it.
    invoked: bool,
    /// Known strings of the bindings its body names (see
    /// [`ConfigModule::escaped`]).
    captured: Vec<String>,
}

struct Frame {
    cond_base: u32,
    returns: Vec<ConfigVal>,
    done: Option<ConfigVal>,
}

/// One path element of an assignment target.
#[derive(Clone, Debug)]
enum Key {
    Name(String),
    Unknown,
}

struct Ev<'s, 'h> {
    src: &'s [u8],
    toks: Vec<Token>,
    close: Vec<u32>,
    line_starts: Vec<u32>,
    dir: String,
    file: String,
    file_name: String,
    scopes: Vec<HashMap<&'s str, ConfigVal>>,
    apis: Vec<ApiBinding<'s>>,
    funcs: Vec<FnDef>,
    /// Source-level facts about the functions of `funcs` made by
    /// [`Ev::make_func`].
    fn_meta: Vec<FnMeta>,
    /// [`ConfigModule::opaque`] and [`ConfigModule::escaped`], collected while
    /// evaluating.
    opaque: RefCell<Vec<(u32, u32)>>,
    escaped: RefCell<Vec<String>>,
    frames: Vec<Frame>,
    host: &'h mut dyn ConfigHost,
    out: ConfigModule,
    loaded: HashMap<String, Result<ConfigModule, String>>,
    /// What [`ConfigHost::load_package`] answered, by specifier.
    packages: HashMap<String, Option<Result<ConfigModule, String>>>,
    /// The functions of other modules that imports made visible here: the
    /// specifier of the module and the name it exports the function as
    /// ([`ConfigHost::call_export`]).
    foreign: HashMap<(u64, u32), (String, String)>,
    instance: u64,
    steps: u32,
    cond: u32,
    nest: u32,
    exhausted: bool,
}

fn is_closer(k: TokKind) -> bool {
    matches!(
        k,
        TokKind::RParen | TokKind::RBracket | TokKind::RBrace | TokKind::TemplateTail
    )
}

fn is_opener(k: TokKind) -> bool {
    matches!(
        k,
        TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead
    )
}

impl<'s> Ev<'s, '_> {
    // ---- token helpers ----

    fn tok(&self, i: usize) -> Token {
        self.toks.get(i).copied().unwrap_or_default()
    }

    fn kind(&self, i: usize, end: usize) -> TokKind {
        if i >= end {
            return TokKind::Eof;
        }
        self.tok(i).kind
    }

    fn text(&self, i: usize) -> &'s [u8] {
        match self.toks.get(i) {
            Some(t) => t.text(self.src),
            None => b"",
        }
    }

    fn ident(&self, i: usize, end: usize) -> Option<&'s str> {
        if i < end && self.tok(i).kind == TokKind::Ident {
            std::str::from_utf8(self.text(i)).ok()
        } else {
            None
        }
    }

    fn is_word(&self, i: usize, end: usize, w: &str) -> bool {
        self.ident(i, end) == Some(w)
    }

    fn is_op(&self, i: usize, end: usize, op: &str) -> bool {
        i < end && self.tok(i).kind == TokKind::Op && self.text(i) == op.as_bytes()
    }

    fn closer_of(&self, i: usize) -> Option<usize> {
        let c = *self.close.get(i)?;
        (c != u32::MAX).then_some(c as usize)
    }

    fn line(&self, i: usize) -> u32 {
        let off = self.toks.get(i).map_or(0, |t| t.start);
        self.line_starts.partition_point(|s| *s <= off) as u32
    }

    /// The known strings the tokens `a..b` can see through bindings: the values
    /// of the identifiers they name (following `.member` chains), for
    /// [`ConfigModule::escaped`].
    fn visible_strings(&self, a: usize, b: usize, out: &mut Vec<String>) {
        fn strings(v: &ConfigVal, depth: u32, out: &mut Vec<String>) {
            if depth > 8 || out.len() > 4096 {
                return;
            }
            match v {
                ConfigVal::Str(Value::Str(s)) => out.push(s.clone()),
                ConfigVal::Arr(xs) | ConfigVal::Alts(xs) => {
                    xs.iter().for_each(|x| strings(x, depth + 1, out))
                }
                ConfigVal::Obj(o) => o
                    .entries
                    .iter()
                    .for_each(|(_, x)| strings(x, depth + 1, out)),
                _ => {}
            }
        }
        let b = b.min(self.toks.len());
        let mut k = a;
        while k < b {
            let t = self.tok(k);
            let member = k > a && matches!(self.tok(k - 1).kind, TokKind::Dot | TokKind::OptChain);
            if t.kind != TokKind::Ident || member {
                k += 1;
                continue;
            }
            let Some(mut v) = self.ident(k, b).and_then(|n| self.lookup(n)).cloned() else {
                k += 1;
                continue;
            };
            k += 1;
            while k + 1 < b
                && matches!(self.tok(k).kind, TokKind::Dot | TokKind::OptChain)
                && let Some(name) = self.ident(k + 1, b)
                && matches!(v, ConfigVal::Obj(_))
                && let Some(x) = v.field(name)
            {
                v = x;
                k += 2;
            }
            strings(&v, 0, out);
        }
    }

    /// Records where the value of property `name` is written (the tokens
    /// `a..b`), for [`ConfigModule::properties`].
    fn note_property(&mut self, name: &str, a: usize, b: usize) {
        let b = b.min(self.toks.len());
        if a < b && self.out.properties.len() < 4096 {
            self.out
                .properties
                .push((name.to_string(), (self.toks[a].start, self.toks[b - 1].end)));
        }
    }

    /// Records that the evaluator could not follow the tokens `a..b`
    /// ([`ConfigModule::opaque`]).
    fn note_opaque(&self, a: usize, b: usize) {
        let b = b.min(self.toks.len());
        if a >= b {
            return;
        }
        self.opaque
            .borrow_mut()
            .push((self.toks[a].start, self.toks[b - 1].end));
        let mut seen = Vec::new();
        self.visible_strings(a, b, &mut seen);
        self.escaped.borrow_mut().extend(seen);
    }

    /// What the function `def` does, if it is a recognized shape.
    fn function_shape(&self, def: &FnDef) -> Option<FunctionShape> {
        let (pa, pb) = def.params;
        let param = self.ident(pa, pb)?;
        let (mut a, mut b) = match def.body {
            Body::Expr(a, b) | Body::Block(a, b) => (a, b),
            _ => return None,
        };
        if matches!(def.body, Body::Block(..)) {
            if self.tok(a).kw != Kw::Return {
                return None;
            }
            a += 1;
            if b > a && self.kind(b - 1, b) == TokKind::Semi {
                b -= 1;
            }
        }
        // `require.resolve(<param>)`.
        if b == a + 6
            && self.tok(a).kw == Kw::Require
            && self.kind(a + 1, b) == TokKind::Dot
            && self.is_word(a + 2, b, "resolve")
            && self.kind(a + 3, b) == TokKind::LParen
            && self.ident(a + 4, b) == Some(param)
            && self.kind(a + 5, b) == TokKind::RParen
        {
            return Some(FunctionShape::RequireResolve);
        }
        self.default_resolver_shape(param, pa, pb, a, b)
    }

    /// `<options>.defaultResolver(<request>, { ...<options>, conditions: C })`
    /// as the whole body `a..b` of a function with the parameters
    /// `(request, options)`: the requests `C` compares `request` with
    /// (`request === 'x'`), or `None` in the shape when `C` does not depend
    /// on the request.
    fn default_resolver_shape(
        &self,
        request: &str,
        pa: usize,
        pb: usize,
        a: usize,
        b: usize,
    ) -> Option<FunctionShape> {
        // The second parameter: after the first top-level comma.
        let comma = self.skip_expr(pa, pb);
        let options = self.ident(comma + 1, pb)?;
        if self.ident(a, b) != Some(options)
            || self.kind(a + 1, b) != TokKind::Dot
            || !self.is_word(a + 2, b, "defaultResolver")
            || self.kind(a + 3, b) != TokKind::LParen
            || self.closer_of(a + 3) != Some(b - 1)
            || self.ident(a + 4, b) != Some(request)
            || self.kind(a + 5, b) != TokKind::Comma
            || self.kind(a + 6, b) != TokKind::LBrace
        {
            return None;
        }
        let c = self.closer_of(a + 6)?;
        // `}` then `)`, with an optional trailing comma.
        if !(c + 1 == b - 1 || (self.kind(c + 1, b) == TokKind::Comma && c + 2 == b - 1)) {
            return None;
        }
        // `{ ...options, conditions: C }` and nothing else.
        if self.kind(a + 7, c) != TokKind::Ellipsis
            || self.ident(a + 8, c) != Some(options)
            || self.kind(a + 9, c) != TokKind::Comma
            || !self.is_word(a + 10, c, "conditions")
            || self.kind(a + 11, c) != TokKind::Colon
        {
            return None;
        }
        let ce = self.skip_expr(a + 12, c);
        if !(ce == c || (self.kind(ce, c) == TokKind::Comma && ce + 1 == c)) {
            return None;
        }
        // Every use of `request` in C is a comparison with a string literal.
        let mut requests = Vec::new();
        let mut k = a + 12;
        while k < ce {
            let t = self.tok(k);
            let member = matches!(self.tok(k - 1).kind, TokKind::Dot | TokKind::OptChain);
            if t.kind == TokKind::Ident && !member && self.ident(k, ce) == Some(request) {
                let eq = |j: usize| self.is_op(j, ce, "===") || self.is_op(j, ce, "==");
                let lit = if eq(k + 1) && self.kind(k + 2, ce) == TokKind::Str {
                    self.str_lit(k + 2)
                } else if k >= a + 14 && eq(k - 1) && self.kind(k - 2, ce) == TokKind::Str {
                    self.str_lit(k - 2)
                } else {
                    None
                };
                requests.push(lit?);
            }
            k += 1;
        }
        Some(FunctionShape::DefaultResolverConditions(
            (!requests.is_empty()).then_some(requests),
        ))
    }

    /// The source text of the tokens `a..b` as the reason of an unknown value.
    /// Also records the range as one the evaluator could not follow.
    fn reason(&self, a: usize, b: usize) -> Box<str> {
        let b = b.min(self.toks.len());
        if a >= b {
            return "".into();
        }
        self.note_opaque(a, b);
        let r = evaluator::reason_text(self.src, &self.toks[a..b]);
        if r.len() > MAX_REASON {
            let mut n = MAX_REASON;
            while !r.is_char_boundary(n) {
                n -= 1;
            }
            format!("{}…", &r[..n]).into()
        } else {
            r
        }
    }

    fn note(&mut self, i: usize, kind: NoteKind, detail: String) {
        let line = if self.toks.is_empty() {
            0
        } else {
            self.line(i)
        };
        self.note_line(line, kind, detail);
    }

    fn note_line(&mut self, line: u32, kind: NoteKind, detail: String) {
        if self.out.notes.len() < 256
            && !self
                .out
                .notes
                .iter()
                .any(|n| n.line == line && n.kind == kind && n.detail == detail)
        {
            self.out.notes.push(ConfigNote { line, kind, detail });
        }
    }

    fn step(&mut self) -> bool {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            self.exhausted = true;
            return false;
        }
        true
    }

    fn str_lit(&self, i: usize) -> Option<String> {
        let t = self.tok(i);
        if !matches!(t.kind, TokKind::Str | TokKind::Template) || t.flags & UNTERMINATED != 0 {
            return None;
        }
        let raw = t.literal_content(self.src)?;
        decode(raw, EscapeRules::JS).ok().map(Cow::into_owned)
    }

    /// Whether the token at `k` ends a statement by ASI: it starts a new line,
    /// the token before it can end an expression, and it can start a
    /// statement.
    fn asi_break(&self, k: usize) -> bool {
        let t = self.tok(k);
        if t.flags & NEWLINE_BEFORE == 0 || k == 0 {
            return false;
        }
        let p = self.tok(k - 1);
        let prev_ends = match p.kind {
            TokKind::Ident => !matches!(
                p.kw,
                Kw::Return
                    | Kw::Typeof
                    | Kw::Case
                    | Kw::Do
                    | Kw::Else
                    | Kw::In
                    | Kw::Instanceof
                    | Kw::New
                    | Kw::Delete
                    | Kw::Void
                    | Kw::Throw
                    | Kw::Yield
                    | Kw::Await
                    | Kw::Extends
            ),
            TokKind::Str
            | TokKind::Template
            | TokKind::TemplateTail
            | TokKind::Number
            | TokKind::Regex
            | TokKind::RParen
            | TokKind::RBracket
            | TokKind::RBrace
            | TokKind::IncDec
            | TokKind::Jsx => true,
            _ => false,
        };
        if !prev_ends {
            return false;
        }
        match t.kind {
            TokKind::Ident => {
                !matches!(t.kw, Kw::In | Kw::Instanceof)
                    && !matches!(self.text(k), b"as" | b"satisfies")
            }
            TokKind::Str
            | TokKind::Template
            | TokKind::TemplateHead
            | TokKind::Number
            | TokKind::Bang
            | TokKind::IncDec
            | TokKind::LBrace
            | TokKind::At => true,
            _ => false,
        }
    }

    /// End of the statement starting at `i`: the index of its `;`, of the
    /// enclosing closer, or of the first token of the next statement (ASI).
    fn stmt_end(&self, i: usize, end: usize) -> usize {
        let mut k = i;
        while k < end {
            let t = self.tok(k);
            if k > i && self.asi_break(k) {
                return k;
            }
            match t.kind {
                TokKind::Semi => return k,
                _ if is_opener(t.kind) => match self.closer_of(k) {
                    Some(c) if c < end => k = c,
                    _ => return end,
                },
                _ if is_closer(t.kind) => return k,
                _ => {}
            }
            k += 1;
        }
        end
    }

    /// End of an expression without evaluating it: the first `,`, `;` or
    /// enclosing closer at nesting 0, or an ASI break.
    fn skip_expr(&self, i: usize, end: usize) -> usize {
        let mut k = i;
        while k < end {
            let t = self.tok(k);
            if k > i && self.asi_break(k) {
                return k;
            }
            match t.kind {
                TokKind::Semi | TokKind::Comma => return k,
                _ if is_opener(t.kind) => match self.closer_of(k) {
                    Some(c) if c < end => k = c,
                    _ => return end,
                },
                _ if is_closer(t.kind) => return k,
                _ => {}
            }
            k += 1;
        }
        end
    }

    /// End of a scalar leaf expression: like [`Ev::skip_expr`], but also stops
    /// at the operators the structural evaluator handles (`?`, `:`, `??`,
    /// `||`, `&&`, `=>`, `=`) and at TypeScript `as` / `satisfies`.
    fn leaf_end(&self, i: usize, end: usize) -> usize {
        let mut k = i;
        while k < end {
            let t = self.tok(k);
            if k > i && self.asi_break(k) {
                return k;
            }
            match t.kind {
                TokKind::Semi
                | TokKind::Comma
                | TokKind::Question
                | TokKind::Colon
                | TokKind::Arrow
                | TokKind::Assign => return k,
                TokKind::Op if matches!(self.text(k), b"??" | b"||" | b"&&") => return k,
                TokKind::Ident if k > i && matches!(self.text(k), b"as" | b"satisfies") => {
                    return k;
                }
                _ if is_opener(t.kind) => match self.closer_of(k) {
                    Some(c) if c < end => k = c,
                    _ => return end,
                },
                _ if is_closer(t.kind) => return k,
                _ => {}
            }
            k += 1;
        }
        end
    }

    /// The `>` that closes the type arguments opened by the `<` at `i`, if
    /// there is one before the statement ends (`<` is a comparison
    /// otherwise).
    fn type_args_end(&self, i: usize, end: usize) -> Option<usize> {
        let mut depth = 0u32;
        let mut k = i;
        while k < end {
            let t = self.tok(k);
            match t.kind {
                TokKind::Lt => depth += 1,
                TokKind::Gt => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(k);
                    }
                }
                TokKind::Semi => return None,
                _ if is_opener(t.kind) => k = self.closer_of(k).filter(|c| *c < end)?,
                _ if is_closer(t.kind) => return None,
                _ => {}
            }
            k += 1;
        }
        None
    }

    /// Skips a TypeScript type (after `:`, `as`, `satisfies`) up to a value
    /// terminator.
    fn skip_type(&self, i: usize, end: usize, stop_at_assign: bool) -> usize {
        let mut k = i;
        while k < end {
            let t = self.tok(k);
            if k > i && self.asi_break(k) && t.kind != TokKind::LBrace {
                return k;
            }
            match t.kind {
                // Type arguments: the comma of `Record<Kind, string>` ends nothing.
                TokKind::Lt if k > i && self.tok(k - 1).kind == TokKind::Ident => {
                    if let Some(c) = self.type_args_end(k, end) {
                        k = c;
                    }
                }
                TokKind::Semi | TokKind::Comma | TokKind::Arrow | TokKind::Question => return k,
                TokKind::Assign if stop_at_assign => return k,
                TokKind::Op if matches!(self.text(k), b"??" | b"||" | b"&&") => return k,
                TokKind::Colon => return k,
                _ if is_opener(t.kind) => match self.closer_of(k) {
                    Some(c) if c < end => k = c,
                    _ => return end,
                },
                _ if is_closer(t.kind) => return k,
                _ => {}
            }
            k += 1;
        }
        end
    }

    // ---- scopes ----

    fn lookup(&self, name: &str) -> Option<&ConfigVal> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn lookup_mut(&mut self, name: &str) -> Option<&mut ConfigVal> {
        self.scopes.iter_mut().rev().find_map(|s| s.get_mut(name))
    }

    fn bind(&mut self, name: &'s str, v: ConfigVal) {
        if let ConfigVal::External { module, path } = &v {
            let m = api::normalize_module(module);
            if api::is_tracked_module(m) && path.len() <= 1 {
                self.apis.push(ApiBinding {
                    local: name,
                    module: Cow::Owned(m.to_string()),
                    member: path
                        .first()
                        .filter(|p| *p != "default")
                        .cloned()
                        .map(Cow::Owned),
                    depth: self.scopes.len() as u32 - 1,
                });
            }
        }
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name, v);
        }
    }

    fn is_conditional(&self) -> bool {
        self.cond > self.frames.last().map_or(0, |f| f.cond_base)
    }

    // ---- statements ----

    /// Registers function declarations of the block `[i, end)` (hoisting).
    fn hoist(&mut self, i: usize, end: usize) {
        let mut k = i;
        while k < end {
            let t = self.tok(k);
            if is_opener(t.kind) {
                k = self.closer_of(k).map_or(end, |c| c + 1);
                continue;
            }
            if t.kind == TokKind::Ident
                && t.kw == Kw::Function
                && let Some(name) = self.ident(k + 1, end)
                && let Some((def, _)) = self.function_at(k, end)
            {
                let f = self.make_func(def, k);
                self.bind(name, f);
            }
            k += 1;
        }
    }

    /// Walks statements in `[i, end)`. Stops early if a function frame
    /// returned.
    fn block(&mut self, mut i: usize, end: usize) {
        while i < end {
            if !self.step() {
                return;
            }
            if self.frames.last().is_some_and(|f| f.done.is_some()) {
                return;
            }
            let next = self.statement(i, end);
            i = if next <= i { i + 1 } else { next };
        }
    }

    /// Walks the statement at `i` as conditional.
    fn cond_stmt(&mut self, i: usize, end: usize) -> usize {
        self.cond += 1;
        let r = if self.kind(i, end) == TokKind::LBrace {
            let c = self.closer_of(i).unwrap_or(end).min(end);
            self.scopes.push(HashMap::new());
            self.block(i + 1, c);
            self.scopes.pop();
            c + 1
        } else {
            self.statement(i, end)
        };
        self.cond -= 1;
        r
    }

    fn statement(&mut self, i: usize, end: usize) -> usize {
        let t = self.tok(i);
        match t.kind {
            TokKind::Semi => return i + 1,
            TokKind::LBrace => {
                let c = self.closer_of(i).unwrap_or(end).min(end);
                self.scopes.push(HashMap::new());
                self.block(i + 1, c);
                self.scopes.pop();
                return c + 1;
            }
            TokKind::Ident => {}
            _ => return self.expr_stmt(i, end),
        }
        let word = self.ident(i, end).unwrap_or("");
        match t.kw {
            Kw::Import if !matches!(self.kind(i + 1, end), TokKind::LParen | TokKind::Dot) => {
                return self.import_decl(i, end);
            }
            Kw::Export => return self.export_decl(i, end),
            Kw::Const | Kw::Let | Kw::Var => {
                if word == "let"
                    && !matches!(
                        self.kind(i + 1, end),
                        TokKind::Ident | TokKind::LBrace | TokKind::LBracket
                    )
                {
                    return self.expr_stmt(i, end);
                }
                let (j, _) = self.decl(i + 1, end);
                return j;
            }
            Kw::Function => {
                return match self.function_at(i, end) {
                    Some((def, j)) => {
                        if let Some(name) = self.ident(i + 1, end) {
                            let f = self.make_func(def, i);
                            self.bind(name, f);
                        }
                        j
                    }
                    None => self.stmt_end(i, end) + 1,
                };
            }
            Kw::Class => return self.skip_braced(i, end),
            Kw::If => {
                let Some(c) = (self.kind(i + 1, end) == TokKind::LParen)
                    .then(|| self.closer_of(i + 1))
                    .flatten()
                else {
                    return self.stmt_end(i, end) + 1;
                };
                let mut j = self.cond_stmt(c + 1, end);
                if self.tok(j).kind == TokKind::Ident && self.tok(j).kw == Kw::Else && j < end {
                    j = self.cond_stmt(j + 1, end);
                }
                return j;
            }
            Kw::Try => {
                let mut j = self.cond_stmt(i + 1, end);
                if self.is_word(j, end, "catch") {
                    j += 1;
                    if self.kind(j, end) == TokKind::LParen {
                        j = self.closer_of(j).map_or(end, |c| c + 1);
                    }
                    j = self.cond_stmt(j, end);
                }
                if self.is_word(j, end, "finally") {
                    j = self.cond_stmt(j + 1, end);
                }
                return j;
            }
            Kw::For | Kw::While | Kw::With => {
                let Some(c) = (self.kind(i + 1, end) == TokKind::LParen)
                    .then(|| self.closer_of(i + 1))
                    .flatten()
                else {
                    // `for await (...)`.
                    if self.kind(i + 2, end) == TokKind::LParen
                        && let Some(c) = self.closer_of(i + 2)
                    {
                        return self.cond_stmt(c + 1, end);
                    }
                    return self.stmt_end(i, end) + 1;
                };
                return self.cond_stmt(c + 1, end);
            }
            Kw::Do => {
                let j = self.cond_stmt(i + 1, end);
                // `while (...)`.
                if self.tok(j).kw == Kw::While
                    && let Some(c) = self.closer_of(j + 1)
                {
                    return c + 1;
                }
                return j;
            }
            Kw::Return => {
                let (v, j) = if matches!(
                    self.kind(i + 1, end),
                    TokKind::Semi | TokKind::RBrace | TokKind::Eof
                ) || self.asi_break(i + 1)
                {
                    (ConfigVal::Null, i + 1)
                } else {
                    self.expr(i + 1, end)
                };
                let conditional = self.is_conditional();
                if let Some(f) = self.frames.last_mut() {
                    if conditional {
                        f.returns.push(v);
                    } else {
                        let mut all = std::mem::take(&mut f.returns);
                        all.push(v);
                        f.done = Some(ConfigVal::alts(all));
                    }
                }
                return self.stmt_end(j, end);
            }
            Kw::Delete => {
                let (target, j) = self.lvalue(i + 1, end);
                if let Some((root, keys)) = target {
                    self.delete(i, root, &keys);
                }
                return self.stmt_end(j, end);
            }
            _ => {}
        }
        match word {
            "async" if self.tok(i + 1).kw == Kw::Function => {
                return match self.function_at(i + 1, end) {
                    Some((def, j)) => {
                        if let Some(name) = self.ident(i + 2, end) {
                            let f = self.make_func(def, i);
                            self.bind(name, f);
                        }
                        j
                    }
                    None => self.stmt_end(i, end) + 1,
                };
            }
            "type" if self.kind(i + 1, end) == TokKind::Ident => {
                return self.stmt_end(i, end) + 1;
            }
            "interface" | "enum" | "namespace" | "abstract"
                if self.kind(i + 1, end) == TokKind::Ident =>
            {
                return self.skip_braced(i, end);
            }
            "declare" if self.kind(i + 1, end) == TokKind::Ident => {
                let j = self.stmt_end(i, end);
                // `declare module 'x' { ... }` / `declare global { ... }`.
                return if self.kind(j, end) == TokKind::LBrace {
                    self.closer_of(j).map_or(end, |c| c + 1)
                } else {
                    j + 1
                };
            }
            _ => {}
        }
        self.expr_stmt(i, end)
    }

    /// Skips a declaration that ends with a `{...}` body (class, interface,
    /// enum).
    fn skip_braced(&self, i: usize, end: usize) -> usize {
        let mut k = i;
        while k < end {
            match self.tok(k).kind {
                TokKind::LBrace => return self.closer_of(k).map_or(end, |c| c + 1),
                TokKind::Semi => return k + 1,
                TokKind::LParen | TokKind::LBracket => {
                    k = self.closer_of(k).unwrap_or(end);
                }
                _ => {}
            }
            k += 1;
        }
        end
    }

    fn expr_stmt(&mut self, i: usize, end: usize) -> usize {
        let e = self.stmt_end(i, end);
        // Assignment targets: `a.b.c = v`, `a['b'] = v`, `a ??= v`.
        let (target, j) = self.lvalue(i, e);
        if let Some((root, keys)) = target {
            match self.tok(j).kind {
                TokKind::Assign if j < e => {
                    if let Some(Key::Name(name)) = keys.last() {
                        self.note_property(name, j + 1, e);
                    }
                    let (v, _) = self.expr(j + 1, e);
                    self.assign(i, root, &keys, v, false);
                    return e;
                }
                TokKind::Op if j < e && matches!(self.text(j), b"??=" | b"||=") => {
                    let (v, _) = self.expr(j + 1, e);
                    self.assign(i, root, &keys, v, true);
                    return e;
                }
                TokKind::LParen if j < e && !keys.is_empty() => {
                    // `arr.push(...)`, `Object.assign(target, ...)`.
                    let method = keys.last().cloned();
                    if let Some(Key::Name(m)) = method {
                        let obj_keys = &keys[..keys.len() - 1];
                        if m == "push" || m == "unshift" {
                            let c = self.closer_of(j).unwrap_or(e).min(e);
                            let args = self.args(j + 1, c);
                            self.push(i, root, obj_keys, args, m == "unshift");
                            return e;
                        }
                        if root == "Object" && obj_keys.is_empty() && m == "assign" {
                            let c = self.closer_of(j).unwrap_or(e).min(e);
                            self.object_assign_stmt(i, j + 1, c);
                            return e;
                        }
                    }
                }
                _ => {}
            }
        }
        if i < e && self.tracked_mutation(i, e) {
            let r = self.reason(i, e);
            self.note(
                i,
                NoteKind::Unsupported,
                format!("statement not evaluated: {r}"),
            );
        }
        e
    }

    /// Whether the statement `[i, e)` starts with a tracked binding (so
    /// skipping it may hide a mutation).
    fn tracked_mutation(&self, i: usize, e: usize) -> bool {
        let Some(name) = self.ident(i, e) else {
            return false;
        };
        if name == "module" || name == "exports" {
            return true;
        }
        matches!(
            self.lookup(name),
            Some(ConfigVal::Obj(_) | ConfigVal::Arr(_) | ConfigVal::Alts(_))
        )
    }

    /// Parses `name(.prop | [key])*` at `i`. Returns the root and keys, and the
    /// index after.
    fn lvalue(&mut self, i: usize, end: usize) -> (Option<(&'s str, Vec<Key>)>, usize) {
        let Some(root) = self.ident(i, end) else {
            return (None, i);
        };
        if self.tok(i).kw != Kw::None && !matches!(root, "module" | "exports") {
            return (None, i);
        }
        let mut keys = Vec::new();
        let mut j = i + 1;
        loop {
            match self.kind(j, end) {
                TokKind::Dot => match self.ident(j + 1, end) {
                    Some(n) => {
                        keys.push(Key::Name(n.to_string()));
                        j += 2;
                    }
                    None => return (None, j),
                },
                TokKind::LBracket => {
                    let c = self.closer_of(j).unwrap_or(end).min(end);
                    let (k, _) = self.expr(j + 1, c);
                    keys.push(match k {
                        ConfigVal::Str(Value::Str(s)) | ConfigVal::Num(s) => Key::Name(s),
                        _ => Key::Unknown,
                    });
                    j = c + 1;
                }
                TokKind::Bang => j += 1,
                _ => break,
            }
        }
        (Some((root, keys)), j)
    }

    fn assign(&mut self, at: usize, root: &'s str, keys: &[Key], v: ConfigVal, nullish: bool) {
        let conditional = self.is_conditional();
        let detail = || {
            let mut s = root.to_string();
            for k in keys {
                match k {
                    Key::Name(n) => {
                        s.push('.');
                        s.push_str(n);
                    }
                    Key::Unknown => s.push_str("[?]"),
                }
            }
            s
        };
        if conditional {
            let d = detail();
            self.note(
                at,
                NoteKind::Conditional,
                format!("{d} assigned conditionally"),
            );
        }
        let combine = |old: Option<ConfigVal>, v: ConfigVal| -> ConfigVal {
            let old_or_null = old.clone().unwrap_or(ConfigVal::Null);
            if nullish {
                return match old {
                    Some(o) if !o.is_null() && !matches!(o, ConfigVal::Unknown(_)) => o,
                    Some(ConfigVal::Unknown(r)) => ConfigVal::alts([ConfigVal::Unknown(r), v]),
                    _ if conditional => ConfigVal::alts([old_or_null, v]),
                    _ => v,
                };
            }
            if conditional {
                ConfigVal::alts([old_or_null, v])
            } else {
                v
            }
        };
        // `module.exports...` / `exports...`.
        let (slot, rest): (Option<&mut ConfigVal>, &[Key]) = match (root, keys.first()) {
            ("module", Some(Key::Name(e))) if e == "exports" => {
                if keys.len() == 1 {
                    let old = self.out.cjs.take();
                    self.out.cjs = Some(combine(old, v));
                    return;
                }
                if self.out.cjs.is_none() {
                    self.out.cjs = Some(ConfigVal::Obj(ConfigObj::default()));
                }
                (self.out.cjs.as_mut(), &keys[1..])
            }
            ("exports", _) if self.lookup("exports").is_none() => {
                if keys.is_empty() {
                    return;
                }
                if self.out.cjs.is_none() {
                    self.out.cjs = Some(ConfigVal::Obj(ConfigObj::default()));
                }
                (self.out.cjs.as_mut(), keys)
            }
            _ => {
                if keys.is_empty() {
                    if self.lookup(root).is_some() {
                        let old = self.lookup(root).cloned();
                        let nv = combine(old, v);
                        if let Some(s) = self.lookup_mut(root) {
                            *s = nv;
                        }
                    }
                    return;
                }
                (self.lookup_mut(root), keys)
            }
        };
        let Some(cur) = slot else {
            return;
        };
        let Some((last, path)) = rest.split_last() else {
            return;
        };
        let d = detail();
        if let Err(why) = set_path(cur, path, last, v, &combine, &d) {
            self.note(
                at,
                NoteKind::Unsupported,
                format!("assignment to {d} ({why})"),
            );
        }
    }

    fn delete(&mut self, at: usize, root: &'s str, keys: &[Key]) {
        let conditional = self.is_conditional();
        let Some((last, path)) = keys.split_last() else {
            return;
        };
        let in_scope = self.lookup(root).is_some();
        let mut cur = if in_scope {
            match self.lookup_mut(root) {
                Some(c) => c,
                None => return,
            }
        } else if root == "module" {
            match self.out.cjs.as_mut() {
                Some(c) => c,
                None => return,
            }
        } else {
            return;
        };
        let path = if !in_scope && root == "module" {
            path.get(1..).unwrap_or(&[])
        } else {
            path
        };
        for k in path {
            match (cur, k) {
                (ConfigVal::Obj(o), Key::Name(n)) => match o.get_mut(n) {
                    Some(x) => cur = x,
                    None => return,
                },
                _ => return,
            }
        }
        match (cur, last) {
            (ConfigVal::Obj(o), Key::Name(n)) => {
                if conditional {
                    if let Some(v) = o.get_mut(n) {
                        *v = ConfigVal::alts([v.clone(), ConfigVal::Null]);
                    }
                    self.note(
                        at,
                        NoteKind::Conditional,
                        format!("{root}.{n} deleted conditionally"),
                    );
                } else {
                    o.remove(n);
                }
            }
            _ => self.note(at, NoteKind::Unsupported, format!("delete on {root}")),
        }
    }

    fn push(&mut self, at: usize, root: &'s str, keys: &[Key], args: Vec<ConfigVal>, front: bool) {
        let conditional = self.is_conditional();
        let mut cur = match self.lookup_mut(root) {
            Some(c) => c,
            None => return,
        };
        for k in keys {
            match (cur, k) {
                (ConfigVal::Obj(o), Key::Name(n)) => match o.get_mut(n) {
                    Some(x) => cur = x,
                    None => return,
                },
                _ => return,
            }
        }
        // Conditional pushes still add the elements: consumers union array elements, so
        // keeping them over-approximates.
        match cur {
            ConfigVal::Arr(a) => {
                if front {
                    let mut n = args;
                    n.append(a);
                    *a = n;
                } else {
                    a.extend(args);
                }
            }
            ConfigVal::Alts(vs) => {
                for v in vs.iter_mut() {
                    if let ConfigVal::Arr(a) = v {
                        a.extend(args.iter().cloned());
                    }
                }
            }
            _ => {
                self.note(
                    at,
                    NoteKind::Unsupported,
                    format!("{root}.push on a non-array"),
                );
                return;
            }
        }
        if conditional {
            self.note(
                at,
                NoteKind::Conditional,
                format!("{root}.push(...) under a condition"),
            );
        }
    }

    /// `Object.assign(target, ...sources)` as a statement.
    // Invariant: the match guard checked that the key is present.
    #[allow(clippy::expect_used)]
    fn object_assign_stmt(&mut self, at: usize, i: usize, c: usize) {
        let Some(root) = self.ident(i, c) else {
            return;
        };
        let (target, j) = self.lvalue(i, c);
        if self.kind(j, c) != TokKind::Comma {
            return;
        }
        let srcs = self.args(j + 1, c);
        let conditional = self.is_conditional();
        if let Some((_, keys)) = target {
            let mut merged = self.lookup(root).cloned();
            if let Some(m) = merged.as_mut() {
                let mut cur = m;
                for k in &keys {
                    match (cur, k) {
                        (ConfigVal::Obj(o), Key::Name(n)) if o.get(n).is_some() => {
                            cur = o.get_mut(n).expect("checked");
                        }
                        _ => return,
                    }
                }
                if let ConfigVal::Obj(o) = cur {
                    for s in srcs {
                        spread_into(o, s, conditional);
                    }
                }
            }
            if let (Some(m), Some(slot)) = (merged, self.lookup_mut(root)) {
                *slot = m;
            }
            if conditional {
                self.note(
                    at,
                    NoteKind::Conditional,
                    format!("Object.assign({root}, ...) under a condition"),
                );
            }
        }
    }

    // ---- declarations ----

    /// `const a = 1, {b} = c;` after the keyword. Returns the end and the bound
    /// names.
    fn decl(&mut self, mut i: usize, end: usize) -> (usize, Vec<&'s str>) {
        let mut names = Vec::new();
        let e = self.stmt_end(i, end);
        loop {
            let pat = i;
            let pat_end = match self.kind(i, e) {
                TokKind::Ident => i + 1,
                TokKind::LBrace | TokKind::LBracket => self.closer_of(i).map_or(e, |c| c + 1),
                _ => return (e, names),
            };
            let mut j = pat_end;
            if self.kind(j, e) == TokKind::Bang {
                j += 1;
            }
            if self.kind(j, e) == TokKind::Colon {
                j = self.skip_type(j + 1, e, true);
            }
            let v = if self.kind(j, e) == TokKind::Assign {
                // `const X = require('y')`: the CommonJS form of an import declaration.
                if self.tok(j + 1).kw == Kw::Require
                    && self.kind(j + 2, e) == TokKind::LParen
                    && self.kind(j + 3, e) == TokKind::Str
                    && self.closer_of(j + 2) == Some(j + 4)
                    && let Some(local) = self.ident(pat, pat_end)
                {
                    self.out.imports.push(ImportInfo {
                        span: (self.toks[pat].start, self.toks[j + 4].end),
                        spec: self.str_lit(j + 3).unwrap_or_default(),
                        locals: vec![local.to_string()],
                        uses: Vec::new(),
                    });
                }
                let (v, k) = self.expr(j + 1, e);
                j = k;
                v
            } else {
                ConfigVal::Null
            };
            self.bind_pattern(pat, pat_end, v, &mut names);
            // Skip anything the expression parser left (an unsupported tail).
            if self.kind(j, e) != TokKind::Comma {
                j = self.skip_expr(j, e);
            }
            if self.kind(j, e) == TokKind::Comma {
                i = j + 1;
                continue;
            }
            let after = if self.kind(e, end) == TokKind::Semi {
                e + 1
            } else {
                e
            };
            return (after, names);
        }
    }

    /// Binds the pattern `[a, b)` (identifier, object or array pattern) to `v`.
    fn bind_pattern(&mut self, a: usize, b: usize, v: ConfigVal, names: &mut Vec<&'s str>) {
        if !self.step() {
            return;
        }
        match self.kind(a, b) {
            TokKind::Ident => {
                if let Some(n) = self.ident(a, b) {
                    names.push(n);
                    self.bind(n, v);
                }
            }
            TokKind::LBrace => {
                let c = self.closer_of(a).unwrap_or(b).min(b);
                let mut k = a + 1;
                let mut used: Vec<String> = Vec::new();
                while k < c {
                    let e = self.skip_expr(k, c);
                    // `...rest`
                    if self.kind(k, e) == TokKind::Ellipsis {
                        if let Some(n) = self.ident(k + 1, e) {
                            let rest = match &v {
                                ConfigVal::Obj(o) => {
                                    let mut r = o.clone();
                                    r.entries.retain(|(key, _)| !used.contains(key));
                                    ConfigVal::Obj(r)
                                }
                                other => other.clone(),
                            };
                            names.push(n);
                            self.bind(n, rest);
                        }
                    } else {
                        let key = match self.kind(k, e) {
                            TokKind::Ident => self.ident(k, e).map(str::to_string),
                            TokKind::Str => self.str_lit(k),
                            TokKind::Number => {
                                std::str::from_utf8(self.text(k)).ok().map(str::to_string)
                            }
                            _ => None,
                        };
                        if let Some(key) = key {
                            let mut sub = self.member(&v, &key);
                            used.push(key.clone());
                            let mut t = k + 1;
                            let (ta, tb);
                            if self.kind(t, e) == TokKind::Colon {
                                t += 1;
                                ta = t;
                                tb = match self.kind(t, e) {
                                    TokKind::LBrace | TokKind::LBracket => {
                                        self.closer_of(t).map_or(e, |x| x + 1)
                                    }
                                    _ => t + 1,
                                };
                                t = tb;
                            } else {
                                ta = k;
                                tb = k + 1;
                            }
                            if self.kind(t, e) == TokKind::Assign {
                                let (d, _) = self.expr(t + 1, e);
                                sub = default_of(sub, d);
                            }
                            self.bind_pattern(ta, tb, sub, names);
                        }
                    }
                    k = e + 1;
                }
            }
            TokKind::LBracket => {
                let c = self.closer_of(a).unwrap_or(b).min(b);
                let mut k = a + 1;
                let mut idx = 0usize;
                while k < c {
                    let e = self.skip_expr(k, c);
                    if k < e {
                        let sub = match &v {
                            ConfigVal::Arr(xs) => xs.get(idx).cloned().unwrap_or(ConfigVal::Null),
                            other => ConfigVal::Unknown(format!("{}[{idx}]", short(other)).into()),
                        };
                        let pe = match self.kind(k, e) {
                            TokKind::LBrace | TokKind::LBracket => {
                                self.closer_of(k).map_or(e, |x| x + 1)
                            }
                            TokKind::Ellipsis => {
                                if let Some(n) = self.ident(k + 1, e) {
                                    let rest = match &v {
                                        ConfigVal::Arr(xs) => {
                                            ConfigVal::Arr(xs.iter().skip(idx).cloned().collect())
                                        }
                                        other => other.clone(),
                                    };
                                    names.push(n);
                                    self.bind(n, rest);
                                }
                                k = e + 1;
                                idx += 1;
                                continue;
                            }
                            _ => k + 1,
                        };
                        let mut sub = sub;
                        if self.kind(pe, e) == TokKind::Assign {
                            let (d, _) = self.expr(pe + 1, e);
                            sub = default_of(sub, d);
                        }
                        self.bind_pattern(k, pe, sub, names);
                    }
                    idx += 1;
                    k = e + 1;
                }
            }
            _ => {}
        }
    }

    // ---- modules ----

    fn load(&mut self, spec: &str) -> Result<ConfigModule, String> {
        if let Some(r) = self.loaded.get(spec) {
            return r.clone();
        }
        let r = self.host.load(spec);
        if r.is_ok() && !self.out.loaded.iter().any(|s| s == spec) {
            self.out.loaded.push(spec.to_string());
        }
        self.loaded.insert(spec.to_string(), r.clone());
        r
    }

    fn is_local_spec(spec: &str) -> bool {
        spec.starts_with("./")
            || spec.starts_with("../")
            || spec.starts_with('/')
            || spec == "."
            || spec == ".."
    }

    /// The module of a bare specifier, when the host follows its package as
    /// source ([`ConfigHost::load_package`]). Node's own modules are never
    /// asked for.
    fn load_package(&mut self, spec: &str) -> Option<Result<ConfigModule, String>> {
        if api::is_tracked_module(api::normalize_module(spec)) || spec.starts_with("node:") {
            return None;
        }
        if let Some(r) = self.packages.get(spec) {
            return r.clone();
        }
        let r = self.host.load_package(spec);
        if matches!(r, Some(Ok(_))) && !self.out.loaded.iter().any(|s| s == spec) {
            self.out.loaded.push(spec.to_string());
        }
        self.packages.insert(spec.to_string(), r.clone());
        r
    }

    /// Remembers where the functions of another module that `v` (what `kind`
    /// imports from `spec`) makes visible come from, so that a call of one
    /// can be handed to the host.
    fn note_foreign(&mut self, spec: &str, kind: ImportKind<'_>, v: &ConfigVal) {
        let own = self.instance;
        let mut add = |f: &FuncVal, export: &str| {
            if f.id.0 != own && self.foreign.len() < 4096 {
                self.foreign
                    .entry(f.id)
                    .or_insert_with(|| (spec.to_string(), export.to_string()));
            }
        };
        match (v, kind) {
            (ConfigVal::Function(f), ImportKind::Named(n)) => add(f, n),
            (ConfigVal::Function(f), _) => add(f, "default"),
            // `const { defineConfig } = require('x')`, `import * as x`: the members.
            (ConfigVal::Obj(o), ImportKind::Require | ImportKind::Namespace) => {
                for (k, x) in &o.entries {
                    if let ConfigVal::Function(f) = x {
                        add(f, k);
                    }
                }
            }
            _ => {}
        }
    }

    /// The value of `require(spec)` / a namespace import.
    fn module_value(&mut self, at: usize, spec: &str, kind: ImportKind<'_>) -> ConfigVal {
        let followed = if Self::is_local_spec(spec) {
            Some(self.load(spec))
        } else {
            self.load_package(spec)
        };
        if let Some(r) = followed {
            return match r {
                Ok(m) => {
                    let v = match (kind, &m.default_function) {
                        // A default-exported factory is a function for its caller
                        // (`require` gives it when it is `module.exports`).
                        (ImportKind::Default | ImportKind::Named("default"), Some(f))
                            if self.host.factories() =>
                        {
                            ConfigVal::Function(f.clone())
                        }
                        (ImportKind::Require, Some(f))
                            if m.esm_default.is_none() && self.host.factories() =>
                        {
                            ConfigVal::Function(f.clone())
                        }
                        (ImportKind::Require, _) => m.require_value(),
                        (ImportKind::Namespace, _) => m.namespace(),
                        (ImportKind::Default, _) => m.import_default(),
                        (ImportKind::Named(n), _) => m.import_named(n),
                    };
                    self.note_foreign(spec, kind, &v);
                    v
                }
                Err(why) => {
                    self.note(at, NoteKind::NotFollowed, format!("'{spec}': {why}"));
                    ConfigVal::Unknown(format!("module '{spec}' ({why})").into())
                }
            };
        }
        let path = match kind {
            ImportKind::Named(n) if n != "default" => vec![n.to_string()],
            _ => Vec::new(),
        };
        self.external(spec.to_string(), path)
    }

    fn external(&mut self, module: String, path: Vec<String>) -> ConfigVal {
        if let Some(v) = self.host.external(&module, &path) {
            return v;
        }
        ConfigVal::External { module, path }
    }

    fn import_decl(&mut self, i: usize, end: usize) -> usize {
        let e = self.stmt_end(i, end);
        let after = if self.kind(e, end) == TokKind::Semi {
            e + 1
        } else {
            e
        };
        let mut j = i + 1;
        // `import type ...`
        if self.is_word(j, e, "type")
            && !matches!(self.kind(j + 1, e), TokKind::Comma)
            && !self.is_word(j + 1, e, "from")
        {
            return after;
        }
        // Side-effect import.
        if self.kind(j, e) == TokKind::Str {
            return after;
        }
        let mut default_local = None;
        let mut ns_local = None;
        let mut named: Vec<(String, &'s str)> = Vec::new();
        if let Some(n) = self.ident(j, e)
            && self.kind(j + 1, e) != TokKind::Star
        {
            if self.kind(j + 1, e) == TokKind::Assign {
                // TS `import x = require('y')`.
                let (v, _) = self.expr(j + 2, e);
                self.bind(n, v);
                return after;
            }
            default_local = Some(n);
            j += 1;
            if self.kind(j, e) == TokKind::Comma {
                j += 1;
            }
        }
        if self.kind(j, e) == TokKind::Star && self.is_word(j + 1, e, "as") {
            ns_local = self.ident(j + 2, e);
            j += 3;
        }
        if self.kind(j, e) == TokKind::LBrace {
            let c = self.closer_of(j).unwrap_or(e).min(e);
            let mut k = j + 1;
            while k < c {
                let ke = self.skip_expr(k, c);
                let mut a = k;
                if self.is_word(a, ke, "type") && a + 1 < ke {
                    k = ke + 1;
                    continue;
                }
                let imported = match self.kind(a, ke) {
                    TokKind::Ident => self.ident(a, ke).map(str::to_string),
                    TokKind::Str => self.str_lit(a),
                    _ => None,
                };
                a += 1;
                let local = if self.is_word(a, ke, "as") {
                    self.ident(a + 1, ke)
                } else {
                    self.ident(k, ke)
                };
                if let (Some(imp), Some(l)) = (imported, local) {
                    named.push((imp, l));
                }
                k = ke + 1;
            }
            j = c + 1;
        }
        if !self.is_word(j, e, "from") {
            return after;
        }
        let Some(spec) = self.str_lit(j + 1) else {
            return after;
        };
        if e > i && e <= self.toks.len() {
            self.out.imports.push(ImportInfo {
                span: (self.toks[i].start, self.toks[e - 1].end),
                spec: spec.clone(),
                locals: default_local
                    .iter()
                    .chain(&ns_local)
                    .copied()
                    .chain(named.iter().map(|(_, l)| *l))
                    .map(str::to_string)
                    .collect(),
                uses: Vec::new(),
            });
        }
        if let Some(n) = default_local {
            let v = self.module_value(i, &spec, ImportKind::Default);
            self.bind(n, v);
        }
        if let Some(n) = ns_local {
            let v = self.module_value(i, &spec, ImportKind::Namespace);
            self.bind(n, v);
        }
        for (imp, l) in named {
            let v = self.module_value(i, &spec, ImportKind::Named(&imp));
            self.bind(l, v);
        }
        after
    }

    fn export_decl(&mut self, i: usize, end: usize) -> usize {
        let j = i + 1;
        let t = self.tok(j);
        if j >= end {
            return end;
        }
        match t.kw {
            Kw::Default => {
                let k = j + 1;
                // `export default function name() {}` / `export default async function`.
                let fn_at = if self.tok(k).kw == Kw::Function {
                    Some(k)
                } else if self.is_word(k, end, "async") && self.tok(k + 1).kw == Kw::Function {
                    Some(k + 1)
                } else {
                    None
                };
                let (v, e) = match fn_at {
                    Some(f) => match self.function_at(f, end) {
                        Some((def, e)) => {
                            let fv = self.make_func(def, f);
                            if let Some(n) = self.ident(f + 1, end) {
                                self.bind(n, fv.clone());
                            }
                            (fv, e)
                        }
                        None => (
                            ConfigVal::unknown("export default function"),
                            self.stmt_end(k, end),
                        ),
                    },
                    None => {
                        if self.tok(k).kw == Kw::Class {
                            (ConfigVal::unknown("class"), self.skip_braced(k, end))
                        } else {
                            let (v, e) = self.expr(k, end);
                            (v, self.stmt_end(e, end))
                        }
                    }
                };
                let conditional = self.is_conditional();
                let old = self.out.esm_default.take();
                self.out.esm_default = Some(if conditional {
                    ConfigVal::alts([old.unwrap_or(ConfigVal::Null), v])
                } else {
                    v
                });
                e
            }
            Kw::Const | Kw::Let | Kw::Var => {
                let (e, names) = self.decl(j + 1, end);
                for n in names {
                    let v = self.lookup(n).cloned().unwrap_or(ConfigVal::Null);
                    self.set_named(n.to_string(), v);
                }
                e
            }
            Kw::Function => match self.function_at(j, end) {
                Some((def, e)) => {
                    let f = self.make_func(def, j);
                    if let Some(n) = self.ident(j + 1, end) {
                        self.bind(n, f.clone());
                        self.set_named(n.to_string(), f);
                    }
                    e
                }
                None => self.stmt_end(j, end) + 1,
            },
            _ if t.kind == TokKind::Assign => {
                // TS `export = value`.
                let (v, e) = self.expr(j + 1, end);
                self.out.cjs = Some(v);
                self.stmt_end(e, end)
            }
            _ if t.kind == TokKind::LBrace => {
                let c = self.closer_of(j).unwrap_or(end).min(end);
                let e = self.stmt_end(c + 1, end);
                let from = if self.is_word(c + 1, end, "from") {
                    self.str_lit(c + 2)
                } else {
                    None
                };
                let mut k = j + 1;
                while k < c {
                    let ke = self.skip_expr(k, c);
                    let name = match self.kind(k, ke) {
                        TokKind::Ident => self.ident(k, ke).map(str::to_string),
                        TokKind::Str => self.str_lit(k),
                        _ => None,
                    };
                    let alias = if self.is_word(k + 1, ke, "as") {
                        match self.kind(k + 2, ke) {
                            TokKind::Ident => self.ident(k + 2, ke).map(str::to_string),
                            TokKind::Str => self.str_lit(k + 2),
                            _ => None,
                        }
                    } else {
                        name.clone()
                    };
                    if let (Some(n), Some(a)) = (name, alias)
                        && n != "type"
                    {
                        let v = match &from {
                            Some(spec) => {
                                let spec = spec.clone();
                                self.module_value(k, &spec, ImportKind::Named(&n))
                            }
                            None => self.lookup(&n).cloned().unwrap_or(ConfigVal::Null),
                        };
                        if a == "default" {
                            self.out.esm_default = Some(v);
                        } else {
                            self.set_named(a, v);
                        }
                    }
                    k = ke + 1;
                }
                if self.kind(e, end) == TokKind::Semi {
                    e + 1
                } else {
                    e
                }
            }
            _ => {
                // `export type`, `export interface`, `export * from`, `export class`,
                // `export async function`, `export enum`, `export declare`.
                if self.is_word(j, end, "async") && self.tok(j + 1).kw == Kw::Function {
                    return match self.function_at(j + 1, end) {
                        Some((def, e)) => {
                            let f = self.make_func(def, j);
                            if let Some(n) = self.ident(j + 2, end) {
                                self.bind(n, f.clone());
                                self.set_named(n.to_string(), f);
                            }
                            e
                        }
                        None => self.stmt_end(j, end) + 1,
                    };
                }
                if matches!(self.ident(j, end), Some("interface" | "enum" | "namespace"))
                    || t.kw == Kw::Class
                {
                    return self.skip_braced(j, end);
                }
                self.stmt_end(j, end) + 1
            }
        }
    }

    fn set_named(&mut self, n: String, v: ConfigVal) {
        match self.out.named.iter_mut().find(|(k, _)| *k == n) {
            Some(slot) => slot.1 = v,
            None => self.out.named.push((n, v)),
        }
    }

    // ---- functions ----

    fn make_func(&mut self, def: FnDef, at: usize) -> ConfigVal {
        let id = self.funcs.len() as u32;
        // The tokens of the body, and the token that ends the definition.
        let (body, last) = match def.body {
            Body::Expr(a, b) => ((a, b), b.saturating_sub(1)),
            Body::Block(a, b) => ((a, b), b),
            Body::Const(_) | Body::NextJestFactory => ((at, at), at),
        };
        let n = self.toks.len();
        if at < n && last < n {
            let mut captured = Vec::new();
            self.visible_strings(body.0, body.1, &mut captured);
            self.fn_meta.push(FnMeta {
                index: id,
                span: (self.toks[at].start, self.toks[last].end),
                shape: self.function_shape(&def),
                invoked: false,
                captured,
            });
        }
        self.funcs.push(def);
        ConfigVal::Function(FuncVal {
            line: self.line(at),
            id: (self.instance, id),
        })
    }

    /// A `function` at `i` (the keyword): its definition and the index after
    /// its body.
    fn function_at(&self, i: usize, end: usize) -> Option<(FnDef, usize)> {
        let mut k = i + 1;
        if self.kind(k, end) == TokKind::Star {
            k += 1;
        }
        if self.kind(k, end) == TokKind::Ident {
            k += 1;
        }
        // Generic parameters `<T>`.
        if self.kind(k, end) == TokKind::Lt {
            while k < end && self.kind(k, end) != TokKind::LParen {
                k += 1;
            }
        }
        if self.kind(k, end) != TokKind::LParen {
            return None;
        }
        let pc = self.closer_of(k)?;
        let mut b = pc + 1;
        if self.kind(b, end) == TokKind::Colon {
            // Return type: up to the body's `{` at nesting 0.
            while b < end && self.kind(b, end) != TokKind::LBrace {
                if matches!(self.kind(b, end), TokKind::LParen | TokKind::LBracket) {
                    b = self.closer_of(b)?;
                }
                b += 1;
            }
            // `: { a: string } {`: take the last brace pair before the next statement.
            while let Some(c) = self.closer_of(b)
                && self.kind(c + 1, end) == TokKind::LBrace
            {
                b = c + 1;
            }
        }
        if self.kind(b, end) != TokKind::LBrace {
            return None;
        }
        let bc = self.closer_of(b)?;
        Some((
            FnDef {
                params: (k + 1, pc),
                body: Body::Block(b + 1, bc),
            },
            bc + 1,
        ))
    }

    /// An arrow function at `i` (`(a) => ...`, `a => ...`, `async (a) => ...`),
    /// with its definition and the index after it.
    fn arrow_at(&self, i: usize, end: usize) -> Option<(FnDef, usize)> {
        let mut k = i;
        if self.is_word(k, end, "async")
            && matches!(self.kind(k + 1, end), TokKind::LParen | TokKind::Ident)
            && self.tok(k + 1).flags & NEWLINE_BEFORE == 0
        {
            k += 1;
        }
        let params;
        let mut a = match self.kind(k, end) {
            TokKind::Ident if self.kind(k + 1, end) == TokKind::Arrow => {
                params = (k, k + 1);
                k + 1
            }
            TokKind::LParen => {
                let c = self.closer_of(k)?;
                params = (k + 1, c);
                let mut a = c + 1;
                if self.kind(a, end) == TokKind::Colon {
                    a = self.skip_type(a + 1, end, false);
                }
                a
            }
            _ => return None,
        };
        if self.kind(a, end) != TokKind::Arrow {
            return None;
        }
        a += 1;
        if self.kind(a, end) == TokKind::LBrace {
            let c = self.closer_of(a)?;
            return Some((
                FnDef {
                    params,
                    body: Body::Block(a + 1, c),
                },
                c + 1,
            ));
        }
        let e = self.skip_expr(a, end);
        Some((
            FnDef {
                params,
                body: Body::Expr(a, e),
            },
            e,
        ))
    }

    // Invariant: the frame was pushed by this function.
    #[allow(clippy::expect_used)]
    fn invoke(&mut self, f: &FuncVal, args: Vec<ConfigVal>, what: &str) -> ConfigVal {
        if f.id.0 != self.instance {
            // A function another module exports: the host may call it there.
            if let Some((spec, export)) = self.foreign.get(&f.id).cloned()
                && let Some(v) = self.host.call_export(&spec, &export, &args)
            {
                return v;
            }
            return ConfigVal::unknown(format!("call of a function from another module ({what})"));
        }
        if self.frames.len() as u32 >= MAX_CALL_DEPTH {
            return ConfigVal::unknown(format!("call depth limit ({what})"));
        }
        let Some(def) = self.funcs.get(f.id.1 as usize).cloned() else {
            return ConfigVal::unknown(what.to_string());
        };
        if let Some(m) = self.fn_meta.iter_mut().find(|m| m.index == f.id.1) {
            m.invoked = true;
        }
        match &def.body {
            Body::Const(v) => return v.clone(),
            Body::NextJestFactory => {
                let custom = match args.into_iter().next() {
                    // `createJestConfig(async () => ({...}))`: next awaits the function.
                    Some(ConfigVal::Function(g)) => {
                        self.invoke(&g, Vec::new(), "custom jest config")
                    }
                    Some(v) => v,
                    None => ConfigVal::Obj(ConfigObj::default()),
                };
                let id = self.funcs.len() as u32;
                self.funcs.push(FnDef {
                    params: (0, 0),
                    body: Body::Const(next_jest_config(custom, &self.dir)),
                });
                return ConfigVal::Function(FuncVal {
                    line: f.line,
                    id: (self.instance, id),
                });
            }
            Body::Expr(..) | Body::Block(..) => {}
        }
        self.scopes.push(HashMap::new());
        // Parameters.
        let (pa, pb) = def.params;
        let mut k = pa;
        let mut idx = 0usize;
        let mut names = Vec::new();
        while k < pb {
            let e = self.skip_expr(k, pb);
            let mut a = k;
            if self.kind(a, e) == TokKind::Ellipsis {
                let rest = ConfigVal::Arr(args.iter().skip(idx).cloned().collect());
                if let Some(n) = self.ident(a + 1, e) {
                    self.bind(n, rest);
                }
                break;
            }
            // TS parameter modifiers.
            while matches!(
                self.ident(a, e),
                Some("public" | "private" | "protected" | "readonly")
            ) {
                a += 1;
            }
            let pe = match self.kind(a, e) {
                TokKind::LBrace | TokKind::LBracket => self.closer_of(a).map_or(e, |c| c + 1),
                _ => a + 1,
            };
            let mut v = args.get(idx).cloned().unwrap_or(ConfigVal::Null);
            let mut d = pe;
            if self.kind(d, e) == TokKind::Question {
                d += 1;
            }
            if self.kind(d, e) == TokKind::Colon {
                d = self.skip_type(d + 1, e, true);
            }
            if self.kind(d, e) == TokKind::Assign {
                let (dv, _) = self.expr(d + 1, e);
                v = default_of(v, dv);
            }
            if args.get(idx).is_none() && v.is_null() && what == "exported config function" {
                v = ConfigVal::unknown("argument of the exported config function");
            }
            self.bind_pattern(a, pe, v, &mut names);
            idx += 1;
            k = e + 1;
        }
        let r = match def.body {
            Body::Expr(a, b) => self.expr(a, b).0,
            Body::Block(a, b) => {
                self.frames.push(Frame {
                    cond_base: self.cond,
                    returns: Vec::new(),
                    done: None,
                });
                self.hoist(a, b);
                self.block(a, b);
                let fr = self.frames.pop().expect("pushed");
                match fr.done {
                    Some(v) => v,
                    None => {
                        let mut all = fr.returns;
                        all.push(ConfigVal::Null);
                        ConfigVal::alts(all)
                    }
                }
            }
            // Handled above.
            Body::Const(_) | Body::NextJestFactory => ConfigVal::Null,
        };
        self.scopes.pop();
        r
    }

    // ---- expressions ----

    fn args(&mut self, i: usize, c: usize) -> Vec<ConfigVal> {
        let mut out = Vec::new();
        let mut k = i;
        while k < c {
            if self.kind(k, c) == TokKind::Ellipsis {
                let (v, e) = self.expr(k + 1, c);
                match v {
                    ConfigVal::Arr(xs) => out.extend(xs),
                    v => out.push(ConfigVal::Unknown(format!("...{}", short(&v)).into())),
                }
                k = self.after_item(e, c);
                continue;
            }
            let (v, e) = self.expr(k, c);
            out.push(v);
            k = self.after_item(e, c);
        }
        out
    }

    /// After an item at `e` in a comma-separated list ending at `c`: skips
    /// anything unparsed up to the next comma and past it.
    fn after_item(&self, e: usize, c: usize) -> usize {
        let e = if self.kind(e, c) == TokKind::Comma {
            e
        } else {
            self.skip_expr(e, c)
        };
        if e < c && self.kind(e, c) == TokKind::Comma {
            e + 1
        } else {
            c.max(e + 1)
        }
    }

    /// Evaluates the expression at `i`; returns the value and the index after
    /// it.
    fn expr(&mut self, i: usize, end: usize) -> (ConfigVal, usize) {
        if !self.step() || i >= end {
            return (
                ConfigVal::unknown("empty expression"),
                i.max(end.min(i + 1)),
            );
        }
        self.nest += 1;
        if self.nest > MAX_NEST {
            self.nest -= 1;
            let e = self.skip_expr(i, end);
            return (ConfigVal::unknown("nesting limit"), e);
        }
        let r = self.ternary(i, end);
        self.nest -= 1;
        r
    }

    fn ternary(&mut self, i: usize, end: usize) -> (ConfigVal, usize) {
        let notes_before = self.out.notes.len();
        let (c, j) = self.logical(i, end);
        if self.kind(j, end) != TokKind::Question {
            return (c, j);
        }
        // The condition only selects between branches, and both are kept: an
        // environment-dependent condition makes nothing unknown.
        let mut k = notes_before;
        while k < self.out.notes.len() {
            if self.out.notes[k].kind == NoteKind::EnvDependent {
                self.out.notes.remove(k);
            } else {
                k += 1;
            }
        }
        let (a, k) = self.expr(j + 1, end);
        if self.kind(k, end) != TokKind::Colon {
            return (ConfigVal::alts([a, ConfigVal::Null]), k);
        }
        let (b, l) = self.expr(k + 1, end);
        let v = match c {
            ConfigVal::Bool(true) => a,
            ConfigVal::Bool(false) | ConfigVal::Null => b,
            _ => ConfigVal::alts([a, b]),
        };
        (v, l)
    }

    fn logical(&mut self, i: usize, end: usize) -> (ConfigVal, usize) {
        let (mut l, mut j) = self.postfix_types(i, end);
        loop {
            let op = if self.is_op(j, end, "??") {
                1
            } else if self.is_op(j, end, "||") {
                2
            } else if self.is_op(j, end, "&&") {
                3
            } else {
                break;
            };
            let (r, k) = self.postfix_types(j + 1, end);
            l = if op == 3 {
                match l {
                    ConfigVal::Null | ConfigVal::Bool(false) => l,
                    _ => r,
                }
            } else {
                or_combine(l, r, op == 2)
            };
            j = k;
        }
        (l, j)
    }

    /// A unary value followed by TypeScript `as T` / `satisfies T` / `!`.
    fn postfix_types(&mut self, i: usize, end: usize) -> (ConfigVal, usize) {
        let (v, mut j) = self.unary(i, end);
        loop {
            if self.is_word(j, end, "as") || self.is_word(j, end, "satisfies") {
                j = self.skip_type(j + 1, end, false);
                continue;
            }
            if self.kind(j, end) == TokKind::Bang {
                j += 1;
                continue;
            }
            break;
        }
        (v, j)
    }

    fn unary(&mut self, i: usize, end: usize) -> (ConfigVal, usize) {
        let t = self.tok(i);
        if t.kind == TokKind::Ident && t.kw == Kw::Await {
            return self.unary(i + 1, end);
        }
        // `<T>value` (TS cast in .ts, not .tsx).
        if self.structural_start(i, end) {
            let (v, j) = self.primary(i, end);
            return self.postfix(v, i, j, end);
        }
        if let Some(r) = self.realpath_expr(i, end) {
            return r;
        }
        self.leaf(i, end)
    }

    /// A scalar expression that is one call of `fs.realpathSync(p)` or of
    /// `path.resolve(...)` / `path.join(...)` with such a call among its
    /// arguments (the leaf evaluator knows no file system). The real path
    /// comes from the host ([`ConfigHost::realpath`]); a path that does not
    /// exist stays as written, since `realpathSync` would throw there and
    /// the config would not load at all.
    fn realpath_expr(&mut self, i: usize, end: usize) -> Option<(ConfigVal, usize)> {
        let e = self.leaf_end(i, end);
        if !(i..e).any(|k| self.is_word(k, e, "realpathSync")) {
            return None;
        }
        // The callee: `require('fs')` or a binding, then `.member` names, then `(`.
        let mut k = i;
        let (module, mut path): (String, Vec<String>) =
            if self.tok(k).kw == Kw::Require && self.kind(k + 1, e) == TokKind::LParen {
                if self.closer_of(k + 1) != Some(k + 3) {
                    return None;
                }
                let m = self.str_lit(k + 2)?;
                k += 4;
                (m, Vec::new())
            } else {
                let Some(ConfigVal::External { module, path }) =
                    self.ident(k, e).and_then(|n| self.lookup(n))
                else {
                    return None;
                };
                k += 1;
                (module.clone(), path.clone())
            };
        while self.kind(k, e) == TokKind::Dot {
            path.push(self.ident(k + 1, e)?.to_string());
            k += 2;
        }
        if self.kind(k, e) != TokKind::LParen || self.closer_of(k) != Some(e - 1) {
            return None;
        }
        let module = api::normalize_module(&module).to_string();
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        let what = match (module.as_str(), path.as_slice()) {
            ("fs", ["realpathSync"] | ["realpathSync", "native"]) => 0,
            ("path" | "path/posix", ["resolve"]) => 1,
            ("path" | "path/posix", ["join"]) => 2,
            _ => return None,
        };
        let args = self.args(k + 1, e - 1);
        let strs: Vec<&str> = args.iter().map(ConfigVal::as_str).collect::<Option<_>>()?;
        let v = match what {
            0 => {
                let p = *strs.first().filter(|p| p.starts_with('/'))?;
                self.host.realpath(p).unwrap_or_else(|| p.to_string())
            }
            1 => {
                // From the last absolute argument on.
                let start = strs.iter().rposition(|s| s.starts_with('/'))?;
                path::normalize_str(&strs[start..].join("/"))
            }
            _ => {
                if !strs.first()?.starts_with('/') {
                    return None;
                }
                path::normalize_str(&strs.join("/"))
            }
        };
        Some((ConfigVal::str(v), e))
    }

    /// Whether the expression at `i` is handled structurally (rather than as a
    /// scalar leaf through the partial evaluator).
    fn structural_start(&self, i: usize, end: usize) -> bool {
        let t = self.tok(i);
        match t.kind {
            TokKind::LBrace | TokKind::LBracket | TokKind::Regex => true,
            TokKind::LParen => true,
            TokKind::Ident => {
                let Some(name) = self.ident(i, end) else {
                    return false;
                };
                let next = self.kind(i + 1, end);
                if next == TokKind::Arrow {
                    return true;
                }
                match t.kw {
                    Kw::Function => return true,
                    Kw::Import => return next == TokKind::LParen,
                    Kw::Require => {
                        return next == TokKind::LParen
                            || (next == TokKind::Dot && self.is_word(i + 2, end, "resolve"));
                    }
                    Kw::New => {
                        return self.is_word(i + 1, end, "RegExp")
                            && self.kind(i + 2, end) == TokKind::LParen;
                    }
                    Kw::Typeof | Kw::Void | Kw::Delete => return false,
                    _ => {}
                }
                match name {
                    "true" | "false" | "null" | "undefined" => return true,
                    "async" => {
                        return self.arrow_at(i, end).is_some()
                            || self.tok(i + 1).kw == Kw::Function;
                    }
                    "Object" => return next == TokKind::Dot,
                    "module" => return next == TokKind::Dot && self.is_word(i + 2, end, "exports"),
                    "exports" if self.lookup(name).is_none() => return true,
                    _ => {}
                }
                match self.lookup(name) {
                    None => false,
                    // `name.replace(...)` on a known string is evaluated here; every other
                    // string expression (`name + '/x'`) goes through the leaf evaluator.
                    Some(ConfigVal::Str(Value::Str(_))) => {
                        next == TokKind::Dot
                            && matches!(self.ident(i + 2, end), Some(m) if is_string_method(m))
                            && self.kind(i + 3, end) == TokKind::LParen
                    }
                    Some(ConfigVal::Str(_) | ConfigVal::CwdPath(_) | ConfigVal::Num(_)) => false,
                    Some(ConfigVal::External { module, .. }) => {
                        !api::is_tracked_module(api::normalize_module(module))
                    }
                    Some(_) => true,
                }
            }
            _ => false,
        }
    }

    fn primary(&mut self, i: usize, end: usize) -> (ConfigVal, usize) {
        let t = self.tok(i);
        match t.kind {
            TokKind::LBrace => {
                let c = self.closer_of(i).unwrap_or(end).min(end);
                (self.object(i + 1, c), c + 1)
            }
            TokKind::LBracket => {
                let c = self.closer_of(i).unwrap_or(end).min(end);
                (self.array(i + 1, c), c + 1)
            }
            TokKind::Regex => {
                let txt = String::from_utf8_lossy(self.text(i)).into_owned();
                let v = match txt.rfind('/') {
                    Some(p) if p > 0 => ConfigVal::Regex {
                        source: txt[1..p].to_string(),
                        flags: txt[p + 1..].to_string(),
                    },
                    _ => ConfigVal::Unknown(txt.into()),
                };
                (v, i + 1)
            }
            TokKind::LParen => {
                if let Some((def, e)) = self.arrow_at(i, end) {
                    return (self.make_func(def, i), e);
                }
                let c = self.closer_of(i).unwrap_or(end).min(end);
                let mut k = i + 1;
                let mut v = ConfigVal::Null;
                while k < c {
                    let (x, e) = self.expr(k, c);
                    v = x;
                    k = self.after_item(e, c);
                }
                (v, c + 1)
            }
            TokKind::Ident => self.ident_primary(i, end),
            _ => self.leaf(i, end),
        }
    }

    fn ident_primary(&mut self, i: usize, end: usize) -> (ConfigVal, usize) {
        let t = self.tok(i);
        let name = self.ident(i, end).unwrap_or("");
        if let Some((def, e)) = self.arrow_at(i, end) {
            return (self.make_func(def, i), e);
        }
        if t.kw == Kw::Function {
            return match self.function_at(i, end) {
                Some((def, e)) => (self.make_func(def, i), e),
                None => {
                    let e = self.skip_expr(i, end);
                    (ConfigVal::unknown(self.reason(i, e)), e)
                }
            };
        }
        if name == "async" && self.tok(i + 1).kw == Kw::Function {
            return match self.function_at(i + 1, end) {
                Some((def, e)) => (self.make_func(def, i), e),
                None => {
                    let e = self.skip_expr(i, end);
                    (ConfigVal::unknown(self.reason(i, e)), e)
                }
            };
        }
        // `new RegExp(source, flags)` with a known source: the same value as a regex
        // literal.
        if t.kw == Kw::New
            && self.is_word(i + 1, end, "RegExp")
            && self.kind(i + 2, end) == TokKind::LParen
        {
            let c = self.closer_of(i + 2).unwrap_or(end).min(end);
            let mut args = self.args(i + 3, c).into_iter();
            let source = args.next();
            let flags = match args.next() {
                None | Some(ConfigVal::Null) => Some(None),
                Some(f) => f.as_str().map(|f| Some(f.to_string())),
            };
            let v = match (source, flags) {
                (Some(ConfigVal::Str(Value::Str(source))), Some(flags)) => ConfigVal::Regex {
                    source,
                    flags: flags.unwrap_or_default(),
                },
                // `new RegExp(/x/)`, `new RegExp(/x/, 'i')`.
                (Some(ConfigVal::Regex { source, flags: own }), Some(flags)) => ConfigVal::Regex {
                    source,
                    flags: flags.unwrap_or(own),
                },
                _ => ConfigVal::unknown(self.reason(i, c + 1)),
            };
            return (v, c + 1);
        }
        match name {
            "true" => return (ConfigVal::Bool(true), i + 1),
            "false" => return (ConfigVal::Bool(false), i + 1),
            "null" | "undefined" => return (ConfigVal::Null, i + 1),
            _ => {}
        }
        // `require(...)`, `require.resolve(...)`, `import(...)`.
        if t.kw == Kw::Require || t.kw == Kw::Import {
            let resolve = self.kind(i + 1, end) == TokKind::Dot;
            let lp = if resolve { i + 3 } else { i + 1 };
            if self.kind(lp, end) != TokKind::LParen {
                return self.leaf(i, end);
            }
            let c = self.closer_of(lp).unwrap_or(end).min(end);
            let args = self.args(lp + 1, c);
            let spec = args.first().and_then(|a| a.as_str()).map(str::to_string);
            let v = match (spec, resolve) {
                // `require.resolve(spec, { paths: [dir, ...] })` searches from each `dir`
                // instead of the module's directory.
                (Some(spec), true) => match args.get(1).and_then(|o| o.field("paths")) {
                    None | Some(ConfigVal::Null) => ConfigVal::Resolve {
                        spec,
                        from_dir: self.dir.clone(),
                    },
                    Some(ConfigVal::Arr(dirs))
                        if !dirs.is_empty() && dirs.iter().all(|d| d.as_str().is_some()) =>
                    {
                        ConfigVal::alts(dirs.iter().map(|d| ConfigVal::Resolve {
                            spec: spec.clone(),
                            from_dir: d.as_str().unwrap_or_default().to_string(),
                        }))
                    }
                    Some(_) => ConfigVal::Unknown(self.reason(i, c + 1)),
                },
                (Some(spec), false) => {
                    let kind = if t.kw == Kw::Import {
                        ImportKind::Namespace
                    } else {
                        ImportKind::Require
                    };
                    self.module_value(i, &spec, kind)
                }
                (None, _) => {
                    let r = self.reason(i, c + 1);
                    self.note(i, NoteKind::Unsupported, format!("computed module: {r}"));
                    ConfigVal::Unknown(r)
                }
            };
            return (v, c + 1);
        }
        if name == "module" && self.is_word(i + 2, end, "exports") {
            let v = self
                .out
                .cjs
                .clone()
                .unwrap_or_else(|| ConfigVal::Obj(ConfigObj::default()));
            return (v, i + 3);
        }
        if name == "exports" && self.lookup(name).is_none() {
            let v = self
                .out
                .cjs
                .clone()
                .unwrap_or_else(|| ConfigVal::Obj(ConfigObj::default()));
            return (v, i + 1);
        }
        if name == "Object" && self.kind(i + 1, end) == TokKind::Dot {
            let m = self.ident(i + 2, end).unwrap_or("");
            if self.kind(i + 3, end) == TokKind::LParen {
                let c = self.closer_of(i + 3).unwrap_or(end).min(end);
                let args = self.args(i + 4, c);
                let v = match m {
                    "assign" => {
                        let mut o = ConfigObj::default();
                        for a in args {
                            spread_into(&mut o, a, false);
                        }
                        ConfigVal::Obj(o)
                    }
                    "freeze" | "seal" => args.into_iter().next().unwrap_or(ConfigVal::Null),
                    "keys" | "values" | "entries" | "fromEntries" => {
                        let why = self.reason(i, c + 1);
                        let listed = args.into_iter().next().and_then(|a| match m {
                            "fromEntries" => object_from_entries(a, &why),
                            _ => object_list(m, a, &why),
                        });
                        listed.unwrap_or_else(|| ConfigVal::unknown(why))
                    }
                    _ => ConfigVal::unknown(self.reason(i, c + 1)),
                };
                return (v, c + 1);
            }
            return self.leaf(i, end);
        }
        match self.lookup(name).cloned() {
            Some(v) => (v, i + 1),
            None => self.leaf(i, end),
        }
    }

    /// Member accesses, indexing and calls after a structural value.
    fn postfix(
        &mut self,
        mut v: ConfigVal,
        start: usize,
        mut j: usize,
        end: usize,
    ) -> (ConfigVal, usize) {
        loop {
            if !self.step() {
                return (v, j);
            }
            match self.kind(j, end) {
                TokKind::Dot | TokKind::OptChain => {
                    if self.kind(j, end) == TokKind::OptChain
                        && matches!(self.kind(j + 1, end), TokKind::LParen | TokKind::LBracket)
                    {
                        j += 1;
                        continue;
                    }
                    let Some(name) = self.ident(j + 1, end) else {
                        break;
                    };
                    let name = name.to_string();
                    if self.kind(j + 2, end) == TokKind::LParen {
                        let c = self.closer_of(j + 2).unwrap_or(end).min(end);
                        let args = self.args(j + 3, c);
                        v = self.method(v, &name, args, start, c + 1);
                        j = c + 1;
                    } else {
                        v = self.member(&v, &name);
                        j += 2;
                    }
                }
                TokKind::LBracket => {
                    let c = self.closer_of(j).unwrap_or(end).min(end);
                    let (k, _) = self.expr(j + 1, c);
                    v = match k {
                        ConfigVal::Str(Value::Str(s)) | ConfigVal::Num(s) => self.member(&v, &s),
                        k => match self.lookup_any(&v, &k) {
                            Some(x) => x,
                            None => ConfigVal::Unknown(self.reason(start, c + 1)),
                        },
                    };
                    j = c + 1;
                }
                TokKind::LParen => {
                    let c = self.closer_of(j).unwrap_or(end).min(end);
                    let args = self.args(j + 1, c);
                    v = self.call(v, args, start, c + 1);
                    j = c + 1;
                }
                TokKind::Bang if self.tok(j).flags & NEWLINE_BEFORE == 0 => j += 1,
                TokKind::Template | TokKind::TemplateHead => {
                    let e = self.leaf_end(j, end);
                    v = ConfigVal::Unknown(self.reason(start, e));
                    j = e;
                }
                _ => break,
            }
        }
        // Operators the structural evaluator does not model (`+`, comparisons...).
        if j >= end
            || self.asi_break(j)
            || self.is_word(j, end, "as")
            || self.is_word(j, end, "satisfies")
        {
            return (v, j);
        }
        let e = self.leaf_end(j, end);
        if e > j {
            let r = self.reason(start, e);
            return (ConfigVal::Unknown(r), e);
        }
        (v, j)
    }

    /// `table[key]` where `key` is not one known string: over an object literal
    /// whose keys are all known (a lookup table, `patterns[process.env.KIND
    /// ?? 'e2e']`), the value is one of the table's values: the entry of
    /// each known alternative of the key, and every entry for an unknown
    /// one. Only for a host that evaluates factories
    /// ([`ConfigHost::factories`]); `None` otherwise and for anything else.
    fn lookup_any(&mut self, table: &ConfigVal, key: &ConfigVal) -> Option<ConfigVal> {
        let ConfigVal::Obj(o) = table else {
            return None;
        };
        if o.open.is_some() || o.entries.is_empty() || !self.host.factories() {
            return None;
        }
        let mut out = Vec::new();
        for k in key.alternatives() {
            match k {
                ConfigVal::Str(Value::Str(s)) | ConfigVal::Num(s) => {
                    out.push(o.get(s).cloned().unwrap_or(ConfigVal::Null))
                }
                ConfigVal::Unknown(_) | ConfigVal::Str(_) | ConfigVal::External { .. } => {
                    out.extend(o.entries.iter().map(|(_, x)| x.clone()))
                }
                _ => return None,
            }
        }
        Some(ConfigVal::alts(out))
    }

    fn member(&mut self, v: &ConfigVal, name: &str) -> ConfigVal {
        match v {
            ConfigVal::External { module, path } => {
                let mut p = path.clone();
                p.push(name.to_string());
                self.external(module.clone(), p)
            }
            ConfigVal::Arr(xs) => match name.parse::<usize>() {
                Ok(i) => xs.get(i).cloned().unwrap_or(ConfigVal::Null),
                Err(_) if name == "length" => ConfigVal::Num(xs.len().to_string()),
                Err(_) => ConfigVal::unknown(format!("array.{name}")),
            },
            ConfigVal::Null => ConfigVal::Null,
            ConfigVal::Alts(vs) => {
                let vs = vs.clone();
                let out: Vec<ConfigVal> = vs.iter().map(|x| self.member(x, name)).collect();
                ConfigVal::alts(out)
            }
            other => other.field(name).unwrap_or_else(|| match other {
                ConfigVal::Obj(_) => ConfigVal::Null,
                _ => ConfigVal::unknown(format!("{}.{name}", short(other))),
            }),
        }
    }

    /// An array kept as a superset of the real value. The imprecision stays on
    /// the value (an unknown alternative next to the superset), so only
    /// consumers of that field report it, not every reader of the file: a
    /// `transformIgnorePatterns.filter(...)` the tracer never reads makes
    /// no test uncertain.
    fn superset(&mut self, v: ConfigVal, start: usize, end: usize) -> ConfigVal {
        let r = self.reason(start, end);
        ConfigVal::alts([
            v,
            ConfigVal::Unknown(format!("array method kept as a superset: {r}").into()),
        ])
    }

    fn method(
        &mut self,
        v: ConfigVal,
        name: &str,
        args: Vec<ConfigVal>,
        start: usize,
        end: usize,
    ) -> ConfigVal {
        match (&v, name) {
            (ConfigVal::Arr(xs), "concat") => {
                let mut out = xs.clone();
                for a in args {
                    match a {
                        ConfigVal::Arr(ys) => out.extend(ys),
                        a => out.push(a),
                    }
                }
                ConfigVal::Arr(out)
            }
            // On an empty array these are exact.
            (ConfigVal::Arr(xs), "filter" | "slice" | "reverse" | "sort" | "flat")
                if xs.is_empty() =>
            {
                v
            }
            // `filter` with a callback whose results are all known: exact.
            (ConfigVal::Arr(xs), "filter")
                if matches!(args.first(), Some(ConfigVal::Function(_))) =>
            {
                let Some(ConfigVal::Function(f)) = args.first().cloned() else {
                    return v;
                };
                let mut out = Vec::new();
                let mut exact = true;
                for (i, x) in xs.clone().into_iter().enumerate() {
                    let r = self.invoke(
                        &f,
                        vec![x.clone(), ConfigVal::Num(i.to_string())],
                        "filter callback",
                    );
                    match r {
                        ConfigVal::Bool(false) | ConfigVal::Null => {}
                        ConfigVal::Bool(true) => out.push(x),
                        _ => {
                            exact = false;
                            out.push(x);
                        }
                    }
                }
                if exact {
                    ConfigVal::Arr(out)
                } else {
                    self.superset(ConfigVal::Arr(out), start, end)
                }
            }
            (ConfigVal::Arr(_), "filter" | "slice" | "reverse" | "sort" | "flat") => {
                self.superset(v, start, end)
            }
            (ConfigVal::Arr(xs), "map" | "flatMap") => {
                if let Some(ConfigVal::Function(f)) = args.first() {
                    let f = f.clone();
                    let xs = xs.clone();
                    let mut out = Vec::new();
                    for (i, x) in xs.into_iter().enumerate() {
                        let r =
                            self.invoke(&f, vec![x, ConfigVal::Num(i.to_string())], "map callback");
                        match (name, r) {
                            ("flatMap", ConfigVal::Arr(ys)) => out.extend(ys),
                            (_, r) => out.push(r),
                        }
                    }
                    ConfigVal::Arr(out)
                } else {
                    ConfigVal::Unknown(self.reason(start, end))
                }
            }
            // `xs.join(sep)` over known strings.
            (ConfigVal::Arr(xs), "join") => {
                let sep = match args.first() {
                    None | Some(ConfigVal::Null) => Some(","),
                    Some(a) => a.as_str(),
                };
                let parts: Option<Vec<&str>> = xs
                    .iter()
                    .map(|x| match x {
                        ConfigVal::Num(n) => Some(n.as_str()),
                        ConfigVal::Null => Some(""),
                        x => x.as_str(),
                    })
                    .collect();
                match (sep, parts) {
                    (Some(sep), Some(parts)) => ConfigVal::str(parts.join(sep)),
                    _ => ConfigVal::Unknown(self.reason(start, end)),
                }
            }
            (ConfigVal::Str(Value::Str(s)), m) if is_string_method(m) => {
                match string_method(s, m, &args) {
                    Some(v) => v,
                    None => ConfigVal::Unknown(self.reason(start, end)),
                }
            }
            (ConfigVal::Str(_), "replace" | "replaceAll" | "split" | "trim" | "toLowerCase") => {
                ConfigVal::Unknown(self.reason(start, end))
            }
            (ConfigVal::Alts(vs), _) => {
                let vs = vs.clone();
                let out: Vec<ConfigVal> = vs
                    .into_iter()
                    .map(|x| self.method(x, name, args.clone(), start, end))
                    .collect();
                ConfigVal::alts(out)
            }
            _ => {
                let f = self.member(&v, name);
                self.call(f, args, start, end)
            }
        }
    }

    fn call(&mut self, f: ConfigVal, args: Vec<ConfigVal>, start: usize, end: usize) -> ConfigVal {
        match f {
            ConfigVal::Function(fv) => self.invoke(&fv, args, "call"),
            ConfigVal::External { module, path } if is_next_jest(&module, &path) => {
                // `nextJest(options)` returns `createJestConfig`.
                let id = self.funcs.len() as u32;
                self.funcs.push(FnDef {
                    params: (0, 0),
                    body: Body::NextJestFactory,
                });
                ConfigVal::Function(FuncVal {
                    line: self.line(start),
                    id: (self.instance, id),
                })
            }
            // `require('fs').realpathSync(p)`: see [`Ev::realpath_expr`].
            ConfigVal::External { module, path }
                if api::normalize_module(&module) == "fs"
                    && matches!(
                        path.iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .as_slice(),
                        ["realpathSync"] | ["realpathSync", "native"]
                    ) =>
            {
                match args.first().and_then(ConfigVal::as_str) {
                    Some(p) if p.starts_with('/') => {
                        ConfigVal::str(self.host.realpath(p).unwrap_or_else(|| p.to_string()))
                    }
                    _ => ConfigVal::Unknown(self.reason(start, end)),
                }
            }
            ConfigVal::External { module, path } => {
                let member = path.last().map(String::as_str).unwrap_or("");
                match member {
                    "defineConfig" | "defineProject" | "defineWorkspace" | "defineVitestConfig"
                    | "defineJestConfig"
                        if !args.is_empty() =>
                    {
                        args.into_iter().next().unwrap_or(ConfigVal::Null)
                    }
                    "mergeConfig" if args.len() >= 2 => {
                        let mut it = args.into_iter();
                        let a = it.next().unwrap_or(ConfigVal::Null);
                        let b = it.next().unwrap_or(ConfigVal::Null);
                        merge_deep(a, b)
                    }
                    _ => {
                        let r = self.reason(start, end);
                        ConfigVal::Unknown(format!("{r} (call into '{module}')").into())
                    }
                }
            }
            _ => ConfigVal::Unknown(self.reason(start, end)),
        }
    }

    fn object(&mut self, i: usize, c: usize) -> ConfigVal {
        let mut o = ConfigObj::default();
        let mut k = i;
        while k < c {
            if !self.step() {
                o.open = Some("evaluation limit".into());
                break;
            }
            let e = self.skip_expr(k, c);
            if k >= e {
                k = e + 1;
                continue;
            }
            if self.kind(k, e) == TokKind::Ellipsis {
                let (v, _) = self.expr(k + 1, e);
                spread_into(&mut o, v, false);
                k = e + 1;
                continue;
            }
            let mut a = k;
            // Modifiers: `get x()`, `set x()`, `async x()`, `*x()`.
            if matches!(self.ident(a, e), Some("get" | "set" | "async"))
                && !matches!(
                    self.kind(a + 1, e),
                    TokKind::Colon | TokKind::LParen | TokKind::Comma | TokKind::Eof
                )
            {
                a += 1;
            }
            if self.kind(a, e) == TokKind::Star {
                a += 1;
            }
            let (key, after_key) = match self.kind(a, e) {
                TokKind::Ident => (self.ident(a, e).map(str::to_string), a + 1),
                TokKind::Str | TokKind::Template => (self.str_lit(a), a + 1),
                TokKind::Number => (
                    std::str::from_utf8(self.text(a)).ok().map(str::to_string),
                    a + 1,
                ),
                TokKind::LBracket => {
                    let kc = self.closer_of(a).unwrap_or(e).min(e);
                    let (kv, _) = self.expr(a + 1, kc);
                    (kv.as_str().map(str::to_string), kc + 1)
                }
                _ => (None, e),
            };
            let value = match self.kind(after_key, e) {
                TokKind::Colon => {
                    if let Some(name) = &key {
                        self.note_property(name, after_key + 1, e);
                    }
                    let (v, _) = self.expr(after_key + 1, e);
                    v
                }
                TokKind::LParen | TokKind::Lt => {
                    // Method: `key(...) { ... }`.
                    let mut p = after_key;
                    while p < e && self.kind(p, e) != TokKind::LParen {
                        p += 1;
                    }
                    match self.closer_of(p) {
                        Some(pc) => {
                            let mut b = pc + 1;
                            if self.kind(b, e) == TokKind::Colon {
                                while b < e && self.kind(b, e) != TokKind::LBrace {
                                    b += 1;
                                }
                            }
                            match self.closer_of(b) {
                                Some(bc) if self.kind(b, e) == TokKind::LBrace => {
                                    let def = FnDef {
                                        params: (p + 1, pc),
                                        body: Body::Block(b + 1, bc),
                                    };
                                    self.make_func(def, a)
                                }
                                _ => ConfigVal::unknown("method"),
                            }
                        }
                        None => ConfigVal::unknown("method"),
                    }
                }
                _ => {
                    // Shorthand `{ key }`.
                    match self.ident(a, e) {
                        Some(n) => match self.lookup(n).cloned() {
                            Some(v) => v,
                            None => self.leaf(a, a + 1).0,
                        },
                        None => ConfigVal::unknown(self.reason(k, e)),
                    }
                }
            };
            match key {
                Some(key) => o.set(key, value),
                None => {
                    o.open = Some(self.reason(k, e));
                }
            }
            k = e + 1;
        }
        ConfigVal::Obj(o)
    }

    fn array(&mut self, i: usize, c: usize) -> ConfigVal {
        let mut out = Vec::new();
        let mut k = i;
        while k < c {
            if !self.step() {
                out.push(ConfigVal::unknown("evaluation limit"));
                break;
            }
            if self.kind(k, c) == TokKind::Comma {
                // Hole.
                k += 1;
                continue;
            }
            if self.kind(k, c) == TokKind::Ellipsis {
                let (v, e) = self.expr(k + 1, c);
                spread_array(&mut out, v);
                k = self.after_item(e, c);
                continue;
            }
            let (v, e) = self.expr(k, c);
            out.push(v);
            k = self.after_item(e, c);
        }
        ConfigVal::Arr(out)
    }

    /// A scalar expression through the partial evaluator.
    fn leaf(&mut self, i: usize, end: usize) -> (ConfigVal, usize) {
        let e = self.leaf_end(i, end).max((i + 1).min(end));
        if i >= e {
            return (ConfigVal::unknown("empty"), e);
        }
        // Numbers and plain booleans are cheap to recognize here.
        if e == i + 1 && self.tok(i).kind == TokKind::Number {
            return (
                ConfigVal::Num(String::from_utf8_lossy(self.text(i)).into_owned()),
                e,
            );
        }
        let mut consts: HashMap<&'s str, (JsVal, bool)> = HashMap::new();
        for scope in &self.scopes {
            for (k, v) in scope {
                if let Some(j) = to_jsval(v) {
                    consts.insert(*k, (j, false));
                }
            }
        }
        for (n, v) in [
            ("__dirname", self.dir.clone()),
            ("__filename", self.file.clone()),
        ] {
            if self.lookup(n).is_none() {
                consts.insert(n, (JsVal::Str(Value::Str(v)), false));
            }
        }
        let loaders: [api::Loader<'s>; 0] = [];
        let sc = Scope {
            src: self.src,
            consts: &consts,
            apis: &self.apis,
            loaders: &loaders,
            file_name: Some(&self.file_name),
        };
        let out = evaluator::evaluate(&sc, &self.toks[i..e]);
        if out.env {
            let r = self.reason(i, e);
            self.note(i, NoteKind::EnvDependent, r.to_string());
        }
        let v = self.convert_jsval(out.val);
        if v.has_unknown() {
            self.note_opaque(i, e);
        }
        // The URL of a directory ends with `/`, and so does its path:
        // `fileURLToPath(new URL('.', import.meta.url))` is `<dir>/`, which configs
        // concatenate (`${root}$1`). The path model drops trailing slashes, so it is
        // put back for exactly this form.
        let v = match v {
            ConfigVal::Str(Value::Str(s)) if !s.ends_with('/') && self.is_dir_url(i, e) => {
                ConfigVal::str(format!("{s}/"))
            }
            v => v,
        };
        (v, e)
    }

    /// Whether the tokens `i..e` are `new URL('<dir>', import.meta.url)`,
    /// optionally inside `fileURLToPath(...)` or followed by `.pathname`,
    /// where `<dir>` names a directory by URL rules: `.`, `..` or a path
    /// ending with `/`.
    fn is_dir_url(&self, i: usize, e: usize) -> bool {
        let (mut a, mut b) = (i, e);
        if self.is_word(a, b, "fileURLToPath")
            && self.kind(a + 1, b) == TokKind::LParen
            && self.closer_of(a + 1) == Some(b - 1)
        {
            (a, b) = (a + 2, b - 1);
        } else if b >= a + 2
            && self.kind(b - 2, b) == TokKind::Dot
            && self.is_word(b - 1, b, "pathname")
        {
            b -= 2;
        }
        if !(self.tok(a).kw == Kw::New
            && self.is_word(a + 1, b, "URL")
            && self.kind(a + 2, b) == TokKind::LParen
            && self.closer_of(a + 2) == Some(b - 1))
        {
            return false;
        }
        let base = ["import", ".", "meta", ".", "url"];
        if b != a + 4 + 1 + base.len() + 1
            || self.kind(a + 4, b) != TokKind::Comma
            || !base
                .iter()
                .enumerate()
                .all(|(k, w)| self.text(a + 5 + k) == w.as_bytes())
        {
            return false;
        }
        self.str_lit(a + 3)
            .is_some_and(|s| s == "." || s == ".." || s.ends_with('/') || s.ends_with("/.."))
    }

    fn convert_jsval(&self, v: JsVal) -> ConfigVal {
        match v {
            JsVal::Str(v) => ConfigVal::Str(v),
            JsVal::Path(Anchor::FileDir, v) => {
                let joined = path::join([Value::str(self.dir.clone()), v]);
                ConfigVal::Str(path::normalize(joined, false))
            }
            JsVal::Path(Anchor::Cwd, v) => ConfigVal::CwdPath(v),
            JsVal::File => ConfigVal::str(self.file.clone()),
            JsVal::Arr(xs) => {
                ConfigVal::Arr(xs.into_iter().map(|x| self.convert_jsval(x)).collect())
            }
            JsVal::Alts(xs) => ConfigVal::alts(xs.into_iter().map(|x| self.convert_jsval(x))),
            JsVal::Unknown(r) => match r.as_ref() {
                "true" => ConfigVal::Bool(true),
                "false" => ConfigVal::Bool(false),
                "null" | "undefined" => ConfigVal::Null,
                _ => ConfigVal::Unknown(r),
            },
            // URLs of local files are used as paths in configs (`new URL('./x',
            // import.meta.url).pathname`, `fileURLToPath(...)`).
            JsVal::FileUrl => ConfigVal::str(self.file.clone()),
            JsVal::Url(Anchor::FileDir, v) => {
                let joined = path::join([Value::str(self.dir.clone()), v]);
                ConfigVal::Str(path::normalize(joined, false))
            }
            other => ConfigVal::Unknown(format!("{other:?}").into()),
        }
    }
}

#[derive(Copy, Clone)]
enum ImportKind<'a> {
    Require,
    Namespace,
    Default,
    Named(&'a str),
}

/// Sets `path.last` inside `cur`, creating intermediate objects.
fn set_path(
    cur: &mut ConfigVal,
    path: &[Key],
    last: &Key,
    v: ConfigVal,
    combine: &dyn Fn(Option<ConfigVal>, ConfigVal) -> ConfigVal,
    detail: &str,
) -> Result<(), &'static str> {
    if let Some((k, rest)) = path.split_first() {
        let Key::Name(name) = k else {
            return Err("computed key");
        };
        let ConfigVal::Obj(o) = cur else {
            return Err("not an object");
        };
        if o.get(name).is_none() {
            o.set(name.clone(), ConfigVal::Obj(ConfigObj::default()));
        }
        let next = o.get_mut(name).ok_or("missing")?;
        return set_path(next, rest, last, v, combine, detail);
    }
    match (cur, last) {
        (ConfigVal::Obj(o), Key::Name(n)) => {
            let old = o.get(n).cloned();
            o.set(n.clone(), combine(old, v));
            Ok(())
        }
        (ConfigVal::Obj(o), Key::Unknown) => {
            o.open = Some("assignment with a computed key".into());
            Ok(())
        }
        (ConfigVal::Arr(a), Key::Name(n)) if n.parse::<usize>().is_ok() => {
            let idx: usize = n.parse().unwrap_or(0);
            if idx < a.len() {
                let old = Some(a[idx].clone());
                a[idx] = combine(old, v);
            } else {
                a.push(v);
            }
            Ok(())
        }
        (cur, _) => {
            // Mutating something we cannot model: it becomes unknown.
            *cur = ConfigVal::alts([
                cur.clone(),
                ConfigVal::Unknown(format!("{detail} = …").into()),
            ]);
            Err("not an object")
        }
    }
}

/// `l ?? r` (`falsy`: `l || r`, where `false` and `''` also select `r`).
fn or_combine(l: ConfigVal, r: ConfigVal, falsy: bool) -> ConfigVal {
    match l {
        ConfigVal::Null => r,
        ConfigVal::Bool(false) if falsy => r,
        ConfigVal::Str(Value::Str(ref s)) if falsy && s.is_empty() => r,
        ConfigVal::Alts(ref vs)
            if !vs
                .iter()
                .any(|v| v.is_null() || matches!(v, ConfigVal::Unknown(_))) =>
        {
            l
        }
        ConfigVal::Unknown(_) | ConfigVal::Alts(_) | ConfigVal::External { .. } => {
            let known: Vec<ConfigVal> = l
                .alternatives()
                .into_iter()
                .filter(|v| !v.is_null())
                .cloned()
                .collect();
            ConfigVal::alts(known.into_iter().chain([r]))
        }
        l => l,
    }
}

/// A destructuring default: `v` unless it is missing.
fn default_of(v: ConfigVal, d: ConfigVal) -> ConfigVal {
    match v {
        ConfigVal::Null => d,
        ConfigVal::Unknown(_) => ConfigVal::alts([v, d]),
        v => v,
    }
}

fn short(v: &ConfigVal) -> String {
    match v {
        ConfigVal::Unknown(r) => r.to_string(),
        ConfigVal::External { module, path } if path.is_empty() => module.clone(),
        ConfigVal::External { module, path } => format!("{module}:{}", path.join(".")),
        ConfigVal::Str(Value::Str(s)) => format!("'{s}'"),
        ConfigVal::Function(_) => "function".into(),
        ConfigVal::Obj(_) => "object".into(),
        ConfigVal::Arr(_) => "array".into(),
        _ => "value".into(),
    }
}

fn to_jsval(v: &ConfigVal) -> Option<JsVal> {
    Some(match v {
        ConfigVal::Str(s) => JsVal::Str(s.clone()),
        ConfigVal::CwdPath(s) => JsVal::Path(Anchor::Cwd, s.clone()),
        ConfigVal::Num(n) => JsVal::Str(Value::Str(n.clone())),
        ConfigVal::Arr(xs) => JsVal::Arr(xs.iter().map(to_jsval).collect::<Option<_>>()?),
        _ => return None,
    })
}

/// `{ ...v }` into `o`.
fn spread_into(o: &mut ConfigObj, v: ConfigVal, conditional: bool) {
    match v {
        ConfigVal::Obj(src) => {
            for (k, x) in src.entries {
                let x = if conditional {
                    ConfigVal::alts([o.get(&k).cloned().unwrap_or(ConfigVal::Null), x])
                } else {
                    x
                };
                o.set(k, x);
            }
            if src.open.is_some() {
                o.open = src.open;
            }
        }
        ConfigVal::Null | ConfigVal::Bool(_) | ConfigVal::Num(_) => {}
        ConfigVal::Alts(vs) => {
            // Per key: the alternatives of each branch (a branch without the key keeps
            // the current value).
            let mut keys: Vec<String> = Vec::new();
            for v in &vs {
                if let ConfigVal::Obj(x) = v {
                    for (k, _) in &x.entries {
                        if !keys.contains(k) {
                            keys.push(k.clone());
                        }
                    }
                }
            }
            for k in keys {
                let cur = o.get(&k).cloned().unwrap_or(ConfigVal::Null);
                let vals: Vec<ConfigVal> = vs
                    .iter()
                    .map(|v| match v {
                        ConfigVal::Obj(x) => x.get(&k).cloned().unwrap_or_else(|| cur.clone()),
                        _ => cur.clone(),
                    })
                    .collect();
                o.set(k, ConfigVal::alts(vals));
            }
            for v in &vs {
                match v {
                    ConfigVal::Obj(x) if x.open.is_some() => o.open = x.open.clone(),
                    ConfigVal::Unknown(r) => o.open = Some(r.clone()),
                    ConfigVal::External { .. } => o.open = Some(short(v).into()),
                    _ => {}
                }
            }
        }
        other => o.open = Some(short(&other).into()),
    }
}

/// `[ ...v ]` into `out`.
fn spread_array(out: &mut Vec<ConfigVal>, v: ConfigVal) {
    match v {
        ConfigVal::Arr(xs) => out.extend(xs),
        ConfigVal::Null => {}
        ConfigVal::Alts(vs) => {
            // Union of the branches (over-approximates).
            for v in vs {
                spread_array(out, v);
            }
        }
        ConfigVal::Str(Value::Str(s)) => {
            out.extend(s.chars().map(|c| ConfigVal::str(c.to_string())))
        }
        other => out.push(ConfigVal::Unknown(format!("...{}", short(&other)).into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Files(Vec<(&'static str, &'static str)>);

    impl ConfigHost for Files {
        fn load(&mut self, spec: &str) -> Result<ConfigModule, String> {
            let name = spec.trim_start_matches("./");
            let (_, src) = self
                .0
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or_else(|| format!("{spec} not found"))?;
            Ok(eval_config(
                src.as_bytes(),
                Dialect::TS,
                &format!("/r/{name}"),
                &mut NoLoad,
            ))
        }

        fn external(&mut self, module: &str, path: &[String]) -> Option<ConfigVal> {
            (module == "vitest/config"
                && path == ["configDefaults".to_string(), "exclude".to_string()])
            .then(|| ConfigVal::Arr(vec![ConfigVal::str("**/node_modules/**")]))
        }
    }

    fn ev(src: &str) -> ConfigModule {
        eval_config(
            src.as_bytes(),
            Dialect::TS,
            "/r/pkg/jest.config.js",
            &mut NoLoad,
        )
    }

    fn ev_with(src: &str, files: Vec<(&'static str, &'static str)>) -> ConfigModule {
        eval_config(
            src.as_bytes(),
            Dialect::TS,
            "/r/jest.config.ts",
            &mut Files(files),
        )
    }

    /// Compact rendering for assertions.
    fn show(v: &ConfigVal) -> String {
        match v {
            ConfigVal::Null => "null".into(),
            ConfigVal::Bool(b) => b.to_string(),
            ConfigVal::Num(n) => n.clone(),
            ConfigVal::Str(Value::Str(s)) => format!("{s:?}"),
            ConfigVal::Str(v) => format!("pat({})", v.to_specifier()),
            ConfigVal::CwdPath(v) => format!("cwd({})", v.to_specifier()),
            ConfigVal::Regex { source, flags } => format!("/{source}/{flags}"),
            ConfigVal::Resolve { spec, from_dir } => format!("resolve({spec} @ {from_dir})"),
            ConfigVal::Arr(xs) => {
                format!("[{}]", xs.iter().map(show).collect::<Vec<_>>().join(", "))
            }
            ConfigVal::Obj(o) => {
                let mut s = o
                    .entries
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", show(v)))
                    .collect::<Vec<_>>();
                if let Some(r) = &o.open {
                    s.push(format!("...?({r})"));
                }
                format!("{{{}}}", s.join(", "))
            }
            ConfigVal::Alts(xs) => {
                format!("({})", xs.iter().map(show).collect::<Vec<_>>().join(" | "))
            }
            ConfigVal::Function(f) => format!("fn@{}", f.line),
            ConfigVal::External { module, path } => format!("ext({module}:{})", path.join(".")),
            ConfigVal::Unknown(r) => format!("?({r})"),
        }
    }

    fn cfg(m: &ConfigModule) -> String {
        m.config().map_or("none".into(), |v| show(&v))
    }

    #[test]
    fn object_literal_and_paths() {
        let m = ev(r#"
            const path = require('path');
            module.exports = {
              rootDir: __dirname,
              roots: ['<rootDir>/src', path.join(__dirname, 'test')],
              moduleNameMapper: {
                '^@/(.*)$': '<rootDir>/src/$1',
                '^services/(.*)$': `${path.resolve(__dirname, '..', 'services')}/$1`,
              },
              testRegex: /\.spec\.ts$/i,
              verbose: false,
              maxWorkers: 2,
              cwdThing: path.resolve('x'),
            };
        "#);
        assert_eq!(
            cfg(&m),
            r#"{rootDir: "/r/pkg", roots: ["<rootDir>/src", "/r/pkg/test"], moduleNameMapper: {^@/(.*)$: "<rootDir>/src/$1", ^services/(.*)$: "/r/services/$1"}, testRegex: /\.spec\.ts$/i, verbose: false, maxWorkers: 2, cwdThing: cwd(x)}"#
        );
        assert!(m.notes.is_empty(), "{:?}", m.notes);
    }

    #[test]
    fn spreads_helpers_and_conditionals() {
        let root = r#"
            const path = require('path');
            const setupFilesAfterEnv = [];
            const config = {
              transform: { '^.+\\.(t|j)sx?$': '@swc/jest' },
              moduleNameMapper: { '^a$': path.resolve(__dirname, 'a.js') },
              setupFilesAfterEnv,
            };
            if (process.env.CI) {
              config.reporters = ['summary'];
            }
            module.exports = config;
        "#;
        let m = ev_with(
            r#"
            const rootConfig = require('./root.js');
            const moduleNameMapper = { ...rootConfig.moduleNameMapper, '^b$': '<rootDir>/b' };
            delete moduleNameMapper['^a$'];
            module.exports = {
              ...rootConfig,
              moduleNameMapper,
              setupFilesAfterEnv: [
                require.resolve('@api/tests/setup.ts'),
                ...(rootConfig.setupFilesAfterEnv ?? []),
                require.resolve('./jest-setup.ts'),
              ],
            };
            "#,
            vec![("root.js", root)],
        );
        assert_eq!(
            cfg(&m),
            r#"{transform: {^.+\.(t|j)sx?$: "@swc/jest"}, moduleNameMapper: {^b$: "<rootDir>/b"}, setupFilesAfterEnv: [resolve(@api/tests/setup.ts @ /r), resolve(./jest-setup.ts @ /r)], reporters: (null | ["summary"])}"#
        );
        assert_eq!(m.loaded, ["./root.js"]);
    }

    #[test]
    fn esm_define_config_and_merge() {
        let m = ev_with(
            r#"
            import { defineConfig, mergeConfig, configDefaults } from 'vitest/config';
            import path from 'node:path';
            import { fileURLToPath } from 'node:url';
            import shared from './shared';
            const __dirname = path.dirname(fileURLToPath(import.meta.url));
            export default mergeConfig(shared, defineConfig({
              resolve: { alias: { '@': path.resolve(__dirname, 'src') } },
              test: {
                setupFiles: ['./setup.ts'],
                exclude: [...configDefaults.exclude, 'e2e/**'],
              },
            }));
            "#,
            vec![(
                "shared",
                "export default { test: { setupFiles: ['./a.ts'], environment: 'node' } }",
            )],
        );
        assert_eq!(
            cfg(&m),
            r#"{test: {setupFiles: ["./a.ts", "./setup.ts"], environment: "node", exclude: ["**/node_modules/**", "e2e/**"]}, resolve: {alias: {@: "/r/src"}}}"#
        );
    }

    #[test]
    fn functions_and_ts() {
        let m = ev(r#"
            import type { Config } from 'jest';
            function make(dir: string, extra?: string[]): Config {
              if (extra) {
                return { roots: [dir, ...extra] };
              }
              return { roots: [dir] };
            }
            const base = (name) => ({ displayName: name, testEnvironment: 'node' });
            export default async (): Promise<Config> => {
              const c = { ...make(__dirname), ...base('pkg') } satisfies Config;
              return c;
            };
        "#);
        assert_eq!(
            cfg(&m),
            r#"{roots: ["/r/pkg"], displayName: "pkg", testEnvironment: "node"}"#
        );
        assert!(m.notes.iter().any(|n| n.kind == NoteKind::FunctionConfig));
    }

    #[test]
    fn unknowns_are_explicit() {
        let m = ev(r#"
            const nextJest = require('next/jest');
            const createJestConfig = nextJest({ dir: './' });
            const x = require(process.env.CFG);
            module.exports = createJestConfig({ testEnvironment: 'jsdom', setupFiles: [process.env.SETUP] });
        "#);
        let v = m.config().unwrap();
        assert!(v.has_unknown());
        let mut rs = Vec::new();
        v.unknown_reasons(&mut rs);
        assert!(rs.iter().any(|r| r.contains("process.env")), "{rs:?}");
        // `next/jest` is modelled: the custom config comes through.
        assert_eq!(
            v.field("testEnvironment")
                .and_then(|e| e.as_str().map(str::to_string)),
            Some("jsdom".into())
        );
        assert!(
            m.notes.iter().any(|n| n.kind == NoteKind::Unsupported),
            "{:?}",
            m.notes
        );
    }

    /// `Object.entries(x).map(...)` rebuilt with `Object.fromEntries`: the
    /// usual way to rewrite every value of a config object (jest
    /// `transform` options).
    #[test]
    fn object_entries_and_from_entries() {
        let m = ev(r#"
            const base = { a: 'x', b: ['y', { z: 1 }] };
            module.exports = {
              keys: Object.keys(base),
              values: Object.values(base),
              mapped: Object.fromEntries(
                Object.entries(base).map(([key, value]) => [key, [value, { extra: true }]]),
              ),
              none: Object.fromEntries(
                Object.entries(undefined ?? {}).map(([key, value]) => [key, value]),
              ),
              open: Object.fromEntries(Object.entries({ ...load(), a: 'x' }).map(([k, v]) => [k, v])),
              computed: Object.fromEntries([[name(), 1], ['b', 2]]),
              notAnObject: Object.entries(load()),
            };
        "#);
        let v = m.config().unwrap();
        assert_eq!(show(&v.field("keys").unwrap()), r#"["a", "b"]"#);
        let mapped = v.field("mapped").unwrap();
        let ConfigVal::Obj(o) = &mapped else {
            panic!("{mapped:?}")
        };
        assert!(o.open.is_none(), "{o:?}");
        assert_eq!(
            o.entries
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        let a = mapped.field("a").unwrap();
        let ConfigVal::Arr(pair) = &a else {
            panic!("{a:?}")
        };
        assert_eq!(pair[0].as_str(), Some("x"));
        assert!(matches!(&pair[1], ConfigVal::Obj(_)), "{pair:?}");
        // An empty object stays empty and closed: no uncertainty for the field.
        let none = v.field("none").unwrap();
        assert!(
            matches!(&none, ConfigVal::Obj(o) if o.entries.is_empty() && o.open.is_none()),
            "{none:?}"
        );
        // Unknown entries leave the object open (a superset), known ones are kept.
        for f in ["open", "computed"] {
            let x = v.field(f).unwrap();
            assert!(
                matches!(&x, ConfigVal::Obj(o) if o.open.is_some()),
                "{f}: {x:?}"
            );
        }
        assert_eq!(
            v.field("open").unwrap().field("a").unwrap().as_str(),
            Some("x")
        );
        assert!(v.field("computed").unwrap().field("b").is_some());
        assert!(matches!(
            v.field("notAnObject").unwrap(),
            ConfigVal::Unknown(_)
        ));
    }

    /// The shape of a vitest `resolve.alias` list whose first `find` is built
    /// with `new RegExp` from a list of generated files (`xs.map(x =>
    /// x.replace(/\./g, '\\.')) .join('|')` inside a negative lookahead).
    #[test]
    fn string_methods_join_and_new_regexp() {
        let m = eval_config(
            br#"
            import { fileURLToPath } from 'node:url';
            import { defineConfig } from 'vitest/config';
            const projectRoot = fileURLToPath(new URL('.', import.meta.url));
            const GENERATED = ['app/routes.mjs', 'lib/valid-paths.mjs'];
            const generatedPattern = GENERATED.map((path) =>
              path.replace(/\./g, '\\.'),
            ).join('|');
            const alias = [
              {
                find: new RegExp(`^#/(?!(?:${generatedPattern})$)(.*)$`),
                replacement: `${projectRoot}$1`,
              },
              {
                find: /^next\/font\/(?:local|google)$/,
                replacement: fileURLToPath(new URL('./test/font-stub.ts', import.meta.url)),
              },
            ];
            export default defineConfig({ resolve: { alias } });
            export const extra = {
              flags: new RegExp('^a/b$', 'i'),
              copy: new RegExp(/x+/g),
              unknown: new RegExp(process.env.PATTERN),
              first: 'a.b.c'.length,
              one: GENERATED[0].replace('.', '-'),
              all: GENERATED[0].replaceAll('/', '\\'),
              amp: GENERATED[0].replace(/\//g, '[$&]'),
              upper: GENERATED[0].toUpperCase(),
              parts: GENERATED[1].split('/'),
              classRegex: GENERATED[0].replace(/[a-z]+/g, 'x'),
              numbers: [1, 2, 3].join('-'),
              mixed: [GENERATED[0], process.env.X].join('|'),
            };
            "#,
            Dialect::TS,
            "/r/app/vitest.config.ts",
            &mut NoLoad,
        );
        let v = m.config().unwrap();
        assert_eq!(
            show(&v),
            r#"{resolve: {alias: [{find: /^#/(?!(?:app/routes\.mjs|lib/valid-paths\.mjs)$)(.*)$/, replacement: "/r/app/$1"}, {find: /^next\/font\/(?:local|google)$/, replacement: "/r/app/test/font-stub.ts"}]}}"#
        );
        let x = m
            .named
            .iter()
            .find(|(n, _)| n == "extra")
            .unwrap()
            .1
            .clone();
        let f = |k: &str| show(&x.field(k).unwrap());
        assert_eq!(f("flags"), "/^a/b$/i");
        assert_eq!(f("copy"), "/x+/g");
        assert!(matches!(x.field("unknown"), Some(ConfigVal::Unknown(_))));
        assert_eq!(f("one"), r#""app/routes-mjs""#);
        assert_eq!(f("all"), r#""app\\routes.mjs""#);
        assert_eq!(f("amp"), r#""app[/]routes.mjs""#);
        assert_eq!(f("upper"), r#""APP/ROUTES.MJS""#);
        assert_eq!(f("parts"), r#"["lib", "valid-paths.mjs"]"#);
        // A regex that is not one literal string needs a regex engine: unknown.
        assert!(matches!(x.field("classRegex"), Some(ConfigVal::Unknown(_))));
        assert_eq!(f("numbers"), r#""1-2-3""#);
        assert!(matches!(x.field("mixed"), Some(ConfigVal::Unknown(_))));
    }

    /// A host with one symlink: `/r/pkg/node_modules/@scope/x` is a pnpm link.
    struct Links;

    impl ConfigHost for Links {
        fn load(&mut self, spec: &str) -> Result<ConfigModule, String> {
            Err(format!("'{spec}' not followed"))
        }

        fn realpath(&mut self, path: &str) -> Option<String> {
            path.strip_prefix("/r/pkg/node_modules/@scope/x")
                .map(|rest| {
                    format!("/r/node_modules/.pnpm/@scope+x@1.0.0/node_modules/@scope/x{rest}")
                })
        }
    }

    /// A jest `moduleNameMapper` target that reaches a sibling package of a
    /// dependency in the package manager's store:
    /// `path.resolve(fs.realpathSync(<link>), '../shared/...')`.
    #[test]
    fn realpath_of_a_known_path() {
        let src = r#"
            const path = require('path');
            const fs = require('fs');
            module.exports = {
              moduleNameMapper: {
                '^@scope/shared$': path.resolve(
                  require('fs').realpathSync(
                    path.resolve(__dirname, 'node_modules/@scope/x'),
                  ),
                  '../shared/dist/index.js',
                ),
              },
              viaBinding: path.join(fs.realpathSync(path.join(__dirname, 'node_modules/@scope/x')), 'a.js'),
              direct: fs.realpathSync.native(__dirname + '/node_modules/@scope/x'),
              missing: require('fs').realpathSync(path.resolve(__dirname, 'nowhere')),
              unknown: fs.realpathSync(process.env.DIR),
            };
        "#;
        let m = eval_config(
            src.as_bytes(),
            Dialect::JS,
            "/r/pkg/jest.config.js",
            &mut Links,
        );
        let v = m.config().unwrap();
        let store = "/r/node_modules/.pnpm/@scope+x@1.0.0/node_modules/@scope";
        assert_eq!(
            show(&v.field("moduleNameMapper").unwrap()),
            format!(r#"{{^@scope/shared$: "{store}/shared/dist/index.js"}}"#)
        );
        assert_eq!(
            show(&v.field("viaBinding").unwrap()),
            format!(r#""{store}/x/a.js""#)
        );
        assert_eq!(show(&v.field("direct").unwrap()), format!(r#""{store}/x""#));
        // A path that does not exist stays as written (`realpathSync` would throw
        // there).
        assert_eq!(show(&v.field("missing").unwrap()), r#""/r/pkg/nowhere""#);
        assert!(matches!(v.field("unknown"), Some(ConfigVal::Unknown(_))));
    }

    /// A jest resolver module: the exported function and its shape.
    #[test]
    fn jest_resolver_shapes() {
        let shape = |src: &str| {
            let m = eval_config(
                src.as_bytes(),
                Dialect::JS,
                "/r/pkg/jest.resolver.cjs",
                &mut NoLoad,
            );
            m.exported_function.and_then(|f| f.shape)
        };
        // The default resolver with one more condition for one request.
        assert_eq!(
            shape(
                r#"
                module.exports = (request, options) =>
                  options.defaultResolver(request, {
                    ...options,
                    conditions:
                      request === 'tree-walker'
                        ? [...(options.conditions ?? []), 'import']
                        : options.conditions,
                  });
                "#
            ),
            Some(FunctionShape::DefaultResolverConditions(Some(vec![
                "tree-walker".into()
            ])))
        );
        // Several requests, a block body, the literal on the left.
        assert_eq!(
            shape(
                r#"
                module.exports = function (path, opts) {
                  return opts.defaultResolver(path, {
                    ...opts,
                    conditions: path === 'a' || 'b/c' === path ? ['import'] : opts.conditions,
                  });
                };
                "#
            ),
            Some(FunctionShape::DefaultResolverConditions(Some(vec![
                "a".into(),
                "b/c".into()
            ])))
        );
        // The same conditions for every request.
        assert_eq!(
            shape(
                "module.exports = (request, options) => options.defaultResolver(request, { \
                 ...options, conditions: ['browser'] });"
            ),
            Some(FunctionShape::DefaultResolverConditions(None))
        );
        // Anything else is not this shape: the request is used in another way, the
        // request is rewritten, more options change, or the resolver does its
        // own lookup.
        for other in [
            "module.exports = (request, options) => options.defaultResolver(request, { \
             ...options, conditions: request.startsWith('@x/') ? ['import'] : options.conditions \
             });",
            "module.exports = (request, options) => options.defaultResolver(request.replace('a', \
             'b'), { ...options, conditions: ['import'] });",
            "module.exports = (request, options) => options.defaultResolver(request, { \
             ...options, conditions: ['import'], basedir: '/x' });",
            "module.exports = (request, options) => require.resolve(request, { paths: \
             [options.basedir] });",
        ] {
            assert_eq!(shape(other), None, "{other}");
        }
    }

    /// What the evaluator could not follow: ranges whose value is unknown,
    /// functions nobody called, and the known strings bindings made visible
    /// there.
    #[test]
    fn opaque_ranges_escaped_values_and_function_shapes() {
        let src = r#"
            const path = require('path');
            const SRC = path.resolve(__dirname, 'src');
            const LIB = path.resolve(__dirname, 'lib');
            const GEN = path.resolve(__dirname, 'gen');
            const OUT = { dir: path.resolve(__dirname, 'out') };
            const files = collect(LIB);
            module.exports = {
              resolve: {
                alias: [
                  { find: '@', replacement: SRC },
                  {
                    find: /^pkg\/(.+)$/,
                    replacement: 'pkg/$1',
                    customResolver: (id: string): string => {
                      return require.resolve(id);
                    },
                  },
                  { find: 'x', replacement: 'y', customResolver: (id) => lookup(id, GEN) },
                ],
              },
              plugins: [plugin({ dir: OUT.dir })],
              pinned: require.resolve('dep', { paths: [path.resolve(__dirname, '../other')] }),
            };
        "#;
        let m = ev(src);
        // Passed to an untracked helper, named in a function nobody called, passed to a
        // plugin: the evaluator cannot say what happens to these.
        assert_eq!(m.escaped, ["/r/pkg/gen", "/r/pkg/lib", "/r/pkg/out"]);
        let text = |&(a, b): &(u32, u32)| &src[a as usize..b as usize];
        let opaque: Vec<&str> = m.opaque.iter().map(text).collect();
        for want in [
            "collect(LIB)",
            "(id) => lookup(id, GEN)",
            "plugin({ dir: OUT.dir })",
        ] {
            assert!(opaque.contains(&want), "{want}: {opaque:?}");
        }
        // The alias target and the modelled resolver are not opaque.
        assert!(!opaque.iter().any(|o| o.contains("SRC")), "{opaque:?}");
        assert!(
            !opaque.iter().any(|o| o.contains("require.resolve(id)")),
            "{opaque:?}"
        );
        let v = m.config().unwrap();
        let alias = v.field("resolve").unwrap().field("alias").unwrap();
        let ConfigVal::Arr(alias) = alias else {
            panic!("{alias:?}")
        };
        let shape = |i: usize| match alias[i].field("customResolver") {
            Some(ConfigVal::Function(f)) => {
                m.function(&f).map(|x| (x.shape.clone(), text(&x.span)))
            }
            _ => None,
        };
        let (s, t) = shape(1).unwrap();
        assert_eq!(s, Some(FunctionShape::RequireResolve));
        assert!(t.starts_with("(id: string)") && t.ends_with('}'), "{t}");
        assert_eq!(shape(2).unwrap().0, None);
        // `require.resolve(x, { paths })` resolves from the given directory.
        assert_eq!(show(&v.field("pinned").unwrap()), "resolve(dep @ /r/other)");
    }

    #[test]
    fn next_jest_async_wrapper() {
        // The shape of `apps/web/jest.config.ts` in a large monorepo.
        let m = ev(r#"
            import nextJest from 'next/jest';
            import type { Config } from 'jest';
            const customConfig: Config = {
              setupFilesAfterEnv: ['<rootDir>/test/setup.ts'],
              testMatch: ['./**/*.test.{ts,tsx}'],
              testPathIgnorePatterns: ['/node_modules/', 'test.mocks'],
            };
            export default async (): Promise<Config> => {
              const config = (await nextJest({ dir: __dirname })(customConfig)()) as Config;
              const finalConfig = {
                ...config,
                transformIgnorePatterns: (config.transformIgnorePatterns ?? []).filter(
                  (pattern) => !pattern.includes('node_modules'),
                ),
              };
              return finalConfig;
            };
        "#);
        let v = m.config().unwrap();
        let tm = v.field("testMatch").unwrap();
        assert_eq!(show(&tm), r#"["./**/*.test.{ts,tsx}"]"#);
        assert_eq!(
            show(&v.field("testPathIgnorePatterns").unwrap()),
            r#"["/node_modules/", "/.next/", "/node_modules/", "test.mocks"]"#
        );
        assert_eq!(
            show(&v.field("setupFilesAfterEnv").unwrap()),
            r#"["<rootDir>/test/setup.ts"]"#
        );
        // `(x ?? []).filter(...)` on an empty array is exact: no note.
        assert!(
            !m.notes.iter().any(|n| n.kind == NoteKind::Unsupported),
            "{:?}",
            m.notes
        );
        // `filter` with a callback the evaluator can decide is exact; otherwise a
        // superset.
        let m = ev(r#"
            const keep = (p) => true;
            module.exports = { a: ['a', 'b', null].filter(keep), b: ['x', 'y'].filter((p) => p !== 'x') };
        "#);
        let v = m.config().unwrap();
        assert_eq!(show(&v.field("a").unwrap()), r#"["a", "b", null]"#);
        // The undecided filter: the superset plus an unknown, on the value only.
        let b = v.field("b").unwrap();
        assert!(
            b.alternatives().iter().any(|x| show(x) == r#"["x", "y"]"#),
            "{}",
            show(&b)
        );
        let mut rs = Vec::new();
        b.unknown_reasons(&mut rs);
        assert!(rs.iter().any(|r| r.contains("superset")), "{rs:?}");
        assert!(
            !m.notes.iter().any(|n| n.kind == NoteKind::Unsupported),
            "{:?}",
            m.notes
        );
        // CJS, `createJestConfig` bound to a name, a function custom config.
        let m = ev(r#"
            const nextJest = require('next/jest');
            const createJestConfig = nextJest({ dir: './' });
            module.exports = createJestConfig(async () => ({ testEnvironment: 'jsdom' }));
        "#);
        // next's own `transform` entry is part of the resolved config.
        assert_eq!(
            cfg(&m),
            r#"{testEnvironment: "jsdom", transform: {^.+\.(js|jsx|ts|tsx|mjs)$: [resolve(next/dist/build/swc/jest-transformer @ /r/pkg), {}]}, testPathIgnorePatterns: ["/node_modules/", "/.next/"]}"#
        );
        // `defineConfig` from `jest` is the identity.
        let m = ev(r#"
            import { defineConfig } from 'jest';
            export default defineConfig({ roots: ['<rootDir>/src'] });
        "#);
        assert_eq!(cfg(&m), r#"{roots: ["<rootDir>/src"]}"#);
    }

    #[test]
    fn env_conditions_keep_both_branches() {
        let m = ev(
            "module.exports = { setupFiles: process.env.CI ? ['a'] : [], x: process.env.X || 'd' \
             };",
        );
        assert_eq!(
            cfg(&m),
            r#"{setupFiles: (["a"] | []), x: (?(process.env.X) | "d")}"#
        );
        assert!(
            m.notes.iter().all(|n| n.detail != "process.env.CI"),
            "{:?}",
            m.notes
        );
        assert!(m.notes.iter().any(|n| n.kind == NoteKind::EnvDependent));
    }

    #[test]
    fn module_exports_members_and_push() {
        let m = ev(r#"
            exports.testEnvironment = 'node';
            module.exports.setupFiles = ['a'];
            const extra = ['b'];
            extra.push('c');
            module.exports.setupFiles = module.exports.setupFiles.concat(extra);
        "#);
        assert_eq!(
            cfg(&m),
            r#"{testEnvironment: "node", setupFiles: ["a", "b", "c"]}"#
        );
    }

    #[test]
    fn workspace_array_and_named_exports() {
        let m = ev(r#"
            import { defineWorkspace } from 'vitest/config'
            export const extra = 1
            export default defineWorkspace([
              'packages/*',
              { extends: './vitest.config.ts', test: { name: 'unit', include: ['**/*.unit.test.ts'] } },
            ])
        "#);
        assert_eq!(
            cfg(&m),
            r#"["packages/*", {extends: "./vitest.config.ts", test: {name: "unit", include: ["**/*.unit.test.ts"]}}]"#
        );
        assert_eq!(m.named.len(), 1);
    }

    #[test]
    fn not_followed_helper() {
        let m =
            ev("module.exports = { ...require('../../jest.config.js'), testMatch: ['**/x.ts'] };");
        let v = m.config().unwrap();
        assert!(matches!(&v, ConfigVal::Obj(o) if o.open.is_some()));
        assert!(m.notes.iter().any(|n| n.kind == NoteKind::NotFollowed));
    }

    /// A host that follows factories: `@acme/e2e` is a workspace package (its
    /// source is `FACTORY`), `./local` a helper, and exported functions are
    /// called.
    struct Factories {
        calls: u32,
    }

    const FACTORY: &str = r#"
        import { join } from 'node:path';
        import { defineConfig as base } from '@playwright/test';
        const KIND = (process.env.KIND ?? 'e2e') as 'e2e' | 'server';
        const patterns: Record<string, string> = { e2e: '*.e2e.ts', server: '*.spec.ts' };
        export function defineConfig(cwd: string) {
          return base({
            globalSetup: join(__dirname, '../setup.ts'),
            testDir: './',
            outputDir: join(cwd, 'out'),
            testMatch: patterns[KIND],
            projects: [{ name: 'a', testMatch: [patterns[KIND], `*.${KIND === 'e2e' ? 'e2e' : 'spec'}.a.ts`] }],
          });
        }
        export default (x) => ({ ...x, wrapped: true });
        export const notAFunction = 1;
    "#;

    impl Factories {
        fn source(spec: &str) -> Option<(&'static str, &'static str)> {
            match spec {
                "@acme/e2e" => Some(("/r/tooling/e2e/src/index.ts", FACTORY)),
                "./local" => Some(("/r/app/local.ts", FACTORY)),
                "./cjs" => Some((
                    "/r/app/cjs.js",
                    "module.exports = function (dir) { return { testDir: dir }; };",
                )),
                _ => None,
            }
        }
    }

    impl ConfigHost for Factories {
        fn load(&mut self, spec: &str) -> Result<ConfigModule, String> {
            let (file, src) = Self::source(spec).ok_or("not found")?;
            Ok(eval_config(src.as_bytes(), Dialect::TS, file, self))
        }

        fn load_package(&mut self, spec: &str) -> Option<Result<ConfigModule, String>> {
            if spec == "@acme/broken" {
                return Some(Err("no source file".into()));
            }
            Self::source(spec)?;
            Some(self.load(spec))
        }

        fn call_export(
            &mut self,
            spec: &str,
            export: &str,
            args: &[ConfigVal],
        ) -> Option<ConfigVal> {
            let (file, src) = Self::source(spec)?;
            self.calls += 1;
            Some(
                eval_export_call(
                    src.as_bytes(),
                    Dialect::TS,
                    file,
                    self,
                    export,
                    args.to_vec(),
                )
                .1,
            )
        }

        fn factories(&mut self) -> bool {
            true
        }
    }

    fn ev_factories(src: &str) -> (ConfigModule, u32) {
        let mut host = Factories { calls: 0 };
        let m = eval_config(
            src.as_bytes(),
            Dialect::TS,
            "/r/app/playwright.config.ts",
            &mut host,
        );
        (m, host.calls)
    }

    #[test]
    fn factory_of_a_followed_package_is_called() {
        let expected = concat!(
            r#"{globalSetup: "/r/tooling/e2e/setup.ts", testDir: "./", outputDir: "/r/app/out", "#,
            r#"testMatch: ("*.e2e.ts" | "*.spec.ts"), "#,
            r#"projects: [{name: "a", testMatch: [("*.e2e.ts" | "*.spec.ts"), pat(*.e2e.a.ts | *.spec.a.ts)]}]}"#
        );
        // An import, a `require` with destructuring, and a namespace.
        for src in [
            "import { defineConfig } from '@acme/e2e';\nexport default defineConfig(__dirname);",
            "const { defineConfig } = require('@acme/e2e');\nmodule.exports = \
             defineConfig(__dirname);",
            "import * as e2e from '@acme/e2e';\nconst config = \
             e2e.defineConfig(__dirname);\nexport default config;",
        ] {
            let (m, calls) = ev_factories(src);
            assert_eq!(cfg(&m), expected, "{src}");
            assert_eq!(calls, 1, "{src}");
        }
        // The result is an object like any other: spread and overridden.
        let (m, _) = ev_factories(
            "import { defineConfig } from '@acme/e2e';\nexport default { \
             ...defineConfig(__dirname), testDir: './spec', testMatch: '*.spec.ts', projects: \
             undefined };",
        );
        assert_eq!(
            cfg(&m),
            r#"{globalSetup: "/r/tooling/e2e/setup.ts", testDir: "./spec", outputDir: "/r/app/out", testMatch: "*.spec.ts", projects: null}"#
        );
        // The same for a local helper, whose `__dirname` is its own; and a default
        // export.
        let (m, _) = ev_factories(
            "import wrap, { defineConfig } from './local';\nexport default wrap({ dir: \
             defineConfig('/x').globalSetup });",
        );
        assert_eq!(cfg(&m), r#"{dir: "/r/setup.ts", wrapped: true}"#);
        // A factory that is `module.exports`.
        let (m, _) = ev_factories("const make = require('./cjs');\nmodule.exports = make('/x');");
        assert_eq!(cfg(&m), r#"{testDir: "/x"}"#);
    }

    #[test]
    fn factory_that_cannot_be_evaluated_is_unknown() {
        // An export that is not a function, and one that does not exist.
        let (m, _) = ev_factories(
            "import { notAFunction, missing } from '@acme/e2e';\nexport default { a: \
             notAFunction('x'), b: missing('x') };",
        );
        let v = m.config().unwrap();
        assert!(v.field("a").unwrap().has_unknown(), "{}", cfg(&m));
        assert!(v.field("b").unwrap().has_unknown(), "{}", cfg(&m));
        // A workspace package the host could not load: unknown, with a note.
        let (m, _) = ev_factories(
            "import { defineConfig } from '@acme/broken';\nexport default defineConfig(__dirname);",
        );
        assert_eq!(cfg(&m), "?(defineConfig(__dirname))",);
        assert!(
            m.notes
                .iter()
                .any(|n| n.kind == NoteKind::NotFollowed && n.detail.contains("@acme/broken")),
            "{:?}",
            m.notes
        );
    }

    #[test]
    fn packages_and_helper_functions_are_not_followed_by_default() {
        // A package's `defineConfig(x)` is `x`, whatever the package: no host is asked.
        let m =
            ev("import { defineConfig } from '@acme/e2e';\nexport default \
                defineConfig(__dirname);");
        assert_eq!(cfg(&m), r#""/r/pkg""#);
        // A followed package is not `External`, but an installed one still is: the
        // identity.
        let (m, calls) = ev_factories(
            "import { defineConfig } from '@playwright/test';\nexport default defineConfig({ \
             testDir: './e2e' });",
        );
        assert_eq!(cfg(&m), r#"{testDir: "./e2e"}"#);
        assert_eq!(calls, 0);
        // A helper's function is not called, and a lookup with an unknown key is
        // unknown.
        let m = ev_with(
            "import { f } from './h';\nconst table = { a: 'x', b: 'y' };\nexport default { v: \
             f('a'), t: table[process.env.K ?? 'a'] };",
            vec![("h", "export const f = (x) => x;")],
        );
        assert_eq!(
            cfg(&m),
            "{v: ?(call of a function from another module (call)), t: ?(table[process.env.K ?? \
             'a'])}"
        );
    }

    #[test]
    fn type_arguments_in_annotations() {
        // The comma of `Record<K, V>` does not end the declarator.
        let m = ev(r#"
            type Kind = 'a' | 'b';
            const table: Record<Kind, string> = { a: 'x', b: 'y' };
            const nested: Partial<Record<Kind, Array<string>>> = { a: ['z'] }, after = 'w';
            const f = (k: string, d: string = 'd'): Record<string, string> => ({ k: d });
            // A comparison after a cast is not a type argument list.
            const cmp = 1 as number < 2 ? 'lt' : 'ge';
            export default { t: table.a, n: nested.a, after, f: f(undefined), cast: (table as Record<string, string>).b, cmp };
        "#);
        assert_eq!(
            cfg(&m),
            r#"{t: "x", n: ["z"], after: "w", f: {k: "d"}, cast: "y", cmp: ("lt" | "ge")}"#
        );
    }

    #[test]
    fn never_panics_on_garbage() {
        for src in [
            "module.exports = {",
            "module.exports = [1, 2, ...",
            "export default (",
            "const = = = ;",
            "}}}{{{",
            "module.exports = a ? : b",
            "export default { a: () => { return { b: } } }",
            "function f() { return f() } module.exports = f()",
            "const a = { ...a }; module.exports = a",
        ] {
            let _ = ev(src);
        }
    }
}
