//! The Go recognizer: imports, the cgo preamble, `//go:embed` and tracked
//! runtime file APIs.
//!
//! | Construct | Reference |
//! |---|---|
//! | `import "p"`, `import x "p"`, `import . "p"`, `import _ "p"`, `import ( … )` | [`RefKind::Import`], [`Phase::Build`], base [`IMPORT_BASE`] |
//! | `import "C"` | the same (resolves to the builtin `C`), plus one reference per `#include` in the preamble: `"x.h"` relative to the file ([`RefKind::Import`], build), `<x.h>` as base [`CGO_SYSTEM_BASE`] (a builtin) |
//! | `//go:embed p1 "p 2" `p3`` | one [`RefKind::Embed`] per pattern, [`Phase::Build`], relative to the file's directory |
//! | calls in [`APIS`] (`os.ReadFile`, `template.ParseFiles`, `http.Dir`, …) | per the table, relative to the working directory with [`UncertaintyKind::CwdDependent`] unless absolute; in `_test.go` files relative to the file's directory, because `go test` runs tests in the package directory |
//!
//! A file with a `//go:build` / `// +build` constraint or a `_GOOS`/`_GOARCH`
//! file-name suffix only builds for some targets: all its references are
//! [`RefFlags::CONDITIONAL`].
//!
//! Path arguments are evaluated over string literals, `+`, parenthesized
//! expressions, `filepath.Join`/`path.Join`, `filepath.Clean`, and
//! package-level string constants assigned once (`const dir = "static"`).
//! Anything else is a wildcard with its source text. Calls are matched through
//! the file's import bindings, so renamed (`import fs "os"`) and dot imports
//! work. `embed.FS` methods (`content.ReadFile`, `fs.ReadFile(content, …)`,
//! `template.ParseFS(content, …)`) read embedded data, which the `//go:embed`
//! references already cover, so they produce nothing.

use std::{borrow::Cow, collections::HashMap, path::Path};

use turborepo_trace_core::{
    Base, Phase, RawRef, RefFlags, RefKind, RefSink, Span, Specifier, Uncertainty, UncertaintyKind,
    api::{self, ApiEntry, ArgRole, CallStyle},
    eval::{
        PatPart, Value,
        anchor::{self, Anchored, JoinStyle},
        path as vpath,
    },
    platform::{GO_KNOWN_ARCH, GO_KNOWN_OS},
};

use crate::{
    constraint::header_constraints,
    lex::{GO_ESCAPES, Kind, Tok, string_value, tokenize},
};

/// Base of Go import references: an import path, interpreted by the resolver.
pub const IMPORT_BASE: Base = Base::Language("go_import");

/// Base of cgo `#include <x.h>` references: a system header.
pub const CGO_SYSTEM_BASE: Base = Base::Language("cgo_system");

/// Tracked runtime file APIs, in the shared table format.
pub const APIS: &[ApiEntry] = &[
    ApiEntry::new(&["os"], "ReadFile", RefKind::FsRead),
    ApiEntry::new(&["os"], "Open", RefKind::FsRead),
    ApiEntry::new(&["os"], "OpenFile", RefKind::FsRead),
    ApiEntry::new(&["os"], "ReadDir", RefKind::FsReadDir),
    ApiEntry::new(&["os"], "DirFS", RefKind::FsReadDir),
    ApiEntry::new(&["os"], "OpenRoot", RefKind::FsReadDir),
    ApiEntry::new(&["os"], "Stat", RefKind::FsRead).flags(RefFlags::OPTIONAL),
    ApiEntry::new(&["os"], "Lstat", RefKind::FsRead).flags(RefFlags::OPTIONAL),
    ApiEntry::new(&["io/ioutil"], "ReadFile", RefKind::FsRead),
    ApiEntry::new(&["io/ioutil"], "ReadDir", RefKind::FsReadDir),
    ApiEntry::new(&["path/filepath"], "Glob", RefKind::FsReadDir),
    ApiEntry::new(&["path/filepath"], "Walk", RefKind::FsReadDir),
    ApiEntry::new(&["path/filepath"], "WalkDir", RefKind::FsReadDir),
    ApiEntry::new(
        &["text/template", "html/template"],
        "ParseFiles",
        RefKind::FsRead,
    ),
    ApiEntry::new(
        &["text/template", "html/template"],
        "ParseGlob",
        RefKind::FsRead,
    ),
    ApiEntry::new(&["net/http"], "Dir", RefKind::FsReadDir),
    ApiEntry::new(&["net/http"], "ServeFile", RefKind::FsRead).arg(2),
    // `plugin.Open(path)` loads a Go plugin (`-buildmode=plugin` shared object).
    ApiEntry::new(&["plugin"], "Open", RefKind::SharedLib).role(ArgRole::LoaderWrapper),
];

/// The anchor of Go path values: the directory of the source file as recorded
/// at build time (`_, file, _, _ := runtime.Caller(0)`; `filepath.Dir(file)`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum GoAnchor {
    /// The source file's directory.
    SrcDir,
}

/// A partially evaluated Go path value (shared [`Anchored`] model).
type GoVal = Anchored<GoAnchor>;

/// Maximum local bindings tracked per file.
const MAX_LOCALS: usize = 256;

/// The record of a path built from `runtime.Caller`.
const CALLER_DETAIL: &str = "path built from runtime.Caller: the source directory at build time, \
                             which exists at run time only next to the source tree";

/// Entries whose argument list is variadic from [`ApiEntry::arg`] on.
const VARIADIC: &[&str] = &["ParseFiles"];

/// Entries whose argument is a glob pattern (`path.Match` syntax).
const GLOBS: &[&str] = &["ParseGlob", "Glob"];

/// Template methods matched on any receiver (`t.ParseFiles`,
/// `template.New(n).ParseGlob`) when a template package is imported.
const TEMPLATE_METHODS: &[&str] = &["ParseFiles", "ParseGlob"];

/// Maximum number of tokens scanned for one call's argument list.
const MAX_ARG_TOKENS: usize = 4096;

/// Whether a file name carries a `_GOOS`/`_GOARCH` constraint (for any target).
pub fn name_is_constrained(name: &str) -> bool {
    let stem = name.split_once('.').map_or(name, |(s, _)| s);
    let stem = stem.strip_suffix("_test").unwrap_or(stem);
    let parts: Vec<&str> = stem.split('_').skip(1).collect();
    parts
        .last()
        .is_some_and(|l| GO_KNOWN_OS.contains(l) || GO_KNOWN_ARCH.contains(l))
}

/// The package name an import binds by default: the last path element, skipping
/// a `/vN` major-version suffix, without a `.vN` (gopkg.in) suffix.
pub fn default_binding(path: &str) -> &str {
    let mut it = path.rsplit('/');
    let last = it.next().unwrap_or(path);
    let is_major = |s: &str| {
        s.strip_prefix('v')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    let name = if is_major(last) {
        it.next().unwrap_or(last)
    } else {
        last
    };
    match name.rsplit_once(".v") {
        Some((n, v)) if !n.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) => n,
        _ => name,
    }
}

