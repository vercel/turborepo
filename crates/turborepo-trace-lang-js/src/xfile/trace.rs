//! Reuse of a binding result on another graph: the questions a run asked.
//!
//! [`bind`](super::bind::bind) reads nothing but its roots and what the
//! [`World`] answers, and it is deterministic: its next question follows from
//! the roots and the answers so far. So a run in another world that gets the
//! same answers to the same questions, in order, is the same run, with the same
//! result. That is what this module checks instead of binding again:
//!
//! - A [`Recorder`] is a world that passes every question on and notes each
//!   distinct one with its answer. [`Recorder::finish`] turns the notes and the
//!   run's result into a [`Trace`]: files by path, answers as digests.
//! - [`replay`] asks the recorded questions of another world (the graph after
//!   an edit). When every answer has the digest it had, the recorded result is
//!   the result there; otherwise there is no reuse and the caller binds.
//!
//! What "the same answer" means:
//!
//! - **Files are compared by path.** The run also depends on the order of file
//!   ids (the roots are bound in that order, loaders are sorted by it), so the
//!   files of a trace must exist in the other world in the same relative order.
//!   In a graph ids follow paths, so this always holds there; it is checked all
//!   the same.
//! - **Facts** are compared by what they are a function of where the world can
//!   name that ([`World::facts_source`]: the file's contents), else by a digest
//!   of the facts ([`World::facts_digest`]). An edit that leaves a file's facts
//!   as they were (a comment, the body of a function no fact describes)
//!   therefore changes nothing; an edit that moves a call to another line
//!   changes the facts, and the whole result is bound again: the run shares its
//!   memo tables and its step budget between the sites, so no part of it is a
//!   function of fewer answers than the ones before it.
//! - Every other answer (the targets of an import, the importers of a module,
//!   what a specifier or a path resolves to, whether a file is code, the tests'
//!   working directories) is a list of files and numbers, compared in order.
//!
//! A question the recorded run did not ask is not asked of the other world
//! either, however the graph changed there: the run never looked.
//!
//! The roots are not part of a world's answers: the caller passes the roots it
//! would bind, and they must be the recorded ones.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use turborepo_trace_core::{Origin, RefKind, timing::Span};
use xxhash_rust::xxh3::{Xxh3, xxh3_64};

use super::{
    bind::{Bound, Closed, ClosedTable, Dep, DepValue, Importer, World},
    facts::FileFacts,
};

/// A digest of the facts of a file: equal for equal facts, 0 for a file without
/// facts.
pub fn facts_digest(facts: Option<&FileFacts>) -> u64 {
    facts.map_or(0, |f| encoded_facts_digest(&super::codec::encode(f)))
}

/// [`facts_digest`] of facts in their encoded form ([`super::codec::encode`]).
pub fn encoded_facts_digest(encoded: &[u8]) -> u64 {
    xxh3_64(encoded).max(1)
}

/// A question with its answer, as the world of the run gave it (file ids of
/// that world).
enum Raw {
    Facts(u32),
    Targets(u32, u32, Vec<u32>),
    Importers(u32, Vec<Importer>),
    Resolve(u32, String, Vec<u32>),
    ModuleAt(String, Vec<u32>),
    IsCode(u32, bool),
    Cwds(Vec<String>),
}

#[derive(Default)]
struct Log {
    facts: HashSet<u32>,
    targets: HashSet<(u32, u32)>,
    importers: HashSet<u32>,
    resolve: HashMap<u32, HashSet<String>>,
    module_at: HashSet<String>,
    is_code: HashSet<u32>,
    cwds: bool,
    /// Each distinct question, in the order first asked.
    asks: Vec<Raw>,
}

/// A world that answers as `w` does and notes every question (see the module
/// documentation).
pub struct Recorder<'w> {
    w: &'w dyn World,
    log: RefCell<Log>,
}

