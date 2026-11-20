//! The Rust recognizer: a state machine over [`crate::lexer`] tokens that finds
//! module declarations, compile-time embeds, build-script inputs and runtime
//! file reads.
//!
//! | Construct | Reference |
//! |---|---|
//! | `mod x;` (with `#[path]`, `#[cfg_attr(…, path = …)]`, inside inline `mod a { … }`) | [`RefKind::Mod`], base [`MOD`] / [`MOD_PATH`] |
//! | `include!`, `include_str!`, `include_bytes!` | [`RefKind::Embed`], Build, file dir (or crate root via `env!("CARGO_MANIFEST_DIR")`) |
//! | `include_dir!`, `sqlx::migrate!`, `#[folder]` (rust-embed), `#[template(path)]` (askama) | [`RefKind::Embed`], Build, crate root / `templates/` |
//! | `insta::assert_*_snapshot!` | [`RefKind::FsRead`] of `snapshots/*.snap` next to the file |
//! | `println!("cargo:rerun-if-changed=…")` | [`RefKind::FsRead`], Build, crate root |
//! | `std::fs::read*`, `File::open`, `fs::read_dir`, `fs::metadata` (also tokio, async-std, fs-err) | Runtime; cwd-relative unless anchored |
//! | `cc::Build::file` / `include` in `build.rs` | Build, crate root |
//!
//! `#[cfg(…)]` and `#![cfg(…)]` on the enclosing items are evaluated for the
//! target platform ([`crate::cfg`]): false drops the reference, unknown flags
//! it CONDITIONAL.
//!
//! Path arguments are partially evaluated: string literals, `concat!`, `env!`
//! (with `CARGO_MANIFEST_DIR` and `OUT_DIR` as anchors), `file!()`, `format!`,
//! `Path::new`, `PathBuf::from`, `.join(…)`, `+` and top-level string
//! `const`s/`static`s. Unknown parts become wildcards.

use std::{cell::RefCell, collections::HashMap};

use turborepo_trace_core::{
    Base, ExtractCx, Part, RawRef, RefFlags, RefKind, RefSink, Span, Specifier, Uncertainty,
    UncertaintyKind,
    api::{self, ApiEntry, ArgRole, CallStyle},
    eval::{
        PatPart, Value,
        anchor::{self, Anchored, JoinStyle},
        path,
    },
    os::OsStrExt,
};

use crate::{
    APIS,
    cfg::{self, CfgEnv, Tri},
    lexer::{LexIssue, Tok, TokKind, tokenize},
};

/// Base anchor of `mod name;` references. Spec: `name`, or `a/b::name` inside
/// inline modules `a` and `b` (each segment is a module name or an inline
/// module's `#[path]`).
pub const MOD: &str = "rust_mod";
/// Base anchor of `#[path = "p"] mod name;` references. Spec: `p`, or `a/b::p`
/// inside inline modules.
pub const MOD_PATH: &str = "rust_mod_path";
/// Base anchor for paths relative to the package root (`CARGO_MANIFEST_DIR`).
pub const CRATE_ROOT: &str = "crate_root";
/// Base anchor for askama/rinja templates (`<crate>/templates`, or
/// `askama.toml` dirs).
pub const TEMPLATES: &str = "askama_templates";
/// Base anchor for paths below `OUT_DIR` (generated; never resolved).
pub const OUT_DIR: &str = "out_dir";

/// Loaders whose argument is a dynamic library loaded at run time (libloading),
/// in the shared table format; cwd-relative unless anchored.
pub const LOADERS: &[ApiEntry] =
    &[ApiEntry::new(&["Library"], "new", RefKind::SharedLib).role(ArgRole::LoaderWrapper)];

/// Maximum tokens an argument list or expression is scanned for; longer ones
/// are unknown.
const MAX_ARG_TOKENS: usize = 512;

/// Words without which a file cannot produce a reference; files without any
/// skip lexing.
const TRIGGERS: &[&[u8]] = &[
    b"mod",
    b"include",
    b"fs",
    b"File",
    b"rerun-if-changed",
    b"migrate",
    b"template",
    b"folder",
    b"snapshot",
    b"read",
    b".file(",
    b"CARGO_MANIFEST_DIR",
    b"Library",
    b"rustc-link",
    b"pkg_config",
];

thread_local! {
    static SCRATCH: RefCell<(Vec<Tok>, Vec<LexIssue>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

/// Extracts references from `src`. Linear; never panics.
pub fn extract<'s>(src: &'s [u8], cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
    if !TRIGGERS
        .iter()
        .any(|w| memchr::memmem::find(src, w).is_some())
    {
        return;
    }
    SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        let (toks, issues) = &mut *s;
        issues.clear();
        tokenize(src, toks, issues);
        for i in issues.iter() {
            out.diagnostic(Uncertainty::new(
                UncertaintyKind::ParseIncomplete,
                i.span,
                i.what,
            ));
        }
        let file_name = cx
            .path
            .file_name()
            .map(|n| String::from_utf8_lossy(n.as_bytes()).into_owned())
            .unwrap_or_default();
        // By the path below the repository root: a checkout below a directory called
        // `tests` holds more than tests.
        let in_test_dir = cx.options.local(cx.path).components().any(|c| {
            let c = c.as_os_str();
            c == "tests" || c == "benches"
        });
        let mut rec = Rec {
            src,
            toks,
            env: CfgEnv::new(cx.options.platform()).with_all_targets(cx.options.all_targets),
            build_script: file_name == "build.rs",
            test_file: in_test_dir,
            file_name,
            scopes: Vec::new(),
            pending: None,
            file_cfg: Tri::True,
            file_test: false,
            consts: HashMap::new(),
            fs_alias: HashMap::new(),
            qual_alias: HashMap::new(),
            insta: None,
            attr_refs: Vec::new(),
            last_closed: None,
        };
        rec.run(out);
    });
}

/// One open delimiter.
#[derive(Clone, Debug)]
struct Scope {
    close: u8,
    cfg: Tri,
    test: bool,
    /// For an inline module body: its directory segment.
    mod_seg: Option<String>,
    /// For a branch of a `cfg_if!`-style chain (`if #[cfg(a)] { … } else …`):
    /// the condition an `else` after it gets, `¬a ∧ ¬(earlier branches)`.
    chain: Option<Tri>,
}

/// Outer attributes waiting for the item they apply to.
#[derive(Clone, Debug, Default)]
struct Pending {
    depth: usize,
    cfg: Option<Tri>,
    test: bool,
    /// `#[path]` values, with the condition of a `cfg_attr` (true for plain
    /// `#[path]`).
    paths: Vec<(String, Tri)>,
    /// Set when the attributes follow `if` (a `cfg_if!` branch): the negation
    /// of the earlier branches of the chain.
    if_chain: Option<Tri>,
}

/// What one attribute says.
#[derive(Clone, Debug, Default)]
struct AttrInfo {
    cfg: Option<Tri>,
    test: bool,
    paths: Vec<(String, Tri)>,
    /// `#[doc = include_str!("…")]` embeds, with the condition of a `cfg_attr`.
    embeds: Vec<(Ev, Span, Tri)>,
}

