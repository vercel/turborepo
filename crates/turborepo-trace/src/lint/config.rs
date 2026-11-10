//! The `[lint]` table of the repository's `turbotrace.toml`.
//!
//! ```toml
//! [lint]
//! baseline = "turbotrace-lint-baseline.json"   # relative to the repository root
//! tests = ["jest", "vitest"]                   # whose cost is reported; default: all
//! exclude = ["**/__fixtures__/**"]             # globs over repo-relative paths
//!
//! [lint.rules]
//! unconstrained-load = "error"                 # off | warn | error
//! unconstrained-read = "off"
//! ```
//!
//! The file is the one the hints are read from
//! (`turborepo_trace_store::Hints`), which accepts and ignores this table. It
//! is parsed here a second time, with a type that knows only `[lint]`: unknown
//! keys inside the table are errors, everything outside it belongs to
//! other readers.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

/// `[lint]` as written. Rule names and severities are checked by the caller,
/// which knows the rules.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// The baseline file, relative to the repository root.
    pub baseline: Option<String>,
    /// The test frameworks whose cost is reported.
    pub tests: Option<Vec<String>>,
    /// Globs of files without findings.
    pub exclude: Vec<String>,
    /// `[lint.rules]`: rule name to `off`, `warn` or `error`.
    pub rules: BTreeMap<String, String>,
    /// The file, for error messages.
    pub source: String,
}

#[derive(Deserialize)]
struct RawRoot {
    lint: Option<RawLint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLint {
    baseline: Option<String>,
    tests: Option<Vec<String>>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    rules: BTreeMap<String, String>,
}

/// Parses the text of a `turbotrace.toml`.
pub fn parse(text: &str, source: &str) -> Result<Config> {
    let raw: RawRoot = toml::from_str(text).map_err(|e| anyhow!("{source}: {e}"))?;
    let source = source.to_owned();
    Ok(match raw.lint {
        Some(l) => Config {
            baseline: l.baseline,
            tests: l.tests,
            exclude: l.exclude,
            rules: l.rules,
            source,
        },
        None => Config {
            source,
            ..Config::default()
        },
    })
}

/// The file `crate::graph::derived` reads the hints from:
/// `<root>/turbotrace.toml`, or the file named by `TURBO_TRACE_DEBUG_HINTS`
/// (empty: none).
fn path(root: &Path) -> Option<PathBuf> {
    match turborepo_trace_core::debug::var_os("HINTS") {
        Some(p) if p.is_empty() => None,
        Some(p) => Some(PathBuf::from(p)),
        None if !turborepo_trace_core::project::reads_turbotrace_toml() => None,
        None => Some(root.join(turborepo_trace_store::HINTS_FILE)),
    }
}

/// The `[lint]` table of the repository at `root`; the defaults when there is
/// no file.
pub fn load(root: &Path) -> Result<Config> {
    let Some(p) = path(root) else {
        return Ok(Config::default());
    };
    match std::fs::read_to_string(&p) {
        Ok(t) => parse(&t, &p.display().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config {
            source: p.display().to_string(),
            ..Config::default()
        }),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", p.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_lint_table_and_ignores_the_rest() {
        let c = parse(
            r#"
[[hint]]
files = "a.js"
loads = []

[lint]
baseline = "ci/lint.json"
tests = ["jest"]
exclude = ["**/__fixtures__/**"]

[lint.rules]
unconstrained-load = "warn"
"#,
            "turbotrace.toml",
        )
        .unwrap();
        assert_eq!(c.baseline.as_deref(), Some("ci/lint.json"));
        assert_eq!(c.tests, Some(vec!["jest".to_owned()]));
        assert_eq!(c.exclude, ["**/__fixtures__/**"]);
        assert_eq!(c.rules["unconstrained-load"], "warn");

        // No table: the defaults.
        let c = parse("[[env]]\nname = \"X\"\nvalues = []\n", "t").unwrap();
        assert_eq!(
            (c.baseline, c.tests, c.exclude.len(), c.rules.len()),
            (None, None, 0, 0)
        );
        // An unknown key in the table is an error that names the file.
        let e = parse("[lint]\nbaselin = \"x\"\n", "turbotrace.toml").unwrap_err();
        assert!(e.to_string().contains("turbotrace.toml") && e.to_string().contains("baselin"));
    }
}