impl<'w> Recorder<'w> {
    /// A recorder in front of `w`.
    pub fn new(w: &'w dyn World) -> Recorder<'w> {
        Recorder {
            w,
            log: RefCell::default(),
        }
    }

    /// The trace of the run that asked the recorded questions from `roots` and
    /// computed `bound`. `None` when the result names a file the run never
    /// asked about.
    pub fn finish(self, roots: &[u32], bound: &Bound) -> Option<Trace> {
        let log = self.log.into_inner();
        // Every file the run saw, in the order of the ids.
        let mut ids: Vec<u32> = roots.to_vec();
        for a in &log.asks {
            match a {
                Raw::Facts(f) | Raw::IsCode(f, _) => ids.push(*f),
                Raw::Targets(f, _, ts) | Raw::Resolve(f, _, ts) => {
                    ids.push(*f);
                    ids.extend(ts);
                }
                Raw::Importers(f, imps) => {
                    ids.push(*f);
                    ids.extend(imps.iter().map(|i| i.file));
                }
                Raw::ModuleAt(_, ts) => ids.extend(ts),
                Raw::Cwds(_) => {}
            }
        }
        ids.sort_unstable();
        ids.dedup();
        let ix = |id: u32| ids.binary_search(&id).ok().map(|i| i as u32);
        let list = |ts: &[u32]| digest_ids(ts.iter().map(|t| ix(*t)));
        let mut asks = Vec::with_capacity(log.asks.len());
        for a in &log.asks {
            asks.push(match a {
                Raw::Facts(f) => Ask::Facts {
                    file: ix(*f)?,
                    source: self.w.facts_source(*f),
                    digest: self.w.facts_digest(*f),
                },
                Raw::Targets(f, at, ts) => Ask::Targets {
                    file: ix(*f)?,
                    at: *at,
                    answer: list(ts),
                },
                Raw::Importers(f, imps) => Ask::Importers {
                    file: ix(*f)?,
                    answer: digest_importers(imps.iter().map(|i| (ix(i.file), i.at))),
                },
                Raw::Resolve(f, spec, ts) => Ask::Resolve {
                    file: ix(*f)?,
                    spec: spec.clone(),
                    answer: list(ts),
                },
                Raw::ModuleAt(path, ts) => Ask::ModuleAt {
                    path: path.clone(),
                    answer: list(ts),
                },
                Raw::IsCode(f, b) => Ask::IsCode {
                    file: ix(*f)?,
                    answer: *b,
                },
                Raw::Cwds(cwds) => Ask::Cwds {
                    answer: digest_strings(cwds),
                },
            });
        }
        Some(Trace {
            files: ids.iter().map(|id| self.w.path(*id)).collect(),
            roots: roots.iter().map(|r| ix(*r)).collect::<Option<_>>()?,
            asks,
            bound: map_bound(bound, &ix)?,
        })
    }
}

impl World for Recorder<'_> {
    fn path(&self, file: u32) -> String {
        self.w.path(file)
    }

    fn facts(&self, file: u32) -> Option<Arc<FileFacts>> {
        let f = self.w.facts(file);
        let mut log = self.log.borrow_mut();
        if log.facts.insert(file) {
            log.asks.push(Raw::Facts(file));
        }
        f
    }

    fn targets(&self, file: u32, at: u32) -> Vec<u32> {
        let ts = self.w.targets(file, at);
        let mut log = self.log.borrow_mut();
        if log.targets.insert((file, at)) {
            log.asks.push(Raw::Targets(file, at, ts.clone()));
        }
        ts
    }

    fn importers(&self, file: u32) -> Vec<Importer> {
        let imps = self.w.importers(file);
        let mut log = self.log.borrow_mut();
        if log.importers.insert(file) {
            log.asks.push(Raw::Importers(file, imps.clone()));
        }
        imps
    }

    fn prefetch(&self, files: &[u32]) {
        self.w.prefetch(files);
    }

    fn expect_importers(&self, files: &[u32]) {
        self.w.expect_importers(files);
    }

    fn resolve(&self, file: u32, spec: &str) -> Vec<u32> {
        let ts = self.w.resolve(file, spec);
        let mut log = self.log.borrow_mut();
        let seen = log.resolve.entry(file).or_default();
        if !seen.contains(spec) {
            seen.insert(spec.to_owned());
            log.asks
                .push(Raw::Resolve(file, spec.to_owned(), ts.clone()));
        }
        ts
    }

    fn module_at(&self, path: &str) -> Vec<u32> {
        let ts = self.w.module_at(path);
        let mut log = self.log.borrow_mut();
        if !log.module_at.contains(path) {
            log.module_at.insert(path.to_owned());
            log.asks.push(Raw::ModuleAt(path.to_owned(), ts.clone()));
        }
        ts
    }

