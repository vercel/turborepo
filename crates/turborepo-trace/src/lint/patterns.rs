//! The rule `test-pattern-above-root`: a test pattern that matches through the
//! directories the checkout lies below.
//!
//! jest and Playwright match their test patterns against the **absolute** path
//! of a file, and so does the trace (`turborepo_trace_frameworks::location`).
//! A pattern that can match the names of directories above the repository root
//! then gives other tests at another place: `testMatch:
//! ['**/src/**/*.test.js']` takes every `*.test.js` of a checkout below a
//! directory called `src`, and `testPathIgnorePatterns: ['/dist/']` leaves a
//! checkout below `dist` without tests. Where the place decides, the graph
//! records the absolute root (a graph built at one place is evaluated again at
//! another), and this rule says so: per pattern, the directories it matches
//! through, and how many files are tests here that are none at a neutral place
//! and the other way round.
//!
//! It reports whether or not the pattern also finds tests the ordinary way, and
//! jest's default patterns too: below a directory called `__tests__` every file
//! is a test for jest, and below `node_modules` none is.
//!
//! The key of a finding is the pattern as written.

use std::{path::Path, sync::Arc};

use anyhow::Result;
use turborepo_trace_core::DirCache;
use turborepo_trace_frameworks::{
    FrameworkInputs, fsx,
    location::{self, Culprit, DiscoverySpec, ProjectSpec, Tree, Verdict},
};
use turborepo_trace_graph::{GraphView, NodeId};

use super::{Ctx, Finding};

const RULE: &str = "test-pattern-above-root";

/// The runners that match absolute paths.
const RUNNERS: &[&str] = &["jest", "playwright"];

/// Loads a config as the graph build does: inside a file-system context
/// confined to the repository, so that the adapter knows the root its discovery
/// is written from.
fn load(
    fw: &dyn turborepo_trace_frameworks::Framework,
    root: &Path,
    config: &Path,
) -> FrameworkInputs {
    let fs = Arc::new(DirCache::new());
    fs.confine(root, []);
    fsx::with_fs(&fs, || fw.load(config))
}

/// The repository's files (from the root, sorted): the nodes of the graph that
/// are files of the tree.
fn tree_files<G: GraphView>(g: &G) -> Vec<String> {
    let mut files: Vec<String> = (0..g.node_count())
        .map(NodeId)
        .filter(|&n| !g.is_external(n))
        .map(|n| g.path(n))
        .filter(|p| !p.is_empty() && !p.starts_with("task:") && !p.starts_with('/'))
        .filter(|p| !p.contains('#'))
        .map(str::to_owned)
        .collect();
    files.sort_unstable();
    files.dedup();
    files
}

