//! Path names of a JS/TS file: the collecting half of the bound for reads whose
//! path cannot be evaluated (`turborepo_trace_graph::names`). What a name is,
//! and the fact a file's names travel in, is defined once for every language in
//! [`turborepo_trace_core::names`]; this module walks the strings of a JS/TS
//! file with the token lexer (one pass, no syntax tree, a pure function of the
//! file's text) and knows the JS ways to name a location.
//!
//! # Strings
//!
//! Every string literal and template literal outside import-specifier positions
//! (those are references already; the module specifiers of jest and vitest
//! mocks and of `require.resolve` too). A template substitution is a hole.
//!
//! A string that is only compared or searched for is not a name: the argument
//! of `startsWith`, `endsWith`, `includes`, `indexOf` and `lastIndexOf`, and
//! the right side of `===`, `!==`, `==` and `!=`. The comparison yields a
//! boolean or an index; the path that is tested got its text somewhere else.
//!
//! A string is an argument of a path function when it is a direct argument (or
//! an element of an array argument) of a function that takes a path: by name
//! for the functions only file APIs have (`readFile`, `existsSync`,
//! `readdirSync`, ... [`file_fn`]), and by binding for the names other APIs
//! share (`join`, `resolve`, `dirname`, `exists`, `open`, `stat`, `access`,
//! `glob`, ... [`shared_fn`]): the callee has to be a member of a binding
//! imported from a path or file module (`path`, `fs`, `fs-extra`, `glob`, ...
//! [`path_module`]) or of an identifier with the customary name of one (`path`,
//! `fs`), or a function imported from such a module. So `resolve('docs')` of a
//! promise, `router.resolve('docs')` and `client.join('events')` name nothing.
//!
//! Consecutive literal arguments of one path function are one string, joined
//! with `/`: `join(root, 'src', 'routes')` names `src/routes`, not the words
//! `src` and `routes` (a literal that is only part of an argument, `'b' + x`,
//! stays on its own).
//!
//! A template that starts with a hole holding one identifier that the file
//! declares as a string constant is reported ([`Parts::templates`]); when the
//! value-flow engine knows the values of that hole, the template is judged on
//! them ([`Followed::leads`]): with a URL in the constant it is a URL, with a
//! directory the name gets longer.
//!
//! # Anchors
//!
//! A path also has a base, and a base is often not a string: `__dirname`,
//! `__filename`, `import.meta.url` and the working directory. [`Names::anchor`]
//! and [`Names::cwd`] say that the file uses one: the file then names
//! everything below its own directory, or below the working directory of the
//! entry that loads it. [`Names::test_path`] is the test runner's name for the
//! running test file (`expect.getState().testPath`). [`Names::ups`] says how
//! far a base can move toward the root: the most parent steps (`..` segments,
//! `dirname` calls) of one expression with the file's location, plus all
//! the parent steps elsewhere in the file (they can be applied to any value). A
//! file with an anchor names everything below that many ancestors of its
//! directory.
//!
//! A literal path from the file's location that ends with a name is not an
//! anchor use: `path.join(__dirname, '../fixtures/a.json')`, `new URL('./data',
//! import.meta.url)`. The extractor evaluates it and emits a reference to
//! exactly that file or directory where it is read; everywhere else it is named
//! by an anchored entry for exactly that path ([`Parts::located`]: the code
//! says where it goes) and by its last segments like any string, and the `..`
//! in it move nothing else.
//!
//! A string whose own parent steps lead out of the directory it starts from
//! (`'../shared/a.json'`, `join(base, '..', 'data')`) is marked as climbing
//! (`turborepo_trace_core::names::Name::climbs`), except inside such a literal
//! path from the file's location, where the anchored entry says where the steps
//! lead. `const __dirname = dirname(fileURLToPath(import.meta.url))` only
//! defines the anchor. Everything else is a use ([`AnchorUse`]): a bare
//! `__dirname` (an argument, a property value), a path with a part that is not
//! a literal, a path that ends with `..` or `.` (a directory no string names).
//!
//! The uses are followed by the value-flow engine ([`crate::flow::anchors`]):
//! where the location or the working directory goes is then a list of entries
//! (`turborepo_trace_core::names::Anchored`) instead of "everything below it"
//! ([`scan_parts`], [`Parts::finish`]). A use that cannot be followed keeps the
//! coarse mark described above.
//!
//! # Not seen
//!
//! String attributes of JSX elements (the lexer returns one token per element),
//! strings built from fragments none of which is a name (`'fix' + 'tures'`),
//! property names and identifiers used as path segments, upward searches that
//! climb further than the file has `..` and `dirname`, and text that is not in
//! the scanned files at all (the environment, the network, third-party
//! packages).

use turborepo_trace_core::names::{
    Collector, HOLE, MAX_STRING, climbs_out, separates, ups_of, words,
};
pub use turborepo_trace_core::names::{MAX_NAMES, MAX_UPS, Name, NameKind, Names};

use crate::lexer::{Dialect, ESCAPES, Kw, Lexer, TokKind, Token, UNTERMINATED};

/// Functions only file APIs have: their string arguments are pieces of a path
/// whatever the callee is bound to (a wrapper of that name reads files too).
fn file_fn(name: &[u8]) -> bool {
    matches!(
        name,
        b"readFile"
            | b"readFileSync"
            | b"readdir"
            | b"readdirSync"
            | b"existsSync"
            | b"statSync"
            | b"lstatSync"
            | b"accessSync"
            | b"openSync"
            | b"opendir"
            | b"opendirSync"
            | b"createReadStream"
            | b"readJson"
            | b"readJsonSync"
            | b"readJSON"
            | b"readJSONSync"
            | b"pathExists"
            | b"pathExistsSync"
            | b"globSync"
            | b"pathToFileURL"
    )
}

/// Functions of path and file modules whose names other APIs use too (a
/// promise's `resolve`, a client's `join`, a store's `exists`): path functions
/// only when the callee is bound to such a module.
fn shared_fn(name: &[u8]) -> bool {
    matches!(
        name,
        b"join"
            | b"resolve"
            | b"relative"
            | b"dirname"
            | b"basename"
            | b"normalize"
            | b"exists"
            | b"stat"
            | b"lstat"
            | b"access"
            | b"open"
            | b"glob"
    )
}

/// Modules whose functions take paths.
fn path_module(spec: &[u8]) -> bool {
    let spec = spec.strip_prefix(b"node:").unwrap_or(spec);
    matches!(
        spec,
        b"path"
            | b"path/posix"
            | b"path/win32"
            | b"pathe"
            | b"upath"
            | b"fs"
            | b"fs/promises"
            | b"fs-extra"
            | b"graceful-fs"
            | b"glob"
            | b"fast-glob"
            | b"globby"
            | b"tinyglobby"
    )
}

/// Identifiers that customarily hold a path or file module (a member call on
/// one counts even when the scan did not see the binding: a parameter, a
/// destructured dependency).
fn path_module_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"path"
            | b"posix"
            | b"win32"
            | b"fs"
            | b"fsp"
            | b"fse"
            | b"fsPromises"
            | b"promises"
            | b"nodePath"
            | b"nodeFs"
    )
}

/// Bindings of path modules kept per file (each call looks through them).
const MAX_BINDINGS: usize = 64;

/// One name bound by an import or a `require`.
struct Binding {
    local: Box<[u8]>,
    /// The name in the module (the local name for a whole module).
    imported: Box<[u8]>,
    /// Inside braces: one export of the module, not the module.
    brace: bool,
}

/// The bindings of path and file modules seen so far in a file: `import path
/// from 'path'`, `import * as p from 'node:path'`, `import { join as j } from
/// 'path'`, `const { resolve } = require('path')`, `const fs = require('fs')`.
/// One pass: a binding counts for the calls after it.
#[derive(Default)]
struct PathBindings {
    /// Local names of whole modules.
    modules: Vec<Box<[u8]>>,
    /// Imported functions.
    functions: Vec<Binding>,
    /// The declaration being read.
    pending: Vec<Binding>,
    /// In the head of an `import` or of a `const`/`let`/`var` declaration.
    open: bool,
    brace: bool,
    /// After the `=` of a declaration: the next tokens decide (`require(`,
    /// `await import(`).
    assigned: bool,
}