    fn is_code(&self, file: u32) -> bool {
        let b = self.w.is_code(file);
        let mut log = self.log.borrow_mut();
        if log.is_code.insert(file) {
            log.asks.push(Raw::IsCode(file, b));
        }
        b
    }

    fn cwds(&self) -> Vec<String> {
        let cwds = self.w.cwds();
        let mut log = self.log.borrow_mut();
        if !log.cwds {
            log.cwds = true;
            log.asks.push(Raw::Cwds(cwds.clone()));
        }
        cwds
    }

    fn file(&self, path: &str) -> Option<u32> {
        self.w.file(path)
    }

    fn facts_source(&self, file: u32) -> Option<u128> {
        self.w.facts_source(file)
    }

    fn facts_digest(&self, file: u32) -> u64 {
        self.w.facts_digest(file)
    }
}

/// A recorded question with a digest of its answer. Files are indices into
/// [`Trace::files`].
#[derive(Clone, Debug, PartialEq)]
enum Ask {
    Facts {
        file: u32,
        /// What the facts were a function of ([`World::facts_source`]).
        source: Option<u128>,
        /// [`World::facts_digest`].
        digest: u64,
    },
    Targets {
        file: u32,
        at: u32,
        answer: u64,
    },
    Importers {
        file: u32,
        answer: u64,
    },
    Resolve {
        file: u32,
        spec: String,
        answer: u64,
    },
    ModuleAt {
        path: String,
        answer: u64,
    },
    IsCode {
        file: u32,
        answer: bool,
    },
    Cwds {
        answer: u64,
    },
}

/// The questions of a binding run with digests of their answers, and its result
/// (see the module documentation).
#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    /// The paths of the files the run saw, in the order of their ids.
    files: Vec<String>,
    /// The roots, in the order they were bound.
    roots: Vec<u32>,
    asks: Vec<Ask>,
    /// The result; its file ids are indices into `files`.
    bound: Bound,
}

fn digest_ids(ids: impl Iterator<Item = Option<u32>>) -> u64 {
    let mut h = Xxh3::new();
    for id in ids {
        // A file outside the trace has no index: it never equals a recorded answer.
        h.update(&id.map_or(u64::MAX, u64::from).to_le_bytes());
    }
    h.digest()
}

fn digest_importers(imps: impl Iterator<Item = (Option<u32>, Option<u32>)>) -> u64 {
    let mut h = Xxh3::new();
    for (file, at) in imps {
        h.update(&file.map_or(u64::MAX, u64::from).to_le_bytes());
        h.update(&at.map_or(u64::MAX, u64::from).to_le_bytes());
    }
    h.digest()
}

fn digest_strings(xs: &[String]) -> u64 {
    let mut h = Xxh3::new();
    for x in xs {
        h.update(&(x.len() as u64).to_le_bytes());
        h.update(x.as_bytes());
    }
    h.digest()
}

/// `b` with every file id replaced by `f` of it; `None` when `f` has none for
/// one.
fn map_bound(b: &Bound, f: &dyn Fn(u32) -> Option<u32>) -> Option<Bound> {
    Some(Bound {
        deps: b
            .deps
            .iter()
            .map(|d| {
                Some(Dep {
                    file: f(d.file)?,
                    sink_file: f(d.sink_file)?,
                    ..d.clone()
                })
            })
            .collect::<Option<_>>()?,
        closed: b
            .closed
            .iter()
            .map(|c| {
                Some(Closed {
                    file: f(c.file)?,
                    ..c.clone()
                })
            })
            .collect::<Option<_>>()?,
        tables: b
            .tables
            .iter()
            .map(|t| {
                Some(ClosedTable {
                    file: f(t.file)?,
                    ..t.clone()
                })
            })
            .collect::<Option<_>>()?,
        files: b.files,
        exhausted: b.exhausted,
    })
}

/// What [`replay`] found.
#[derive(Debug, PartialEq)]
pub struct Replayed {
    /// The result of the recorded run, with the file ids of the world it was
    /// replayed in.
    pub bound: Bound,
    /// The trace changed: the facts of a file had another source and the same
    /// digest, and the trace now names the new source (worth storing again:
    /// the next replay need not look at those facts).
    pub updated: bool,
    /// The files of the trace ([`Trace::paths`]) as ids of the world it was
    /// replayed in, ascending.
    pub files: Vec<u32>,
}

