//! The output formats of `turbo trace lint`: `text`, `json` and `github`.

use std::io::{self, Write};

use serde_json::json;

use super::{Cost, Item, Report, Severity, Stale, Status};

/// `1 test`, `2 tests`.
fn tests(n: usize) -> String {
    format!("{n} test{}", if n == 1 { "" } else { "s" })
}

/// `holds N tests on the floor (M only this site)`; for a site that a bound
/// from outside keeps off the floor, `reached by N tests, …`.
fn cost_line(c: Cost) -> String {
    if c.reached == 0 {
        "holds no test on the floor (no test reaches this file)".to_owned()
    } else if c.bounded {
        format!(
            "reached by {}, each selected when the change affects a package the load can reach \
             ({} reach no other unconstrained site)",
            tests(c.reached),
            c.exclusive
        )
    } else {
        format!(
            "holds {} on the floor ({} only this site)",
            tests(c.reached),
            c.exclusive
        )
    }
}

/// The last line of the text and github formats.
fn summary(r: &Report) -> String {
    let mut s = format!(
        "{} new, {} baselined, {} suppressed, {} stale",
        r.count("new"),
        r.count("baselined"),
        r.count("suppressed"),
        r.stale.len()
    );
    if let Some((floor, entries)) = r.floor {
        s.push_str(&format!("; {floor} of {} on the floor", tests(entries)));
    }
    s
}

fn stale_message(s: &Stale) -> String {
    let n = match s.entry.count {
        1 => String::new(),
        n => format!(" ({n} times)"),
    };
    format!("{}  {}  {}{n}", s.entry.file, s.entry.rule, s.entry.key)
}

/// One block per new finding.
fn block(i: &Item, out: &mut dyn Write) -> io::Result<()> {
    let f = &i.finding;
    writeln!(out, "{}:{}  {}  {}", f.file, f.line, f.rule, f.message)?;
    if let Some(c) = f.cost {
        writeln!(out, "  {}", cost_line(c))?;
    }
    for (k, line) in f.fix.lines().enumerate() {
        writeln!(out, "  {}{line}", if k == 0 { "fix: " } else { "     " })?;
    }
    writeln!(out)
}

/// The readable report: new findings (errors, then warnings), stale baseline
/// entries, the baselined findings as a list by cost, and the summary line.
pub fn text(r: &Report, out: &mut dyn Write) -> io::Result<()> {
    let new = |sev: Severity| {
        r.items
            .iter()
            .filter(move |i| i.status == Status::New && i.severity == sev)
    };
    for i in new(Severity::Error) {
        block(i, out)?;
    }
    if new(Severity::Warn).next().is_some() {
        writeln!(out, "warnings (they do not fail the check):")?;
        for i in new(Severity::Warn) {
            block(i, out)?;
        }
    }
    if !r.stale.is_empty() {
        writeln!(
            out,
            "stale baseline entries: {} lists findings that are gone. Update the baseline and \
             commit the file.",
            r.baseline.as_deref().unwrap_or("the baseline")
        )?;
        for s in &r.stale {
            writeln!(
                out,
                "  {}{}",
                stale_message(s),
                if s.severity == Severity::Error {
                    ""
                } else {
                    "  (warning)"
                }
            )?;
        }
        writeln!(out)?;
    }
    let mut baselined: Vec<&Item> = r
        .items
        .iter()
        .filter(|i| i.status == Status::Baselined)
        .collect();
    if !baselined.is_empty() {
        // Most tests first; the sort is stable, so ties stay in file and line order.
        baselined.sort_by_key(|i| std::cmp::Reverse(i.finding.cost.map_or(0, |c| c.reached)));
        writeln!(
            out,
            "baselined: {} finding(s) accepted by {}, by the tests they hold on the floor",
            baselined.len(),
            r.baseline.as_deref().unwrap_or("the baseline")
        )?;
        for i in baselined {
            let f = &i.finding;
            let held = f.cost.map_or("-".to_owned(), |c| c.reached.to_string());
            writeln!(
                out,
                "{held:>8}  {}:{}  {}  {}",
                f.file, f.line, f.rule, f.message
            )?;
        }
        writeln!(out)?;
    }
    writeln!(out, "{}", summary(r))
}

