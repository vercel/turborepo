//! The lint baseline: the findings a repository has accepted, in a committed
//! file.
//!
//! ```json
//! {
//!   "version": 1,
//!   "entries": [
//!     { "rule": "unconstrained-load", "file": "src/plugins.js", "key": "computed_specifier/param: name; parameter exports.load#0", "count": 1 }
//!   ]
//! }
//! ```
//!
//! One entry per line, sorted by file, rule and key, so that a change to the
//! file reviews as a diff of whole findings. An entry has no line number and no
//! cost: both change with unrelated edits. `count` is the number of findings of
//! the file with that rule and key.
//!
//! Matching is by multiset: of `n` findings with one (rule, file, key) and an
//! entry with count `c`, the first `min(n, c)` in line order are baselined and
//! the others are new; if `c > n`, the entry is stale by `c - n`. A stale entry
//! fails the check: it would let the same finding come back unnoticed.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::Finding;

/// The file name looked up in the repository root.
pub const DEFAULT_FILE: &str = "turbotrace-lint-baseline.json";

/// The format version this binary reads and writes.
const VERSION: u32 = 1;

/// What identifies a finding in the baseline.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    /// Repo-relative file. First: entries sort by file.
    pub file: String,
    /// Rule name.
    pub rule: String,
    /// [`Finding::key`].
    pub key: String,
}

impl Key {
    /// The key of a finding.
    pub fn of(f: &Finding) -> Key {
        Key {
            file: f.file.clone(),
            rule: f.rule.to_owned(),
            key: f.key.clone(),
        }
    }
}

/// One line of the baseline file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Rule name.
    pub rule: String,
    /// Repo-relative file.
    pub file: String,
    /// [`Finding::key`].
    pub key: String,
    /// Findings of the file with this rule and key.
    #[serde(default = "one")]
    pub count: usize,
}

fn one() -> usize {
    1
}

impl Entry {
    fn id(&self) -> Key {
        Key {
            file: self.file.clone(),
            rule: self.rule.clone(),
            key: self.key.clone(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    version: u32,
    #[serde(default)]
    entries: Vec<Entry>,
}

/// A baseline: entries sorted by file, rule and key, each (file, rule, key)
/// once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Baseline {
    /// The entries.
    pub entries: Vec<Entry>,
}

/// The result of [`Baseline::matches`].
#[derive(Debug, PartialEq, Eq)]
pub struct Matched {
    /// Per finding, in the order given: whether an entry accepts it.
    pub baselined: Vec<bool>,
    /// Entries in scope without a finding; `count` is the number missing.
    pub stale: Vec<Entry>,
}

impl Baseline {
    fn from_counts(counts: BTreeMap<Key, usize>) -> Baseline {
        Baseline {
            entries: counts
                .into_iter()
                .filter(|(_, count)| *count > 0)
                .map(|(k, count)| Entry {
                    rule: k.rule,
                    file: k.file,
                    key: k.key,
                    count,
                })
                .collect(),
        }
    }

    /// The baseline that accepts exactly these findings.
    pub fn from_keys(keys: impl IntoIterator<Item = Key>) -> Baseline {
        let mut counts: BTreeMap<Key, usize> = BTreeMap::new();
        for k in keys {
            *counts.entry(k).or_default() += 1;
        }
        Baseline::from_counts(counts)
    }

    /// Parses a baseline file. Entries with the same rule, file and key (a
    /// merge of two branches) add up.
    pub fn parse(text: &str) -> Result<Baseline> {
        let raw: RawFile = serde_json::from_str(text)?;
        if raw.version != VERSION {
            bail!(
                "format version {} (this build reads version {VERSION})",
                raw.version
            );
        }
        let mut counts: BTreeMap<Key, usize> = BTreeMap::new();
        for e in &raw.entries {
            *counts.entry(e.id()).or_default() += e.count;
        }
        Ok(Baseline::from_counts(counts))
    }

    /// Reads `path`. A missing file is an empty baseline.
    pub fn load(path: &Path) -> Result<Baseline> {
        match std::fs::read_to_string(path) {
            Ok(t) => Baseline::parse(&t).with_context(|| format!("baseline {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Baseline::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read baseline {}", path.display())),
        }
    }

    /// The file's text: pretty-printed, one entry per line.
    pub fn render(&self) -> String {
        let s = |v: &str| serde_json::Value::String(v.to_owned()).to_string();
        let mut out = format!("{{\n  \"version\": {VERSION},\n  \"entries\": [");
        for (i, e) in self.entries.iter().enumerate() {
            out.push_str(if i == 0 { "\n" } else { ",\n" });
            out.push_str(&format!(
                "    {{ \"rule\": {}, \"file\": {}, \"key\": {}, \"count\": {} }}",
                s(&e.rule),
                s(&e.file),
                s(&e.key),
                e.count
            ));
        }
        out.push_str(if self.entries.is_empty() {
            "]\n}\n"
        } else {
            "\n  ]\n}\n"
        });
        out
    }

    /// Writes the file, creating its directory if needed.
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        std::fs::write(path, self.render())
            .with_context(|| format!("cannot write baseline {}", path.display()))
    }

    /// Number of findings the baseline accepts.
    pub fn total(&self) -> usize {
        self.entries.iter().map(|e| e.count).sum()
    }

    /// Matches `findings` against the entries. Entries for which `in_scope` is
    /// false (a rule that did not run, an excluded file) are never stale.
    pub fn matches(&self, findings: &[Key], in_scope: &dyn Fn(&Entry) -> bool) -> Matched {
        let mut left: BTreeMap<Key, usize> =
            self.entries.iter().map(|e| (e.id(), e.count)).collect();
        let baselined = findings
            .iter()
            .map(|k| match left.get_mut(k) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    true
                }
                _ => false,
            })
            .collect();
        let stale = self
            .entries
            .iter()
            .filter(|e| in_scope(e))
            .filter_map(|e| match left[&e.id()] {
                0 => None,
                count => Some(Entry { count, ..e.clone() }),
            })
            .collect();
        Matched { baselined, stale }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(file: &str, key: &str) -> Key {
        Key {
            file: file.to_owned(),
            rule: "unconstrained-load".to_owned(),
            key: key.to_owned(),
        }
    }

