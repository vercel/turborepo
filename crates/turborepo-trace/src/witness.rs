//! `turbo trace why`: why a test is selected, as chains from the change to the
//! test.
//!
//! The selection keeps one result per selected entry: the first of its
//! mechanisms that selects it (a change that reaches it through the graph, a
//! changed task, a name, a bounded load, an open uncertainty). `why` prints
//! that result link by link: what changed, the construct that makes the change
//! matter (with its source line), the kind of link, and every hop to the test.
//!
//! A test is often selected through more than one mechanism, and removing the
//! construct of the first one then changes nothing. So every mechanism is also
//! run by itself ([`Alone`]: the same code, into an empty report), and so is
//! the selection without any change. What each of them selects is a witness of
//! its own. One witness per mechanism is known, as in the selection itself:
//! the list is a lower bound.

use std::{collections::HashMap, io::Write, path::Path};

use anyhow::Result;
use serde::Serialize;
use turborepo_trace_graph::{
    AffectedReport, Cause, Change, EntryResult, GraphView, NodeId, NodeSet, Origin,
    UncertaintyKind, output,
};
use turborepo_trace_store::Graph;

use crate::{ExitCode, api::Reason, lint::Derived};

/// What `why` prints.
#[derive(Clone, Debug, Default)]
pub struct WhyOpts {
    /// The test to explain (as `explain` takes an entry).
    pub test: Option<String>,
    /// JSON instead of text.
    pub json: bool,
}

/// What each mechanism of the selection selects by itself: one report per
/// mechanism, filled by the same call that adds to the selection, and the
/// selection without a change. Off (nothing is kept, nothing runs twice)
/// unless `why` asked.
#[derive(Default)]
pub(crate) struct Alone {
    on: bool,
    layers: Vec<AffectedReport>,
    /// Index into `layers` of the selection without a change.
    floor: Option<usize>,
}

impl Alone {
    pub(crate) fn new(on: bool) -> Self {
        Alone {
            on,
            ..Default::default()
        }
    }

    /// An empty report for one mechanism to fill, shaped like `like`.
    pub(crate) fn layer(&mut self, like: &AffectedReport) -> Option<&mut AffectedReport> {
        if !self.on {
            return None;
        }
        self.layers.push(AffectedReport {
            policy: like.policy,
            entry_count: like.entry_count,
            entries: Vec::new(),
            seed_count: 0,
            reached_count: 0,
            unmatched: Vec::new(),
        });
        self.layers.last_mut()
    }

    /// The selection without a change, when `why` asked: `f` evaluates it.
    pub(crate) fn no_change(&mut self, f: impl FnOnce() -> AffectedReport) {
        if self.on {
            self.floor = Some(self.layers.len());
            self.layers.push(f());
        }
    }
}

/// What `why` reads of an `affected` run.
pub(crate) struct Input<'a> {
    pub root: &'a Path,
    pub g: &'a Graph,
    pub dg: &'a Derived<'a>,
    /// The selection.
    pub report: &'a AffectedReport,
    pub alone: &'a Alone,
    /// The entries of the selection that are no graph node.
    pub missing: &'a [String],
    /// Files that look like tests and are no entry: they always run.
    pub unknown: &'a NodeSet,
    pub changes: &'a [Change],
}

/// A line of source.
#[derive(Serialize)]
struct Code {
    file: String,
    line: u32,
    text: String,
}

impl std::fmt::Display for Code {
    /// `src/a.js:3: <text>`; without the line when there is none.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            0 => write!(f, "{}: {}", self.file, self.text),
            line => write!(f, "{}:{line}: {}", self.file, self.text),
        }
    }
}

/// A file of a chain, and the reference that leads to the file before it.
#[derive(Serialize)]
struct Hop {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    via: Option<String>,
}

/// One way a test is selected.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Witness {
    /// As `turbo trace affected --json` gives it.
    reason: Reason,
    /// The kind of link between the change and the code.
    link: &'static str,
    /// It selects the test whatever changes, also when nothing does.
    no_change: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed: Option<String>,
    /// The sentence `--explain` prints for it.
    description: String,
    /// What makes the construct broad, where that is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<&'static str>,
    code: Vec<Code>,
    /// From the change's side to the test.
    chain: Vec<Hop>,
    /// The file to ask about next: the witness holds because this file
    /// depends on the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<String>,
}

const ROOT_NAME: &str = "it names the repository root: every path of the repository is below it";
const CWD: &str = "it reads relative to the working directory";
const ROOT_LISTING: &str = "it lists or reads below the repository root";
const DECLARED: &str =
    "a load with a computed target, bounded only by the packages its package declares";
const ENV: &str = "an environment variable chooses the path";

