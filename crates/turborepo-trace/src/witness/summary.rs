//! `turbo trace why --summary`: the constructs that select the tests of a
//! change, ranked by the tests each selects first.
//!
//! Every selected test is counted once, under the construct of its first
//! witness (the one `turbo trace affected` reports). Next to the count: how
//! many of those tests another kind of link selects too, so that removing the
//! construct alone does not unselect them. A construct that `turbo trace lint`
//! reports has the cost of that finding, as `lint` computes it.

use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
};

use anyhow::Result;
use serde::Serialize;
use turborepo_trace_graph::{Cause, EntryResult, GraphView, output};

use super::{Code, Input, UNKNOWN_TEST, WhyOpts, reason_of, source_line};
use crate::{ExitCode, api::Reason};

/// A construct and the tests it selects first.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    /// Selected tests whose first witness is this construct.
    tests: usize,
    /// Of those, the tests another kind of link selects too: removing this
    /// construct does not unselect them.
    also_selected: usize,
    /// Of those, the tests selected with no change at all.
    no_change: usize,
    reason: String,
    link: &'static str,
    construct: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<Code>,
    /// A test it selects, for `turbo trace why <test>`.
    example: String,
    /// The cost of the `turbo trace lint` finding at the construct.
    #[serde(skip_serializing_if = "Option::is_none")]
    lint: Option<LintCost>,
}

#[derive(Serialize)]
struct LintCost {
    reached: usize,
    exclusive: usize,
    bounded: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    tests: usize,
    selected: usize,
    no_change: usize,
    changes: Vec<String>,
    constructs: Vec<Row>,
}

/// `<file>:<line> <field>: ...`: the place of a record about a runner's
/// configuration (`crate::lint`, `at_configs`).
fn config_place(detail: &str) -> Option<(String, u32)> {
    let (at, rest) = detail.split_once(' ')?;
    let (file, line) = at.rsplit_once(':')?;
    rest.split_once(": ")?;
    Some((file.to_owned(), line.parse().ok().filter(|l| *l > 0)?))
}

impl Input<'_> {
    /// The construct of a witness as one line, and its place.
    fn construct(&self, e: &EntryResult) -> (String, Option<(String, u32)>) {
        let g = self.dg;
        let first = e.witness.first().map_or("", |&n| g.path(n));
        let place = self.link(e).2.into_iter().next();
        match (&e.cause, &e.uncertain) {
            (Some(Cause::Named { by, line, .. }), _) => {
                (format!("{first}:{line} names {by}"), place)
            }
            (Some(Cause::Scope { dir, pattern, .. }), _) => (
                format!("{first} reads {}", output::join_scope(dir, pattern)),
                place,
            ),
            // Whichever file lists it: a listing resolved for each test's working
            // directory is on the test itself.
            (Some(Cause::Membership { dir, pattern, .. }), _) => (
                format!("a listing of {}", output::join_scope(dir, pattern)),
                None,
            ),
            (Some(Cause::Probe { probe, .. }), _) => (format!("{first} probes {probe}"), None),
            (Some(Cause::Package { package }), _) => {
                (format!("package {}", g.package_info(*package).name), None)
            }
            (Some(Cause::Modified { path } | Cause::Deleted { path }), _) => {
                if first == path && e.witness.len() == 1 {
                    ("the test file itself".to_owned(), None)
                } else if first == path {
                    (path.clone(), None)
                } else {
                    (format!("{first} takes {path}"), None)
                }
            }
            (None, Some(u)) => match (&u.bound, u.reasons.first()) {
                // Whatever the load site: the package the loads reach, and why.
                (Some(b), _) => {
                    let reached = b.split_once("can reach ").map_or(b.as_str(), |x| x.1);
                    (format!("a bounded load can reach {reached}"), place)
                }
                (None, Some((kind, 0, detail))) if config_place(detail).is_some() => (
                    format!("{detail} ({})", kind.as_str()),
                    config_place(detail),
                ),
                (None, Some((kind, 0, detail))) => {
                    let detail: String = detail.chars().take(120).collect();
                    (
                        format!("{} {}: {detail}", g.path(u.node), kind.as_str()),
                        None,
                    )
                }
                (None, Some((kind, line, _))) => (
                    format!("{}:{line} {}", g.path(u.node), kind.as_str()),
                    place,
                ),
                (None, None) => (g.path(u.node).to_owned(), None),
            },
            (None, None) => (first.to_owned(), None),
        }
    }
}

