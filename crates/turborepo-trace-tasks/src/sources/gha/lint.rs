//! `paths` filter lint: a workflow whose `on.<event>.paths` / `paths-ignore`
//! filter misses files its recognized tasks depend on does not run on that
//! event when those files change.
//!
//! For every `gha:` job of a workflow with filters, and every filtered event,
//! the files of the job's task closure (its own steps and the tasks they
//! reference, such as `turbo run build`) are checked against that event's
//! filter with GitHub's semantics (patterns in order, the last match wins, `!`
//! negates): inputs, configs and entries that exist, and recognized memberships
//! (represented by a sample path). Not checked: the conservative fallbacks for
//! unknown commands (whole directories), whole-repository memberships, and
//! workflow/action files under `.github/` (leaving those out of a filter is
//! usually deliberate). Entries' traced closures are not expanded here (they
//! live in the graph store), so this is a lower bound of what is missed.

use std::{collections::BTreeSet, path::Path};

use serde::Serialize;
use turborepo_trace_frameworks::fsx;

use super::{PATHS_IGNORE_REASON, PATHS_REASON, event_filters};
use crate::{
    TaskSet,
    sources::taskfile::parse_yaml,
    spec::{Membership, TaskId},
};

/// One workflow job and event whose filter misses inputs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PathsFinding {
    /// The job.
    pub task: TaskId,
    /// The event (`push`, `pull_request`).
    pub event: String,
    /// The event's `paths` patterns.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// The event's `paths-ignore` patterns.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths_ignore: Vec<String>,
    /// Repo-relative files (or `dir/<pattern>` memberships) the filter does not
    /// cover.
    pub missing: Vec<String>,
}

/// Whether a membership is a conservative fallback rather than a recognized
/// input set.
fn is_fallback(m: &Membership) -> bool {
    let r = m.reason.as_str();
    [
        "unknown",
        "uncertain",
        "unsupported",
        "not fully",
        "computed",
        "unreadable",
        "publish",
        "pack",
    ]
    .iter()
    .any(|k| r.contains(k))
}

fn rel(root: &Path, p: &Path) -> Option<String> {
    let r = p.strip_prefix(root).ok()?;
    Some(r.to_string_lossy().replace('\\', "/"))
}

/// Whether `path` passes an ordered pattern list (`!` negates; the last match
/// wins).
fn ordered_match(patterns: &[String], path: &str) -> bool {
    let mut hit = false;
    for p in patterns {
        match p.strip_prefix('!') {
            Some(n) => {
                if crate::glob::matches(n, path) {
                    hit = false;
                }
            }
            None => {
                if crate::glob::matches(p, path) || crate::glob::matches(&format!("{p}/**"), path) {
                    hit = true;
                }
            }
        }
    }
    hit
}

/// Whether a change to `path` triggers the event.
fn triggers(paths: &[String], ignore: &[String], path: &str) -> bool {
    if !paths.is_empty() {
        return ordered_match(paths, path);
    }
    !ordered_match(ignore, path)
}

/// A sample file path standing for a membership (`src/**/*.ts` → `src/x.ts`).
fn sample(dir: &str, pattern: &str) -> String {
    let p = pattern
        .replace("**/", "")
        .replace("**", "x")
        .replace(['*', '?'], "x");
    let p = crate::glob::expand_braces(&p)
        .into_iter()
        .next()
        .unwrap_or_default();
    if dir.is_empty() {
        p
    } else {
        format!("{dir}/{p}")
    }
}

/// Lints every `gha:` job of `set` whose workflow has `paths` / `paths-ignore`
/// filters.
pub fn lint_paths(set: &TaskSet) -> Vec<PathsFinding> {
    let root = set.root.as_path();
    let mut out = Vec::new();
    for t in set.tasks.iter().filter(|t| t.id.kind() == "gha") {
        let short = t.id.short();
        let Some((wf, _)) = short.rsplit_once('#') else {
            continue;
        };
        let Some(doc) = fsx::read_to_string(root.join(wf))
            .ok()
            .and_then(|s| parse_yaml(&s))
        else {
            continue;
        };
        let events = event_filters(&doc);
        if events.is_empty() {
            continue;
        }
        // The files the job's closure depends on (repo-relative), and sample paths for
        // its memberships.
        let mut files: BTreeSet<String> = BTreeSet::new();
        let mut members: BTreeSet<(String, String)> = BTreeSet::new();
        for task in set.closure(&t.id) {
            let s = &task.spec;
            for p in s
                .inputs
                .iter()
                .chain(s.configs.iter())
                .chain(s.entries.iter().map(|e| &e.path))
            {
                if let Some(r) = rel(root, p)
                    && !r.is_empty()
                    && !r.starts_with(".github/")
                    && fsx::is_file(p)
                {
                    files.insert(r);
                }
            }
            for m in &s.memberships {
                if m.reason == PATHS_REASON || m.reason == PATHS_IGNORE_REASON || is_fallback(m) {
                    continue;
                }
                let Some(d) = rel(root, &m.dir) else { continue };
                if (d.is_empty() && m.pattern == "**") || d.starts_with(".github") {
                    continue;
                }
                let shown = if d.is_empty() {
                    m.pattern.clone()
                } else {
                    format!("{d}/{}", m.pattern)
                };
                members.insert((shown, sample(&d, &m.pattern)));
            }
        }
        for (event, paths, ignore) in events {
            let mut missing: BTreeSet<String> = files
                .iter()
                .filter(|f| !triggers(&paths, &ignore, f))
                .cloned()
                .collect();
            for (shown, sp) in &members {
                if !triggers(&paths, &ignore, sp) {
                    missing.insert(shown.clone());
                }
            }
            if !missing.is_empty() {
                out.push(PathsFinding {
                    task: t.id.clone(),
                    event,
                    paths,
                    paths_ignore: ignore,
                    missing: missing.into_iter().collect(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_filter_semantics() {
        let paths: Vec<String> = ["src/**", "!src/**/*.md", "src/keep.md", "package.json"]
            .map(String::from)
            .to_vec();
        assert!(triggers(&paths, &[], "src/a/b.ts"));
        assert!(!triggers(&paths, &[], "src/a/b.md"));
        assert!(triggers(&paths, &[], "src/keep.md"));
        assert!(triggers(&paths, &[], "package.json"));
        assert!(!triggers(&paths, &[], "lib/x.ts"));
        let ignore: Vec<String> = ["docs/**".to_string()].to_vec();
        assert!(!triggers(&[], &ignore, "docs/a.md"));
        assert!(triggers(&[], &ignore, "lib/x.ts"));
        // A directory pattern covers what is below it.
        assert!(triggers(&["tools".to_string()], &[], "tools/a.js"));
        assert_eq!(sample("src", "**/*.{ts,tsx}"), "src/x.ts");
    }
}
