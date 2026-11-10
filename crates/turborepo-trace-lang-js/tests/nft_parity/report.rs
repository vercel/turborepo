//! Classification, the summary table, `report.json` and the baseline check.

use std::{collections::BTreeSet, fmt::Write as _, path::Path};

use serde_json::{Value, json};

/// Outcome of one case (or one sub-check of a case), worst last.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Not run.
    Skip,
    /// Same file set as nft.
    Pass,
    /// Every nft file plus extra ones.
    Superset,
    /// At least one nft file is missing (extra files may also be present).
    Missing,
    /// The trace failed, panicked or was inconsistent between cold and warm
    /// runs.
    Error,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Skip => "SKIP",
            Status::Pass => "PASS",
            Status::Superset => "SUPERSET",
            Status::Missing => "MISSING",
            Status::Error => "ERROR",
        }
    }

    pub fn classify(missing: &BTreeSet<String>, extra: &BTreeSet<String>) -> Status {
        match (missing.is_empty(), extra.is_empty()) {
            (true, true) => Status::Pass,
            (true, false) => Status::Superset,
            (false, _) => Status::Missing,
        }
    }
}

/// A comparison of one trace against one expected list.
#[derive(Debug, Default, Clone)]
pub struct Check {
    /// `cold`, `cached`, or `closure:<entry>` (multi-input).
    pub label: String,
    pub missing: BTreeSet<String>,
    pub extra: BTreeSet<String>,
}

impl Check {
    pub fn new(label: &str, expected: &BTreeSet<String>, actual: &BTreeSet<String>) -> Check {
        Check {
            label: label.to_owned(),
            missing: expected.difference(actual).cloned().collect(),
            extra: actual.difference(expected).cloned().collect(),
        }
    }

    pub fn status(&self) -> Status {
        Status::classify(&self.missing, &self.extra)
    }
}

/// Result of one case.
#[derive(Debug, Clone)]
pub struct CaseResult {
    pub name: String,
    pub status: Status,
    pub inputs: Vec<String>,
    pub test_opts: Value,
    pub emulated: Vec<String>,
    pub unsupported: Vec<String>,
    pub npm_packages: BTreeSet<String>,
    pub checks: Vec<Check>,
    /// Files outside the base (nft drops them from `fileList`; we report them).
    pub outside_base: BTreeSet<String>,
    pub uncertainties: usize,
    pub error: Option<String>,
    pub skip: Option<String>,
    pub ms: f64,
}

impl CaseResult {
    /// The first check, which defines missing/extra for the table.
    pub fn cold(&self) -> Option<&Check> {
        self.checks.first()
    }

    fn json(&self) -> Value {
        json!({
            "name": self.name,
            "status": self.status.as_str(),
            "inputs": self.inputs,
            "test_opts": self.test_opts,
            "emulated_options": self.emulated,
            "unsupported_options": self.unsupported,
            "requires_npm": self.npm_packages,
            "missing": self.cold().map(|c| c.missing.clone()).unwrap_or_default(),
            "extra": self.cold().map(|c| c.extra.clone()).unwrap_or_default(),
            "checks": self.checks.iter().map(|c| json!({
                "label": c.label,
                "status": c.status().as_str(),
                "missing": c.missing,
                "extra": c.extra,
            })).collect::<Vec<_>>(),
            "outside_base": self.outside_base,
            "uncertainties": self.uncertainties,
            "error": self.error,
            "skip_reason": self.skip,
            "ms": (self.ms * 1000.0).round() / 1000.0,
        })
    }
}

pub fn counts(results: &[CaseResult]) -> [(Status, usize); 5] {
    [
        Status::Pass,
        Status::Superset,
        Status::Missing,
        Status::Error,
        Status::Skip,
    ]
    .map(|s| (s, results.iter().filter(|r| r.status == s).count()))
}

fn short_list(set: &BTreeSet<String>, case: &str, max: usize) -> String {
    let prefix = format!("test/unit/{case}/");
    let mut items: Vec<String> = set
        .iter()
        .take(max)
        .map(|s| s.strip_prefix(&prefix).unwrap_or(s).to_owned())
        .collect();
    if set.len() > max {
        items.push(format!("... +{}", set.len() - max));
    }
    items.join(", ")
}