/// The result of the recorded run, when a run in `w` from `roots` asks the
/// recorded questions and gets the recorded answers (see the module
/// documentation). `None`: it does not, or `w` cannot tell.
pub fn replay(trace: &mut Trace, w: &dyn World, roots: &[u32]) -> Option<Replayed> {
    static FILES: Span = Span::new("replay: files by path");
    static FACTS: Span = Span::new("replay: facts");
    static TARGETS: Span = Span::new("replay: targets of imports");
    static IMPORTERS: Span = Span::new("replay: importers");
    static RESOLVE: Span = Span::new("replay: specifiers and paths");
    // The files, in the same relative order.
    let files_time = FILES.enter();
    let mut ids: Vec<u32> = Vec::with_capacity(trace.files.len());
    for path in &trace.files {
        let id = w.file(path)?;
        if ids.last().is_some_and(|last| *last >= id) {
            return None;
        }
        ids.push(id);
    }
    let id = |i: u32| ids.get(i as usize).copied();
    let ix = |id: u32| ids.binary_search(&id).ok().map(|i| i as u32);
    let list = |ts: Vec<u32>| digest_ids(ts.into_iter().map(ix));
    if trace.roots.len() != roots.len()
        || trace
            .roots
            .iter()
            .zip(roots)
            .any(|(i, r)| id(*i) != Some(*r))
    {
        return None;
    }
    drop(files_time);
    let mut updated = false;
    // The facts first: they are what an edit changes most often, and each takes a
    // comparison of two numbers.
    for facts in [true, false] {
        if !facts {
            // The importers of the modules the run asked about, in one pass over the edges.
            let _t = IMPORTERS.enter();
            let asked: Vec<u32> = trace
                .asks
                .iter()
                .filter_map(|a| match a {
                    Ask::Importers { file, .. } => id(*file),
                    _ => None,
                })
                .collect();
            w.expect_importers(&asked);
        }
        for a in &mut trace.asks {
            if matches!(a, Ask::Facts { .. }) != facts {
                continue;
            }
            let _t = match a {
                Ask::Facts { .. } => FACTS.enter(),
                Ask::Targets { .. } => TARGETS.enter(),
                Ask::Importers { .. } => IMPORTERS.enter(),
                _ => RESOLVE.enter(),
            };
            let same = match a {
                Ask::Facts {
                    file,
                    source,
                    digest,
                } => {
                    let f = id(*file)?;
                    let now = w.facts_source(f);
                    if source.is_some() && *source == now {
                        true
                    } else if w.facts_digest(f) == *digest {
                        // (A file without a nameable source stays without one: nothing
                        // to store again.)
                        if *source != now {
                            *source = now;
                            updated = true;
                        }
                        true
                    } else {
                        false
                    }
                }
                Ask::Targets { file, at, answer } => list(w.targets(id(*file)?, *at)) == *answer,
                Ask::Importers { file, answer } => {
                    let imps = w.importers(id(*file)?);
                    digest_importers(imps.iter().map(|i| (ix(i.file), i.at))) == *answer
                }
                Ask::Resolve { file, spec, answer } => list(w.resolve(id(*file)?, spec)) == *answer,
                Ask::ModuleAt { path, answer } => list(w.module_at(path)) == *answer,
                Ask::IsCode { file, answer } => w.is_code(id(*file)?) == *answer,
                Ask::Cwds { answer } => digest_strings(&w.cwds()) == *answer,
            };
            if !same {
                return None;
            }
        }
    }
    let bound = map_bound(&trace.bound, &id)?;
    Some(Replayed {
        bound,
        updated,
        files: ids,
    })
}

// ---- the stored form ----