/// A partially evaluated path: an anchor plus a value relative to it.
#[derive(Clone, Debug)]
struct Ev {
    anchor: Anchor,
    val: Value,
    /// An environment variable (other than the anchors) the value depends on.
    env: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Anchor {
    /// No anchor: interpreted by the API (file dir, crate root or cwd).
    Rel,
    /// The directory of the current file (`file!()`).
    FileDir,
    /// `env!("CARGO_MANIFEST_DIR")`.
    CrateRoot,
    /// `env!("OUT_DIR")`.
    OutDir,
}

impl Ev {
    fn str(s: impl Into<String>) -> Ev {
        Ev {
            anchor: Anchor::Rel,
            val: Value::str(s),
            env: None,
        }
    }

    fn unknown(reason: impl Into<Box<str>>) -> Ev {
        Ev {
            anchor: Anchor::Rel,
            val: Value::unknown(reason),
            env: None,
        }
    }

    fn anchored(anchor: Anchor) -> Ev {
        Ev {
            anchor,
            val: Value::str(""),
            env: None,
        }
    }

    /// The shared anchored value ([`Anchored`]); `Rel` is no anchor.
    fn into_shared(self) -> (Anchored<Anchor>, Option<String>) {
        let anchor = (self.anchor != Anchor::Rel).then_some(self.anchor);
        (
            Anchored {
                anchor,
                val: self.val,
            },
            self.env,
        )
    }

    fn from_shared(a: Anchored<Anchor>, env: Option<String>) -> Ev {
        Ev {
            anchor: a.anchor.unwrap_or(Anchor::Rel),
            val: a.val,
            env,
        }
    }

    /// String concatenation (`concat!`, `format!`, `+`): the shared
    /// [`Anchored::concat`] (an anchor is only meaningful at the start).
    fn concat(self, b: Ev) -> Ev {
        let (a, ea) = self.into_shared();
        let (b, eb) = b.into_shared();
        Ev::from_shared(a.concat(b), ea.or(eb))
    }

    /// `Path::join`: the shared [`Anchored::join`] with [`JoinStyle::Reset`]
    /// (an absolute or anchored argument replaces the base).
    fn join(self, b: Ev) -> Ev {
        let (a, ea) = self.into_shared();
        let (b, eb) = b.into_shared();
        Ev::from_shared(a.join(b, JoinStyle::Reset), ea.or(eb))
    }
}

/// How an API anchors an unanchored relative path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum RelBase {
    FileDir,
    CrateRoot,
    Templates,
    /// The runtime working directory (crate root in tests and build scripts).
    Cwd,
}

struct Rec<'a> {
    src: &'a [u8],
    toks: &'a [Tok],
    env: CfgEnv<'static>,
    build_script: bool,
    test_file: bool,
    file_name: String,
    scopes: Vec<Scope>,
    pending: Option<Pending>,
    file_cfg: Tri,
    file_test: bool,
    consts: HashMap<String, Ev>,
    /// `use std::fs::read_to_string as rts;` → `rts` = (`fs`,
    /// `read_to_string`).
    fs_alias: HashMap<String, (String, String)>,
    /// `use std::fs as stdfs;` → `stdfs` = `fs`.
    qual_alias: HashMap<String, String>,
    /// First insta assertion: span and flags.
    insta: Option<(Span, RefFlags)>,
    /// Embeds found in attributes (`#[folder]`, `#[template]`, `#[doc =
    /// include_str!]`) with their `cfg_attr` condition, emitted when the
    /// attribute list ends (a later `#[cfg]` on the same item still
    /// applies).
    attr_refs: Vec<(Ev, RelBase, Span, Tri)>,
    /// The last closed `}`: its token index and the chain condition of its
    /// scope.
    last_closed: Option<(usize, Option<Tri>)>,
}