struct Recognizer<'s, 'o> {
    src: &'s [u8],
    toks: Vec<Tok>,
    out: &'o mut RefSink<'s>,
    flags: RefFlags,
    /// A `_test.go` file: `go test` runs in the package directory, so relative
    /// paths are anchored there.
    is_test: bool,
    /// Import bindings: package name → import path (the last import wins).
    bindings: HashMap<&'s str, Cow<'s, str>>,
    /// Dot-imported paths, deduplicated.
    dot: Vec<Cow<'s, str>>,
    /// Whether a template package is imported.
    template_import: bool,
    /// Package-level string constants assigned once, collected on first use
    /// (only files with a tracked API call need them).
    consts: Option<HashMap<&'s [u8], Option<String>>>,
    /// Local and package-level bindings `name := expr` / `name = expr` in
    /// source order (name, value, span of the value), collected with
    /// `consts`.
    locals: Option<Vec<(&'s [u8], GoVal, Span)>>,
    /// The file's base name (`runtime.Caller(0)`'s file).
    file_name: Option<String>,
    /// `time/tzdata` is imported (the zoneinfo database is embedded).
    tzdata_import: bool,
    /// The first `time.LoadLocation` call.
    load_location: Option<Span>,
    /// Arguments of tracked APIs whose value is anchored at the source
    /// directory.
    anchored_args: Vec<Span>,
}

/// Finds the references in Go source `src`. `path` (when known) decides
/// file-name constraints. Linear; never panics.
pub fn extract<'s>(src: &'s [u8], path: Option<&Path>, out: &mut RefSink<'s>) {
    let (toks, errs) = tokenize(src);
    for e in errs {
        out.diagnostic(Uncertainty::new(
            UncertaintyKind::ParseIncomplete,
            e.span,
            e.to_string(),
        ));
    }
    let name_constrained = path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .is_some_and(name_is_constrained);
    let is_test = path
        .and_then(|p| p.file_name())
        .is_some_and(|n| n.as_encoded_bytes().ends_with(b"_test.go"));
    let flags = if name_constrained || header_constraints(src).is_constrained() {
        RefFlags::CONDITIONAL
    } else {
        RefFlags::empty()
    };
    let mut r = Recognizer {
        src,
        toks,
        out,
        flags,
        is_test,
        bindings: HashMap::new(),
        dot: Vec::new(),
        template_import: false,
        consts: None,
        locals: None,
        file_name: path
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(str::to_owned),
        tzdata_import: false,
        load_location: None,
        anchored_args: Vec::new(),
    };
    r.run();
    r.finish();
}