/// The report as JSON:
///
/// ```json
/// {
///   "version": 1,
///   "summary": { "ok": false, "new": 1, "errors": 1, "warnings": 0, "baselined": 0,
///                "suppressed": 0, "stale": 0, "tests": 6, "floor": 3 },
///   "findings": [ { "rule": "…", "severity": "error", "status": "new", "file": "…",
///                   "line": 3, "key": "…", "message": "…",
///                   "cost": { "reached": 3, "exclusive": 2 }, "fix": "…" } ],
///   "stale": [ { "rule": "…", "file": "…", "key": "…", "count": 1 } ]
/// }
/// ```
///
/// Findings are ordered by file, line, rule and key. A suppressed finding has
/// `reason`. `cost` is null for a rule that does not compute it; `tests` and
/// `floor` are null when no rule computed the floor.
pub fn json(r: &Report, out: &mut dyn Write) -> io::Result<()> {
    let new = |sev: Severity| {
        r.items
            .iter()
            .filter(|i| i.status == Status::New && i.severity == sev)
            .count()
    };
    let findings: Vec<serde_json::Value> = r
        .items
        .iter()
        .map(|i| {
            let f = &i.finding;
            let mut v = json!({
                "rule": f.rule,
                "severity": i.severity.as_str(),
                "status": i.status.as_str(),
                "file": f.file,
                "line": f.line,
                "key": f.key,
                "message": f.message,
                "cost": f.cost.map(|c| {
                    let mut v = json!({ "reached": c.reached, "exclusive": c.exclusive });
                    if c.bounded {
                        v["bounded"] = json!(true);
                    }
                    v
                }),
                "fix": f.fix,
            });
            if let Status::Suppressed(reason) = &i.status {
                v["reason"] = json!(reason);
            }
            v
        })
        .collect();
    let stale: Vec<serde_json::Value> = r
        .stale
        .iter()
        .map(|s| {
            json!({
                "rule": s.entry.rule,
                "file": s.entry.file,
                "key": s.entry.key,
                "count": s.entry.count,
                "severity": s.severity.as_str(),
            })
        })
        .collect();
    let v = json!({
        "version": 1,
        "summary": {
            "ok": !r.failed(),
            "new": r.count("new"),
            "errors": new(Severity::Error),
            "warnings": new(Severity::Warn),
            "baselined": r.count("baselined"),
            "suppressed": r.count("suppressed"),
            "stale": r.stale.len(),
            "tests": r.floor.map(|(_, entries)| entries),
            "floor": r.floor.map(|(floor, _)| floor),
        },
        "findings": findings,
        "stale": stale,
    });
    serde_json::to_writer_pretty(&mut *out, &v)?;
    writeln!(out)
}

/// Escapes the message of a workflow command.
fn gh_data(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escapes a property value of a workflow command.
fn gh_property(s: &str) -> String {
    gh_data(s).replace(':', "%3A").replace(',', "%2C")
}

fn gh_command(sev: Severity) -> &'static str {
    if sev == Severity::Error {
        "error"
    } else {
        "warning"
    }
}

/// GitHub Actions workflow commands: one annotation per new finding and per
/// stale entry, then the summary line.
///
/// ```text
/// ::error file=src/load.js,line=3,title=turbo trace unconstrained-load::<message>. Holds 3 tests. Fix: <first fix line>
/// ```
pub fn github(r: &Report, out: &mut dyn Write) -> io::Result<()> {
    for i in r.items.iter().filter(|i| i.status == Status::New) {
        let f = &i.finding;
        let mut props = format!("file={}", gh_property(&f.file));
        if f.line > 0 {
            props.push_str(&format!(",line={}", f.line));
        }
        props.push_str(&format!(
            ",title={}",
            gh_property(&format!("turbo trace {}", f.rule))
        ));
        let mut msg = format!("{}.", f.message);
        match f.cost {
            Some(c) if c.bounded => msg.push_str(&format!(" Reached by {}.", tests(c.reached))),
            Some(c) => msg.push_str(&format!(" Holds {}.", tests(c.reached))),
            None => {}
        }
        if let Some(fix) = f.fix.lines().next() {
            msg.push_str(&format!(" Fix: {fix}"));
        }
        writeln!(
            out,
            "::{} {props}::{}",
            gh_command(i.severity),
            gh_data(&msg)
        )?;
    }
    for s in &r.stale {
        let file = r.baseline.as_deref().unwrap_or(&s.entry.file);
        writeln!(
            out,
            "::{} file={},title={}::{}",
            gh_command(s.severity),
            gh_property(file),
            gh_property("turbo trace stale baseline entry"),
            gh_data(&format!(
                "The baseline lists a finding that is gone: {}. Update the baseline and commit \
                 the file.",
                stale_message(s)
            ))
        )?;
    }
    writeln!(out, "{}", summary(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_workflow_commands() {
        assert_eq!(gh_data("50% of a\r\nb: c, d"), "50%25 of a%0D%0Ab: c, d");
        assert_eq!(gh_property("a:b,c%\n"), "a%3Ab%2Cc%25%0A");
    }

    #[test]
    fn counts_tests() {
        assert_eq!(tests(1), "1 test");
        assert_eq!(tests(0), "0 tests");
        assert_eq!(
            cost_line(Cost {
                reached: 3,
                exclusive: 2,
                bounded: false
            }),
            "holds 3 tests on the floor (2 only this site)"
        );
        assert_eq!(
            cost_line(Cost {
                reached: 3,
                exclusive: 1,
                bounded: true
            }),
            "reached by 3 tests, each selected when the change affects a package the load can \
             reach (1 reach no other unconstrained site)"
        );
        assert_eq!(
            cost_line(Cost {
                reached: 0,
                exclusive: 0,
                bounded: false
            }),
            "holds no test on the floor (no test reaches this file)"
        );
    }
}