impl<'a> Rec<'a> {
    fn tok(&self, i: usize) -> Option<&'a Tok> {
        self.toks.get(i)
    }

    fn punct(&self, i: usize, b: u8) -> bool {
        self.tok(i).is_some_and(|t| t.is_punct(b))
    }

    fn ident(&self, i: usize) -> Option<&'a str> {
        let t = self.tok(i)?;
        match t.kind {
            TokKind::Ident | TokKind::RawIdent => std::str::from_utf8(t.bytes(self.src)).ok(),
            _ => None,
        }
    }

    fn keyword(&self, i: usize, kw: &str) -> bool {
        self.tok(i).is_some_and(|t| t.is_ident(self.src, kw))
    }

    fn path_sep(&self, i: usize) -> bool {
        self.punct(i, b':') && self.punct(i + 1, b':')
    }

    fn text(&self, start: usize, end: usize) -> String {
        let (Some(a), Some(b)) = (
            self.tok(start),
            end.checked_sub(1).and_then(|e| self.tok(e)),
        ) else {
            return String::new();
        };
        let s = self
            .src
            .get(a.start as usize..b.end as usize)
            .unwrap_or_default();
        let mut t = String::from_utf8_lossy(s).into_owned();
        if t.len() > 80 {
            let mut cut = 77;
            while !t.is_char_boundary(cut) {
                cut -= 1;
            }
            t.truncate(cut);
            t.push_str("...");
        }
        t
    }

    fn span(&self, start: usize, end_incl: usize) -> Span {
        match (self.tok(start), self.tok(end_incl)) {
            (Some(a), Some(b)) if b.end >= a.start => Span::new(a.start, b.end),
            (Some(a), _) => a.span(),
            _ => Span::DUMMY,
        }
    }

    /// The index of the delimiter closing the one at `open`, scanning at most
    /// [`MAX_ARG_TOKENS`] tokens. Returns the end of input (or the cap) if
    /// unclosed.
    fn matching(&self, open: usize) -> usize {
        let mut depth = 0usize;
        let lim = (open + MAX_ARG_TOKENS).min(self.toks.len());
        for j in open..lim {
            match self.toks[j].kind {
                TokKind::Punct(b'(' | b'[' | b'{') => depth += 1,
                TokKind::Punct(b')' | b']' | b'}') => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return j;
                    }
                }
                _ => {}
            }
        }
        lim
    }

    /// Top-level comma-separated ranges inside the delimiters at `open..close`.
    fn args(&self, open: usize, close: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        let mut start = open + 1;
        for j in open + 1..close.min(self.toks.len()) {
            match self.toks[j].kind {
                TokKind::Punct(b'(' | b'[' | b'{') => depth += 1,
                TokKind::Punct(b')' | b']' | b'}') => depth = depth.saturating_sub(1),
                TokKind::Punct(b',') if depth == 0 => {
                    out.push((start, j));
                    start = j + 1;
                }
                _ => {}
            }
        }
        if start < close.min(self.toks.len()) {
            out.push((start, close.min(self.toks.len())));
        }
        out
    }

    // ---- state ----

    fn depth(&self) -> usize {
        self.scopes.len()
    }

    fn pending_active(&self) -> Option<&Pending> {
        self.pending.as_ref().filter(|p| p.depth <= self.depth())
    }

    /// The cfg state and test-ness at the current position.
    fn state(&self) -> (Tri, bool) {
        let (mut c, mut t) = (self.file_cfg, self.file_test || self.test_file);
        if let Some(s) = self.scopes.last() {
            c = c.and(s.cfg);
            t |= s.test;
        }
        if let Some(p) = self.pending_active() {
            if let Some(pc) = p.cfg {
                c = c.and(pc);
            }
            t |= p.test;
        }
        (c, t)
    }

    /// Flags for a reference at the current position, or `None` if cfg'd out.
    fn flags(&self) -> Option<RefFlags> {
        match self.state().0 {
            Tri::False => None,
            Tri::Unknown => Some(RefFlags::CONDITIONAL),
            Tri::True => Some(RefFlags::empty()),
        }
    }

    fn inline_dir(&self) -> String {
        let segs: Vec<&str> = self
            .scopes
            .iter()
            .filter_map(|s| s.mod_seg.as_deref())
            .collect();
        segs.join("/")
    }

    // ---- main loop ----

    fn run<'s>(&mut self, out: &mut RefSink<'s>) {
        let n = self.toks.len();
        let mut i = 0usize;
        while i < n {
            let t = self.toks[i];
            match t.kind {
                TokKind::Punct(b'#') => {
                    if self.punct(i + 1, b'[') {
                        i = self.outer_attr(i + 1);
                        if !self.punct(i, b'#') {
                            self.flush_attr_refs(out);
                        }
                        continue;
                    }
                    if self.punct(i + 1, b'!') && self.punct(i + 2, b'[') {
                        i = self.inner_attr(i + 2);
                        // Inner attributes belong to the enclosing scope: flush after the
                        // last one, before outer attributes of the next item (whose `cfg`
                        // must not apply to `#![doc = include_str!(…)]`).
                        if !(self.punct(i, b'#') && self.punct(i + 1, b'!')) {
                            self.flush_attr_refs(out);
                        }
                        continue;
                    }
                }
                TokKind::Punct(c @ (b'{' | b'(' | b'[')) => {
                    self.open(c, None);
                }
                TokKind::Punct(c @ (b'}' | b')' | b']')) => {
                    // Pop to the matching opener within a few levels; a stray closer is
                    // ignored (error recovery).
                    if let Some(k) = self.scopes.iter().rev().take(4).position(|s| s.close == c) {
                        let keep = self.scopes.len() - 1 - k;
                        let chain = self.scopes.get(keep).and_then(|s| s.chain);
                        self.scopes.truncate(keep);
                        if c == b'}' {
                            self.last_closed = Some((i, chain));
                        }
                    }
                    if self
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.depth > self.depth())
                    {
                        self.pending = None;
                    }
                }
                TokKind::Punct(b';' | b',') => {
                    if self
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.depth >= self.depth())
                    {
                        self.pending = None;
                    }
                }
                TokKind::Ident => {
                    if let Some(next) = self.ident_at(i, out) {
                        i = next;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if let Some((span, flags)) = self.insta.take() {
            out.push(
                RawRef::new(
                    RefKind::FsRead,
                    Specifier::Pattern(vec![
                        Part::Lit("snapshots/".into()),
                        Part::Wild {
                            reason: "insta snapshot name".into(),
                        },
                        Part::Lit(".snap".into()),
                    ]),
                    span,
                )
                .with_flags(flags),
            );
        }
    }

    fn open(&mut self, c: u8, mod_seg: Option<String>) {
        let close = match c {
            b'{' => b'}',
            b'(' => b')',
            _ => b']',
        };
        let (mut cfg, mut test) = match self.scopes.last() {
            Some(s) => (s.cfg, s.test),
            None => (Tri::True, false),
        };
        let mut chain = None;
        if c == b'{'
            && let Some(p) = self.pending.take_if(|p| p.depth == self.scopes.len())
        {
            let pc = p.cfg.unwrap_or(Tri::True);
            if let Some(before) = p.if_chain {
                cfg = cfg.and(before);
                chain = Some(before.and(!pc));
            }
            cfg = cfg.and(pc);
            test |= p.test;
        }
        self.scopes.push(Scope {
            close,
            cfg,
            test,
            mod_seg,
            chain,
        });
    }

    /// Handles an identifier at `i`; returns the next index if it consumed
    /// tokens.
    fn ident_at<'s>(&mut self, i: usize, out: &mut RefSink<'s>) -> Option<usize> {
        let word = self.toks[i].bytes(self.src);
        match word {
            b"mod" => return self.module(i, out),
            b"else" if self.punct(i + 1, b'{') => {
                // The final `else` of a cfg_if! chain.
                if let Some((k, Some(c))) = self.last_closed
                    && k + 1 == i
                {
                    self.open(b'{', None);
                    if let Some(s) = self.scopes.last_mut() {
                        s.cfg = s.cfg.and(c);
                    }
                    return Some(i + 2);
                }
            }
            b"use" => return Some(self.use_tree(i)),
            b"const" | b"static" => self.constant(i, i + 1, out),
            b"let" => {
                let at = if self.keyword(i + 1, "mut") {
                    i + 2
                } else {
                    i + 1
                };
                self.constant(i, at, out);
            }
            _ => {}
        }
        let name = std::str::from_utf8(word).ok()?;
        if self.punct(i + 1, b'!') {
            if let Some(open) = Some(i + 2).filter(|&o| {
                self.tok(o)
                    .is_some_and(|t| matches!(t.kind, TokKind::Punct(b'(' | b'[' | b'{')))
            }) {
                self.macro_call(i, name, open, out);
            }
            return None;
        }
        if self.punct(i + 1, b'(') {
            self.call(i, name, out);
        }
        None
    }

    // ---- attributes ----

    fn outer_attr(&mut self, open: usize) -> usize {
        let close = self.attr_close(open);
        let info = self.attr_info(open + 1, close);
        self.doc_embeds(&info.embeds);
        let depth = self.depth();
        // `if #[cfg(a)] {` (cfg_if!): the branch also requires that earlier branches of
        // an `else if` chain did not match.
        let if_chain = (open >= 2 && self.keyword(open - 2, "if")).then(|| {
            match (
                open >= 4 && self.keyword(open - 3, "else"),
                self.last_closed,
            ) {
                (true, Some((k, Some(c)))) if k + 4 == open => c,
                _ => Tri::True,
            }
        });
        let p = match &mut self.pending {
            Some(p) if p.depth == depth => p,
            _ => self.pending.insert(Pending {
                depth,
                ..Default::default()
            }),
        };
        if if_chain.is_some() {
            p.if_chain = if_chain;
        }
        if let Some(c) = info.cfg {
            p.cfg = Some(p.cfg.map_or(c, |pc| pc.and(c)));
        }
        p.test |= info.test;
        p.paths.extend(info.paths);
        close + 1
    }

    fn inner_attr(&mut self, open: usize) -> usize {
        let close = self.attr_close(open);
        let info = self.attr_info(open + 1, close);
        self.doc_embeds(&info.embeds);
        match self.scopes.last_mut() {
            Some(s) => {
                if let Some(c) = info.cfg {
                    s.cfg = s.cfg.and(c);
                }
                s.test |= info.test;
            }
            None => {
                if let Some(c) = info.cfg {
                    self.file_cfg = self.file_cfg.and(c);
                }
                self.file_test |= info.test;
            }
        }
        close + 1
    }

    /// The `]` closing the attribute opened at `open` (bracket-balanced,
    /// uncapped: the main loop skips the attribute, so this stays linear).
    fn attr_close(&self, open: usize) -> usize {
        let mut depth = 0usize;
        for j in open..self.toks.len() {
            match self.toks[j].kind {
                TokKind::Punct(b'(' | b'[' | b'{') => depth += 1,
                TokKind::Punct(b')' | b']' | b'}') => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return j;
                    }
                }
                _ => {}
            }
        }
        self.toks.len()
    }

    /// Interprets the attribute contents `toks[start..end]`, and records embeds
    /// from attribute APIs (`#[folder]`, `#[template]`).
    fn attr_info(&mut self, start: usize, end: usize) -> AttrInfo {
        let mut info = AttrInfo::default();
        // Path: a::b::c
        let mut j = start;
        let mut last = None;
        while j < end {
            match self.ident(j) {
                Some(id) => {
                    last = Some(id);
                    j += 1;
                    if self.path_sep(j) {
                        j += 2;
                        continue;
                    }
                }
                None if self.path_sep(j) => {
                    j += 2;
                    continue;
                }
                None => {}
            }
            break;
        }
        let Some(name) = last else {
            return info;
        };
        match name {
            "cfg" if self.punct(j, b'(') => {
                let mut k = j + 1;
                let pred = cfg::parse(self.toks, self.src, &mut k, end);
                info.cfg = Some(pred.eval(&self.env));
                info.test = pred.mentions("test");
            }
            "cfg_attr" if self.punct(j, b'(') => {
                let close = self.attr_close(j).min(end);
                let mut k = j + 1;
                let pred = cfg::parse(self.toks, self.src, &mut k, close);
                let cond = pred.eval(&self.env);
                if cond == Tri::False {
                    return info;
                }
                // The remaining comma-separated attributes.
                for (a, b) in self.args(j, close).into_iter().skip(1) {
                    let inner = self.attr_info(a, b);
                    info.test |= inner.test;
                    info.embeds.extend(
                        inner
                            .embeds
                            .into_iter()
                            .map(|(e, s, c)| (e, s, c.and(cond))),
                    );
                    info.paths
                        .extend(inner.paths.into_iter().map(|(p, c)| (p, c.and(cond))));
                    if let Some(c) = inner.cfg {
                        // `cfg_attr(p, cfg(x))` is `cfg(any(not(p), x))`.
                        info.cfg = Some((!cond).or(c));
                    }
                }
            }
            "path" if self.punct(j, b'=') => {
                if let Some(s) = self.tok(j + 1).and_then(|t| t.text(self.src)) {
                    info.paths.push((s.into_owned(), Tri::True));
                }
            }
            "test" | "bench" => info.test = true,
            "doc" if self.punct(j, b'=') => {
                // `include_str!`/`include_bytes!` anywhere in the value, also nested in
                // `concat!("```\n", include_str!("../examples/x.rs"), "```")`.
                let mut k = j + 1;
                while k + 2 < end {
                    if matches!(self.ident(k), Some("include_str" | "include_bytes"))
                        && self.punct(k + 1, b'!')
                    {
                        let open = k + 2;
                        let close = self.matching(open).min(end);
                        let ev = match self.args(open, close).first() {
                            Some(&(a, b)) => self.eval_range(a, b),
                            None => Ev::unknown("include_str!()"),
                        };
                        let span = self.span(k, close.min(end.saturating_sub(1)));
                        info.embeds.push((ev, span, Tri::True));
                        k = close;
                    }
                    k += 1;
                }
            }
            "folder" if self.punct(j, b'=') => {
                // rust-embed: relative to the crate root; `$CARGO_MANIFEST_DIR` interpolation.
                if let Some(s) = self.tok(j + 1).and_then(|t| t.text(self.src)) {
                    let span = self.span(start, end.saturating_sub(1));
                    self.attr_embed(&s, RelBase::CrateRoot, span);
                }
            }
            "template" if self.punct(j, b'(') => {
                let close = self.attr_close(j).min(end);
                for (a, _) in self.args(j, close) {
                    if self.keyword(a, "path")
                        && self.punct(a + 1, b'=')
                        && let Some(s) = self.tok(a + 2).and_then(|t| t.text(self.src))
                    {
                        let span = self.span(start, end.saturating_sub(1));
                        self.attr_embed(&s, RelBase::Templates, span);
                    }
                }
            }
            _ => {}
        }
        info
    }

    fn doc_embeds(&mut self, embeds: &[(Ev, Span, Tri)]) {
        for (ev, span, cond) in embeds {
            self.attr_refs
                .push((ev.clone(), RelBase::FileDir, *span, *cond));
        }
    }

    /// Records an attribute embed; [`Rec::flush_attr_refs`] moves it into the
    /// sink.
    fn attr_embed(&mut self, value: &str, base: RelBase, span: Span) {
        let ev = expand_manifest_var(value);
        self.attr_refs.push((ev, base, span, Tri::True));
    }

    // ---- items ----

    fn module<'s>(&mut self, i: usize, out: &mut RefSink<'s>) -> Option<usize> {
        let name = self.ident(i + 1)?.to_owned();
        let j = i + 2;
        if self.punct(j, b';') {
            self.mod_decl(&name, self.span(i, j), out);
            self.pending = None;
            return Some(j + 1);
        }
        if self.punct(j, b'{') {
            let seg = self
                .pending_active()
                .and_then(|p| p.paths.iter().find(|(_, c)| *c == Tri::True))
                .map(|(p, _)| p.clone())
                .unwrap_or(name);
            self.open(b'{', Some(seg));
            return Some(j + 1);
        }
        None
    }

    fn mod_decl<'s>(&mut self, name: &str, span: Span, out: &mut RefSink<'s>) {
        let Some(flags) = self.flags() else {
            return;
        };
        let inline = self.inline_dir();
        let spec = |s: &str| {
            if inline.is_empty() {
                s.to_owned()
            } else {
                format!("{inline}::{s}")
            }
        };
        let paths = self
            .pending_active()
            .map(|p| p.paths.clone())
            .unwrap_or_default();
        if let Some((p, _)) = paths.iter().find(|(_, c)| *c == Tri::True) {
            out.push(mod_ref(MOD_PATH, spec(p), span, flags));
            return;
        }
        let unknown: Vec<&String> = paths
            .iter()
            .filter(|(_, c)| *c == Tri::Unknown)
            .map(|(p, _)| p)
            .collect();
        if unknown.is_empty() {
            out.push(mod_ref(MOD, spec(name), span, flags));
            return;
        }
        let alt = flags | RefFlags::CONDITIONAL | RefFlags::OPTIONAL;
        out.push(mod_ref(MOD, spec(name), span, alt));
        for p in unknown {
            out.push(mod_ref(MOD_PATH, spec(p), span, alt));
        }
    }

    /// `use` trees: records aliases of tracked fs functions and modules.
    /// Returns the index after the terminating `;`.
    fn use_tree(&mut self, i: usize) -> usize {
        let mut path: Vec<String> = Vec::new();
        let mut marks: Vec<usize> = Vec::new();
        let mut alias: Option<String> = None;
        let mut expect_alias = false;
        let mut leaf = false;
        let mut j = i + 1;
        let lim = (i + MAX_ARG_TOKENS).min(self.toks.len());
        while j < lim {
            let t = self.toks[j];
            match t.kind {
                TokKind::Punct(b';') => break,
                TokKind::Ident | TokKind::RawIdent => {
                    let id = String::from_utf8_lossy(t.bytes(self.src)).into_owned();
                    if id == "as" && t.kind == TokKind::Ident {
                        expect_alias = true;
                    } else if expect_alias {
                        alias = Some(id);
                        expect_alias = false;
                    } else {
                        path.push(id);
                        leaf = true;
                    }
                }
                TokKind::Punct(b'{') => marks.push(path.len()),
                TokKind::Punct(b',') => {
                    if leaf {
                        self.use_leaf(&path, alias.take());
                    }
                    leaf = false;
                    alias = None;
                    path.truncate(marks.last().copied().unwrap_or(0));
                }
                TokKind::Punct(b'}') => {
                    if leaf {
                        self.use_leaf(&path, alias.take());
                    }
                    leaf = false;
                    alias = None;
                    path.truncate(marks.pop().unwrap_or(0));
                }
                TokKind::Punct(b'*') => leaf = false,
                _ => {}
            }
            j += 1;
        }
        if leaf {
            self.use_leaf(&path, alias);
        }
        self.pending = None;
        j + 1
    }

    fn use_leaf(&mut self, path: &[String], alias: Option<String>) {
        let mut path = path.to_vec();
        if path.last().is_some_and(|l| l == "self") {
            path.pop();
        }
        let Some(name) = path.last().cloned() else {
            return;
        };
        let local = alias.unwrap_or_else(|| name.clone());
        let parent = path.len().checked_sub(2).map(|k| path[k].as_str());
        if matches!(parent, Some("fs" | "fs_err"))
            && api::lookup(APIS, Some("fs"), &name, CallStyle::Call).is_some()
        {
            self.fs_alias.insert(local, ("fs".into(), name));
        } else if (name == "fs" || name == "fs_err") && local != name {
            self.qual_alias.insert(local, "fs".into());
        } else if name == "File" && local != name {
            self.qual_alias.insert(local, "File".into());
        }
    }

    /// `const NAME: T = <string expr>;` and `static NAME: T = …;` at any depth.
    /// `const|static NAME[: T] = expr;` and `let [mut] NAME[: T] = expr;` (the
    /// name at token `at`): a known string or anchored value is remembered
    /// for later uses of `NAME`; an anchored path value
    /// (`concat!(env!("CARGO_MANIFEST_DIR"), "/x")`, `Path::new(env!("
    /// CARGO_MANIFEST_DIR")).join("x")`) is also an asset, because it is
    /// used at run time even when no tracked API reads it (the shared
    /// anchored-asset policy).
    fn constant<'s>(&mut self, i: usize, at: usize, out: &mut RefSink<'s>) {
        let Some(name) = self.ident(at).map(str::to_owned) else {
            return;
        };
        let lim = (i + MAX_ARG_TOKENS).min(self.toks.len());
        let mut depth = 0usize;
        let mut eq = None;
        let mut end = None;
        for j in at + 1..lim {
            match self.toks[j].kind {
                TokKind::Punct(b'(' | b'[' | b'{') => depth += 1,
                TokKind::Punct(b')' | b']' | b'}') => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                TokKind::Punct(b'=') if depth == 0 && eq.is_none() => eq = Some(j),
                TokKind::Punct(b';') if depth == 0 => {
                    end = Some(j);
                    break;
                }
                _ => {}
            }
        }
        let (Some(eq), Some(end)) = (eq, end) else {
            return;
        };
        // `==`, `=>`, `<=`, ... are not the binding `=`.
        if self
            .toks
            .get(eq + 1)
            .is_some_and(|t| matches!(t.kind, TokKind::Punct(b'=' | b'>')))
        {
            return;
        }
        // A binding behind a false `#[cfg]` must not shadow the live one.
        let Some(flags) = self.flags() else {
            return;
        };
        let mut k = eq + 1;
        let ev = self.expr(&mut k, end, 0);
        if k != end {
            return;
        }
        if matches!(ev.anchor, Anchor::CrateRoot | Anchor::FileDir) {
            let n = path::normalize(ev.val.clone(), true);
            let file = (ev.anchor == Anchor::FileDir).then_some(self.file_name.as_str());
            if !anchor::is_trivial(&n, file) && anchor::has_static_prefix(&n) {
                let span = self.span(eq + 1, end - 1);
                let phase = if self.build_script {
                    turborepo_trace_core::Phase::Build
                } else {
                    turborepo_trace_core::Phase::Runtime
                };
                out.push(
                    self.path_ref(RefKind::Asset, ev.clone(), RelBase::Cwd, span, flags)
                        .with_phase(phase),
                );
            }
        }
        if ev.val.as_str().is_some() || ev.anchor != Anchor::Rel {
            self.consts.insert(name, ev);
        }
    }

    // ---- macros and calls ----

    fn qualifier(&self, i: usize) -> Option<&'a str> {
        if i >= 3 && self.path_sep(i - 2) {
            self.ident(i - 3)
        } else {
            None
        }
    }

    fn macro_call<'s>(&mut self, i: usize, name: &str, open: usize, out: &mut RefSink<'s>) {
        let close = self.matching(open);
        let span = self.span(i, close.min(self.toks.len().saturating_sub(1)));
        match name {
            "include" | "include_str" | "include_bytes" => {
                let Some(flags) = self.flags() else { return };
                let args = self.args(open, close);
                let ev = match args.first() {
                    Some(&(a, b)) => self.eval_range(a, b),
                    None => Ev::unknown(format!("{name}!()")),
                };
                let r = self.path_ref(RefKind::Embed, ev, RelBase::FileDir, span, flags);
                // `include!` splices Rust source: the included file is parsed as part of the
                // module tree too (it can declare modules and embed further files).
                if name == "include"
                    && r.base != Base::Language(OUT_DIR)
                    && !matches!(r.spec, Specifier::Unknown)
                {
                    let mut m = r.clone();
                    m.kind = RefKind::Mod;
                    m.uncertainty = None;
                    out.push(r);
                    out.push(m);
                } else {
                    out.push(r);
                }
            }
            "include_dir" => {
                let Some(flags) = self.flags() else { return };
                let args = self.args(open, close);
                let ev = match args.first() {
                    Some(&(a, b)) => {
                        let ev = self.eval_range(a, b);
                        match ev.val.as_str() {
                            Some(s) if ev.anchor == Anchor::Rel => expand_manifest_var(s),
                            _ => ev,
                        }
                    }
                    None => Ev::unknown("include_dir!()"),
                };
                out.push(self.path_ref(RefKind::Embed, ev, RelBase::CrateRoot, span, flags));
            }
            "migrate" if self.qualifier(i).is_none_or(|q| q == "sqlx") => {
                let Some(flags) = self.flags() else { return };
                let args = self.args(open, close);
                let ev = match args.first() {
                    Some(&(a, b)) => self.eval_range(a, b),
                    None => Ev::str("migrations"),
                };
                out.push(self.path_ref(RefKind::Embed, ev, RelBase::CrateRoot, span, flags));
            }
            "println" | "print" | "writeln" | "write" => {
                let arg = usize::from(name.starts_with("write"));
                let args = self.args(open, close);
                let Some(&(a, _)) = args.get(arg) else { return };
                if !self.tok(a).is_some_and(|t| {
                    t.text(self.src).is_some_and(|s| {
                        s.contains("rerun-if-changed") || s.contains("rustc-link-")
                    })
                }) {
                    return;
                }
                let Some(flags) = self.flags() else { return };
                let fmt = self.format(open, close, arg);
                self.rerun_if_changed(fmt, span, flags, out);
            }
            n if n.starts_with("assert_") && n.ends_with("snapshot") => {
                if self.insta.is_none()
                    && let Some(flags) = self.flags()
                {
                    self.insta = Some((span, flags));
                }
            }
            _ => {}
        }
    }

    fn flush_attr_refs<'s>(&mut self, out: &mut RefSink<'s>) {
        let refs = std::mem::take(&mut self.attr_refs);
        for (ev, base, span, cond) in refs {
            let flags = match (self.flags(), cond) {
                (None, _) | (_, Tri::False) => continue,
                (Some(f), Tri::Unknown) => f | RefFlags::CONDITIONAL,
                (Some(f), Tri::True) => f,
            };
            out.push(self.path_ref(RefKind::Embed, ev, base, span, flags));
        }
    }

    // Invariant: `lines` starts with one entry and never shrinks.
    #[allow(clippy::expect_used)]
    fn rerun_if_changed<'s>(&self, fmt: Ev, span: Span, flags: RefFlags, out: &mut RefSink<'s>) {
        // One reference per `cargo:rerun-if-changed=` line of the printed text.
        let parts = match &fmt.val {
            Value::Str(s) => vec![PatPart::Lit(s.clone())],
            Value::Pattern(p) => p.clone(),
            _ => return,
        };
        // Split the parts into lines.
        let mut lines: Vec<Vec<PatPart>> = vec![Vec::new()];
        for p in parts {
            match p {
                PatPart::Lit(s) => {
                    let mut first = true;
                    for seg in s.split('\n') {
                        if !first {
                            lines.push(Vec::new());
                        }
                        first = false;
                        if !seg.is_empty() {
                            lines
                                .last_mut()
                                .expect("non-empty")
                                .push(PatPart::Lit(seg.into()));
                        }
                    }
                }
                w => lines.last_mut().expect("non-empty").push(w),
            }
        }
        for line in lines {
            let Some(PatPart::Lit(head)) = line.first() else {
                continue;
            };
            let instr = |key: &str| {
                head.strip_prefix("cargo:")
                    .or_else(|| head.strip_prefix("cargo::"))
                    .and_then(|h| h.strip_prefix(key))
                    .and_then(|h| h.strip_prefix('='))
            };
            if let Some(lib) = instr("rustc-link-lib") {
                self.link_lib(lib, line.len() > 1, span, flags, out);
                continue;
            }
            if let Some(dir) = instr("rustc-link-search") {
                // `[KIND=]PATH`: the directory the linker searches (native libraries built or
                // vendored by the build script).
                let dir = match dir.split_once('=') {
                    Some(("native" | "crate" | "dependency" | "framework" | "all", d)) => d,
                    _ => dir,
                };
                let mut parts = vec![PatPart::Lit(dir.to_owned())];
                parts.extend(line.iter().skip(1).cloned());
                let ev = Ev {
                    anchor: Anchor::Rel,
                    val: Value::pattern(parts),
                    env: fmt.env.clone(),
                };
                out.push(
                    self.path_ref(RefKind::FsReadDir, ev, RelBase::CrateRoot, span, flags)
                        .with_phase(turborepo_trace_core::Phase::Build),
                );
                continue;
            }
            let rest = head
                .strip_prefix("cargo:rerun-if-changed=")
                .or_else(|| head.strip_prefix("cargo::rerun-if-changed="));
            let Some(rest) = rest else { continue };
            let mut parts = vec![PatPart::Lit(rest.to_owned())];
            parts.extend(line.into_iter().skip(1));
            let ev = Ev {
                anchor: Anchor::Rel,
                val: Value::pattern(parts),
                env: fmt.env.clone(),
            };
            out.push(
                self.path_ref(RefKind::FsRead, ev, RelBase::CrateRoot, span, flags)
                    .with_phase(turborepo_trace_core::Phase::Build),
            );
        }
    }

    /// `cargo:rustc-link-lib=[KIND[:MODIFIERS]=]NAME[:RENAME]`: a native
    /// library the linker finds in link-search directories or system paths
    /// (a dynamic one is also needed at run time). Recorded as an
    /// uncertainty: the file is located outside the source tree.
    fn link_lib<'s>(
        &self,
        spec: &str,
        partial: bool,
        span: Span,
        flags: RefFlags,
        out: &mut RefSink<'s>,
    ) {
        let name = spec.rsplit_once('=').map_or(spec, |(_, n)| n);
        let name = name.split(':').next().unwrap_or(name);
        let kind = spec
            .split_once('=')
            .map(|(k, _)| k.split(':').next().unwrap_or(k));
        let shown = if partial || name.is_empty() {
            "(computed)"
        } else {
            name
        };
        let linkage = match kind {
            Some("static") => "statically linked",
            Some("framework") => "a macOS framework",
            _ => "located by the linker and the dynamic loader",
        };
        out.push(
            RawRef::new(RefKind::SharedLib, Specifier::Unknown, span)
                .with_phase(turborepo_trace_core::Phase::Build)
                .with_flags(flags)
                .with_uncertainty(Uncertainty::new(
                    UncertaintyKind::EnvDependent,
                    span,
                    format!(
                        "build script links native library `{shown}` (cargo:rustc-link-lib), \
                         {linkage} in link-search directories or system paths"
                    ),
                )),
        );
    }

    fn call<'s>(&mut self, i: usize, name: &str, out: &mut RefSink<'s>) {
        if i > 0 && self.keyword(i - 1, "fn") {
            return;
        }
        // `pkg_config::probe_library("x")` / `Config::new().probe("x")` in build
        // scripts: a system library found through pkg-config.
        if self.build_script
            && (name == "probe_library" || (name == "probe" && i > 0 && self.punct(i - 1, b'.')))
            && let Some(flags) = self.flags()
        {
            let close = self.matching(i + 1);
            let span = self.span(i, close.min(self.toks.len().saturating_sub(1)));
            let lib = self
                .args(i + 1, close)
                .first()
                .map(|&(a, b)| self.eval_range(a, b).val)
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "(computed)".into());
            out.push(
                RawRef::new(RefKind::SharedLib, Specifier::Unknown, span)
                    .with_phase(turborepo_trace_core::Phase::Build)
                    .with_flags(flags)
                    .with_uncertainty(Uncertainty::new(
                        UncertaintyKind::EnvDependent,
                        span,
                        format!("pkg-config locates system library `{lib}` at build time"),
                    )),
            );
            return;
        }
        let method = i > 0 && self.punct(i - 1, b'.');
        let entry = if method {
            if !self.build_script {
                return;
            }
            api::lookup(APIS, Some("cc::Build"), name, CallStyle::Call)
        } else {
            let qual = self
                .qualifier(i)
                .map(|q| self.qual_alias.get(q).map_or(q, String::as_str));
            match qual {
                Some(q) => api::lookup(APIS, Some(q), name, CallStyle::Call)
                    .or_else(|| api::lookup(LOADERS, Some(q), name, CallStyle::Call)),
                None => self
                    .fs_alias
                    .get(name)
                    .and_then(|(q, n)| api::lookup(APIS, Some(q), n, CallStyle::Call)),
            }
        };
        let Some(e) = entry else { return };
        let Some(flags) = self.flags() else { return };
        let open = i + 1;
        let close = self.matching(open);
        let args = self.args(open, close);
        let Some(&(a, b)) = args.get(e.arg as usize) else {
            return;
        };
        let ev = self.eval_range(a, b);
        let base = match e.base {
            api::BaseRule::PackageRoot => RelBase::CrateRoot,
            api::BaseRule::FileDir => RelBase::FileDir,
            _ => RelBase::Cwd,
        };
        let span = self.span(i, close.min(self.toks.len().saturating_sub(1)));
        // `Library::new("libfoo.so")`: a bare name is searched on the library path.
        let bare = e.kind == RefKind::SharedLib
            && ev.anchor == Anchor::Rel
            && ev.val.as_str().is_some_and(|s| !s.contains('/'));
        let mut r = self
            .path_ref(e.kind, ev, base, span, flags | e.flags)
            .with_phase(e.phase);
        if bare {
            r.uncertainty = Some(Uncertainty::new(
                UncertaintyKind::EnvDependent,
                span,
                format!(
                    "{} is a bare library name: the dynamic loader searches LD_LIBRARY_PATH and \
                     system paths, not the working directory",
                    r.spec
                ),
            ));
        }
        out.push(r);
    }

    // ---- references ----

    fn path_ref<'s>(
        &self,
        kind: RefKind,
        ev: Ev,
        rel: RelBase,
        span: Span,
        flags: RefFlags,
    ) -> RawRef<'s> {
        let test_cwd = self.build_script || self.state().1;
        let mut uncertainty = None;
        let (base, val) = match ev.anchor {
            Anchor::OutDir => {
                let rest = match &ev.val {
                    Value::Str(s) => s.clone(),
                    v => v.to_specifier().to_string(),
                };
                let spec = format!("$OUT_DIR{rest}");
                return RawRef::new(kind, Specifier::Literal(spec.clone().into()), span)
                    .with_base(Base::Language(OUT_DIR))
                    .with_flags(flags)
                    .with_uncertainty(Uncertainty::new(
                        UncertaintyKind::GeneratedFile,
                        span,
                        format!("{spec} is generated by the build script"),
                    ));
            }
            Anchor::CrateRoot => (Base::Language(CRATE_ROOT), strip_root(ev.val)),
            Anchor::FileDir => (Base::FileDir, ev.val),
            Anchor::Rel => {
                if path::starts_absolute(&ev.val) {
                    (Base::FileDir, ev.val)
                } else {
                    match rel {
                        RelBase::FileDir => (Base::FileDir, ev.val),
                        RelBase::CrateRoot => (Base::Language(CRATE_ROOT), ev.val),
                        RelBase::Templates => (Base::Language(TEMPLATES), ev.val),
                        RelBase::Cwd if test_cwd => (Base::Language(CRATE_ROOT), ev.val),
                        RelBase::Cwd => {
                            uncertainty = Some(Uncertainty::new(
                                UncertaintyKind::CwdDependent,
                                span,
                                format!(
                                    "{} is relative to the working directory",
                                    ev.val.to_specifier()
                                ),
                            ));
                            (Base::Cwd, ev.val)
                        }
                    }
                }
            }
        };
        let spec = val.to_specifier();
        match &val {
            Value::Unknown { reason } => {
                let kind_u = if ev.env.is_some() {
                    UncertaintyKind::EnvDependent
                } else {
                    UncertaintyKind::ComputedSpecifier
                };
                uncertainty = Some(Uncertainty::new(kind_u, span, reason.to_string()));
            }
            Value::Pattern(_) if ev.env.is_some() => {
                uncertainty = Some(Uncertainty::new(
                    UncertaintyKind::EnvDependent,
                    span,
                    format!(
                        "{spec} depends on environment variable {}",
                        ev.env.as_deref().unwrap_or("?")
                    ),
                ));
            }
            _ => {}
        }
        let mut r = RawRef::new(kind, spec, span)
            .with_base(base)
            .with_flags(flags);
        r.uncertainty = uncertainty;
        r
    }

    // ---- evaluation ----

    fn eval_range(&self, a: usize, b: usize) -> Ev {
        let mut k = a;
        let ev = self.expr(&mut k, b, 0);
        if k < b {
            // Trailing tokens we do not understand.
            return Ev {
                val: ev.val.concat(Value::unknown(self.text(k, b))),
                ..ev
            };
        }
        ev
    }

    fn expr(&self, i: &mut usize, end: usize, depth: u32) -> Ev {
        let mut acc = self.postfix(i, end, depth);
        while *i < end && self.punct(*i, b'+') {
            *i += 1;
            let rhs = self.postfix(i, end, depth);
            acc = acc.concat(rhs);
        }
        acc
    }

    fn postfix(&self, i: &mut usize, end: usize, depth: u32) -> Ev {
        let mut ev = self.primary(i, end, depth);
        while *i < end {
            if self.punct(*i, b'?') {
                *i += 1;
                continue;
            }
            if !self.punct(*i, b'.') {
                break;
            }
            let Some(m) = self.ident(*i + 1) else { break };
            let start = *i;
            *i += 2;
            let mut arg = None;
            if self.punct(*i, b'(') {
                let close = self.matching(*i).min(end);
                if let Some(&(a, b)) = self.args(*i, close).first() {
                    arg = Some(self.eval_depth(a, b, depth + 1));
                }
                *i = close + 1;
            }
            ev = match (m, arg) {
                ("join", Some(a)) => ev.join(a),
                ("parent", None) => match path::dirname(ev.val.clone(), ev.anchor != Anchor::Rel) {
                    Some(v) => Ev { val: v, ..ev },
                    None => Ev::unknown(self.text(start, *i)),
                },
                (
                    "to_string" | "to_owned" | "as_str" | "as_ref" | "into" | "as_path"
                    | "to_path_buf" | "display" | "clone" | "unwrap" | "borrow" | "as_os_str"
                    | "into_os_string" | "to_str" | "to_string_lossy" | "expect",
                    _,
                ) => ev,
                _ => Ev {
                    val: ev
                        .val
                        .concat(Value::unknown(self.text(start, (*i).min(end)))),
                    ..ev
                },
            };
        }
        ev
    }

    fn eval_depth(&self, a: usize, b: usize, depth: u32) -> Ev {
        let mut k = a;
        let ev = self.expr(&mut k, b, depth);
        if k < b {
            return Ev {
                val: ev.val.concat(Value::unknown(self.text(k, b))),
                ..ev
            };
        }
        ev
    }

    fn primary(&self, i: &mut usize, end: usize, depth: u32) -> Ev {
        if depth > 16 {
            *i = end;
            return Ev::unknown("expression too deep");
        }
        while *i < end && (self.punct(*i, b'&') || self.punct(*i, b'*')) {
            *i += 1;
        }
        let Some(t) = self.tok(*i).filter(|_| *i < end) else {
            return Ev::unknown("missing argument");
        };
        match t.kind {
            TokKind::Str(_) => {
                *i += 1;
                match t.text(self.src) {
                    Some(s) => Ev::str(s.into_owned()),
                    None => Ev::unknown("undecodable string"),
                }
            }
            TokKind::Num | TokKind::Char => {
                *i += 1;
                Ev::str(String::from_utf8_lossy(t.bytes(self.src)).trim_matches('\''))
            }
            TokKind::Punct(b'(') => {
                let close = self.matching(*i).min(end);
                let ev = self.eval_depth(*i + 1, close, depth + 1);
                *i = close + 1;
                ev
            }
            TokKind::Ident | TokKind::RawIdent | TokKind::Punct(b':') => {
                let start = *i;
                let mut segs: Vec<&str> = Vec::new();
                loop {
                    if self.path_sep(*i) {
                        *i += 2;
                        continue;
                    }
                    if let Some(s) = self.ident(*i).filter(|_| *i < end) {
                        segs.push(s);
                        *i += 1;
                        if self.path_sep(*i) && !self.punct(*i + 2, b'<') {
                            continue;
                        }
                    }
                    break;
                }
                let Some(&last) = segs.last() else {
                    *i = (*i + 1).min(end);
                    return Ev::unknown(self.text(start, *i));
                };
                if self.punct(*i, b'!') && *i + 1 < end {
                    let open = *i + 1;
                    let close = self.matching(open).min(end);
                    *i = close + 1;
                    return self.eval_macro(last, open, close, depth, start);
                }
                if self.punct(*i, b'(') {
                    let open = *i;
                    let close = self.matching(open).min(end);
                    *i = close + 1;
                    let callee = segs
                        .len()
                        .checked_sub(2)
                        .map_or(("", last), |k| (segs[k], last));
                    return match callee {
                        ("Path" | "PathBuf" | "String" | "OsStr" | "OsString", _)
                        | (_, "Path" | "PathBuf") => match self.args(open, close).first() {
                            Some(&(a, b)) => self.eval_depth(a, b, depth + 1),
                            None => Ev::unknown(self.text(start, *i)),
                        },
                        _ => Ev::unknown(self.text(start, (*i).min(end))),
                    };
                }
                if segs.len() == 1
                    && let Some(c) = self.consts.get(last)
                {
                    return c.clone();
                }
                Ev::unknown(self.text(start, *i))
            }
            _ => {
                *i += 1;
                Ev::unknown(self.text(*i - 1, *i))
            }
        }
    }

    fn eval_macro(&self, name: &str, open: usize, close: usize, depth: u32, start: usize) -> Ev {
        let args = self.args(open, close);
        let first_text = || {
            args.first()
                .and_then(|&(a, _)| self.tok(a))
                .and_then(|t| t.text(self.src))
        };
        match name {
            "concat" => args
                .iter()
                .map(|&(a, b)| self.eval_depth(a, b, depth + 1))
                .fold(Ev::str(""), Ev::concat),
            "env" => match first_text().as_deref() {
                Some("CARGO_MANIFEST_DIR") => Ev::anchored(Anchor::CrateRoot),
                Some("OUT_DIR") => Ev::anchored(Anchor::OutDir),
                Some(v) => Ev {
                    env: Some(v.to_owned()),
                    ..Ev::unknown(format!("env!({v:?})"))
                },
                None => Ev::unknown("env!(…)"),
            },
            "option_env" => {
                let v = first_text().map(|s| s.into_owned()).unwrap_or_default();
                Ev {
                    env: Some(v.clone()),
                    ..Ev::unknown(format!("option_env!({v:?})"))
                }
            }
            "file" => Ev {
                anchor: Anchor::FileDir,
                val: Value::str(self.file_name.clone()),
                env: None,
            },
            "format" => self.format(open, close, 0),
            _ => Ev::unknown(self.text(start, (close + 1).min(self.toks.len()))),
        }
    }

    /// Evaluates a `format!`-style argument list whose format string is
    /// argument `fmt_arg`.
    fn format(&self, open: usize, close: usize, fmt_arg: usize) -> Ev {
        let args = self.args(open, close);
        let Some(fmt) = args
            .get(fmt_arg)
            .and_then(|&(a, _)| self.tok(a))
            .and_then(|t| t.text(self.src))
        else {
            return Ev::unknown("format string");
        };
        let rest = &args[fmt_arg + 1..];
        let mut positional: Vec<Ev> = Vec::new();
        let mut named: HashMap<&str, Ev> = HashMap::new();
        for &(a, b) in rest {
            if let Some(n) = self.ident(a)
                && self.punct(a + 1, b'=')
                && !self.punct(a + 2, b'=')
            {
                named.insert(n, self.eval_depth(a + 2, b, 1));
            } else {
                positional.push(self.eval_depth(a, b, 1));
            }
        }
        let mut acc = Ev::str("");
        let mut lit = String::new();
        let mut next_pos = 0usize;
        let mut chars = fmt.char_indices().peekable();
        while let Some((k, c)) = chars.next() {
            match c {
                '{' if chars.peek().map(|p| p.1) == Some('{') => {
                    chars.next();
                    lit.push('{');
                }
                '}' if chars.peek().map(|p| p.1) == Some('}') => {
                    chars.next();
                    lit.push('}');
                }
                '{' => {
                    let close_at = fmt[k..].find('}').map(|p| k + p);
                    let Some(ce) = close_at else {
                        lit.push_str(&fmt[k..]);
                        break;
                    };
                    let inner = &fmt[k + 1..ce];
                    let key = inner.split(':').next().unwrap_or("").trim();
                    let v = if key.is_empty() {
                        let v = positional.get(next_pos).cloned();
                        next_pos += 1;
                        v
                    } else if let Ok(n) = key.parse::<usize>() {
                        positional.get(n).cloned()
                    } else {
                        named
                            .get(key)
                            .cloned()
                            .or_else(|| self.consts.get(key).cloned())
                    };
                    acc = acc.concat(Ev::str(std::mem::take(&mut lit)));
                    acc = acc.concat(v.unwrap_or_else(|| Ev::unknown(format!("{{{inner}}}"))));
                    while chars.peek().is_some_and(|p| p.0 <= ce) {
                        chars.next();
                    }
                }
                c => lit.push(c),
            }
        }
        acc.concat(Ev::str(lit))
    }
}