impl<'s> Recognizer<'s, '_> {
    fn text(&self, t: &Tok) -> &'s [u8] {
        let src: &'s [u8] = self.src;
        &src[t.span.range()]
    }

    fn newline_between(&self, a: &Tok, b: &Tok) -> usize {
        let (s, e) = (a.span.end as usize, b.span.start as usize);
        if s >= e {
            return 0;
        }
        memchr::memchr_iter(b'\n', &self.src[s..e]).count()
    }

    /// Index of the next non-comment token at or after `i`.
    fn next_sig(&self, mut i: usize) -> Option<usize> {
        while i < self.toks.len() {
            if !self.toks[i].is_comment() {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// Records `Name [Type] = "lit"` at brace depth 0 where `Name` starts its
    /// line.
    fn collect_consts(&mut self) {
        let mut consts = HashMap::new();
        let sig: Vec<usize> = (0..self.toks.len())
            .filter(|&i| !self.toks[i].is_comment())
            .collect();
        let mut depth = 0i32;
        for (k, &i) in sig.iter().enumerate() {
            let t = self.toks[i];
            if t.is_punct(self.src, b'{') {
                depth += 1;
            } else if t.is_punct(self.src, b'}') {
                depth -= 1;
            }
            if depth != 0 || !t.is_punct(self.src, b'=') || k == 0 {
                continue;
            }
            // `=` must not be part of `==`, `!=`, `:=`, `<=`, ...
            let prev = self.toks[sig[k - 1]];
            if prev.kind == Kind::Punct {
                continue;
            }
            let Some(&vi) = sig.get(k + 1) else { continue };
            let v = self.toks[vi];
            if !v.is_string() {
                continue;
            }
            if let Some(&ni) = sig.get(k + 2) {
                let n = self.toks[ni];
                let continues = self.newline_between(&v, &n) == 0
                    && !n.is_punct(self.src, b';')
                    && !n.is_punct(self.src, b')');
                if continues {
                    continue;
                }
            }
            // The name: the first token on the `=` line, after `const`/`var`.
            let mut start = k - 1;
            while start > 0 && self.newline_between(&self.toks[sig[start - 1]], &prev) == 0 {
                let p = self.toks[sig[start - 1]];
                if p.is_punct(self.src, b'(') || p.is_punct(self.src, b';') {
                    break;
                }
                start -= 1;
            }
            let mut first = self.toks[sig[start]];
            if (first.is_ident(self.src, b"const") || first.is_ident(self.src, b"var"))
                && start + 1 < k
            {
                first = self.toks[sig[start + 1]];
            }
            // Only `Name =` or `Name Type =`.
            let between = k - start;
            if first.kind != Kind::Ident || between > 3 {
                continue;
            }
            let name = self.text(&first);
            let value = string_value(self.src, &v).map(Cow::into_owned);
            consts
                .entry(name)
                .and_modify(|e| *e = None)
                .or_insert(value);
        }
        self.consts = Some(consts);
    }

    fn run(&mut self) {
        // Comment group tracking for the cgo preamble: (first, last) token index.
        let mut group: Option<(usize, usize)> = None;
        // The last four significant token indices, most recent first.
        let mut sig: [Option<usize>; 4] = [None; 4];
        let mut i = 0;
        while i < self.toks.len() {
            let t = self.toks[i];
            if t.is_comment() {
                group = match group {
                    Some((f, l)) if self.newline_between(&self.toks[l], &t) <= 1 => Some((f, i)),
                    _ => Some((i, i)),
                };
                if t.kind == Kind::LineComment {
                    self.embed_directive(&t);
                }
                i += 1;
                continue;
            }
            let pending = group
                .take()
                .filter(|&(_, l)| self.newline_between(&self.toks[l], &t) <= 1);
            if t.is_ident(self.src, b"import") && sig[0].is_none_or(|p| !self.is_dot(p)) {
                i = self.import_decl(i, pending);
                sig = [None; 4];
                continue;
            }
            if t.is_punct(self.src, b'(') {
                self.maybe_call(i, &sig);
            }
            sig = [Some(i), sig[0], sig[1], sig[2]];
            i += 1;
        }
    }

    fn is_dot(&self, i: usize) -> bool {
        self.toks[i].is_punct(self.src, b'.')
    }

    /// Parses an import declaration starting at the `import` keyword. Returns
    /// the index after it.
    fn import_decl(&mut self, kw: usize, preamble: Option<(usize, usize)>) -> usize {
        let Some(j) = self.next_sig(kw + 1) else {
            return self.toks.len();
        };
        if !self.toks[j].is_punct(self.src, b'(') {
            return self.import_spec(j, preamble);
        }
        // Block: specs until `)`. Comments directly before a spec are its doc (the cgo
        // preamble for `"C"`), else the declaration's doc applies.
        let mut i = j + 1;
        let mut group: Option<(usize, usize)> = None;
        while i < self.toks.len() {
            let t = self.toks[i];
            if t.is_comment() {
                group = match group {
                    Some((f, l)) if self.newline_between(&self.toks[l], &t) <= 1 => Some((f, i)),
                    _ => Some((i, i)),
                };
                i += 1;
                continue;
            }
            let doc = group
                .take()
                .filter(|&(_, l)| self.newline_between(&self.toks[l], &t) <= 1)
                .or(preamble);
            if t.is_punct(self.src, b')') {
                return i + 1;
            }
            if t.is_punct(self.src, b';') {
                i += 1;
                continue;
            }
            if !(t.is_string() || t.kind == Kind::Ident || t.is_punct(self.src, b'.')) {
                // Malformed: resume normal scanning here.
                return i;
            }
            i = self.import_spec(i, doc);
        }
        i
    }

    /// Parses `[name] "path"` at `i`. Returns the index after it.
    fn import_spec(&mut self, i: usize, preamble: Option<(usize, usize)>) -> usize {
        let t = self.toks[i];
        let (name, si) = if t.is_string() {
            (None, i)
        } else if t.kind == Kind::Ident || t.is_punct(self.src, b'.') {
            match self.next_sig(i + 1) {
                Some(si) if self.toks[si].is_string() => (Some(self.text(&t)), si),
                _ => return i + 1,
            }
        } else {
            return i;
        };
        let st = self.toks[si];
        let Some(path) = string_value(self.src, &st) else {
            self.out.diagnostic(Uncertainty::new(
                UncertaintyKind::ParseIncomplete,
                st.span,
                "malformed import path",
            ));
            return si + 1;
        };
        if path == "text/template" || path == "html/template" {
            self.template_import = true;
        }
        if path == "time/tzdata" {
            self.tzdata_import = true;
        }
        match name {
            Some(b".") => {
                if !self.dot.contains(&path) {
                    self.dot.push(path.clone());
                }
            }
            Some(b"_") => {}
            Some(n) => {
                if let Ok(n) = std::str::from_utf8(n) {
                    self.bindings.insert(n, path.clone());
                }
            }
            None => {
                let b: &'s str = match &path {
                    Cow::Borrowed(p) => default_binding(p),
                    Cow::Owned(_) => "",
                };
                // An import path written with escapes (legal, never seen in practice) binds
                // no name for API matching.
                if !b.is_empty() {
                    self.bindings.insert(b, path.clone());
                }
            }
        }
        let is_c = path == "C";
        self.out.push(
            RawRef::new(RefKind::Import, Specifier::Literal(path), st.span)
                .with_base(IMPORT_BASE)
                .with_phase(Phase::Build)
                .with_flags(self.flags),
        );
        if is_c && let Some((f, l)) = preamble {
            self.cgo_preamble(f, l);
        }
        si + 1
    }

    /// `#include` lines of the cgo preamble in comment tokens `first..=last`.
    fn cgo_preamble(&mut self, first: usize, last: usize) {
        for ti in first..=last {
            let t = self.toks[ti];
            let text = self.text(&t);
            let base = t.span.start as usize;
            let (body, off) = match t.kind {
                Kind::LineComment => (&text[2.min(text.len())..], 2),
                Kind::BlockComment => {
                    let end = text.len().saturating_sub(2).max(2.min(text.len()));
                    (&text[2.min(text.len())..end], 2)
                }
                _ => continue,
            };
            let mut line_start = 0;
            for line in body.split(|&b| b == b'\n') {
                let lead = line.len() - line.trim_ascii_start().len();
                let l = &line[lead..];
                let rest_off = if l.starts_with(b"#include") {
                    Some(8)
                } else if l.starts_with(b"#import") {
                    Some(7)
                } else {
                    None
                };
                if let Some(n) = rest_off {
                    let r = &l[n..];
                    let ws = r.len() - r.trim_ascii_start().len();
                    let r = &r[ws..];
                    let (close, system) = match r.first() {
                        Some(b'"') => (b'"', false),
                        Some(b'<') => (b'>', true),
                        _ => {
                            line_start += line.len() + 1;
                            continue;
                        }
                    };
                    if let Some(end) = memchr::memchr(close, &r[1..]) {
                        let name = &r[1..1 + end];
                        let abs = base + off + line_start + lead + n + ws + 1;
                        let src: &'s [u8] = self.src;
                        if let Ok(s) = std::str::from_utf8(&src[abs..abs + name.len()])
                            && !s.is_empty()
                        {
                            let span = Span::from_usize(abs, abs + name.len());
                            let r = RawRef::new(RefKind::Import, Specifier::literal(s), span)
                                .with_phase(Phase::Build)
                                .with_flags(self.flags);
                            self.out.push(if system {
                                r.with_base(CGO_SYSTEM_BASE)
                            } else {
                                r
                            });
                        }
                    }
                }
                line_start += line.len() + 1;
            }
        }
    }

    /// `//go:embed` patterns in a line comment.
    fn embed_directive(&mut self, t: &Tok) {
        let src: &'s [u8] = self.src;
        let text = &src[t.span.range()];
        let Some(rest) = text.strip_prefix(b"//go:embed") else {
            return;
        };
        if !rest.first().is_some_and(|b| matches!(b, b' ' | b'\t')) {
            return;
        }
        let base = t.span.start as usize + b"//go:embed".len();
        let mut i = 0;
        while i < rest.len() {
            match rest[i] {
                b' ' | b'\t' | b'\r' => i += 1,
                q @ (b'"' | b'`') => {
                    let start = i;
                    let mut j = i + 1;
                    while j < rest.len() && rest[j] != q {
                        if q == b'"' && rest[j] == b'\\' {
                            j += 1;
                        }
                        j += 1;
                    }
                    if j >= rest.len() {
                        self.out.diagnostic(Uncertainty::new(
                            UncertaintyKind::ParseIncomplete,
                            Span::from_usize(base + start, base + rest.len()),
                            "unterminated //go:embed pattern",
                        ));
                        return;
                    }
                    let inner = &rest[start + 1..j];
                    let value = if q == b'`' {
                        std::str::from_utf8(inner).ok().map(Cow::Borrowed)
                    } else {
                        turborepo_trace_core::lexkit::decode(inner, GO_ESCAPES).ok()
                    };
                    let span = Span::from_usize(base + start, base + j + 1);
                    self.push_embed(value, span);
                    i = j + 1;
                }
                _ => {
                    let start = i;
                    while i < rest.len() && !matches!(rest[i], b' ' | b'\t' | b'\r') {
                        i += 1;
                    }
                    let value = std::str::from_utf8(&rest[start..i]).ok().map(Cow::Borrowed);
                    self.push_embed(value, Span::from_usize(base + start, base + i));
                }
            }
        }
    }

    fn push_embed(&mut self, value: Option<Cow<'s, str>>, span: Span) {
        match value {
            Some(v) if !v.is_empty() => self.out.push(
                RawRef::new(RefKind::Embed, Specifier::Literal(v), span)
                    .with_phase(Phase::Build)
                    .with_flags(self.flags),
            ),
            _ => self.out.diagnostic(Uncertainty::new(
                UncertaintyKind::ParseIncomplete,
                span,
                "malformed //go:embed pattern",
            )),
        }
    }

    fn binding(&self, name: &[u8]) -> Option<&str> {
        let name = std::str::from_utf8(name).ok()?;
        self.bindings.get(name).map(|p| &**p)
    }

    /// Checks whether the `(` at `k` opens a call of a tracked API.
    fn maybe_call(&mut self, k: usize, sig: &[Option<usize>; 4]) {
        let tok = |i: Option<usize>| i.map(|i| self.toks[i]);
        let Some(p1) = tok(sig[0]).filter(|t| t.kind == Kind::Ident) else {
            return;
        };
        let fname = self.text(&p1);
        let Ok(fname) = std::str::from_utf8(fname) else {
            return;
        };
        let p2 = tok(sig[1]);
        let p2_dot = p2.is_some_and(|t| t.is_punct(self.src, b'.'));
        let mut entry: Option<&'static ApiEntry> = None;
        if p2_dot {
            let p3 = tok(sig[2]);
            let p4_dot = tok(sig[3]).is_some_and(|t| t.is_punct(self.src, b'.'));
            if let Some(p3) = p3.filter(|t| t.kind == Kind::Ident)
                && !p4_dot
                && let Some(module) = self.binding(self.text(&p3))
            {
                let load_location = module == "time" && fname == "LoadLocation";
                entry = api::lookup(APIS, Some(module), fname, CallStyle::Call);
                if load_location && self.load_location.is_none() {
                    self.load_location = Some(p1.span);
                }
            }
            if entry.is_none() && TEMPLATE_METHODS.contains(&fname) && self.template_import {
                entry = api::lookup(APIS, Some("text/template"), fname, CallStyle::Call);
            }
        } else if !self.dot.is_empty() && !p2.is_some_and(|t| t.is_ident(self.src, b"func")) {
            entry = self
                .dot
                .iter()
                .find_map(|m| api::lookup(APIS, Some(m), fname, CallStyle::Call));
        }
        let Some(e) = entry else { return };
        let Some(args) = self.call_args(k) else {
            return;
        };
        self.ensure_bindings();
        let variadic = VARIADIC.contains(&e.path);
        let glob = GLOBS.contains(&e.path);
        let wanted: Vec<(usize, usize)> = if variadic {
            args.iter().skip(e.arg as usize).copied().collect()
        } else {
            args.get(e.arg as usize).copied().into_iter().collect()
        };
        for (a, b) in wanted {
            let span = Span::new(self.toks[a].span.start, self.toks[b - 1].span.end);
            let mut v = self.eval_sum(a, b);
            if glob {
                v.val = glob_value(v.val);
            }
            if v.anchor.is_some() {
                self.anchored_args.push(span);
            }
            let r = match v.anchor {
                Some(GoAnchor::SrcDir) => RawRef::new(e.kind, v.normalized().to_specifier(), span)
                    .with_base(Base::FileDir)
                    .with_phase(e.phase)
                    .with_flags(e.flags | self.flags)
                    .with_uncertainty(Uncertainty::new(
                        UncertaintyKind::EnvDependent,
                        span,
                        CALLER_DETAIL,
                    )),
                None => self.api_ref(e, v.val.to_specifier(), span),
            };
            self.out.push(r);
        }
    }

    /// Top-level argument token ranges `[a, b)` of the call whose `(` is at
    /// `k`.
    fn call_args(&self, k: usize) -> Option<Vec<(usize, usize)>> {
        let mut depth = 0usize;
        let mut args = Vec::new();
        let mut start = k + 1;
        let end = self.toks.len().min(k + MAX_ARG_TOKENS);
        for i in k + 1..end {
            let t = self.toks[i];
            if t.kind != Kind::Punct {
                continue;
            }
            match self.src[t.span.start as usize] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth > 0 => depth -= 1,
                b')' => {
                    if start < i {
                        args.push((start, i));
                    }
                    return Some(
                        args.into_iter()
                            .map(|(a, b)| self.trim_comments(a, b))
                            .filter(|(a, b)| a < b)
                            .collect(),
                    );
                }
                b',' if depth == 0 => {
                    args.push((start, i));
                    start = i + 1;
                }
                _ => {}
            }
        }
        None
    }

    fn trim_comments(&self, mut a: usize, mut b: usize) -> (usize, usize) {
        while a < b && self.toks[a].is_comment() {
            a += 1;
        }
        while b > a && self.toks[b - 1].is_comment() {
            b -= 1;
        }
        (a, b)
    }

    fn source(&self, a: usize, b: usize) -> String {
        let s = self.toks[a].span.start as usize;
        let e = self.toks[b - 1].span.end as usize;
        let text = String::from_utf8_lossy(&self.src[s..e]);
        if text.len() > 80 {
            let mut cut = 77;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            format!("{}...", &text[..cut])
        } else {
            text.into_owned()
        }
    }

    /// `a + b + ...` over tokens `[a, b)`.
    fn eval_sum(&self, a: usize, b: usize) -> GoVal {
        let (a, b) = self.trim_comments(a, b);
        if a >= b {
            return GoVal::unknown("empty expression");
        }
        let mut depth = 0usize;
        let mut terms = Vec::new();
        let mut start = a;
        for i in a..b {
            let t = self.toks[i];
            if t.kind != Kind::Punct {
                continue;
            }
            match self.src[t.span.start as usize] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b'+' if depth == 0 => {
                    terms.push((start, i));
                    start = i + 1;
                }
                _ => {}
            }
        }
        terms.push((start, b));
        if terms.len() == 1 {
            return self.eval_term(a, b);
        }
        terms
            .into_iter()
            .map(|(x, y)| self.eval_term(x, y))
            .reduce(GoVal::concat)
            .unwrap_or_else(|| GoVal::unknown("empty expression"))
    }

    /// The binding of local or package-level name `name` (the last one before
    /// `before`).
    fn local(&self, name: &[u8], before: u32) -> Option<&GoVal> {
        self.locals
            .as_ref()?
            .iter()
            .rev()
            .find(|(n, _, sp)| *n == name && sp.start < before)
            .map(|(_, v, _)| v)
    }

    fn eval_term(&self, a: usize, b: usize) -> GoVal {
        let (a, b) = self.trim_comments(a, b);
        if a >= b {
            return GoVal::unknown("empty expression");
        }
        let t = self.toks[a];
        if b - a == 1 {
            if t.is_string() {
                return match string_value(self.src, &t) {
                    Some(s) => GoVal::str(s.into_owned()),
                    None => GoVal::unknown(self.source(a, b)),
                };
            }
            if t.kind == Kind::Ident
                && let Some(Some(v)) = self.consts.as_ref().and_then(|c| c.get(self.text(&t)))
            {
                return GoVal::str(v.clone());
            }
            if t.kind == Kind::Ident
                && let Some(v) = self.local(self.text(&t), t.span.start)
            {
                return v.clone();
            }
            return GoVal::unknown(self.source(a, b));
        }
        if t.is_punct(self.src, b'(') && self.toks[b - 1].is_punct(self.src, b')') {
            return self.eval_sum(a + 1, b - 1);
        }
        // pkg.Func(args)
        if b - a >= 4
            && t.kind == Kind::Ident
            && self.toks[a + 1].is_punct(self.src, b'.')
            && self.toks[a + 2].kind == Kind::Ident
            && self.toks[a + 3].is_punct(self.src, b'(')
            && self.toks[b - 1].is_punct(self.src, b')')
            && let Some(module) = self.binding(self.text(&t))
            && let Some(args) = self.call_args(a + 3)
            && args.last().is_none_or(|&(_, e)| e == b - 1)
        {
            let f = self.text(&self.toks[a + 2]);
            let vals =
                || -> Vec<GoVal> { args.iter().map(|&(x, y)| self.eval_sum(x, y)).collect() };
            match (module, f) {
                // `filepath.Join` cleans its result.
                ("path/filepath" | "path", b"Join") => {
                    let joined = vals()
                        .into_iter()
                        .reduce(|x, y| x.join(y, JoinStyle::Append))
                        .unwrap_or_else(|| GoVal::str(""));
                    return clean(joined);
                }
                ("path/filepath" | "path", b"Clean") if args.len() == 1 => {
                    return clean(vals().remove(0));
                }
                ("path/filepath", b"Abs") if args.len() == 1 => return vals().remove(0),
                ("path/filepath" | "path", b"Dir") if args.len() == 1 => {
                    let v = clean(vals().remove(0));
                    return v
                        .parent()
                        .unwrap_or_else(|| GoVal::unknown(self.source(a, b)));
                }
                ("fmt", b"Sprintf") if !args.is_empty() => {
                    let v = vals();
                    return sprintf(&v[0], &v[1..]);
                }
                _ => {}
            }
        }
        GoVal::unknown(self.source(a, b))
    }

    /// Collects `consts` and `locals` once.
    fn ensure_bindings(&mut self) {
        if self.consts.is_none() {
            self.collect_consts();
        }
        if self.locals.is_none() {
            self.collect_locals();
        }
    }

    /// `a, b := x`, `var a = x`, `a = x` anywhere in the file, in source order.
    /// A single target gets the value; `_, file, _, _ := runtime.Caller(0)`
    /// binds `file` to the source file. Scopes are not tracked: later
    /// bindings of a name shadow earlier ones for uses after them.
    fn collect_locals(&mut self) {
        self.locals = Some(Vec::new());
        let sig: Vec<usize> = (0..self.toks.len())
            .filter(|&i| !self.toks[i].is_comment())
            .collect();
        for k in 1..sig.len() {
            let t = self.toks[sig[k]];
            if !t.is_punct(self.src, b'=') {
                continue;
            }
            let prev = self.toks[sig[k - 1]];
            let short = prev.is_punct(self.src, b':') && prev.span.end == t.span.start;
            if prev.kind == Kind::Punct && !short {
                continue;
            }
            // `=` must not start `==`.
            if sig.get(k + 1).is_some_and(|&n| {
                self.toks[n].is_punct(self.src, b'=') && self.toks[n].span.start == t.span.end
            }) {
                continue;
            }
            // Targets: identifiers separated by commas, back to the start of the line.
            let mut j = if short { k - 1 } else { k };
            let mut targets: Vec<&'s [u8]> = Vec::new();
            let mut ok = true;
            while j > 0 {
                let p = self.toks[sig[j - 1]];
                if p.kind == Kind::Ident {
                    let w = self.text(&p);
                    if w == b"var" || w == b"const" {
                        break;
                    }
                    targets.push(w);
                    j -= 1;
                    if j > 0 && self.toks[sig[j - 1]].is_punct(self.src, b',') {
                        j -= 1;
                        continue;
                    }
                    break;
                }
                ok = false;
                break;
            }
            if !ok || targets.is_empty() {
                continue;
            }
            if j > 0 {
                let before = self.toks[sig[j - 1]];
                let line_start = self.newline_between(&before, &self.toks[sig[j]]) > 0
                    || before.is_punct(self.src, b'{')
                    || before.is_punct(self.src, b';')
                    || before.is_punct(self.src, b'(')
                    || before.is_ident(self.src, b"var")
                    || before.is_ident(self.src, b"const");
                if !line_start {
                    continue;
                }
            }
            targets.reverse();
            // The value: up to the end of the line at depth 0.
            let start = k + 1;
            let mut end = start;
            let mut depth = 0i32;
            while end < sig.len() {
                let tk = self.toks[sig[end]];
                if end > start && depth == 0 {
                    let pv = self.toks[sig[end - 1]];
                    let cont = pv.is_punct(self.src, b'+')
                        || pv.is_punct(self.src, b',')
                        || pv.is_punct(self.src, b'.')
                        || pv.is_punct(self.src, b'(');
                    if self.newline_between(&pv, &tk) > 0 && !cont {
                        break;
                    }
                }
                if tk.kind == Kind::Punct {
                    match self.src[tk.span.start as usize] {
                        b'(' | b'[' | b'{' => depth += 1,
                        b')' | b']' | b'}' => {
                            depth -= 1;
                            if depth < 0 {
                                break;
                            }
                        }
                        b';' if depth == 0 => break,
                        _ => {}
                    }
                }
                end += 1;
            }
            if end <= start || end - start > 256 {
                continue;
            }
            let (a, b) = (sig[start], sig[end - 1] + 1);
            let span = Span::new(self.toks[a].span.start, self.toks[b - 1].span.end);
            let caller = self.toks[a].kind == Kind::Ident
                && self.binding(self.text(&self.toks[a])) == Some("runtime")
                && self
                    .toks
                    .get(a + 2)
                    .is_some_and(|c| c.is_ident(self.src, b"Caller"));
            let mut bound: Vec<(&'s [u8], GoVal)> = Vec::new();
            if caller && targets.len() >= 2 {
                if let Some(f) = &self.file_name {
                    bound.push((
                        targets[1],
                        GoVal::rel(GoAnchor::SrcDir, Value::str(f.clone())),
                    ));
                }
            } else if targets.len() == 1 {
                bound.push((targets[0], self.eval_sum(a, b)));
            }
            let locals = self.locals.get_or_insert_with(Vec::new);
            for (n, v) in bound {
                if n != b"_" && locals.len() < MAX_LOCALS {
                    locals.push((n, v, span));
                }
            }
        }
    }

    /// Paths joined from the file's own location that are not bound to a name:
    /// `return filepath.Join(filepath.Dir(file), "..", "schemas",
    /// "job.json")`, an argument of a helper. Every `filepath.Join(…)` /
    /// `path.Join(…)` call whose value is a literal path to a file anchored
    /// at the source directory, except inside the argument of a tracked API
    /// (that call is a reference already).
    fn joined_from_location(&self) -> Vec<(Value, Span)> {
        let mut out = Vec::new();
        for i in 2..self.toks.len() {
            if !(self.toks[i].is_ident(self.src, b"Join")
                && self.toks[i - 1].is_punct(self.src, b'.')
                && self.toks[i - 2].kind == Kind::Ident
                && matches!(
                    self.binding(self.text(&self.toks[i - 2])),
                    Some("path/filepath" | "path")
                ))
            {
                continue;
            }
            let Some(open) = self
                .next_sig(i + 1)
                .filter(|&o| self.toks[o].is_punct(self.src, b'('))
            else {
                continue;
            };
            // The matching `)`.
            let mut depth = 0usize;
            let mut close = None;
            for j in open..self.toks.len().min(open + MAX_ARG_TOKENS) {
                let t = self.toks[j];
                if t.is_punct(self.src, b'(') {
                    depth += 1;
                } else if t.is_punct(self.src, b')') {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
            }
            let Some(close) = close else { continue };
            let span = Span::new(self.toks[i - 2].span.start, self.toks[close].span.end);
            if self
                .anchored_args
                .iter()
                .any(|a| a.start <= span.start && span.end <= a.end)
            {
                continue;
            }
            let v = self.eval_term(i - 2, close + 1);
            if v.anchor != Some(GoAnchor::SrcDir) {
                continue;
            }
            // A literal path to a file is a reference to exactly that file. A directory is
            // a base for something this call does not say: the names scan
            // counts it as a use of the location instead (`crate::names`), with
            // the same test.
            let n = v.normalized();
            if n.as_str().is_some_and(crate::names::literal_file_path) {
                out.push((n, span));
            }
        }
        out
    }

    /// End of file: anchored path values bound to names or joined from the
    /// file's location become assets (they never reached a tracked API in a
    /// way we model), and `time.LoadLocation` without `time/tzdata` reads
    /// the system zoneinfo database.
    fn finish(&mut self) {
        if memchr::memmem::find(self.src, b"runtime.Caller").is_some() {
            self.ensure_bindings();
            let file_name = self.file_name.clone();
            let mut seen: Vec<Value> = Vec::new();
            let joined = self.joined_from_location();
            let assets: Vec<(Value, Span)> = self
                .locals
                .iter()
                .flatten()
                .filter(|(_, v, _)| v.anchor == Some(GoAnchor::SrcDir))
                .map(|(_, v, sp)| (v.normalized(), *sp))
                .chain(joined)
                .filter(|(n, _)| {
                    !anchor::is_trivial(n, file_name.as_deref()) && anchor::has_static_prefix(n)
                })
                .collect();
            for (n, span) in assets {
                if seen.contains(&n) {
                    continue;
                }
                seen.push(n.clone());
                self.out.push(
                    RawRef::new(RefKind::Asset, n.to_specifier(), span)
                        .with_base(Base::FileDir)
                        .with_flags(self.flags)
                        .with_uncertainty(Uncertainty::new(
                            UncertaintyKind::EnvDependent,
                            span,
                            CALLER_DETAIL,
                        )),
                );
            }
        }
        if let Some(span) = self.load_location
            && !self.tzdata_import
        {
            self.out.push(
                RawRef::new(RefKind::FsRead, Specifier::Unknown, span)
                    .with_flags(self.flags | RefFlags::OPTIONAL)
                    .with_uncertainty(Uncertainty::new(
                        UncertaintyKind::EnvDependent,
                        span,
                        "time.LoadLocation reads the system zoneinfo database ($ZONEINFO, \
                         /usr/share/zoneinfo) unless the binary imports time/tzdata",
                    )),
            );
        }
    }

    /// A reference for API entry `e` with the evaluated argument `spec`.
    fn api_ref(&self, e: &ApiEntry, spec: Specifier<'static>, span: Span) -> RawRef<'s> {
        let absolute = match &spec {
            Specifier::Literal(s) => s.starts_with('/'),
            Specifier::Pattern(parts) => {
                matches!(parts.first(), Some(turborepo_trace_core::Part::Lit(s)) if s.starts_with('/'))
            }
            _ => false,
        };
        let known = !matches!(spec, Specifier::Unknown);
        let detail = format!("{spec} is relative to the working directory");
        let r = RawRef::new(e.kind, spec, span)
            .with_base(if self.is_test {
                Base::FileDir
            } else {
                Base::Cwd
            })
            .with_phase(e.phase)
            .with_flags(e.flags | self.flags);
        if known && !absolute && !self.is_test {
            r.with_uncertainty(Uncertainty::new(
                UncertaintyKind::CwdDependent,
                span,
                detail,
            ))
        } else {
            r
        }
    }
}

/// `filepath.Clean` on an anchored or plain value.
fn clean(v: GoVal) -> GoVal {
    let anchored = v.anchor.is_some();
    GoVal {
        anchor: v.anchor,
        val: vpath::normalize(v.val, anchored),
    }
}

/// `fmt.Sprintf(format, args...)` with a known format: `%s`, `%v`, `%d`, `%q`
/// take the next argument (`%q` quoted is unknown), `%%` is `%`; other verbs
/// are wildcards. An unknown format is unknown.
fn sprintf(format: &GoVal, args: &[GoVal]) -> GoVal {
    let Some(f) = format.val.as_str().filter(|_| format.anchor.is_none()) else {
        return GoVal::unknown("fmt.Sprintf with a computed format");
    };
    let mut out = GoVal::str("");
    let mut lit = String::new();
    let mut next = 0usize;
    let mut chars = f.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            lit.push(c);
            continue;
        }
        // Flags, width and precision.
        let mut verb = None;
        while let Some(&n) = chars.peek() {
            chars.next();
            if n.is_ascii_alphabetic() || n == '%' {
                verb = Some(n);
                break;
            }
        }
        match verb {
            Some('%') => lit.push('%'),
            Some(v) => {
                out = out.concat(GoVal::str(std::mem::take(&mut lit)));
                let arg = args.get(next).cloned();
                next += 1;
                let piece = match (v, arg) {
                    ('s' | 'v' | 'd', Some(a)) => a,
                    _ => GoVal::unknown(format!("%{v}")),
                };
                out = out.concat(piece);
            }
            None => lit.push('%'),
        }
    }
    out.concat(GoVal::str(lit))
}