impl PathBindings {
    /// Feeds one token (with the one before it).
    fn token(&mut self, t: &Token, prev: &Token, src: &[u8]) {
        match t.kind {
            TokKind::Ident if matches!(t.kw, Kw::Import | Kw::Const | Kw::Let | Kw::Var) => {
                // `import(` is a call; `await import(` after `=` is read below.
                if t.kw == Kw::Import && self.assigned {
                    return;
                }
                self.pending.clear();
                self.open = true;
                self.brace = false;
                self.assigned = false;
            }
            _ if !self.open => {}
            TokKind::Str => {
                // `from 'spec'`, `require('spec')`, `import('spec')`.
                let spec = src
                    .get(t.start as usize + 1..(t.end as usize).saturating_sub(1))
                    .unwrap_or(b"");
                let is_spec =
                    prev.kw == Kw::From || (self.assigned && prev.kind == TokKind::LParen);
                if is_spec && path_module(spec) {
                    for b in self.pending.drain(..) {
                        if b.brace && self.functions.len() < MAX_BINDINGS {
                            self.functions.push(b);
                        } else if !b.brace && self.modules.len() < MAX_BINDINGS {
                            self.modules.push(b.local);
                        }
                    }
                }
                self.pending.clear();
                self.open = false;
            }
            TokKind::LBrace if !self.assigned => self.brace = true,
            TokKind::RBrace if !self.assigned => self.brace = false,
            TokKind::Assign if !self.assigned => self.assigned = true,
            TokKind::Ident if self.assigned => {
                if !matches!(t.kw, Kw::Require | Kw::Await | Kw::Import) {
                    self.open = false;
                }
            }
            TokKind::LParen if self.assigned => {}
            TokKind::Ident if matches!(t.kw, Kw::From | Kw::As | Kw::Type) => {}
            TokKind::Ident => {
                let name: Box<[u8]> = t.text(src).into();
                // `a as b`, `{ a: b }`: the second is the local name.
                let renames = prev.kw == Kw::As || (self.brace && prev.kind == TokKind::Colon);
                match self.pending.last_mut() {
                    Some(last) if renames => last.local = name,
                    _ => self.pending.push(Binding {
                        local: name.clone(),
                        imported: name,
                        brace: self.brace,
                    }),
                }
            }
            TokKind::Comma | TokKind::Colon | TokKind::Star => {}
            _ => self.open = false,
        }
    }

    /// Whether the call `callee(` (with the two tokens before the callee) is a
    /// call of a path function.
    fn is_path_call(&self, callee: &[u8], before: &Token, receiver: &Token, src: &[u8]) -> bool {
        if file_fn(callee) {
            return true;
        }
        if matches!(before.kind, TokKind::Dot | TokKind::OptChain) {
            return shared_fn(callee)
                && receiver.kind == TokKind::Ident
                && (path_module_name(receiver.text(src))
                    || self.modules.iter().any(|m| **m == *receiver.text(src)));
        }
        self.functions
            .iter()
            .any(|b| *b.local == *callee && (shared_fn(&b.imported) || file_fn(&b.imported)))
    }
}

/// Functions whose first argument is a module specifier (a reference already).
fn module_fn(name: &[u8]) -> bool {
    matches!(
        name,
        b"mock"
            | b"doMock"
            | b"unmock"
            | b"dontMock"
            | b"unstable_mockModule"
            | b"requireActual"
            | b"requireMock"
            | b"importActual"
            | b"importMock"
            | b"createRequire"
    )
}

/// Decodes the escapes a path can contain. Any other escape (a newline, a
/// unicode escape) becomes a space: it separates words.
fn unescape(raw: &[u8]) -> Option<String> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c == b'\\' {
            match raw.get(i + 1) {
                Some(&n @ (b'\\' | b'/' | b'\'' | b'"' | b'`' | b'$' | b'.' | b'-' | b'_')) => {
                    out.push(n)
                }
                _ => out.push(b' '),
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// An open bracket, for telling literal paths below the file's location from
/// other uses of the location.
#[derive(Default)]
struct Group {
    depth: u32,
    /// Byte offset of the bracket.
    start: u32,
    /// Number of anchor uses found before the bracket ([`Found::uses`]).
    uses_from: usize,
    /// `(` after `join`, `resolve` or `URL`.
    builds_path: bool,
    /// Only string literals, commas and the file's location so far.
    pure: bool,
    /// `{`.
    brace: bool,
    /// Line of a use of the file's location inside.
    anchor: Option<u32>,
    /// The last literal inside ends with a segment that is not `.` or `..`: the
    /// path is named by its own last segments.
    down: bool,
    /// `..` segments of the strings inside.
    ups: u32,
    /// `(` after `dirname`.
    dirname: bool,
    /// For a `dirname` group: only `fileURLToPath(import.meta.url)`-like tokens
    /// so far.
    idiom: bool,
    /// For a `dirname` group: `__filename` or `import.meta` inside.
    file_anchor: bool,
    /// `__dirname = dirname(...)`.
    defines_dirname: bool,
    /// `__filename = fileURLToPath(...)`.
    defines_filename: bool,
    /// `(` after `resolve`: an absolute piece restarts the path.
    resolve: bool,
    /// `(` after `URL`.
    url: bool,
    /// The location inside is the file, not its directory (`__filename`,
    /// `import.meta.filename`, `import.meta.url`).
    file_token: bool,
    /// The string literals inside after the location (for `URL`: all of them),
    /// and whether one of them could not be read.
    literals: Vec<String>,
    unread_literal: bool,
    /// Names of strings inside whose parent steps lead out of where they start
    /// (indices into the collector's names): marked when the group is
    /// closed, unless the group is a literal path from the file's location.
    climbing: Vec<usize>,
}

/// Where a literal path from the file's location leads, relative to the file's
/// directory (normalized, leading `..` for directories above it): the pieces of
/// `g` after the location. `None` when it cannot be said (an absolute piece of
/// `resolve`, a URL with a scheme, a path below the file itself, control
/// characters). A piece with a wildcard ends the path before it.
fn located_rel(g: &Group) -> Option<String> {
    if g.unread_literal {
        return None;
    }
    let mut out: Vec<&str> = Vec::new();
    let mut ups = 0u32;
    // The file's name is the first segment of the location; the first `..` removes
    // it.
    let mut file = g.file_token && !g.url;
    'pieces: for piece in &g.literals {
        if piece.chars().any(|c| c < ' ') {
            return None;
        }
        if ((g.resolve || g.url) && piece.starts_with('/')) || (g.url && piece.contains(':')) {
            return None;
        }
        for seg in piece.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if out.pop().is_none() {
                        if file {
                            file = false;
                        } else {
                            ups += 1;
                        }
                    }
                }
                s if s.contains(['*', '?', '{', '}', '[', ']']) => break 'pieces,
                s => out.push(s),
            }
        }
    }
    if file {
        return None;
    }
    let mut rel: Vec<&str> = vec![".."; ups as usize];
    rel.extend(out);
    Some(rel.join("/"))
}

/// Whether one rule of the scan or of [`crate::flow::anchors`] is switched off
/// for an A/B measurement: `TURBO_TRACE_DEBUG_ANCHORS_OFF` is a comma-separated
/// list of `merge` (consecutive literal arguments are one name), `table`
/// (directory consumers), `env` (environment tails), `leads` (templates with a
/// leading constant), and the uses of a location that name nothing: `relative`
/// (the base of `path.relative`), `tests` (`.length`, the searching methods and
/// the comparing matchers of `expect`), `local` (parameters of the file's own
/// functions), `message` (`console` and error constructors), `child` (the
/// working directory of a child process) and `writes` (a path that is only
/// written). Read when a file is traced; graphs built with different values are
/// not interchangeable.
///
/// None of these is the switch of an assumption: each rule says what the
/// process does not read through the value. `child` in particular is no
/// statement about what the child reads: a program that code starts is outside
/// the model for every caller, with or without a working directory (README,
/// "Limitations"), and the rule only keeps a directory that is handed to it
/// from counting as a path this process reads.
pub(crate) fn rule_off(rule: &str) -> bool {
    static OFF: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        turborepo_trace_core::debug::var("ANCHORS_OFF")
            .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
            .unwrap_or_default()
    });
    OFF.iter().any(|r| r == rule)
}