/// What a difference is made of, in words: the pattern and the directories it
/// matches through, for the file `example`.
fn because(spec: &DiscoverySpec, root: &Path, example: &str) -> (String, String) {
    let names = location::names(spec, root, example);
    let through = if names.is_empty() {
        "the directories".to_owned()
    } else {
        names
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match location::culprit(spec, root, example) {
        Some(Culprit::Pattern {
            text,
            leaves_out: false,
        }) => (
            text.clone(),
            format!("the test pattern `{text}` matches through {through} above the checkout"),
        ),
        Some(Culprit::Pattern {
            text,
            leaves_out: true,
        }) => (
            text.clone(),
            format!(
                "the pattern `{text}`, which leaves files out, matches through {through} above \
                 the checkout"
            ),
        ),
        Some(Culprit::InstalledPackages) => (
            "node_modules".to_owned(),
            "jest leaves out every file below a `node_modules` directory, and the checkout lies \
             below one"
                .to_owned(),
        ),
        None => (
            "location".to_owned(),
            format!("the test patterns match through {through} above the checkout"),
        ),
    }
}

/// What to do about a finding whose culprit is `key` (the pattern as the config
/// writes it, or `node_modules` / `location`). A pattern already written from
/// the project root (`<rootDir>/src/`) answers differently here only through
/// the characters of the checkout's path that it reads as pattern syntax
/// (`c++`, `[x]`), or by stepping above the root with `..`: writing it from the
/// root again would change nothing.
fn fix_for(key: &str) -> String {
    if key == "node_modules" {
        return "Check the repository out outside a `node_modules` directory: jest takes every \
                file below one for a file of an installed package."
            .to_owned();
    }
    if key.starts_with("<rootDir>") {
        return "The pattern is written from the project root and still matches differently here: \
                the path of the checkout has characters that the pattern reads as pattern syntax \
                (`+`, `*`, `?`, `[`, `{`, `(`), or the pattern steps above the root with `..`. \
                Check the repository out at a path without such characters, and write the \
                pattern without `..`."
            .to_owned();
    }
    "Write the pattern from the project root (`<rootDir>/…`), or so that it cannot match a \
     directory above the repository (a longer path, an anchor inside the repository)."
        .to_owned()
}

/// The findings of `test-pattern-above-root`.
pub fn above_root(ctx: &Ctx<'_, '_>) -> Result<Vec<Finding>> {
    let files = tree_files(ctx.derived);
    let tree = Tree::new(&files);
    // The graph holds the root for a project whose tests the place decides (or may
    // decide): only the configs of those are loaded. Playwright's configs are no
    // part of a graph: each is looked at.
    let recorded: std::collections::BTreeSet<String> = ctx
        .graph
        .located()
        .into_iter()
        .filter(|(_, _, root)| !root.is_empty())
        .map(|(config, _, _)| config)
        .collect();
    let mut out: Vec<Finding> = Vec::new();
    for runner in RUNNERS {
        let Some(fw) = turborepo_trace_frameworks::framework(runner) else {
            continue;
        };
        // jest: the configs the graph holds a root for, and no other is loaded.
        let configs: Vec<&String> = if *runner == "jest" {
            recorded.iter().collect()
        } else {
            files
                .iter()
                .filter(|rel| fw.is_config_file(&ctx.root.join(rel)))
                .collect()
        };
        for rel in configs {
            let config = ctx.root.join(rel);
            if !fw.is_config_file(&config) {
                continue;
            }
            let inputs = load(fw, ctx.root, &config);
            for project in &inputs.projects {
                let Some((spec, fell_back)) = ProjectSpec::of(&project.discovery) else {
                    continue;
                };
                let file = project
                    .config_file
                    .as_deref()
                    .and_then(|c| c.strip_prefix(ctx.root).ok())
                    .map_or_else(|| rel.clone(), |c| c.to_string_lossy().into_owned());
                if *runner == "jest" && !recorded.contains(&file) {
                    continue;
                }
                let Verdict {
                    dependent,
                    gained,
                    lost,
                } = location::verdict(&spec, fell_back, ctx.root, &tree);
                if !dependent {
                    continue;
                }
                let spec = spec.applying(fell_back);
                let example = gained.first().or(lost.first());
                let (key, why) = match example {
                    Some(e) => because(spec, ctx.root, e),
                    None => (
                        "location".to_owned(),
                        "a pattern that leaves files out of the runner's file map matches through \
                         the directories above the checkout"
                            .to_owned(),
                    ),
                };
                if out.iter().any(|o| o.file == file && o.key == key) {
                    continue;
                }
                let counts = format!(
                    "{} file(s) are tests here and none in a checkout elsewhere, {} are tests \
                     elsewhere and none here{}",
                    gained.len(),
                    lost.len(),
                    example.map_or_else(String::new, |e| format!("; for example {e}"))
                );
                let fix = fix_for(&key);
                out.push(Finding {
                    rule: RULE,
                    file,
                    line: 0,
                    key,
                    message: format!(
                        "{why}: which files are tests depends on where the repository is checked \
                         out ({counts})"
                    ),
                    cost: None,
                    fix,
                });
            }
        }
    }
    Ok(out)
}

/// For `affected --explain`: when the answer "is the file at `rel` (from the
/// root) a test" is another one here than at a neutral place, the sentence that
/// says so. `None` when the place decides nothing for it.
pub fn above_root_note(root: &Path, rel: &str) -> Option<String> {
    let abs = root.join(rel);
    for runner in RUNNERS {
        let Some(fw) = turborepo_trace_frameworks::framework(runner) else {
            continue;
        };
        let fs = Arc::new(DirCache::new());
        fs.confine(root, []);
        let Some(config) =
            fsx::with_fs(&fs, || fw.config_for(&abs)).filter(|c| c.starts_with(root))
        else {
            continue;
        };
        let inputs = fsx::with_fs(&fs, || fw.load(&config));
        for project in &inputs.projects {
            let Some((spec, fell_back)) = ProjectSpec::of(&project.discovery) else {
                continue;
            };
            let spec = spec.applying(fell_back);
            let one = [rel.to_owned()];
            let v = location::differences(
                &ProjectSpec {
                    configured: spec.clone(),
                    default: None,
                },
                false,
                root,
                &Tree::new(&one),
            );
            if !v.dependent {
                continue;
            }
            let (_, why) = because(spec, root, rel);
            let at = project
                .config_file
                .as_deref()
                .unwrap_or(&config)
                .strip_prefix(root)
                .map_or_else(|_| config.clone(), Path::to_path_buf);
            let what = if !v.gained.is_empty() {
                format!("{rel} is a {runner} test here and none in a checkout elsewhere")
            } else if !v.lost.is_empty() {
                format!("{rel} is no {runner} test here and one in a checkout elsewhere")
            } else {
                format!("{rel} is left out of the {runner} file map at one of the two places")
            };
            return Some(format!(
                "{what}: {why} ({}); which files are tests depends on where the repository is \
                 checked out",
                at.display()
            ));
        }
    }
    None
}