const MAGIC: &[u8; 8] = b"DTXTRAC1";

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, mut v: u32) {
        while v >= 0x80 {
            self.0.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.0.push(v as u8);
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn span(&mut self, s: [u32; 2]) {
        self.u32(s[0]);
        self.u32(s[1]);
    }
    fn strs(&mut self, xs: &[String]) {
        self.u32(xs.len() as u32);
        for x in xs {
            self.str(x);
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl R<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.at)?;
        self.at += 1;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let mut v = 0u32;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            v |= u32::from(b & 0x7f).checked_shl(shift)?;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    fn u64(&mut self) -> Option<u64> {
        let b = self.b.get(self.at..self.at + 8)?;
        self.at += 8;
        Some(u64::from_le_bytes(b.try_into().ok()?))
    }
    /// A count of items that each take at least one byte.
    fn len(&mut self) -> Option<usize> {
        let n = self.u32()? as usize;
        (n <= self.b.len() - self.at).then_some(n)
    }
    fn str(&mut self) -> Option<String> {
        let n = self.len()?;
        let s = std::str::from_utf8(self.b.get(self.at..self.at + n)?).ok()?;
        self.at += n;
        Some(s.to_owned())
    }
    fn span(&mut self) -> Option<[u32; 2]> {
        Some([self.u32()?, self.u32()?])
    }
    fn strs(&mut self) -> Option<Vec<String>> {
        let n = self.len()?;
        let mut xs = Vec::with_capacity(n);
        for _ in 0..n {
            xs.push(self.str()?);
        }
        Some(xs)
    }
}

impl Trace {
    /// Number of files the run saw.
    pub fn files(&self) -> usize {
        self.files.len()
    }

    /// The paths of the files the run saw, in the order of their ids: every
    /// file of a question, of an answer or of the result. The run read the
    /// contents of no other file (its roots are among them).
    pub fn paths(&self) -> &[String] {
        &self.files
    }

    /// Number of distinct questions the run asked.
    pub fn questions(&self) -> usize {
        self.asks.len()
    }

    /// The trace as bytes ([`Trace::from_bytes`]).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        w.0.extend_from_slice(MAGIC);
        w.strs(&self.files);
        w.u32(self.roots.len() as u32);
        for r in &self.roots {
            w.u32(*r);
        }
        w.u32(self.asks.len() as u32);
        for a in &self.asks {
            match a {
                Ask::Facts {
                    file,
                    source,
                    digest,
                } => {
                    w.u8(u8::from(source.is_some()));
                    w.u32(*file);
                    if let Some(s) = source {
                        w.0.extend_from_slice(&s.to_le_bytes());
                    }
                    w.u64(*digest);
                }
                Ask::Targets { file, at, answer } => {
                    w.u8(2);
                    w.u32(*file);
                    w.u32(*at);
                    w.u64(*answer);
                }
                Ask::Importers { file, answer } => {
                    w.u8(3);
                    w.u32(*file);
                    w.u64(*answer);
                }
                Ask::Resolve { file, spec, answer } => {
                    w.u8(4);
                    w.u32(*file);
                    w.str(spec);
                    w.u64(*answer);
                }
                Ask::ModuleAt { path, answer } => {
                    w.u8(5);
                    w.str(path);
                    w.u64(*answer);
                }
                Ask::IsCode { file, answer } => {
                    w.u8(6 + u8::from(*answer));
                    w.u32(*file);
                }
                Ask::Cwds { answer } => {
                    w.u8(8);
                    w.u64(*answer);
                }
            }
        }
        let b = &self.bound;
        w.u32(b.deps.len() as u32);
        for d in &b.deps {
            w.u32(d.file);
            w.u32(d.line);
            w.u8(kind_code(d.kind));
            w.u32(d.sink_file);
            w.u32(d.sink_line);
            w.span(d.sink_site);
            match &d.value {
                DepValue::Path(s) => {
                    w.u8(0);
                    w.str(s);
                }
                DepValue::Below(s) => {
                    w.u8(1);
                    w.str(s);
                }
                DepValue::Cwd(s) => {
                    w.u8(2);
                    w.str(s);
                }
                DepValue::Spec(s) => {
                    w.u8(3);
                    w.str(s);
                }
                DepValue::PerTestCwd(s) => {
                    w.u8(4);
                    w.str(s);
                }
                DepValue::Open { origin, detail } => {
                    w.u8(5);
                    w.u8(Origin::ALL.iter().position(|o| o == origin).unwrap_or(0) as u8);
                    w.str(detail);
                }
            }
        }
        w.u32(b.closed.len() as u32);
        for c in &b.closed {
            w.u32(c.file);
            w.span(c.at);
            w.span(c.site);
            w.u8(u8::from(c.open));
            w.strs(&c.specs);
        }
        w.u32(b.tables.len() as u32);
        for t in &b.tables {
            w.u32(t.file);
            w.span(t.at);
            w.strs(&t.specs);
        }
        w.u32(b.files as u32);
        w.u8(u8::from(b.exhausted));
        w.0
    }

    /// The trace [`Trace::to_bytes`] wrote; `None` for anything else.
    pub fn from_bytes(bytes: &[u8]) -> Option<Trace> {
        let mut r = R {
            b: bytes.strip_prefix(MAGIC)?,
            at: 0,
        };
        let files = r.strs()?;
        let nfiles = files.len() as u32;
        let file = |r: &mut R<'_>| r.u32().filter(|f| *f < nfiles);
        let n = r.len()?;
        let mut roots = Vec::with_capacity(n);
        for _ in 0..n {
            roots.push(file(&mut r)?);
        }
        let n = r.len()?;
        let mut asks = Vec::with_capacity(n);
        for _ in 0..n {
            asks.push(match r.u8()? {
                tag @ (0 | 1) => {
                    let f = file(&mut r)?;
                    let source = if tag == 1 {
                        let b = r.b.get(r.at..r.at + 16)?;
                        r.at += 16;
                        Some(u128::from_le_bytes(b.try_into().ok()?))
                    } else {
                        None
                    };
                    Ask::Facts {
                        file: f,
                        source,
                        digest: r.u64()?,
                    }
                }
                2 => Ask::Targets {
                    file: file(&mut r)?,
                    at: r.u32()?,
                    answer: r.u64()?,
                },
                3 => Ask::Importers {
                    file: file(&mut r)?,
                    answer: r.u64()?,
                },
                4 => Ask::Resolve {
                    file: file(&mut r)?,
                    spec: r.str()?,
                    answer: r.u64()?,
                },
                5 => Ask::ModuleAt {
                    path: r.str()?,
                    answer: r.u64()?,
                },
                tag @ (6 | 7) => Ask::IsCode {
                    file: file(&mut r)?,
                    answer: tag == 7,
                },
                8 => Ask::Cwds { answer: r.u64()? },
                _ => return None,
            });
        }
        let n = r.len()?;
        let mut deps = Vec::with_capacity(n);
        for _ in 0..n {
            let f = file(&mut r)?;
            let line = r.u32()?;
            let kind = *RefKind::ALL.get(r.u8()? as usize)?;
            let sink_file = file(&mut r)?;
            let sink_line = r.u32()?;
            let sink_site = r.span()?;
            let value = match r.u8()? {
                0 => DepValue::Path(r.str()?),
                1 => DepValue::Below(r.str()?),
                2 => DepValue::Cwd(r.str()?),
                3 => DepValue::Spec(r.str()?),
                4 => DepValue::PerTestCwd(r.str()?),
                5 => DepValue::Open {
                    origin: *Origin::ALL.get(r.u8()? as usize)?,
                    detail: r.str()?,
                },
                _ => return None,
            };
            deps.push(Dep {
                file: f,
                line,
                kind,
                sink_file,
                sink_line,
                sink_site,
                value,
            });
        }
        let n = r.len()?;
        let mut closed = Vec::with_capacity(n);
        for _ in 0..n {
            closed.push(Closed {
                file: file(&mut r)?,
                at: r.span()?,
                site: r.span()?,
                open: r.u8()? == 1,
                specs: r.strs()?,
            });
        }
        let n = r.len()?;
        let mut tables = Vec::with_capacity(n);
        for _ in 0..n {
            tables.push(ClosedTable {
                file: file(&mut r)?,
                at: r.span()?,
                specs: r.strs()?,
            });
        }
        let bound = Bound {
            deps,
            closed,
            tables,
            files: r.u32()? as usize,
            exhausted: r.u8()? == 1,
        };
        (r.at == r.b.len()).then_some(Trace {
            files,
            roots,
            asks,
            bound,
        })
    }
}

fn kind_code(k: RefKind) -> u8 {
    RefKind::ALL.iter().position(|x| *x == k).unwrap_or(0) as u8
}