/// A use of the file's own location or of the working directory that is not a
/// literal path below the location: the byte offset of `__dirname`,
/// `__filename`, the `meta` of `import.meta`, or of `cwd`, `INIT_CWD`, `PWD`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AnchorUse {
    /// Byte offset of the token.
    pub at: u32,
    /// The working directory; otherwise the file's location.
    pub cwd: bool,
}

/// Most anchor uses of one file that are followed ([`Parts::uses`] is empty
/// beyond).
pub const MAX_USES: usize = 64;

/// Parent steps (`..` segments, `dirname` calls) found at one place.
#[derive(Copy, Clone, Debug)]
struct Step {
    /// Byte offset: of the bracket of the expression they are in, or of the
    /// string or the `dirname` itself.
    at: u32,
    ups: u32,
    /// The expression also has the file's location.
    anchored: bool,
}

/// What the scan finds besides names.
#[derive(Default)]
struct Found {
    /// Line of the first use of the file's location.
    anchor: Option<u32>,
    steps: Vec<Step>,
    uses: Vec<AnchorUse>,
    /// The literal paths from the file's location ([`Parts::located`]).
    located: Vec<turborepo_trace_core::names::Anchored>,
}

/// What following the anchor uses of a file found ([`crate::flow::anchors`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Followed {
    /// Every use of the file's location was followed: [`Followed::entries`] say
    /// where it goes.
    pub location: bool,
    /// The same for the working directory.
    pub cwd: bool,
    /// The byte ranges of the expressions that were evaluated on the way: their
    /// parent steps are part of the entries.
    pub covered: Vec<(u32, u32)>,
    /// Where the followed anchors go.
    pub entries: Vec<turborepo_trace_core::names::Anchored>,
    /// Templates whose leading hole has known values ([`Parts::templates`]):
    /// the byte offset of the template and the values.
    pub leads: Vec<(u32, Vec<String>)>,
}

/// A template that starts with a hole holding one identifier that the file
/// declares as a constant with a string or a template (`` `${BASE}/apps/${id}`
/// `` with `const BASE = 'https://…'` or `` const BASE = `https://${HOST}` ``): what it names
/// depends on the constant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeadTemplate {
    /// Byte offset of the template.
    pub at: u32,
    /// The decoded template, holes as [`HOLE`] (the first character is the
    /// leading hole).
    text: String,
    line: u32,
    path_arg: bool,
}

/// Most templates with a leading constant that are evaluated per file.
pub const MAX_LEAD_TEMPLATES: usize = 64;

/// The names of a file before its anchors are settled ([`scan_parts`]).
#[derive(Debug)]
pub struct Parts {
    names: Names,
    anchor: Option<u32>,
    steps: Vec<Step>,
    /// The anchor uses, in source order; empty when there are more than
    /// [`MAX_USES`].
    pub uses: Vec<AnchorUse>,
    /// The templates whose leading hole is a string constant of the file, at
    /// most [`MAX_LEAD_TEMPLATES`].
    pub templates: Vec<LeadTemplate>,
    /// The literal paths from the file's location that end with a name
    /// (`join(__dirname, '../fixtures')`): what is at or below each is named,
    /// wherever the value goes. They are no anchor uses, so they are part
    /// of the names with or without a follower.
    pub located: Vec<turborepo_trace_core::names::Anchored>,
}

impl Parts {
    /// The names. Without `followed`, a file that uses its location or the
    /// working directory names everything below it, raised by all its
    /// parent steps. With it, the followed anchors are named by its entries
    /// alone, and the parent steps inside the evaluated expressions are not
    /// counted again.
    pub fn finish(self, followed: Option<&Followed>) -> Names {
        let mut names = self.names;
        let covered =
            |at: u32| followed.is_some_and(|f| f.covered.iter().any(|&(a, b)| a <= at && at < b));
        let location = followed.is_some_and(|f| f.location);
        let cwd = followed.is_some_and(|f| f.cwd);
        // The parent steps of one expression apply to the location once: the deepest
        // climb of any of them counts, not their sum. Steps elsewhere can be applied to
        // any value.
        let (mut detached, mut anchored) = (0u32, 0u32);
        for s in self.steps.iter().filter(|s| !covered(s.at)) {
            if s.anchored && !location {
                anchored = anchored.max(s.ups.min(MAX_UPS));
            } else {
                detached = (detached + s.ups).min(MAX_UPS);
            }
        }
        names.ups = (detached + anchored).min(MAX_UPS);
        names.anchor = if location { None } else { self.anchor };
        if cwd {
            names.cwd = None;
        }
        names.anchored = self.located.clone();
        if let Some(f) = followed {
            names.anchored.extend(f.entries.iter().cloned());
            // A template whose leading hole is known is judged on its values: a URL is
            // not a path, and a known directory makes the name longer. The names that
            // only the template gave are replaced.
            for (at, values) in &f.leads {
                let Some(t) = self.templates.iter().find(|t| t.at == *at) else {
                    continue;
                };
                names.names.retain(|n| n.span != Some((*at, *at)));
                let mut c = Collector::new();
                for v in values {
                    c.at = Some(*at);
                    c.string(
                        &format!("{v}{}", &t.text[HOLE.len_utf8()..]),
                        t.path_arg,
                        t.line,
                    );
                }
                for n in c.finish().names {
                    if !names
                        .names
                        .iter()
                        .any(|x| (&x.shape, &x.rest, x.kind) == (&n.shape, &n.rest, n.kind))
                    {
                        names.names.push(n);
                    }
                }
            }
        }
        names
    }
}