pub(super) fn summary(i: &Input<'_>, opts: &WhyOpts, out: &mut dyn Write) -> Result<ExitCode> {
    let by = i.by_entry();
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    let mut count =
        |construct: String, test: &str, also: bool, floor: bool, new: &dyn Fn() -> Row| {
            let r = rows.entry(construct).or_insert_with(new);
            r.tests += 1;
            r.also_selected += usize::from(also);
            r.no_change += usize::from(floor);
            if r.example.is_empty() {
                r.example = test.to_owned();
            }
        };
    let blank = |reason: &Reason, link: &'static str, construct: &str| Row {
        tests: 0,
        also_selected: 0,
        no_change: 0,
        reason: serde_json::to_value(reason).unwrap_or_default()["__typename"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        link,
        construct: construct.to_owned(),
        class: None,
        code: None,
        example: String::new(),
        lint: None,
    };
    let mut places: HashMap<String, (String, u32)> = HashMap::new();
    for e in &i.report.entries {
        if !i.entries.contains(e.entry) {
            continue;
        }
        let found = i.witnesses(&by, e.entry);
        let (construct, place) = i.construct(e);
        if let Some(p) = place {
            places.entry(construct.clone()).or_insert(p);
        }
        count(
            construct.clone(),
            i.g.path(e.entry),
            found.len() > 1,
            found.iter().any(|w| w.1),
            &|| {
                let (link, class, _) = i.link(e);
                Row {
                    class,
                    ..blank(&reason_of(i.g, e), link, &construct)
                }
            },
        );
    }
    for n in i.unknown.iter() {
        let c = format!("unknown tests: {UNKNOWN_TEST}");
        count(c.clone(), i.g.path(n), false, true, &|| {
            blank(&Reason::UnknownTest, "unknown test", &c)
        });
    }
    for m in i.missing {
        let c = "test files that are not in the graph".to_owned();
        count(c.clone(), m, false, true, &|| {
            blank(&Reason::NotInGraph, "not in the graph", &c)
        });
    }
    let mut rows: Vec<Row> = rows.into_values().collect();
    rows.sort_by(|a, b| {
        b.tests
            .cmp(&a.tests)
            .then_with(|| a.construct.cmp(&b.construct))
    });
    let shown = if opts.json || opts.top == 0 {
        rows.len()
    } else {
        opts.top.min(rows.len())
    };
    // The lint cost and the source line of the constructs that are listed.
    let lint = (i.lint)();
    for r in rows.iter_mut().take(shown) {
        if let Some((file, line)) = places.get(&r.construct) {
            r.code = source_line(i.root, file, *line);
            r.lint = lint.get(&(file.clone(), *line)).map(|c| LintCost {
                reached: c.reached,
                exclusive: c.exclusive,
                bounded: c.bounded,
            });
        }
    }
    let s = Summary {
        tests: i.entries.len() + i.unknown.len() + i.missing.len(),
        selected: rows.iter().map(|r| r.tests).sum(),
        no_change: rows.iter().map(|r| r.no_change).sum(),
        changes: i.changes.iter().take(50).map(|c| c.path.clone()).collect(),
        constructs: rows,
    };
    if opts.json {
        serde_json::to_writer_pretty(&mut *out, &s)?;
        writeln!(out)?;
        return Ok(ExitCode::SUCCESS);
    }
    writeln!(
        out,
        "{} of {} tests selected by {}; {} of them with no change at all",
        s.selected,
        s.tests,
        i.change_text(),
        s.no_change
    )?;
    writeln!(
        out,
        "  tests: selected first through the construct; also: of those, selected through another \
         kind of link too (removing the construct does not unselect them)"
    )?;
    writeln!(out, "  tests   also  construct")?;
    for r in s.constructs.iter().take(shown) {
        writeln!(
            out,
            "  {:5}  {:5}  {} ({}): {}",
            r.tests, r.also_selected, r.link, r.reason, r.construct
        )?;
        if let Some(c) = &r.code {
            writeln!(out, "                {c}")?;
        }
        if let Some(c) = r.class {
            writeln!(out, "                class: {c}")?;
        }
        if let Some(l) = &r.lint {
            writeln!(
                out,
                "                turbo trace lint: {} test(s) reach it, {} only it{}",
                l.reached,
                l.exclusive,
                if l.bounded { " (a bounded load)" } else { "" }
            )?;
        }
        writeln!(
            out,
            "                for example: turbo trace why {}",
            r.example
        )?;
    }
    let rest = &s.constructs[shown..];
    if !rest.is_empty() {
        writeln!(
            out,
            "  … and {} more construct(s) that select {} test(s) first (--top 0 lists them)",
            rest.len(),
            rest.iter().map(|r| r.tests).sum::<usize>()
        )?;
    }
    Ok(ExitCode::SUCCESS)
}