/// Human-readable summary table (Markdown).
pub fn table(results: &[CaseResult], header: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{header}\n");
    let _ = writeln!(s, "| status | count |\n|---|---|");
    for (st, n) in counts(results) {
        let _ = writeln!(s, "| {} | {n} |", st.as_str());
    }
    let _ = writeln!(
        s,
        "\n| case | status | missing | extra | notes |\n|---|---|---|---|---|"
    );
    for r in results {
        let (missing, extra) = match r.cold() {
            Some(c) => (
                short_list(&c.missing, &r.name, 4),
                short_list(&c.extra, &r.name, 4),
            ),
            None => Default::default(),
        };
        let mut notes: Vec<String> = Vec::new();
        if let Some(e) = &r.error {
            notes.push(format!("error: {e}"));
        }
        if let Some(k) = &r.skip {
            notes.push(format!("skip: {k}"));
        }
        let cold = r.cold();
        for c in r.checks.iter().skip(1) {
            let same_as_cold = cold.is_some_and(|k| k.missing == c.missing && k.extra == c.extra);
            if c.status() != Status::Pass && !same_as_cold {
                notes.push(format!(
                    "{} {}: -[{}] +[{}]",
                    c.label,
                    c.status().as_str(),
                    short_list(&c.missing, &r.name, 3),
                    short_list(&c.extra, &r.name, 3)
                ));
            }
        }
        if !r.unsupported.is_empty() {
            notes.push(format!("unsupported: {}", r.unsupported.join(" ")));
        }
        if !r.emulated.is_empty() {
            notes.push(format!("emulated: {}", r.emulated.join(" ")));
        }
        if !r.outside_base.is_empty() {
            notes.push(format!("outside base: {}", r.outside_base.len()));
        }
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} |",
            r.name,
            r.status.as_str(),
            missing.replace('|', "\\|"),
            extra.replace('|', "\\|"),
            notes.join("; ").replace('|', "\\|")
        );
    }
    s
}

pub fn report_json(results: &[CaseResult], meta: Value) -> Value {
    let summary: serde_json::Map<String, Value> = counts(results)
        .iter()
        .map(|(s, n)| (s.as_str().to_owned(), json!(n)))
        .collect();
    json!({
        "meta": meta,
        "summary": summary,
        "cases": results.iter().map(CaseResult::json).collect::<Vec<_>>(),
    })
}

/// Baseline: the set of cases that passed when it was recorded.
pub struct Baseline {
    pub pass: BTreeSet<String>,
}

impl Baseline {
    pub fn load(path: &Path) -> Option<Baseline> {
        let text = std::fs::read_to_string(path).ok()?;
        let v: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{}: invalid JSON: {e}", path.display()));
        let pass = v["pass"]
            .as_array()
            .unwrap_or_else(|| panic!("{}: missing `pass` array", path.display()))
            .iter()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect();
        Some(Baseline { pass })
    }

    /// Cases that pass in the baseline but now run and do not pass.
    pub fn regressions(&self, results: &[CaseResult]) -> Vec<String> {
        results
            .iter()
            .filter(|r| self.pass.contains(&r.name))
            .filter(|r| r.status != Status::Pass && r.status != Status::Skip)
            .map(|r| format!("{} is now {}", r.name, r.status.as_str()))
            .collect()
    }

    /// Cases that pass now but are not in the baseline.
    pub fn improvements(&self, results: &[CaseResult]) -> Vec<String> {
        results
            .iter()
            .filter(|r| r.status == Status::Pass && !self.pass.contains(&r.name))
            .map(|r| r.name.clone())
            .collect()
    }

    /// The updated pass set: current passes, plus baseline passes that were
    /// skipped this run (for example because nft's npm dependencies were
    /// not installed).
    pub fn updated(old: Option<&Baseline>, results: &[CaseResult]) -> BTreeSet<String> {
        let mut pass: BTreeSet<String> = results
            .iter()
            .filter(|r| r.status == Status::Pass)
            .map(|r| r.name.clone())
            .collect();
        if let Some(old) = old {
            pass.extend(
                results
                    .iter()
                    .filter(|r| r.status == Status::Skip && old.pass.contains(&r.name))
                    .map(|r| r.name.clone()),
            );
        }
        pass
    }
}