/// Whether the template at byte offset `at` of `src` starts with a hole that
/// holds one identifier: the identifier.
fn lead_identifier(src: &[u8], at: u32) -> Option<&[u8]> {
    let rest = src.get(at as usize..)?.strip_prefix(b"`${")?;
    let end = rest
        .iter()
        .position(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$')))?;
    (end > 0 && !rest[0].is_ascii_digit() && rest.get(end) == Some(&b'}')).then(|| &rest[..end])
}

/// Ends the group `g` inside `parent`.
fn close_group(g: Group, parent: Option<&mut Group>, out: &mut Found, collector: &mut Collector) {
    // A literal path from the file's location that ends with a name: the extractor
    // emits it as a reference where it is read, and everywhere else an anchored
    // entry names exactly where it leads (its last segments are a name too,
    // like any string). (One that ends with `..` or `.` is a directory no
    // string names: a use of the location.)
    if g.builds_path
        && g.pure
        && g.down
        && let Some(line) = g.anchor
        && let Some(rel) = located_rel(&g)
    {
        out.uses.truncate(g.uses_from);
        out.located.push(turborepo_trace_core::names::Anchored {
            cwd: false,
            line,
            rel: rel.into(),
            pattern: turborepo_trace_core::names::EVERYTHING_BELOW.into(),
            conditional: false,
        });
        return;
    }
    // Anywhere else, parent steps that lead out of where a string starts say that
    // the path leaves.
    for &i in &g.climbing {
        collector.mark_climbs(i);
    }
    // `const __filename = fileURLToPath(import.meta.url)` only defines the anchor.
    if g.defines_filename && g.pure && g.ups == 0 {
        out.uses.truncate(g.uses_from);
        return;
    }
    let mut anchor = g.anchor;
    let mut ups = g.ups;
    if g.dirname {
        if g.idiom && g.file_anchor {
            // `dirname(fileURLToPath(import.meta.url))`: the file's directory.
            if g.defines_dirname {
                anchor = None;
                out.uses.truncate(g.uses_from);
            }
        } else {
            ups += 1;
        }
    }
    // Inside a `dirname(...)` that may still be that idiom, the outer group
    // decides.
    if let Some(p) = parent
        && p.dirname
        && p.idiom
    {
        p.anchor = p.anchor.or(anchor);
        p.ups += ups;
        return;
    }
    match anchor {
        Some(line) => {
            out.anchor.get_or_insert(line);
            out.steps.push(Step {
                at: g.start,
                ups,
                anchored: true,
            });
        }
        // Without the location: steps that can be applied to any value.
        None if ups > 0 => out.steps.push(Step {
            at: g.start,
            ups,
            anchored: false,
        }),
        None => {}
    }
}

/// A literal that is a whole argument of a path function.
struct Literal {
    value: String,
    line: u32,
    at: u32,
    depth: u32,
}

/// Adds the names of the string `value` (at `line`, byte offset `at`) and
/// counts its parent steps, in the innermost open group.
fn add_string(
    value: &str,
    (line, at): (u32, u32),
    path_arg: bool,
    groups: &mut [Group],
    collector: &mut Collector,
    found: &mut Found,
) {
    collector.at = Some(at);
    // One word: the string itself, in its position. Several: each on its own.
    let whole = !value.chars().any(separates);
    for word in words(value) {
        let ups = ups_of(word);
        let name = collector.word_at(word, path_arg && whole, line);
        let climbing = name.filter(|_| climbs_out(word));
        match groups.last_mut() {
            Some(g) => {
                g.ups += ups;
                g.down = ends_named(word);
                // Decided when the group is closed.
                g.climbing.extend(climbing);
            }
            None => {
                if ups > 0 {
                    found.steps.push(Step {
                        at,
                        ups,
                        anchored: false,
                    });
                }
                if let Some(i) = climbing {
                    collector.mark_climbs(i);
                }
            }
        }
    }
}

/// Whether the path `word` ends with a named segment (not `.` or `..`): then
/// its last segments are a name of what it leads to.
fn ends_named(word: &str) -> bool {
    word.rsplit('/')
        .find(|s| !s.is_empty())
        .is_some_and(|s| s != "." && s != "..")
}

/// Ends a run of consecutive literal arguments of one path function: together
/// they are one path (`join(root, 'src', 'routes')` names `src/routes`, not the
/// words `src` and `routes`).
fn flush_run(
    run: &mut Vec<Literal>,
    groups: &mut [Group],
    collector: &mut Collector,
    found: &mut Found,
) {
    let Some(first) = run.first() else { return };
    let at = (first.line, first.at);
    let mut value = String::new();
    for l in run.iter() {
        let part = if value.is_empty() {
            l.value.as_str()
        } else {
            l.value.trim_start_matches('/')
        };
        if part.is_empty() {
            continue;
        }
        if !value.is_empty() && !value.ends_with('/') {
            value.push('/');
        }
        value.push_str(part);
    }
    run.clear();
    if value.len() <= MAX_STRING {
        add_string(&value, at, true, groups, collector, found);
    }
}

/// The names of the file `src`.
pub fn scan(src: &[u8], dialect: Dialect) -> Names {
    scan_parts(src, dialect).finish(None)
}

/// The names of the file `src` and its anchor uses, for [`Parts::finish`].
pub fn scan_parts(src: &[u8], dialect: Dialect) -> Parts {
    let mut collector = Collector::new();
    let mut found = Found::default();
    if dialect.dts {
        return Parts {
            names: collector.finish(),
            anchor: None,
            steps: Vec::new(),
            uses: Vec::new(),
            templates: Vec::new(),
            located: Vec::new(),
        };
    }
    // Identifiers declared as `const X = '<string>'` or with a template, and the
    // templates that start with a hole holding one identifier.
    let mut consts: std::collections::HashSet<&[u8]> = std::collections::HashSet::new();
    let mut leads: Vec<(LeadTemplate, &[u8])> = Vec::new();
    // Consecutive literal arguments of a path function, and the last literal whose
    // next token decides whether it is a whole argument.
    let mut run: Vec<Literal> = Vec::new();
    let mut pending: Option<Literal> = None;
    let merge = !rule_off("merge");
    let mut lx = Lexer::new(src, dialect);
    // Open templates: text so far, line, decodable, byte offset.
    let mut templates: Vec<(String, u32, bool, u32)> = Vec::new();
    // Context by depth: 2 = arguments of a path function, 1 = an array, 0 =
    // anything else.
    let mut ctx: Vec<u8> = vec![0; 16];
    let mut groups: Vec<Group> = Vec::new();
    // The open `dirname(` groups that can still be the idiom (indices into
    // `groups`): the only ones a token has to update, so deep nesting stays
    // linear.
    let mut live: Vec<usize> = Vec::new();
    let mut bindings = PathBindings::default();
    // The last five tokens, newest first.
    let mut hist = [Token::default(); 5];
    let mut prev = Token::default();
    let mut prev2 = Token::default();
    let mut line = 1u32;
    let mut line_at = 0usize;
    loop {
        let t = lx.next_token();
        if t.kind == TokKind::Eof {
            break;
        }
        let ident = |tok: &Token, name: &[u8]| tok.kind == TokKind::Ident && tok.text(src) == name;
        // A literal argument is whole when a `,` or the closing `)` follows it.
        if let Some(p) = pending.take() {
            let closes = t.kind == TokKind::RParen && t.depth + 1 == p.depth;
            if closes || (t.kind == TokKind::Comma && t.depth == p.depth) {
                run.push(p);
                if closes {
                    flush_run(&mut run, &mut groups, &mut collector, &mut found);
                }
            } else {
                flush_run(&mut run, &mut groups, &mut collector, &mut found);
                add_string(
                    &p.value,
                    (p.line, p.at),
                    true,
                    &mut groups,
                    &mut collector,
                    &mut found,
                );
            }
        }
        // A run goes on over `,` and further literals (decided where the string is
        // handled below); any other argument ends it.
        if run.first().is_some_and(|f| {
            !(t.depth == f.depth
                && matches!(t.kind, TokKind::Comma | TokKind::Str | TokKind::Template))
        }) {
            flush_run(&mut run, &mut groups, &mut collector, &mut found);
        }
        // `import.meta.<x>`: `t` is `<x>`.
        let after_meta = prev.kind == TokKind::Dot && ident(&prev2, b"meta");
        // `dirname` that is not called: passed as a value (`xs.map(path.dirname)`), not
        // imported or destructured.
        if ident(&prev, b"dirname")
            && !(prev2.kind == TokKind::Dot && ident(&hist[2], b"meta"))
            && t.kind != TokKind::LParen
            && !groups.last().is_some_and(|g| g.brace)
            && hist[1].kw != Kw::Import
        {
            found.steps.push(Step {
                at: prev.start,
                ups: 1,
                anchored: false,
            });
        }
        // Closers end their groups.
        if matches!(
            t.kind,
            TokKind::RParen
                | TokKind::RBracket
                | TokKind::RBrace
                | TokKind::TemplateMiddle
                | TokKind::TemplateTail
        ) {
            while groups.last().is_some_and(|g| g.depth >= t.depth) {
                if let Some(g) = groups.pop() {
                    if live.last() == Some(&groups.len()) {
                        live.pop();
                    }
                    close_group(g, groups.last_mut(), &mut found, &mut collector);
                }
            }
        }
        bindings.token(&t, &prev, src);
        let opener = matches!(
            t.kind,
            TokKind::LParen
                | TokKind::LBracket
                | TokKind::LBrace
                | TokKind::TemplateHead
                | TokKind::TemplateMiddle
        );
        if opener {
            if let Some(g) = groups.last_mut() {
                g.pure = false;
            }
            for &i in &live {
                groups[i].idiom &= t.kind == TokKind::LParen;
            }
            live.retain(|&i| groups[i].idiom);
            let call = t.kind == TokKind::LParen && prev.kind == TokKind::Ident;
            let callee = if call { prev.text(src) } else { b"" };
            let dirname =
                callee == b"dirname" && !(prev2.kind == TokKind::Dot && ident(&hist[2], b"meta"));
            // `__dirname = dirname(` or `__dirname = path.dirname(`.
            let defines_dirname = dirname
                && ((prev2.kind == TokKind::Assign && ident(&hist[2], b"__dirname"))
                    || (prev2.kind == TokKind::Dot
                        && hist[3].kind == TokKind::Assign
                        && ident(&hist[4], b"__dirname")));
            groups.push(Group {
                depth: t.depth,
                start: t.start,
                uses_from: found.uses.len(),
                builds_path: matches!(callee, b"join" | b"resolve" | b"URL"),
                resolve: callee == b"resolve",
                url: callee == b"URL",
                pure: true,
                brace: t.kind == TokKind::LBrace,
                dirname,
                idiom: dirname,
                defines_dirname,
                defines_filename: callee == b"fileURLToPath"
                    && prev2.kind == TokKind::Assign
                    && ident(&hist[2], b"__filename"),
                ..Group::default()
            });
            if dirname {
                live.push(groups.len() - 1);
            }
        } else if !matches!(
            t.kind,
            TokKind::RParen | TokKind::RBracket | TokKind::RBrace | TokKind::TemplateTail
        ) {
            // An ordinary token inside the innermost group.
            let anchor_part = match t.kind {
                TokKind::Ident => {
                    let after_dot = matches!(prev.kind, TokKind::Dot | TokKind::OptChain);
                    (!after_dot && (ident(&t, b"__dirname") || ident(&t, b"__filename")))
                        || t.kw == Kw::Import
                        || (ident(&t, b"meta")
                            && prev.kind == TokKind::Dot
                            && prev2.kw == Kw::Import)
                        || (after_meta && matches!(t.text(src), b"url" | b"dirname" | b"filename"))
                }
                TokKind::Dot => prev.kw == Kw::Import || ident(&prev, b"meta"),
                _ => false,
            };
            if let Some(g) = groups.last_mut()
                && !(anchor_part
                    || matches!(t.kind, TokKind::Str | TokKind::Template | TokKind::Comma))
            {
                g.pure = false;
            }
            let idiom_part = match t.kind {
                TokKind::Dot => true,
                TokKind::Ident => {
                    t.kw == Kw::Import
                        || t.kw == Kw::New
                        || matches!(
                            t.text(src),
                            b"fileURLToPath"
                                | b"URL"
                                | b"url"
                                | b"meta"
                                | b"filename"
                                | b"pathname"
                                | b"__filename"
                        )
                }
                _ => false,
            };
            let file_anchor = ident(&t, b"__filename") || ident(&t, b"meta");
            for &i in &live {
                groups[i].idiom &= idiom_part;
                groups[i].file_anchor |= file_anchor;
            }
            live.retain(|&i| groups[i].idiom);
        }
        let mut here = || {
            let to = (t.start as usize).min(src.len());
            if to > line_at {
                line += memchr::memchr_iter(b'\n', &src[line_at..to]).count() as u32;
                line_at = to;
            }
            line
        };
        let d = t.depth as usize;
        match t.kind {
            TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                if ctx.len() <= d + 1 {
                    ctx.resize(d + 2, 0);
                }
                ctx[d + 1] = match t.kind {
                    TokKind::LParen
                        if prev.kind == TokKind::Ident
                            && bindings.is_path_call(prev.text(src), &prev2, &hist[2], src) =>
                    {
                        2
                    }
                    TokKind::LBracket => 1,
                    _ => 0,
                };
            }
            TokKind::Ident => {
                let text = t.text(src);
                let after_dot = matches!(prev.kind, TokKind::Dot | TokKind::OptChain);
                // `const __dirname = ...` declares the name.
                let declared = matches!(prev.kw, Kw::Const | Kw::Let | Kw::Var);
                match text {
                    b"__dirname" | b"__filename" if !after_dot && !declared => {
                        let line = here();
                        found.uses.push(AnchorUse {
                            at: t.start,
                            cwd: false,
                        });
                        match groups.last_mut() {
                            Some(g) => {
                                g.anchor = g.anchor.or(Some(line));
                                g.file_token |= text == b"__filename";
                                // The path starts again at the location.
                                if !g.url {
                                    g.literals.clear();
                                }
                            }
                            None => {
                                found.anchor.get_or_insert(line);
                            }
                        }
                    }
                    b"meta" if prev.kind == TokKind::Dot && prev2.kw == Kw::Import => {
                        let line = here();
                        found.uses.push(AnchorUse {
                            at: t.start,
                            cwd: false,
                        });
                        match groups.last_mut() {
                            Some(g) => {
                                g.anchor = g.anchor.or(Some(line));
                                // `import.meta.dirname` is the directory; `url` and
                                // `filename` (decided by the next tokens) are the file.
                                g.file_token = true;
                                if !g.url {
                                    g.literals.clear();
                                }
                            }
                            None => {
                                found.anchor.get_or_insert(line);
                            }
                        }
                    }
                    b"dirname" if after_meta => {
                        if let Some(g) = groups.last_mut() {
                            g.file_token = false;
                        }
                    }
                    b"cwd" | b"INIT_CWD" | b"PWD" if after_dot => {
                        collector.names.cwd.get_or_insert_with(&mut here);
                        found.uses.push(AnchorUse {
                            at: t.start,
                            cwd: true,
                        });
                    }
                    b"testPath" => {
                        collector.names.test_path.get_or_insert_with(&mut here);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        let at = |i: usize| ctx.get(i).copied().unwrap_or(0);
        let path_arg = at(d) == 2 || (at(d) == 1 && d > 0 && at(d - 1) == 2);
        let import = prev.kw == Kw::From
            || prev.kw == Kw::Import
            || (prev.kind == TokKind::LParen
                && (matches!(prev2.kw, Kw::Require | Kw::Import)
                    || (prev2.kind == TokKind::Ident && module_fn(prev2.text(src)))
                    // `require.resolve('spec')`, `import.meta.resolve('spec')`.
                    || (ident(&prev2, b"resolve")
                        && hist[2].kind == TokKind::Dot
                        && (hist[3].kw == Kw::Require || ident(&hist[3], b"meta")))));
        let text = |a: usize, b: usize| -> Option<String> {
            let raw = src.get(t.start as usize + a..(t.end as usize).checked_sub(b)?)?;
            if raw.len() > MAX_STRING {
                return None;
            }
            if t.flags & ESCAPES != 0 {
                unescape(raw)
            } else {
                std::str::from_utf8(raw).ok().map(str::to_owned)
            }
        };
        // A complete string: its text, line and byte offset.
        let mut string: Option<(String, u32, u32)> = None;
        match t.kind {
            TokKind::Str | TokKind::Template if t.flags & UNTERMINATED == 0 && !import => {
                if let Some(s) = text(1, 1) {
                    string = Some((s, here(), t.start));
                }
            }
            TokKind::TemplateHead => {
                let ln = here();
                match text(1, 2).filter(|_| !import) {
                    Some(mut s) => {
                        s.push(HOLE);
                        templates.push((s, ln, true, t.start));
                    }
                    None => templates.push((String::new(), ln, false, t.start)),
                }
            }
            TokKind::TemplateMiddle => {
                let s = text(1, 2);
                if let Some(top) = templates.last_mut() {
                    match s {
                        Some(s) if top.0.len() + s.len() <= MAX_STRING => {
                            top.0.push_str(&s);
                            top.0.push(HOLE);
                        }
                        _ => top.2 = false,
                    }
                }
            }
            TokKind::TemplateTail => {
                let s = text(1, 1);
                if let Some(mut top) = templates.pop()
                    && let (Some(s), true) = (s, top.2)
                {
                    top.0.push_str(&s);
                    string = Some((top.0, top.1, top.3));
                }
            }
            _ => {}
        }
        // The literals of a group that may be a literal path from the file's location.
        if matches!(t.kind, TokKind::Str | TokKind::Template)
            && let Some(g) = groups.last_mut()
            && g.builds_path
        {
            match &string {
                Some((value, _, _)) => g.literals.push(value.clone()),
                None => g.unread_literal = true,
            }
        }
        if matches!(
            t.kind,
            TokKind::Str | TokKind::Template | TokKind::TemplateHead
        ) && prev.kind == TokKind::Assign
            && prev2.kind == TokKind::Ident
            && hist[2].kw == Kw::Const
            && consts.len() < 4096
        {
            consts.insert(prev2.text(src));
        }
        if let Some((value, ln, at)) = string {
            let whole = !value.chars().any(separates);
            // A string that is compared or searched for is not a piece of a path there:
            // `seg === '..'`, `rel.startsWith('..')`.
            let tested = (prev.kind == TokKind::Op
                && matches!(prev.text(src), b"===" | b"!==" | b"==" | b"!="))
                || (prev.kind == TokKind::LParen
                    && prev2.kind == TokKind::Ident
                    && matches!(
                        prev2.text(src),
                        b"startsWith" | b"endsWith" | b"includes" | b"indexOf" | b"lastIndexOf"
                    ));
            // A literal that is a whole argument of a path function joins the literals
            // around it: decided by the next token.
            let argument = at_depth(&ctx, d) == 2
                && merge
                && whole
                && !value.is_empty()
                && matches!(t.kind, TokKind::Str | TokKind::Template)
                && matches!(prev.kind, TokKind::Comma | TokKind::LParen);
            if tested {
                // Neither a name nor a parent step; a literal group stays literal.
                flush_run(&mut run, &mut groups, &mut collector, &mut found);
            } else if argument {
                pending = Some(Literal {
                    value,
                    line: ln,
                    at,
                    depth: t.depth,
                });
            } else {
                flush_run(&mut run, &mut groups, &mut collector, &mut found);
                add_string(
                    &value,
                    (ln, at),
                    path_arg,
                    &mut groups,
                    &mut collector,
                    &mut found,
                );
                if t.kind == TokKind::TemplateTail
                    && value.starts_with(HOLE)
                    && leads.len() < MAX_LEAD_TEMPLATES
                    && let Some(id) = lead_identifier(src, at)
                {
                    leads.push((
                        LeadTemplate {
                            at,
                            text: value,
                            line: ln,
                            path_arg,
                        },
                        id,
                    ));
                }
            }
        }
        hist.rotate_right(1);
        hist[0] = t;
        prev2 = prev;
        prev = t;
    }
    // An unterminated file: what is pending is taken as it is.
    if let Some(p) = pending.take() {
        run.push(p);
    }
    flush_run(&mut run, &mut groups, &mut collector, &mut found);
    while let Some(g) = groups.pop() {
        close_group(g, groups.last_mut(), &mut found, &mut collector);
    }
    if found.uses.len() > MAX_USES {
        found.uses.clear();
    }
    Parts {
        names: collector.finish(),
        anchor: found.anchor,
        steps: found.steps,
        uses: found.uses,
        located: found.located,
        templates: leads
            .into_iter()
            .filter(|(_, id)| consts.contains(id))
            .map(|(t, _)| t)
            .collect(),
    }
}

/// The context of bracket depth `i` (see `ctx` in [`scan_parts`]).
fn at_depth(ctx: &[u8], i: usize) -> u8 {
    ctx.get(i).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<(String, NameKind)> {
        scan(src.as_bytes(), Dialect::TS)
            .names
            .into_iter()
            .map(|n| (n.shape.into(), n.kind))
            .collect()
    }

    fn shapes(src: &str) -> Vec<String> {
        names(src).into_iter().map(|n| n.0).collect()
    }

    #[test]
    fn paths_with_separators() {
        assert_eq!(
            names(
                "const a = 'fixtures/a.json'; f(\"./data\", '../x/y.sql', '/var/run/token', \
                 'dir/');"
            ),
            [
                ("fixtures/a.json".to_owned(), NameKind::Path),
                ("data".to_owned(), NameKind::Path),
                ("x/y.sql".to_owned(), NameKind::Path),
                ("var/run/token".to_owned(), NameKind::Path),
                ("dir".to_owned(), NameKind::Path),
            ]
        );
    }

    #[test]
    fn files_and_words() {
        // An extension makes a single segment a name; a bare word is one only as an
        // argument of a path function (directly or in an array argument).
        assert_eq!(
            names(
                "const { join } = require('path'); const s = 'schema.sql'; const w = 'fixtures'; \
                 join(dir, 'testdata', name); path.resolve(...['migrations']); \
                 fs.readFileSync('Dockerfile'); g('user');"
            ),
            [
                ("schema.sql".to_owned(), NameKind::File),
                ("testdata".to_owned(), NameKind::Word),
                ("migrations".to_owned(), NameKind::Word),
                ("Dockerfile".to_owned(), NameKind::Word),
            ]
        );
        // Numbers and versions are not files.
        assert!(shapes("x = '1.5'; y = '10.0.1'; z = '.json'; w = '.';").is_empty());
    }

    #[test]
    fn consecutive_literal_arguments_are_one_name() {
        let pre = "import { join, resolve } from 'path'; import fs from 'fs';";
        for (src, want) in [
            // The literals of one call are one path.
            ("join(root, 'src', 'routes');", vec!["src/routes"]),
            (
                "join(root, 'src', 'routes', name, 'index.ts');",
                vec!["src/routes", "index.ts"],
            ),
            (
                "resolve('fixtures', 'a', 'b.json');",
                vec!["fixtures/a/b.json"],
            ),
            // A literal that is only part of an argument is on its own.
            ("join(root, 'a', 'b' + x);", vec!["a", "b"]),
            ("join(root, x + 'a', 'b');", vec!["a", "b"]),
            // A single literal is as before.
            ("join(dir, 'testdata', name);", vec!["testdata"]),
            // A nested call ends the run.
            (
                "join(root, 'a', join('b', 'c'), 'd');",
                vec!["a", "b/c", "d"],
            ),
            // Parent steps restart the shape.
            ("join(root, 'a', '..', 'b');", vec!["b"]),
            (
                "fs.readFileSync(join(base, 'conf', 'app.yaml'));",
                vec!["conf/app.yaml"],
            ),
        ] {
            assert_eq!(shapes(&format!("{pre} {src}")), want, "{src}");
        }
        assert_eq!(
            scan(
                b"import { join } from 'path'; join(root, 'a', '..', '..', 'b');",
                Dialect::TS
            )
            .ups,
            2
        );
    }

    #[test]
    fn path_functions_by_binding() {
        // Names that other APIs share count only for a callee bound to a path or file
        // module: imported, required, or a member of `path` / `fs`.
        for src in [
            "import { join } from 'node:path'; join(dir, 'testdata');",
            "import { join as j } from 'path'; j(dir, 'testdata');",
            "import * as p from 'path'; p.resolve('testdata');",
            "import nodePath from 'path'; nodePath.join(a, 'testdata');",
            "const p = require('path'); p.join(a, 'testdata');",
            "const { resolve: r, join } = require('path'); r('testdata');",
            "const fsx = await import('fs-extra'); fsx.exists('testdata');",
            "import fs from 'fs'; fs.promises.stat('testdata');",
            "function f(path) { return path.join(a, 'testdata'); }",
            // Functions only file APIs have count whatever they are bound to.
            "helpers.readJsonSync('testdata'); existsSync('testdata');",
        ] {
            assert_eq!(shapes(src), ["testdata"], "{src}");
        }
        for src in [
            "new Promise((resolve) => resolve('docs'));",
            "Promise.resolve('docs'); router.resolve('docs'); client.join('events');",
            "import { join } from './strings'; join(a, 'docs');",
            "cache.exists('docs'); db.open('docs'); acl.access('docs'); stat('docs');",
            "new URL('docs', base); url.resolve(base, 'docs'); parts.join('docs');",
        ] {
            assert!(shapes(src).is_empty(), "{src}");
        }
        // A binding counts for the calls after it, in any scope.
        assert_eq!(
            shapes("join('early'); const { join } = require('path'); join('late');"),
            ["late"]
        );
    }

    #[test]
    fn tested_strings_are_not_names() {
        assert!(
            shapes(
                "if (p.startsWith('docs/') || q.includes('a/b.json') || r === 'x/y.sql' || \
                 s.endsWith('/c/d.txt') || t.indexOf('e/f') !== -1 || u != 'g/h.md') f();"
            )
            .is_empty()
        );
        // The same string where it is a value is a name.
        assert_eq!(
            shapes("const prefix = 'docs/'; if (p.startsWith(prefix)) read(p);"),
            ["docs"]
        );
        // A tested string does not make a literal path below the location something
        // else.
        assert_eq!(
            loc("const f = join(__dirname, 'fixtures'); if (x === '..') f();"),
            (false, 0)
        );
    }

    #[test]
    fn rests_of_templates() {
        let got: Vec<(String, Option<String>)> = scan(
            b"a = `/docs/configuration${x}`; b = `${u}/services/${id}/apply`; c = `fixtures/${n}.json`; d = `out/${n}`;",
            Dialect::TS,
        )
        .names
        .into_iter()
        .map(|n| (n.shape.into(), n.rest.map(Into::into)))
        .collect();
        assert_eq!(
            got,
            [
                ("docs".to_owned(), Some("configuration*".to_owned())),
                ("services".to_owned(), Some("*/apply".to_owned())),
                ("fixtures".to_owned(), Some("*.json".to_owned())),
                ("out".to_owned(), None),
            ]
        );
    }

    #[test]
    fn templates_and_globs_have_holes() {
        assert_eq!(
            shapes(
                "a = `fixtures/${name}.json`; b = `${root}/snapshots/${id}/out.txt`; c = \
                 'src/**/*.test.ts'; d = `${a}/${b}`; e = `${dir}/schema.sql`; f = \
                 'data/*.{json,yaml}'; g = `pre-${x}/y`;"
            ),
            ["fixtures", "snapshots", "src", "schema.sql", "data", "y"]
        );
        // A nested template is scanned on its own.
        assert_eq!(
            shapes("a = `${cond ? `x/y.json` : 'z/w.json'}/tail`;"),
            ["x/y.json", "z/w.json", "tail"]
        );
    }

    #[test]
    fn dots_and_scopes() {
        assert_eq!(
            shapes(
                "a = 'a/../b/c.json'; b = './x/./y'; c = '@scope/pkg/data/file.json'; d = \
                 '@scope/pkg';"
            ),
            ["b/c.json", "y", "data/file.json"]
        );
    }

    #[test]
    fn not_names() {
        assert!(
            shapes(
                "a = 'https://example.com/a/b'; b = '//cdn/x.js'; c = 'application json'; d = \
                 'a:b/c'; e = '/users/:id'; g = 'hello'; h = '#/components/x'; j = \
                 'C:\\\\dir\\\\file'; k = 'node:fs/promises'; l = 'and or';"
            )
            .is_empty()
        );
    }

    #[test]
    fn words_inside_strings() {
        // A path inside a command line, a flag value, a message or a list; an escape
        // that is not part of a name separates words; a query or fragment is
        // cut off.
        assert_eq!(
            shapes(
                "a = 'node scripts/build.js --out=dist/app'; b = 'cannot read conf/app.json!'; c \
                 = 'a/b.txt,c/d.txt'; d = 'x/y.sql\\nz/w.sql'; e = './worker.js?worker'; f = 'see \
                 docs/guide.md#usage';"
            ),
            [
                "scripts/build.js",
                "dist/app",
                "conf/app.json",
                "a/b.txt",
                "c/d.txt",
                "x/y.sql",
                "z/w.sql",
                "worker.js",
                "docs/guide.md"
            ]
        );
        // A word of several is never in a path-function position.
        assert!(shapes("join('fixtures data')").is_empty());
    }

    fn loc(src: &str) -> (bool, u32) {
        let n = scan(src.as_bytes(), Dialect::TS);
        (n.anchor.is_some(), n.ups)
    }

    #[test]
    fn literal_paths_below_the_location_are_not_anchor_uses() {
        // The extractor emits these as references to exactly that path.
        for src in [
            "const env = path.resolve(__dirname, '../../.env.test');",
            "const f = join(__dirname, 'fixtures', 'a.json');",
            "const u = new URL('./data/x.json', import.meta.url);",
            "const f = path.join(import.meta.dirname, '../x/y');",
            "x = require(path.join(__dirname, 'x.js'));",
            "const __dirname = dirname(fileURLToPath(import.meta.url)); const f = join(__dirname, \
             'fixtures/a.json');",
            "const __dirname = path.dirname(fileURLToPath(import.meta.url));",
            "const __filename = fileURLToPath(import.meta.url); const __dirname = \
             dirname(__filename);",
            "import { dirname } from 'path'; const { dirname: d } = require('path'); export { \
             dirname };",
        ] {
            assert_eq!(loc(src), (false, 0), "{src}");
        }
    }

    /// The anchored entries of the file's location: `(relative directory,
    /// pattern)`.
    fn located(src: &str) -> Vec<(String, String)> {
        scan(src.as_bytes(), Dialect::TS)
            .anchored
            .into_iter()
            .map(|a| {
                assert!(!a.cwd && !a.conditional);
                (a.rel.into(), a.pattern.into())
            })
            .collect()
    }

    #[test]
    fn a_literal_path_from_the_location_names_where_it_leads() {
        let at = |rel: &str| vec![(rel.to_owned(), "**".to_owned())];
        for (src, rel) in [
            (
                "const env = path.resolve(__dirname, '../../.env.test');",
                "../../.env.test",
            ),
            (
                "const f = join(__dirname, 'fixtures', 'a.json');",
                "fixtures/a.json",
            ),
            (
                "const d = path.join(__dirname, '..', '..', 'shared');",
                "../../shared",
            ),
            ("const d = path.join(__dirname, 'a/../b/./c/');", "b/c"),
            // A URL is resolved against the file: `./x` and `x` are beside it.
            (
                "const u = new URL('./data/x.json', import.meta.url);",
                "data/x.json",
            ),
            (
                "const u = new URL('../x.json', import.meta.url);",
                "../x.json",
            ),
            (
                "const f = path.join(import.meta.dirname, '../x/y');",
                "../x/y",
            ),
            // From the file itself, the first parent step is its directory.
            ("const f = path.join(__filename, '..', 'x.json');", "x.json"),
            (
                "const f = path.resolve(import.meta.filename, '../../x');",
                "../x",
            ),
            // A wildcard ends the path: what is below the directory before it.
            (
                "const g = path.join(__dirname, 'fixtures', '*.json');",
                "fixtures",
            ),
            // The path starts again at the location.
            ("const f = path.resolve('a', __dirname, 'b');", "b"),
        ] {
            assert_eq!(located(src), at(rel), "{src}");
            assert_eq!(loc(src), (false, 0), "{src}");
        }
        // Where it cannot be said, the location is used like any other value.
        for src in [
            "const f = path.resolve(__dirname, '/etc/x');",
            "const f = path.join(__filename, 'x');",
            "const u = new URL('file:///x/y', import.meta.url);",
        ] {
            assert!(located(src).is_empty(), "{src}");
            assert!(loc(src).0, "{src}");
        }
        // Not a literal path: no entry from the scan (the follower's).
        assert!(located("const f = path.join(__dirname, name, 'x');").is_empty());
        assert!(located("const f = path.join(__dirname, '..');").is_empty());
    }

    /// The names whose string climbs out of where it starts.
    fn climbing(src: &str) -> Vec<String> {
        scan(src.as_bytes(), Dialect::TS)
            .names
            .into_iter()
            .filter(|n| n.climbs)
            .map(|n| n.shape.into())
            .collect()
    }

    #[test]
    fn strings_that_climb_are_marked() {
        let pre = "import path from 'path';";
        for (src, want) in [
            ("const a = '../shared/a.json';", vec!["shared/a.json"]),
            (
                "load('../../data/x.sql', 'local/y.sql');",
                vec!["data/x.sql"],
            ),
            (
                "const d = path.join(base, '..', 'other', 'fixtures');",
                vec!["other/fixtures"],
            ),
            ("const d = path.join(base, 'a', '..', 'b');", vec![]),
            ("const d = `../snapshots/${name}.snap`;", vec!["snapshots"]),
            // One climbing string marks the name, wherever else it occurs.
            (
                "const a = 'shared/a.json'; const b = '../shared/a.json';",
                vec!["shared/a.json"],
            ),
            // Inside a literal path from the file's location the anchored entry says
            // where the steps lead: the string does not climb by itself.
            (
                "const d = path.join(__dirname, '..', '..', 'shared');",
                vec![],
            ),
            (
                "const d = path.resolve(__dirname, '../shared/a.json');",
                vec![],
            ),
            // With a part that is not literal it is a use of the location, and the
            // string climbs.
            (
                "const d = path.join(__dirname, name, '../shared/x');",
                vec!["shared/x"],
            ),
        ] {
            assert_eq!(climbing(&format!("{pre} {src}")), want, "{src}");
        }
    }

    #[test]
    fn templates_with_a_leading_string_constant_are_reported() {
        let p = scan_parts(
            b"const API = 'https://api.example.com'; const u = `${API}/apps/${id}/x`; const v = `${other}/apps/y`; const w = `${API.x}/z/q`;",
            Dialect::TS,
        );
        assert_eq!(p.templates.len(), 1);
        // Without values the template names as before.
        let n = p.finish(None);
        assert!(n.names.iter().any(|x| &*x.shape == "apps"));
        // With a URL in the hole it names nothing; `${other}/apps/y` still does.
        let p = scan_parts(
            b"const API = 'https://api.example.com'; const u = `${API}/apps/${id}/x`; const v = `${other}/docs/y`;",
            Dialect::TS,
        );
        let at = p.templates[0].at;
        let f = Followed {
            leads: vec![(at, vec!["https://api.example.com".to_owned()])],
            ..Followed::default()
        };
        let n = p.finish(Some(&f));
        let shapes: Vec<&str> = n.names.iter().map(|x| &*x.shape).collect();
        assert_eq!(shapes, ["docs/y"]);
        // A constant declared with a template counts too (its value is the engine's).
        let p = scan_parts(
            b"const HOST = 'example.com'; const BASE = `https://${HOST}`; const u = `${BASE}/apps/${slug}`;",
            Dialect::TS,
        );
        assert_eq!(p.templates.len(), 1);
        // With a directory in the hole the name is the whole path.
        let p = scan_parts(
            b"const BASE = 'fixtures/data'; const f = `${BASE}/a.json`;",
            Dialect::TS,
        );
        let at = p.templates[0].at;
        let f = Followed {
            leads: vec![(at, vec!["fixtures/data".to_owned()])],
            ..Followed::default()
        };
        let n = p.finish(Some(&f));
        let shapes: Vec<&str> = n.names.iter().map(|x| &*x.shape).collect();
        assert_eq!(shapes, ["fixtures/data", "fixtures/data/a.json"]);
    }

    #[test]
    fn a_literal_path_that_ends_with_a_parent_step_is_an_anchor_use() {
        // Its value is a directory that none of its strings names.
        assert_eq!(
            loc("const d = path.join(__dirname, 'fixtures', '..');"),
            (true, 1)
        );
        assert_eq!(
            loc("const d = path.resolve(__dirname, 'a/b', '../..', '.');"),
            (true, 2)
        );
        assert_eq!(
            loc("const d = path.join(__dirname, '..', 'fixtures');"),
            (false, 0)
        );
    }

    #[test]
    fn other_uses_of_the_location_are_anchors() {
        for (src, ups) in [
            // Bare: passed on, stored, exported.
            ("helper(__dirname);", 0),
            ("module.exports = { rootDir: __dirname };", 0),
            ("const here = __dirname;", 0),
            (
                "const here = path.dirname(fileURLToPath(import.meta.url)); helper(here);",
                0,
            ),
            ("const u = import.meta.url;", 0),
            // A part that is not a literal.
            ("join(__dirname, name);", 0),
            ("join(__dirname, 'fixtures', name);", 0),
            ("join(__dirname, '..', '..', envPath);", 2),
            ("fs.readFileSync(__dirname + '/../x.json');", 1),
            ("readFile(`${__dirname}/../../x/${name}`);", 2),
            // Only up: an ancestor directory, which is never emitted as a reference.
            ("const ROOT = path.join(__dirname, '..', '..');", 2),
            ("const up = new URL('..', import.meta.url);", 1),
            ("const dir = path.resolve(__dirname);", 0),
            ("const up = dirname(dirname(__filename));", 1),
            ("const up = dirname(__dirname);", 1),
            ("const up = dirname(import.meta.dirname);", 1),
        ] {
            assert_eq!(loc(src), (true, ups), "{src}");
        }
    }

    #[test]
    fn ups_count_parent_steps() {
        // A literal path below the location moves nothing else; the rest adds up.
        assert_eq!(
            loc(
                "const a = join(__dirname, '..', '..', 'x'); const b = '../../y/z'; \
                 path.dirname(p); rootDir = import.meta.dirname;"
            ),
            (true, 3)
        );
        // Without the location: how far above another base (the working directory, a
        // parameter) the file can reach.
        assert_eq!(
            loc("const b = resolve(process.cwd(), '..', '..'); xs.map(path.dirname);"),
            (false, 3)
        );
        assert_eq!(loc("import x from '../../x';"), (false, 0));
        assert_eq!(
            loc("const parts = [dir, '..', name].join('/');"),
            (false, 1)
        );
        // The parent steps of expressions with the location do not add up: the deepest
        // one counts, plus the steps without it.
        assert_eq!(
            loc(
                "const a = resolve(__dirname, '..', name); const b = resolve(__dirname, '../..', \
                 name);"
            ),
            (true, 2)
        );
        assert_eq!(
            loc("const a = resolve(__dirname, '..', name); const b = join(a, '..');"),
            (true, 2)
        );
        // A string that is compared or searched for moves nothing.
        assert_eq!(
            loc("if (seg === '..' || rel.startsWith('..') || p.includes('../')) throw e;"),
            (false, 0)
        );
    }

    #[test]
    fn data_files() {
        // JSON is scanned as JavaScript.
        let n = scan(
            br#"{"routes": [{"module": "svc/handlers/get", "n": 1}], "main": "dist/index.js"}"#,
            Dialect::JS,
        );
        let got: Vec<&str> = n.names.iter().map(|n| &*n.shape).collect();
        assert_eq!(got, ["svc/handlers/get", "dist/index.js"]);
    }

    #[test]
    fn jsx_attribute_strings_are_not_seen() {
        // Documented gap: the lexer returns one token for a JSX element.
        let n = scan(
            b"const a = <img src=\"/images/a.png\" alt={'x/y.png'} />;",
            Dialect::TSX,
        );
        let got: Vec<&str> = n.names.iter().map(|n| &*n.shape).collect();
        assert_eq!(got, ["x/y.png"]);
    }

    #[test]
    fn import_specifiers_are_not_names() {
        assert!(
            shapes(
                "import a from './dep/a'; import './side/effect'; export * from './re/export'; \
                 const b = require('./dep/b'); const c = await import('./dep/c');"
            )
            .is_empty()
        );
        // Nor are the module specifiers of mocks.
        assert!(
            shapes("jest.mock('./dep/a'); vi.mock('../x/y', f); jest.requireActual('./z/w');")
                .is_empty()
        );
        // Nor are module specifiers that are only resolved.
        assert!(
            shapes(
                "require.resolve('@scope/pkg/src/x'); require.resolve(`@scope/pkg/src/${n}`); \
                 import.meta.resolve('./dep/a');"
            )
            .is_empty()
        );
        // The same string elsewhere is a name.
        assert_eq!(shapes("load('./dep/a');"), ["dep/a"]);
    }

    #[test]
    fn anchors() {
        let n = scan(
            b"const a = __dirname;\nconst b = import.meta.url;\nconst c = process.cwd();\nconst t = expect.getState().testPath;\n",
            Dialect::TS,
        );
        assert_eq!((n.anchor, n.cwd, n.test_path), (Some(1), Some(3), Some(4)));
        let n = scan(b"const o = { cwd: 1 }; x.__dirname;", Dialect::TS);
        assert_eq!((n.anchor, n.cwd), (None, None));
        assert!(scan(b"const a = 1;", Dialect::TS).is_empty());
    }

    #[test]
    fn lines_and_dedup() {
        let n = scan(
            b"\n\nconst a = 'x/y';\nconst b = 'x/y';\nconst c = `p/q`;\n",
            Dialect::TS,
        );
        let got: Vec<(&str, u32)> = n.names.iter().map(|n| (&*n.shape, n.line)).collect();
        assert_eq!(got, [("x/y", 3), ("p/q", 5)]);
    }

    #[test]
    fn comments_and_regexes_are_not_scanned() {
        assert!(shapes("// 'a/b.json'\n/* 'c/d.json' */ const r = /e\\/f.json/;").is_empty());
    }

    #[test]
    fn declaration_files_have_no_names() {
        assert!(scan(b"declare const a: 'x/y';", Dialect::DTS).is_empty());
    }
}