/// What tells two witnesses of one entry apart: where the chain starts and by
/// which kind of link.
fn key(e: &EntryResult) -> (u32, u8, u32) {
    let first = e.witness.first().map_or(u32::MAX, |n| n.0);
    match (&e.cause, &e.uncertain) {
        (Some(Cause::Named { line, .. }), _) => (first, 1, *line),
        (Some(_), _) => (first, 0, 0),
        (None, Some(u)) => (first, 2 + u8::from(u.bound.is_some()), 0),
        (None, None) => (first, 4, 0),
    }
}

fn source_line(root: &Path, file: &str, line: u32) -> Option<Code> {
    let text = std::fs::read_to_string(root.join(file)).ok()?;
    let text = text.lines().nth(line.checked_sub(1)? as usize)?.trim();
    Some(Code {
        file: file.to_owned(),
        line,
        text: text.chars().take(140).collect(),
    })
}

fn reason_of(g: &Graph, e: &EntryResult) -> Reason {
    let (class, via) = crate::selection::reason_of(g, e);
    crate::api::reason(class, via.as_deref())
}

/// `FileChanged docs/a.md`.
fn reason_text(r: &Reason) -> String {
    let v = serde_json::to_value(r).unwrap_or_default();
    let name = v["__typename"].as_str().unwrap_or("Uncertain").to_owned();
    let field = v.as_object().and_then(|o| {
        o.iter()
            .find(|(k, _)| *k != "__typename")
            .and_then(|(_, v)| v.as_str().map(str::to_owned))
    });
    match field.filter(|f| !f.is_empty()) {
        Some(f) => format!("{name} {f}"),
        None => name,
    }
}

