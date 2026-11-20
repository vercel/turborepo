//! Where vitest runs start: the working directories they give its tests, and
//! the files that only they make tests ([`Graph::node_claims`]).
//!
//! vitest's root is the directory a run is started in (or `--root`): the base
//! of its patterns and the working directory of its tests. Which config the
//! run has depends on that directory and on the version: a config in the
//! directory, else (up to version 4) the one of the nearest directory above,
//! else none. A shard cannot know where runs start (the tasks of every package
//! say it), so a vitest test record lists the ways its file can be a test
//! ([`crate::fw::claim`]), the task records say where vitest starts
//! ([`crate::tasks::encode_start`]), and the composed graph puts the two
//! together here. Both sides are records of shards, so a refreshed graph
//! decides as a fresh one does.
//!
//! Nothing here takes a test or a working directory away: a record whose
//! nearest config matches the file holds with the config's directories,
//! whatever starts where (a config directory is a project). The starts add:
//!
//! | Start | Adds |
//! |---|---|
//! | a run with a config of its own (`own`, `named`) at or above the nearest config of the file | the run's directory and the directory the command is started in, as working directories |
//! | a run without a config that loads the one above (`above`, `either`), in a directory between the nearest config and the file, when the config's patterns match the file from that directory | the same; and the file as a test, when the config does not match it from its own directory and the file does not look like a test (one that does always runs, and stays so) |
//! | the same, when the config above lists projects and one matches the file | the same directories |
//! | a run without a config that has the defaults (`none`, `either`), in a directory above the file, when vitest's default patterns match it | the same directories |
//! | a command that names a test runner and is not followed to where it runs it (`launch:<dir>`), at or above the file | `<dir>` and the directory of the file's package |
//! | a run with a config named on the command line (`named`) | the files its `include` globs take, as files that always run ([`Graph::named_run_tests`]) |

use std::collections::BTreeSet;

use super::Graph;
use crate::{fw::claim, shard::ArchivedTestRec};

/// The framework whose records are claims.
const VITEST: &str = "vitest";

/// Which config a recognized run has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Config {
    Own,
    Named,
    Above,
    None,
    Either,
}

/// A recognized vitest run.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Run {
    config: Config,
    /// The directory of the project (from the root, empty for the root).
    dir: String,
    /// The directory the command is started in.
    from: String,
    /// The directory of the nearest config above `dir`, for a run without one.
    above: Option<String>,
}

/// Where the tasks of the graph start test runners.
#[derive(Debug, Default)]
pub(super) struct Starts {
    runs: Vec<Run>,
    /// Directories where a command names a test runner without being followed
    /// to where it runs it (empty: the repository root, also for a directory
    /// nothing says).
    launches: BTreeSet<String>,
    /// The runs with a config named on the command line: the directory, and the
    /// config's `include` globs from there.
    named: BTreeSet<(String, Vec<String>)>,
}

/// A directory as records write it (`.` for the root) to the form used here
/// (empty for the root).
fn dir(d: &str) -> &str {
    if d == "." { "" } else { d }
}

/// The directory of a package from its record's root.
pub(super) fn package_dir(root: &str) -> &str {
    dir(root.trim_matches('/'))
}

/// Whether `path` is `dir` or lies below it.
fn below(path: &str, dir: &str) -> bool {
    dir.is_empty()
        || path
            .strip_prefix(dir)
            .is_some_and(|r| r.is_empty() || r.starts_with('/'))
}

/// What a test record comes to.
pub(super) struct Claim<'g> {
    /// The working directories, when the record holds.
    pub cwds: Option<Vec<&'g str>>,
    /// Whether the nearest config of the file matches it (always, for a record
    /// of another framework): the record does not hold by a run alone.
    pub by_config: bool,
}

impl Graph {
    /// Where the tasks start test runners, read once.
    fn starts(&self) -> &Starts {
        self.starts.get_or_init(|| {
            let mut out = Starts::default();
            for s in &self.shards {
                for t in s.get().tasks.iter() {
                    for r in t.runs.iter() {
                        let r = r.as_str();
                        if let Some(d) = r.strip_prefix("launch:") {
                            let d = if d == "*" { "" } else { dir(d) };
                            out.launches.insert(d.to_owned());
                            continue;
                        }
                        let Some(rest) = r
                            .strip_prefix("start:")
                            .and_then(|r| r.strip_prefix(VITEST))
                            .and_then(|r| r.strip_prefix(':'))
                        else {
                            continue;
                        };
                        let mut parts = rest.split('\t');
                        let config = match parts.next() {
                            Some("own") => Config::Own,
                            Some("named") => Config::Named,
                            Some("above") => Config::Above,
                            Some("none") => Config::None,
                            Some("either") => Config::Either,
                            _ => continue,
                        };
                        let Some(d) = parts.next() else { continue };
                        let from = parts.next().unwrap_or(d);
                        let above = parts.next().filter(|a| !a.is_empty());
                        if config == Config::Named {
                            let globs = parts.map(str::to_owned).collect();
                            out.named.insert((dir(d).to_owned(), globs));
                        }
                        out.runs.push(Run {
                            config,
                            dir: dir(d).to_owned(),
                            from: dir(from).to_owned(),
                            above: above.map(|a| dir(a).to_owned()),
                        });
                    }
                }
            }
            out.runs.sort();
            out.runs.dedup();
            out
        })
    }