fn mod_ref<'s>(base: &'static str, spec: String, span: Span, flags: RefFlags) -> RawRef<'s> {
    RawRef::new(RefKind::Mod, Specifier::Literal(spec.into()), span)
        .with_base(Base::Language(base))
        .with_flags(flags)
}

/// Strips the leading `/` of a value anchored at a directory.
fn strip_root(v: Value) -> Value {
    match v {
        Value::Str(s) => Value::Str(s.trim_start_matches('/').to_owned()),
        Value::Pattern(mut p) => {
            if let Some(PatPart::Lit(s)) = p.first_mut() {
                *s = s.trim_start_matches('/').to_owned();
            }
            Value::pattern(p)
        }
        v => v,
    }
}

/// `$CARGO_MANIFEST_DIR/x` (include_dir, rust-embed interpolation) as a
/// crate-root anchor.
fn expand_manifest_var(s: &str) -> Ev {
    for var in ["$CARGO_MANIFEST_DIR", "${CARGO_MANIFEST_DIR}"] {
        if let Some(rest) = s.strip_prefix(var) {
            return Ev {
                anchor: Anchor::CrateRoot,
                val: Value::str(rest.trim_start_matches('/')),
                env: None,
            };
        }
    }
    if s.contains('$') {
        return Ev {
            env: Some(s.to_owned()),
            ..Ev::unknown(format!("{s} (environment interpolation)"))
        };
    }
    Ev::str(s)
}