/// Turns `path.Match` metacharacters (`*`, `?`, `[…]`) in the known parts of
/// `v` into wildcards, so the resolver globs them. `?` and classes
/// over-approximate to `*`.
fn glob_value(v: Value) -> Value {
    fn split(s: &str, out: &mut Vec<PatPart>) {
        let b = s.as_bytes();
        let mut lit = String::new();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'*' | b'?' => {
                    push_lit(out, &mut lit);
                    push_wild(out, &s[i..=i]);
                    i += 1;
                }
                b'[' => {
                    push_lit(out, &mut lit);
                    let end = s[i..].find(']').map_or(b.len(), |e| i + e + 1);
                    push_wild(out, &s[i..end]);
                    i = end;
                }
                b'\\' if i + 1 < b.len() => {
                    let ch = s[i + 1..].chars().next().unwrap_or('\\');
                    lit.push(ch);
                    i += 1 + ch.len_utf8();
                }
                _ => {
                    let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
                    lit.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
        push_lit(out, &mut lit);
    }
    fn push_lit(out: &mut Vec<PatPart>, lit: &mut String) {
        if !lit.is_empty() {
            out.push(PatPart::Lit(std::mem::take(lit)));
        }
    }
    fn push_wild(out: &mut Vec<PatPart>, reason: &str) {
        if !matches!(out.last(), Some(PatPart::Wild { .. })) {
            out.push(PatPart::Wild {
                reason: reason.into(),
            });
        }
    }
    let has_meta = |s: &str| s.bytes().any(|b| matches!(b, b'*' | b'?' | b'[' | b'\\'));
    match v {
        Value::Str(s) if has_meta(&s) => {
            let mut parts = Vec::new();
            split(&s, &mut parts);
            Value::pattern(parts)
        }
        Value::Pattern(parts) => {
            let mut out = Vec::new();
            for p in parts {
                match p {
                    PatPart::Lit(s) => split(&s, &mut out),
                    w => out.push(w),
                }
            }
            Value::pattern(out)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs_at(src: &str, path: &str) -> Vec<RawRef<'static>> {
        let mut sink = RefSink::new();
        extract(src.as_bytes(), Some(Path::new(path)), &mut sink);
        let (refs, _) = sink.into_parts();
        refs.into_iter()
            .map(|r| RawRef {
                spec: r.spec.into_owned(),
                ..r
            })
            .collect()
    }

    fn refs(src: &str) -> Vec<RawRef<'static>> {
        refs_at(src, "/p/a.go")
    }

    fn specs(src: &str) -> Vec<(RefKind, String)> {
        refs(src)
            .into_iter()
            .map(|r| (r.kind, r.spec.to_string()))
            .collect()
    }

    #[test]
    fn imports_with_aliases_blank_and_dot() {
        let src = "package main\n\nimport \"fmt\"\nimport (\n\tf \"os\" // c\n\t_ \"embed\"\n\t. \
                   \"strings\"\n\t\"github.com/x/y/v2\"\n)\n\nfunc main() { fmt.Println(\"import \
                   \\\"no\\\"\") }\n";
        let got = specs(src);
        assert_eq!(
            got,
            [
                (RefKind::Import, "fmt".into()),
                (RefKind::Import, "os".into()),
                (RefKind::Import, "embed".into()),
                (RefKind::Import, "strings".into()),
                (RefKind::Import, "github.com/x/y/v2".into()),
            ]
        );
        let r = &refs(src)[0];
        assert_eq!(r.phase, Phase::Build);
        assert_eq!(r.base, IMPORT_BASE);
        assert!(r.flags.is_empty());
    }

    #[test]
    fn imports_in_comments_and_strings_are_ignored() {
        let src =
            "package a\n// import \"x\"\n/* import \"y\" */\nvar s = `import \"z\"`\nvar r = 'i'\n";
        assert!(specs(src).is_empty());
    }

    #[test]
    fn cgo_preamble_includes() {
        let src = "package a\n\n// #cgo LDFLAGS: -lm\n// #include <math.h>\n// #include \
                   \"local.h\"\nimport \"C\"\n\n/*\n#include \"other.h\"\n*/\n\nimport \"fmt\"\n";
        let r = refs(src);
        let got: Vec<_> = r
            .iter()
            .map(|r| (r.spec.to_string(), r.base.clone()))
            .collect();
        assert_eq!(
            got,
            [
                ("C".into(), IMPORT_BASE),
                ("math.h".into(), CGO_SYSTEM_BASE),
                ("local.h".into(), Base::FileDir),
                // other.h is separated from `import "fmt"` (not "C") by a blank line anyway.
                ("fmt".into(), IMPORT_BASE),
            ]
        );
        assert_eq!(&src[r[2].span.range()], "local.h");
        // Block comment preamble directly before import "C".
        let src = "package a\n/*\n#include <stdio.h>\n  #include \"x/y.h\"\n*/\nimport \"C\"\n";
        let r = refs(src);
        assert_eq!(r.len(), 3);
        assert_eq!(&src[r[2].span.range()], "x/y.h");
        // A blank line detaches the comment.
        let src = "package a\n// #include <stdio.h>\n\nimport \"C\"\n";
        assert_eq!(refs(src).len(), 1);
        // Inside an import block.
        let src = "package a\nimport (\n\t\"fmt\"\n\t// #include \"z.h\"\n\t\"C\"\n)\n";
        let r = refs(src);
        assert_eq!(r.len(), 3);
        assert_eq!(r[2].spec.to_string(), "z.h");
    }

    #[test]
    fn embed_directives() {
        let src = "package a\nimport _ \"embed\"\n//go:embed static/* \"a b.txt\" `c.txt` \
                   all:tmpl\nvar fs embed.FS\n// go:embed nope\n//go:embedded nope\n";
        let r = refs(src);
        let embeds: Vec<_> = r
            .iter()
            .filter(|r| r.kind == RefKind::Embed)
            .map(|r| r.spec.to_string())
            .collect();
        assert_eq!(embeds, ["static/*", "a b.txt", "c.txt", "all:tmpl"]);
        let e = r.iter().find(|r| r.kind == RefKind::Embed).unwrap();
        assert_eq!(e.phase, Phase::Build);
        assert_eq!(e.base, Base::FileDir);
        assert_eq!(&src[e.span.range()], "static/*");
    }

    #[test]
    fn constrained_files_are_conditional() {
        let r = refs("//go:build linux\n\npackage a\nimport \"x\"\n");
        assert!(r[0].flags.contains(RefFlags::CONDITIONAL));
        let r = refs_at("package a\nimport \"x\"\n", "/p/a_windows_amd64.go");
        assert!(r[0].flags.contains(RefFlags::CONDITIONAL));
        let r = refs_at("package a\nimport \"x\"\n", "/p/a_test.go");
        assert!(r[0].flags.is_empty());
        assert!(name_is_constrained("x_linux_test.go"));
        assert!(!name_is_constrained("linux.go"));
    }

    #[test]
    fn runtime_file_apis() {
        let src = r#"package a
import (
	"os"
	fs2 "io/ioutil"
	"path/filepath"
	"html/template"
	"net/http"
)
const dir = "conf"
const (
	tmplDir = "tmpl"
	other string = "x" + "y"
)
func f(name string) {
	os.ReadFile("data/a.json")
	fs2.ReadFile(filepath.Join(dir, "b.yaml"))
	os.Open("/etc/abs.conf")
	os.ReadFile(name)
	os.Stat(dir + "/" + name)
	template.Must(template.ParseFiles("t/a.html", tmplDir+"/b.html"))
	template.ParseGlob("views/*.tmpl")
	t.ParseGlob("more/[ab].tmpl")
	http.Handle("/", http.FileServer(http.Dir("public")))
	http.ServeFile(w, r, "index.html")
	x.os.ReadFile("not-tracked")
	ReadFile("not-tracked")
}
"#;
        let r = refs(src);
        let got: Vec<_> = r
            .iter()
            .filter(|r| r.kind != RefKind::Import)
            .map(|r| {
                (
                    r.kind,
                    r.spec.to_string(),
                    r.base.clone(),
                    r.uncertainty.as_ref().map(|u| u.kind),
                )
            })
            .collect();
        let cwd = Some(UncertaintyKind::CwdDependent);
        assert_eq!(
            got,
            [
                (RefKind::FsRead, "data/a.json".into(), Base::Cwd, cwd),
                (RefKind::FsRead, "conf/b.yaml".into(), Base::Cwd, cwd),
                (RefKind::FsRead, "/etc/abs.conf".into(), Base::Cwd, None),
                (RefKind::FsRead, "<unknown>".into(), Base::Cwd, None),
                (RefKind::FsRead, "conf/*".into(), Base::Cwd, cwd),
                (RefKind::FsRead, "t/a.html".into(), Base::Cwd, cwd),
                (RefKind::FsRead, "tmpl/b.html".into(), Base::Cwd, cwd),
                (RefKind::FsRead, "views/*.tmpl".into(), Base::Cwd, cwd),
                (RefKind::FsRead, "more/*.tmpl".into(), Base::Cwd, cwd),
                (RefKind::FsReadDir, "public".into(), Base::Cwd, cwd),
                (RefKind::FsRead, "index.html".into(), Base::Cwd, cwd),
            ]
        );
        assert!(r.iter().any(|r| r.flags.contains(RefFlags::OPTIONAL)));
        let span_text = &src[r
            .iter()
            .find(|r| r.kind == RefKind::FsReadDir)
            .unwrap()
            .span
            .range()];
        assert_eq!(span_text, "\"public\"");
    }

    #[test]
    fn test_files_read_relative_to_the_package() {
        let r = refs_at(
            "package a\nimport \"os\"\nfunc T() { os.ReadFile(\"testdata/x.json\") }\n",
            "/p/a_test.go",
        );
        let read = r.iter().find(|r| r.kind == RefKind::FsRead).unwrap();
        assert_eq!(read.base, Base::FileDir);
        assert!(read.uncertainty.is_none());
    }

    #[test]
    fn paths_joined_from_the_location_are_assets_wherever_they_are() {
        // Returned by a helper (the real shape), passed to a function, bound to a name,
        // and given to a tracked API: one reference each, none twice.
        let src = r#"package a

import (
	"os"
	"path/filepath"
	"runtime"
)

func schemaPath() string {
	_, thisFile, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(thisFile), "..", "schemas", "job.schema.json")
}

func other() {
	_, file, _, _ := runtime.Caller(0)
	load(filepath.Join(filepath.Dir(file), "data", "a.json"))
	bound := filepath.Join(filepath.Dir(file), "data", "b.json")
	os.ReadFile(filepath.Join(filepath.Dir(file), "data", "c.json"))
	root := filepath.Join(filepath.Dir(file), "..", "..")
	walk(filepath.Join(filepath.Dir(file), "..", "services"))
	_ = bound
	_ = root
}
"#;
        let r = refs_at(src, "/p/a_test.go");
        let mut got: Vec<(RefKind, String, Base)> = r
            .iter()
            .filter(|r| r.kind != RefKind::Import)
            .map(|r| (r.kind, r.spec.to_string(), r.base.clone()))
            .collect();
        got.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(
            got,
            [
                (
                    RefKind::Asset,
                    "../schemas/job.schema.json".to_owned(),
                    Base::FileDir
                ),
                (RefKind::Asset, "data/a.json".to_owned(), Base::FileDir),
                (RefKind::Asset, "data/b.json".to_owned(), Base::FileDir),
                (RefKind::FsRead, "data/c.json".to_owned(), Base::FileDir),
            ],
            "a path that only goes up, or to a directory that is not bound to a name, is no asset"
        );
        assert!(r.iter().filter(|r| r.kind == RefKind::Asset).all(|r| {
            r.uncertainty
                .as_ref()
                .is_some_and(|u| u.kind == UncertaintyKind::EnvDependent)
        }));
    }

    #[test]
    fn dot_imports_and_shadowing() {
        let src = "package a\nimport . \"os\"\nfunc ReadFile(p string) {}\nfunc g() { \
                   ReadFile(\"x.txt\") }\n";
        let got = specs(src);
        assert_eq!(got.last().unwrap(), &(RefKind::FsRead, "x.txt".to_string()));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn bindings() {
        assert_eq!(default_binding("github.com/a/b/v3"), "b");
        assert_eq!(default_binding("gopkg.in/yaml.v3"), "yaml");
        assert_eq!(default_binding("os"), "os");
        assert_eq!(default_binding("html/template"), "template");
    }

    #[test]
    fn malformed_input_does_not_panic() {
        for src in [
            "import",
            "import (",
            "import \"",
            "import x",
            "//go:embed \"",
            "os.ReadFile(",
            "package a\nimport \"C\"",
            "/*",
            "'",
            "import (\n\t\"a\"\n\tb\n",
            "x = \"",
            "const = \"a\"",
            "= \"a\"",
        ] {
            refs(src);
        }
    }
}