    /// What the test record `t` of the file at `path` (from the repository
    /// root), in the package at `package`, comes to: see the module
    /// documentation. A record of another framework holds as it is.
    fn test_claim<'g>(&'g self, t: &'g ArchivedTestRec, path: &str, package: &'g str) -> Claim<'g> {
        let own = || t.cwds.iter().map(|c| c.as_str());
        if t.framework.as_str() != VITEST || t.claim.is_empty() {
            return Claim {
                cwds: Some(own().collect()),
                by_config: true,
            };
        }
        let mut home: Option<&str> = None;
        let mut listed = false;
        let mut below_all = false;
        let mut below_dirs: Vec<&str> = Vec::new();
        let mut defaults = false;
        for c in t.claim.iter() {
            let c = c.as_str();
            if let Some(h) = c.strip_prefix(claim::HOME) {
                home = Some(dir(h));
            } else if let Some(b) = c.strip_prefix(claim::BELOW) {
                if b == "*" {
                    below_all = true;
                } else {
                    below_dirs.push(dir(b));
                }
            } else if c == claim::LISTED {
                listed = true;
            } else if c == claim::DEFAULTS {
                defaults = true;
            }
        }
        let starts = self.starts();
        // A config directory is a project: the record's own directories.
        let mut held = home.is_some();
        let mut cwds: Vec<&'g str> = if held { own().collect() } else { Vec::new() };
        for run in &starts.runs {
            let under = below(path, &run.dir);
            let hit = match run.config {
                // The run's config, or (a config below the run's root, which the run
                // does not read) the nearest config standing in for it.
                Config::Own | Config::Named => under && home.is_some_and(|h| below(h, &run.dir)),
                Config::Above | Config::None | Config::Either => {
                    // With the config above: its patterns from the run's directory, or
                    // the projects it lists.
                    let from_below = under
                        && (below_all
                            || below_dirs.contains(&run.dir.as_str())
                            || home.is_some_and(|h| h != run.dir && below(h, &run.dir)));
                    let lists = listed && home.is_some() && home == run.above.as_deref();
                    let loaded = run.config != Config::None && (from_below || lists);
                    // The file is a test of the run, also when its config's own
                    // directory does not make it one.
                    held |= loaded;
                    // With the defaults: only the directories, of a file that is a test.
                    loaded || (run.config != Config::Above && under && defaults && home.is_some())
                }
            };
            if hit {
                cwds.push(run.dir.as_str());
                cwds.push(run.from.as_str());
            }
        }
        // A command that names a runner at or above the file: maybe it starts one in
        // its own directory, maybe in the file's package.
        if held {
            for w in starts.launches.iter().filter(|w| below(path, w)) {
                cwds.push(w.as_str());
                if below(package, w) {
                    cwds.push(package);
                }
            }
        }
        cwds.sort_unstable();
        cwds.dedup();
        Claim {
            cwds: held.then_some(cwds),
            by_config: home.is_some(),
        }
    }

    /// The claims of the test records of node `ni`.
    pub(super) fn node_claims(&self, ni: u32) -> Vec<(&ArchivedTestRec, Claim<'_>)> {
        let node = self.nodes[ni as usize];
        if node.shard == super::NONE {
            return Vec::new();
        }
        let shard = self.shards[node.shard as usize].get();
        // Test records are sorted by file.
        let tests = &shard.tests;
        let lo = tests.partition_point(|t| t.file.to_native() < node.file);
        if tests
            .get(lo)
            .is_none_or(|t| t.file.to_native() != node.file)
        {
            return Vec::new();
        }
        let path = turborepo_trace_graph::GraphView::path(self, turborepo_trace_graph::NodeId(ni));
        let package = package_dir(shard.package.root.as_str());
        tests[lo..]
            .iter()
            .take_while(|t| t.file.to_native() == node.file)
            .map(|t| (t, self.test_claim(t, path, package)))
            .collect()
    }

    /// Whether the test record `t` of node `n` holds.
    pub(super) fn record_holds(
        &self,
        n: turborepo_trace_graph::NodeId,
        t: &ArchivedTestRec,
    ) -> bool {
        self.node_claims(n.0)
            .iter()
            .any(|(r, c)| std::ptr::eq(*r, t) && c.cwds.is_some())
    }

    /// Whether a config of a test framework makes `n` a test: not a file that
    /// is a test only by a vitest run started below a config that leaves it
    /// out from its own directory.
    pub fn test_by_config(&self, n: turborepo_trace_graph::NodeId) -> bool {
        self.node_claims(n.0)
            .iter()
            .any(|(_, c)| c.cwds.is_some() && c.by_config)
    }

    /// The files that a run with a config named on the command line takes by
    /// the config's `include` globs: tests of that run, whatever else is known
    /// of them (a file among them that no framework's record makes a test is
    /// one that always runs).
    pub fn named_run_tests(&self) -> Vec<turborepo_trace_graph::NodeId> {
        use turborepo_trace_graph::{GraphView, NodeId, glob::Glob};
        let runs: Vec<(&str, Vec<Glob>)> = self
            .starts()
            .named
            .iter()
            .map(|(d, globs)| {
                (
                    d.as_str(),
                    globs.iter().filter_map(|g| Glob::new(g).ok()).collect(),
                )
            })
            .filter(|(_, globs): &(&str, Vec<Glob>)| !globs.is_empty())
            .collect();
        if runs.is_empty() {
            return Vec::new();
        }
        (0..self.nodes.len() as u32)
            .map(NodeId)
            .filter(|&n| {
                if self.is_external(n) || self.lang(n).is_none() {
                    return false;
                }
                let path = self.path(n);
                runs.iter().any(|(d, globs)| {
                    let rel = if d.is_empty() {
                        Some(path)
                    } else {
                        path.strip_prefix(d).and_then(|r| r.strip_prefix('/'))
                    };
                    rel.is_some_and(|rel| globs.iter().any(|g| g.matches(rel)))
                })
            })
            .collect()
    }
}