impl Input<'_> {
    fn is_task(&self, n: NodeId) -> bool {
        turborepo_trace_store::tasks::is_pseudo_lang(self.g.lang(n))
    }

    /// The kind of link of `e`, what makes it broad, and the lines of its
    /// construct (file and line; 0: no line).
    fn link(&self, e: &EntryResult) -> (&'static str, Option<&'static str>, Vec<(String, u32)>) {
        let g = self.dg;
        let first = e.witness.first().copied();
        let at = |line: u32| first.map(|n| (g.path(n).to_owned(), line));
        let broad = |dir: &str| dir.is_empty().then_some(ROOT_LISTING);
        match (&e.cause, &e.uncertain) {
            (
                Some(Cause::Named {
                    by,
                    line,
                    site,
                    site_line,
                    ..
                }),
                _,
            ) => {
                let class = if by.contains("(the repository root)") {
                    Some(ROOT_NAME)
                } else if by.starts_with("the working directory") {
                    Some(CWD)
                } else {
                    None
                };
                let lines = at(*line).into_iter().chain([(site.clone(), *site_line)]);
                ("name reference", class, lines.collect())
            }
            (Some(Cause::Scope { dir, pattern, .. }), _) => {
                let line = first.and_then(|n| {
                    g.uncertainties(n)
                        .find(|u| {
                            u.scope.is_some_and(|s| {
                                s.dir == dir.as_str() && s.pattern == pattern.as_str()
                            })
                        })
                        .map(|u| u.line)
                });
                (
                    "scope membership",
                    broad(dir),
                    at(line.unwrap_or(0)).into_iter().collect(),
                )
            }
            (Some(Cause::Membership { dir, .. }), _) => ("directory listing", broad(dir), vec![]),
            (Some(Cause::Probe { .. }), _) => ("probed path", None, vec![]),
            (Some(Cause::Package { .. }), _) => ("package dependency", None, vec![]),
            (Some(Cause::Modified { path } | Cause::Deleted { path }), _) => {
                let own = first.is_some_and(|n| g.path(n) == path);
                let link = match first {
                    Some(n) if self.is_task(n) => "task definition",
                    Some(n) if own && n == e.entry => "the test itself",
                    _ if e.witness.iter().any(|&n| self.is_task(n)) => "task input",
                    Some(_) if !own => "resolution input",
                    _ => match e
                        .witness
                        .get(1)
                        .and_then(|&reader| self.edge(reader, first?))
                    {
                        Some((kind, _)) if kind.starts_with("fs_") || kind == "asset" => "asset",
                        _ => "import",
                    },
                };
                (link, None, vec![])
            }
            (None, Some(u)) => {
                let site = g.path(u.node).to_owned();
                let mut lines: Vec<(String, u32)> = Vec::new();
                for r in &u.reasons {
                    if r.1 > 0 && lines.len() < 2 && !lines.iter().any(|l| l.1 == r.1) {
                        lines.push((site.clone(), r.1));
                    }
                }
                let origin = |o: Origin| {
                    g.uncertainties(u.node)
                        .any(|x| x.origin == o && u.reasons.iter().any(|r| r.1 == x.line))
                };
                let kind = |k: UncertaintyKind| u.reasons.iter().any(|r| r.0 == k);
                let (link, class) = if u.bound.is_some() {
                    ("bounded load", Some(DECLARED))
                } else if self.is_task(u.node) {
                    ("task with an open uncertainty", None)
                } else if kind(UncertaintyKind::CwdDependent) || origin(Origin::Cwd) {
                    ("open load or read", Some(CWD))
                } else if kind(UncertaintyKind::EnvDependent) || origin(Origin::Env) {
                    ("open load or read", Some(ENV))
                } else {
                    ("open load or read", None)
                };
                (link, class, lines)
            }
            (None, None) => ("import", None, vec![]),
        }
    }

    /// The reference of `reader` that leads to `target`: its kind and how it is
    /// written.
    fn edge(&self, reader: NodeId, target: NodeId) -> Option<(&'static str, String)> {
        let e = self.dg.out_edges(reader).find(|e| e.to == Some(target))?;
        let spec = e.specifier.filter(|s| !s.is_empty());
        let spec = spec.map(|s| format!(" {s:?}")).unwrap_or_default();
        let line = match e.line {
            0 => String::new(),
            l => format!(" at {}:{l}", self.dg.path(reader)),
        };
        Some((
            e.kind().as_str(),
            format!("{}{spec}{line}", e.kind().as_str()),
        ))
    }

    fn witness(&self, e: &EntryResult, no_change: bool) -> Witness {
        let g = self.dg;
        let (link, class, lines) = self.link(e);
        let description = match (&e.cause, &e.uncertain) {
            // Each kind once, however many records the site has.
            (None, Some(u)) => {
                let mut kinds: Vec<&str> = Vec::new();
                for r in &u.reasons {
                    if !kinds.contains(&r.0.as_str()) {
                        kinds.push(r.0.as_str());
                    }
                }
                let bound = u.bound.as_ref().map(|b| format!("; {b}"));
                format!(
                    "uncertain ({}{})",
                    kinds.join(", "),
                    bound.unwrap_or_default()
                )
            }
            _ => output::affected_why(g, e)
                .trim_end_matches(" at")
                .trim_end_matches(':')
                .to_owned(),
        };
        // A bound that holds because another file depends on the change: that file
        // is the next thing to ask about.
        let next = e
            .uncertain
            .as_ref()
            .and_then(|u| u.bound.as_deref())
            .and_then(|b| b.split_once(" depends on the change)"))
            .and_then(|(head, _)| head.rsplit_once('('))
            .map(|(_, file)| file.to_owned())
            .filter(|file| e.witness.last().is_none_or(|&n| g.path(n) != file));
        let changed = match &e.cause {
            Some(
                Cause::Modified { path }
                | Cause::Deleted { path }
                | Cause::Membership { path, .. }
                | Cause::Scope { path, .. }
                | Cause::Named { path, .. },
            ) => Some(path.clone()),
            Some(Cause::Probe { added, .. }) => Some(added.clone()),
            _ => None,
        };
        let mut chain: Vec<Hop> = Vec::new();
        for (i, &n) in e.witness.iter().enumerate() {
            chain.push(Hop {
                file: g.path(n).to_owned(),
                via: i
                    .checked_sub(1)
                    .and_then(|p| self.edge(n, e.witness[p]))
                    .map(|x| x.1),
            });
        }
        // A site without a line of code (a record of a runner's configuration, of a
        // task): its detail is what there is to read.
        let mut code: Vec<Code> = Vec::new();
        for (i, (f, l)) in lines.iter().enumerate() {
            if !lines[..i].contains(&(f.clone(), *l)) {
                code.extend(source_line(self.root, f, *l));
            }
        }
        if let (true, Some(u)) = (code.is_empty(), &e.uncertain) {
            code.extend(u.reasons.iter().take(1).map(|r| Code {
                file: g.path(u.node).to_owned(),
                line: r.1,
                text: r.2.chars().take(200).collect(),
            }));
        }
        Witness {
            reason: reason_of(self.g, e),
            link,
            no_change,
            changed,
            description,
            class,
            code,
            chain,
            next,
        }
    }

    /// Every known witness of the entry `n`, the selection's own first. The
    /// flag: it needs no change.
    fn witnesses<'a>(
        &'a self,
        by: &[HashMap<u32, &'a EntryResult>],
        n: NodeId,
    ) -> Vec<(&'a EntryResult, bool)> {
        let floor = self
            .alone
            .floor
            .and_then(|f| by.get(f + 1)?.get(&n.0))
            .map(|e| key(e));
        let mut out: Vec<(&EntryResult, bool)> = Vec::new();
        for e in by.iter().filter_map(|m| m.get(&n.0)) {
            if !out.iter().any(|o| key(o.0) == key(e)) {
                out.push((e, floor == Some(key(e))));
            }
        }
        out
    }

    /// The selection and each mechanism's own report, by entry.
    fn by_entry(&self) -> Vec<HashMap<u32, &EntryResult>> {
        std::iter::once(self.report)
            .chain(&self.alone.layers)
            .map(|r| r.entries.iter().map(|e| (e.entry.0, e)).collect())
            .collect()
    }

    fn change_text(&self) -> String {
        match self.changes {
            [] => "no change".to_owned(),
            [c] => format!("the change to {} ({})", c.path, c.kind.as_str()),
            cs => format!("{} changed paths", cs.len()),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Explanation {
    test: String,
    selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<Reason>,
    /// Selected whatever changes, also when nothing does.
    no_change: bool,
    witnesses: Vec<Witness>,
}

const UNKNOWN_TEST: &str =
    "it looks like a test file, and no configuration of the test frameworks asked for lists it";

/// `why <test>`: exit code 1 when the change does not select the test.
fn explain(i: &Input<'_>, rel: &str, json: bool, out: &mut dyn Write) -> Result<ExitCode> {
    let node = i.g.node_by_path(rel);
    let by = i.by_entry();
    let mut x = Explanation {
        test: rel.to_owned(),
        selected: true,
        reason: None,
        no_change: false,
        witnesses: Vec::new(),
    };
    let mut note = String::new();
    if node.is_some_and(|n| i.unknown.contains(n)) {
        (x.reason, x.no_change) = (Some(Reason::UnknownTest), true);
        note = format!("an unknown test, which always runs: {UNKNOWN_TEST}");
    } else if let Some(n) = node.filter(|n| by[0].contains_key(&n.0)) {
        let found = i.witnesses(&by, n);
        x.no_change = found.iter().any(|w| w.1);
        x.witnesses = found
            .iter()
            .map(|(e, floor)| i.witness(e, *floor))
            .collect();
        x.reason = x.witnesses.first().map(|w| w.reason.clone());
    } else if i.missing.iter().any(|m| m == rel) {
        x.reason = Some(Reason::NotInGraph);
        note = "not in the graph (not a traced source file), so it always runs".to_owned();
    } else {
        x.selected = false;
        if node.is_none() {
            note = "it is not a file of the graph".to_owned();
        }
    }
    let code = ExitCode(u8::from(!x.selected));
    if json {
        serde_json::to_writer_pretty(&mut *out, &x)?;
        writeln!(out)?;
        return Ok(code);
    }
    match &x.reason {
        Some(r) => writeln!(
            out,
            "{rel}: selected by {} ({}){}",
            i.change_text(),
            reason_text(r),
            if x.no_change {
                "; it is selected with no change at all too"
            } else {
                ""
            }
        )?,
        None => writeln!(
            out,
            "{rel}: not selected by this change ({})",
            i.change_text()
        )?,
    }
    if !note.is_empty() {
        writeln!(out, "  {note}")?;
    }
    let n = x.witnesses.len();
    if n > 1 {
        writeln!(
            out,
            "  selected through {n} independent witnesses: each selects the test by itself, so \
             removing one does not unselect it. The first is the one `turbo trace affected` \
             reports; also:{}",
            x.witnesses[1..]
                .iter()
                .map(|w| format!(" {}", w.link))
                .collect::<Vec<_>>()
                .join(",")
        )?;
    }
    for (k, w) in x.witnesses.iter().enumerate() {
        writeln!(
            out,
            "  {}. {}{}: {}",
            k + 1,
            w.link,
            if w.no_change {
                " (needs no change)"
            } else {
                ""
            },
            w.description
        )?;
        if let Some(c) = w.class {
            writeln!(out, "     class: {c}")?;
        }
        for c in &w.code {
            writeln!(out, "     {c}")?;
        }
        if let Some(file) = &w.next {
            writeln!(
                out,
                "     next: turbo trace why {file}  (why that file depends on the change)"
            )?;
        }
        for (j, h) in w.chain.iter().enumerate() {
            let via = h
                .via
                .as_ref()
                .map(|v| format!("  ({v})"))
                .unwrap_or_default();
            let arrow = if j == 0 { "   " } else { "-> " };
            writeln!(out, "     {arrow}{}{via}", h.file)?;
        }
    }
    if n == 1 {
        writeln!(
            out,
            "  no other kind of link selects it (one witness per kind is known: the same kind may \
             hold more constructs)"
        )?;
    }
    Ok(code)
}

/// Writes what `opts` asks for. `test`: the test's path relative to the
/// repository root.
pub(crate) fn write(
    i: &Input<'_>,
    opts: &WhyOpts,
    test: Option<&str>,
    out: &mut dyn Write,
) -> Result<ExitCode> {
    match test {
        Some(rel) => explain(i, rel, opts.json, out),
        None => anyhow::bail!("no test to explain"),
    }
}