    fn all(_: &Entry) -> bool {
        true
    }

    #[test]
    fn round_trip_is_sorted_and_one_entry_per_line() {
        let b = Baseline::from_keys([
            key("src/b.js", "k"),
            key("src/a.js", "k2"),
            key("src/b.js", "k"),
        ]);
        let text = b.render();
        assert_eq!(
            text,
            r#"{
  "version": 1,
  "entries": [
    { "rule": "unconstrained-load", "file": "src/a.js", "key": "k2", "count": 1 },
    { "rule": "unconstrained-load", "file": "src/b.js", "key": "k", "count": 2 }
  ]
}
"#
        );
        assert_eq!(Baseline::parse(&text).unwrap(), b);
        assert_eq!(b.total(), 3);
        // Keys are JSON strings: quotes and backslashes survive.
        let odd = Baseline::from_keys([key("a.js", "x \"y\" \\ `z`")]);
        assert_eq!(Baseline::parse(&odd.render()).unwrap(), odd);
        // Empty.
        let empty = Baseline::default();
        assert_eq!(
            empty.render(),
            "{\n  \"version\": 1,\n  \"entries\": []\n}\n"
        );
        assert_eq!(Baseline::parse(&empty.render()).unwrap(), empty);
    }

    #[test]
    fn rejects_other_versions_and_unknown_fields() {
        assert!(
            Baseline::parse(r#"{"version": 2, "entries": []}"#)
                .unwrap_err()
                .to_string()
                .contains("format version 2")
        );
        assert!(Baseline::parse(r#"{"version": 1, "entries": [], "x": 1}"#).is_err());
        assert!(Baseline::parse("not json").is_err());
        // Duplicate entries add up; a missing count is one.
        let b = Baseline::parse(
            r#"{"version": 1, "entries": [
                {"rule": "unconstrained-load", "file": "a.js", "key": "k"},
                {"rule": "unconstrained-load", "file": "a.js", "key": "k", "count": 2}
            ]}"#,
        )
        .unwrap();
        assert_eq!(b.entries.len(), 1);
        assert_eq!(b.entries[0].count, 3);
    }

    #[test]
    fn matches_by_multiset() {
        let b = Baseline::from_keys([key("a.js", "k"), key("a.js", "k"), key("b.js", "k")]);
        // Exactly the baseline.
        let m = b.matches(
            &[key("a.js", "k"), key("a.js", "k"), key("b.js", "k")],
            &all,
        );
        assert_eq!(m.baselined, [true, true, true]);
        assert!(m.stale.is_empty());
        // One more of a known key: the first two are baselined, the third is new. A key
        // the baseline does not have is new.
        let m = b.matches(
            &[
                key("a.js", "k"),
                key("a.js", "k"),
                key("a.js", "k"),
                key("b.js", "k"),
                key("b.js", "other"),
                key("c.js", "k"),
            ],
            &all,
        );
        assert_eq!(m.baselined, [true, true, false, true, false, false]);
        assert!(m.stale.is_empty());
        // One of two gone, and a whole entry gone: stale by the missing count.
        let m = b.matches(&[key("a.js", "k")], &all);
        assert_eq!(m.baselined, [true]);
        let stale: Vec<(&str, usize)> =
            m.stale.iter().map(|e| (e.file.as_str(), e.count)).collect();
        assert_eq!(stale, [("a.js", 1), ("b.js", 1)]);
        // Entries out of scope are never stale.
        let m = b.matches(&[], &|e: &Entry| e.file != "a.js");
        let stale: Vec<&str> = m.stale.iter().map(|e| e.file.as_str()).collect();
        assert_eq!(stale, ["b.js"]);
        // The same key under another rule is another finding.
        let other = Key {
            rule: "unconstrained-read".to_owned(),
            ..key("b.js", "k")
        };
        let m = b.matches(&[other], &all);
        assert_eq!(m.baselined, [false]);
    }
}
