//! Lockfile and manifest diffs → changed third-party packages (third-party
//! packages are opaque: a change is seen by package, not by file).
//!
//! For every changed lockfile (pnpm, npm, yarn, bun, `Cargo.lock`,
//! `go.mod`/`go.sum`, `uv.lock`, `poetry.lock`, `Pipfile.lock`,
//! `requirements*.txt`):
//!
//! - the importers it governs are the first-party packages below its directory
//!   that no deeper lockfile of the same ecosystem governs;
//! - with the old contents (`--since <rev>`, through `git show`), the old and
//!   new lockfiles are parsed and diffed; every package in
//!   `affected_packages().closure_changed` (its own identity or anything in its
//!   dependency closure changed) that is a node of the graph is changed for
//!   those importers, and the lockfile path itself is dropped from the file
//!   changes (its only effect on resolution is the package identities). A
//!   Python lockfile is dropped only when nothing it locks changed: a Python
//!   file that imports a module of a distribution that is not installed has the
//!   lockfile itself as an input. `go.mod` stays a file change: it also carries
//!   `replace` and toolchain directives, and nothing in the repository says
//!   which module depends on which. For the same reason `go.sum` stays a file
//!   change unless both sides list the same module hashes
//!   ([`crate::gosumdiff`]);
//! - without old contents (or when either side does not parse), every external
//!   package of that ecosystem is changed for those importers (conservative),
//!   and the lockfile stays a file change.
//!
//! A `package.json` whose dependency ranges changed while no JS lockfile
//! changed counts as a manifest change of its importer: each dependency whose
//! range was added, removed or edited is changed for that importer (the next
//! install may resolve it differently). The file itself stays a change too
//! (every resolution that read it depends on it).

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
};

use turborepo_trace_graph::{Change, ChangeKind, GraphView, PackageId, PackageVersion};
use turborepo_trace_lockfile::{LockDiff, Lockfile, LockfileKind};
use turborepo_trace_store::Graph;

/// The debug variable (`TURBO_TRACE_DEBUG_EXACT_BASE`) that makes `--since
/// <rev>` mean exactly `<rev>`, like `--exact-base`. For tooling that passes a
/// base it has resolved itself to binaries of any age, in clones that hold
/// single commits without history. A binary
/// from before the merge-base rule ignores the variable and compares
/// with the revision anyway.
pub const EXACT_BASE_ENV: &str = "EXACT_BASE";

/// Whether `--since <rev>` is taken as it is ([`EXACT_BASE_ENV`],
/// `--exact-base`).
pub fn exact_base(flag: bool) -> bool {
    flag || turborepo_trace_core::debug::var_os(EXACT_BASE_ENV).is_some_and(|v| v == "1")
}

/// Access to the base of a change: the commit the working tree is compared
/// with, and the source of old file contents.
///
/// **The base of `--since <rev>` is the merge base of `<rev>` and `HEAD`**: the
/// latest commit of `<rev>`'s history that the branch contains. The changes are
/// the two-tree diff between that commit and the working tree, so they are the
/// branch's own: what `<rev>` gained after the fork is not a change, and
/// neither is what a merge of `<rev>` into the branch brought in (only how the
/// branch resolved it). No commit is walked: a file that was changed and
/// changed back is not a change.
///
/// - `<rev>` is an ancestor of `HEAD` (a rebased branch, a pull request's merge
///   ref with `<rev>` as its first parent): the merge base is `<rev>` itself.
/// - Several merge bases (a criss-cross merge): the changes are the union of
///   the diffs against each, and there is no single old version of a file, so
///   [`Git::show`] gives none and every field-level comparison falls back to
///   "the whole file changed".
/// - No merge base can be found, or the checkout is shallow and its cut-off
///   history could hide one: an error ([`BaseUnknown`]). A base is never
///   guessed.
/// - `exact` ([`exact_base`]): the base is `<rev>`, without the merge-base
///   step.
pub struct Git {
    /// The repository toplevel.
    pub toplevel: PathBuf,
    /// The graph root relative to the toplevel (`""` or `sub/dir/`).
    pub prefix: String,
    /// The base commit the working tree is compared with: the resolved merge
    /// base (the first of several), or the revision as given in exact mode.
    pub rev: String,
    /// The revision as given (`--since`).
    pub given: String,
    /// The commit the given revision names.
    pub given_commit: String,
    /// Every base: one, or several merge bases of a criss-cross history.
    pub bases: Vec<String>,
    /// Whether the revision was taken as it is.
    pub exact: bool,
    /// The files of the base that the config rules read ([`Git::show_config`]),
    /// by root-relative path: each is read once, whichever rule asks first.
    configs: BaseFiles,
    /// The changed paths that are symbolic links at the base
    /// ([`Git::links_at_base`]).
    links: std::sync::Mutex<BTreeSet<String>>,
}

/// Files read at the base, by root-relative path (`None`: not there).
type BaseFiles = std::sync::Arc<std::sync::Mutex<HashMap<String, Option<std::sync::Arc<[u8]>>>>>;

/// The merge base of the revision and `HEAD` can't be told from this checkout.
#[derive(Debug)]
pub struct BaseUnknown(pub String);

impl std::fmt::Display for BaseUnknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BaseUnknown {}

impl Git {
    /// The base of `rev` in the repository containing `root`: the merge base of
    /// `rev` and `HEAD`, or `rev` itself when `exact`.
    // Invariant: the merge base is asked for whenever `exact` is false, the branch this is on.
    #[allow(clippy::expect_used)]
    pub fn new(root: &Path, rev: &str, exact: bool) -> anyhow::Result<Git> {
        // A revision git would read as an option is refused, as turbo refuses it.
        crate::adapter::validate_ref(rev).map_err(|e| anyhow::anyhow!(e))?;
        // Every git call at once (process startup dominates).
        let commit = format!("{rev}^{{commit}}");
        let (top, valid, merge, is_shallow) = std::thread::scope(|s| {
            let valid = s.spawn(|| {
                git(
                    root,
                    &[
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        "--end-of-options",
                        &commit,
                    ],
                )
            });
            // Every merge base: one, or several in a criss-cross history.
            let merge = (!exact).then(|| {
                s.spawn(|| {
                    Command::new("git")
                        .arg("-C")
                        .arg(root)
                        .args(["merge-base", "--all", "--end-of-options", rev, "HEAD"])
                        .output()
                })
            });
            let shallow = s.spawn(|| {
                git(root, &["rev-parse", "--is-shallow-repository"])
                    .is_ok_and(|s| s.trim() == "true")
            });
            let top = git(root, &["rev-parse", "--show-toplevel"]);
            (
                top,
                valid
                    .join()
                    .unwrap_or_else(|p| std::panic::resume_unwind(p)),
                merge.map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p))),
                shallow
                    .join()
                    .unwrap_or_else(|p| std::panic::resume_unwind(p)),
            )
        });
        let top = top?;
        let toplevel = std::fs::canonicalize(top.trim())?;
        let prefix = root
            .strip_prefix(&toplevel)
            .map(|p| {
                let s = p.to_string_lossy().replace('\\', "/");
                if s.is_empty() { s } else { format!("{s}/") }
            })
            .unwrap_or_default();
        // Validate the revision early, with a readable error.
        if valid.is_err() {
            if is_shallow {
                return Err(BaseUnknown(format!(
                    "the revision {rev:?} is not in this shallow checkout: fetch it (`git fetch \
                     --depth=1 origin <commit>`), and for a pull request's merge ref fetch with a \
                     depth of at least 2 so that its parents are there"
                ))
                .into());
            }
            anyhow::bail!("unknown revision {rev:?}");
        }
        let given_commit = valid.unwrap_or_default().trim().to_owned();
        if exact {
            return Ok(Git {
                toplevel,
                prefix,
                rev: given_commit.clone(),
                given: rev.to_owned(),
                bases: vec![given_commit.clone()],
                given_commit,
                exact: true,
                configs: BaseFiles::default(),
                links: Default::default(),
            });
        }
        let found = merge.expect("asked for unless exact")?;
        let bases: Vec<String> = String::from_utf8_lossy(&found.stdout)
            .lines()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect();
        if !found.status.success() || bases.is_empty() {
            let have = |r: &str| {
                git(&toplevel, &["rev-list", "--count", r])
                    .map(|n| n.trim().to_owned())
                    .unwrap_or_else(|_| "?".to_owned())
            };
            return Err(BaseUnknown(if is_shallow {
                format!(
                    "the merge base of {rev:?} and HEAD is not in this shallow checkout ({} \
                     commit(s) of HEAD's history and {} of {rev:?}'s are here): fetch the history \
                     between them (`git fetch --deepen=<n>`, or `git fetch --unshallow`), or pass \
                     the base commit your CI already knows with --exact-base",
                    have("HEAD"),
                    have(rev)
                )
            } else {
                format!(
                    "{rev:?} and HEAD have no common ancestor, so there is no merge base to \
                     compare with (--exact-base compares with {rev:?} itself)"
                )
            })
            .into());
        }
        // A shallow checkout cuts history off. A merge base found in what is left is
        // only certain when no cut lies between the two tips and the bases:
        // behind a cut there may be a later common ancestor. Not so when one
        // tip is an ancestor of the other (a pull request's merge ref compared
        // with its first parent): that tip is the merge base whatever the cut
        // hides.
        let tip_is_base = || {
            let commit = |r: &str| {
                git(&toplevel, &["rev-parse", "--verify", "--quiet", r])
                    .map(|s| s.trim().to_owned())
                    .ok()
            };
            bases.len() == 1
                && (bases[0] == given_commit
                    || commit("HEAD").as_deref() == Some(bases[0].as_str()))
        };
        if is_shallow && !tip_is_base() {
            let mut args = vec!["rev-list", rev, "HEAD", "--not"];
            args.extend(bases.iter().map(String::as_str));
            let between = git(&toplevel, &args)?;
            let cut: std::collections::HashSet<String> =
                git(&toplevel, &["rev-parse", "--git-path", "shallow"])
                    .ok()
                    .and_then(|p| {
                        let p = PathBuf::from(p.trim());
                        let p = if p.is_absolute() { p } else { toplevel.join(p) };
                        std::fs::read_to_string(p).ok()
                    })
                    .unwrap_or_default()
                    .lines()
                    .map(|l| l.trim().to_owned())
                    .collect();
            if let Some(at) = between.lines().map(str::trim).find(|c| cut.contains(*c)) {
                return Err(BaseUnknown(format!(
                    "the merge base of {rev:?} and HEAD can't be told from this shallow checkout: \
                     its history is cut off at {} before the two meet, and a later common commit \
                     may lie behind the cut. Fetch more history (`git fetch --deepen=<n>`, or \
                     `git fetch --unshallow`), or pass the base commit your CI already knows with \
                     --exact-base",
                    &at[..at.len().min(12)]
                ))
                .into());
            }
        }
        Ok(Git {
            toplevel,
            prefix,
            rev: bases[0].clone(),
            given: rev.to_owned(),
            given_commit,
            bases,
            exact: false,
            configs: BaseFiles::default(),
            links: Default::default(),
        })
    }

    /// The base as a reader should see it in a report: the revision as given
    /// when that is the base itself, else the resolved commit and where it
    /// comes from.
    pub fn shown(&self) -> String {
        if self.exact || (self.bases.len() == 1 && self.rev == self.given_commit) {
            self.given.clone()
        } else if self.bases.len() > 1 {
            format!(
                "{} merge bases of {} and HEAD",
                self.bases.len(),
                self.given
            )
        } else {
            format!(
                "{} (the merge base of {} and HEAD)",
                &self.rev[..self.rev.len().min(12)],
                self.given
            )
        }
    }

    /// One line for stderr: what the working tree is compared with.
    pub fn describe(&self) -> String {
        let short = |s: &str| s[..s.len().min(12)].to_owned();
        if self.exact {
            format!("base {} (exactly the revision given)", self.given)
        } else if self.bases.len() > 1 {
            format!(
                "base: {} merge bases of {} and HEAD ({}): the changes against each count, and no \
                 file has one old version, so changed manifests, lockfiles and configs count whole",
                self.bases.len(),
                self.given,
                self.bases
                    .iter()
                    .map(|b| short(b))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else if self.rev == self.given_commit {
            format!(
                "base {} ({}, which HEAD contains)",
                short(&self.rev),
                self.given
            )
        } else {
            format!(
                "base {} (the merge base of {} and HEAD)",
                short(&self.rev),
                self.given
            )
        }
    }

    /// The contents of root-relative `path` at the base. None when the history
    /// has several merge bases: there is no single old version to compare
    /// with.
    pub fn show(&self, path: &str) -> Option<String> {
        if self.bases.len() > 1 {
            return None;
        }
        let spec = format!("{}:{}{}", self.rev, self.prefix, path);
        let o = Command::new("git")
            .arg("-C")
            .arg(&self.toplevel)
            .args(["show", &spec])
            .output()
            .ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    }

    /// A reader of the bytes of root-relative paths at the base that does not
    /// borrow `self`; `None` when the history has several merge bases (as
    /// [`Git::show`]). It reads a file once, and shares what it read with
    /// [`Git::show_config`] and [`Git::preload`].
    pub fn reader(&self) -> Option<impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static> {
        if self.bases.len() > 1 {
            return None;
        }
        let (toplevel, at, configs) = (
            self.toplevel.clone(),
            format!("{}:{}", self.rev, self.prefix),
            std::sync::Arc::clone(&self.configs),
        );
        Some(move |path: &str| {
            let known = |c: &BaseFiles| c.lock().ok()?.get(path).cloned();
            if let Some(b) = known(&configs) {
                return b.map(|b| b.to_vec());
            }
            let o = Command::new("git")
                .arg("-C")
                .arg(&toplevel)
                .args(["show", &format!("{at}{path}")])
                .output()
                .ok()?;
            let bytes = o.status.success().then_some(o.stdout);
            if let Ok(mut c) = configs.lock() {
                c.insert(path.to_owned(), bytes.as_deref().map(std::sync::Arc::from));
            }
            bytes
        })
    }

    /// [`Git::show`] for the small files that several rules read at the base (a
    /// changed `tsconfig.json`, a config it extends): read once and kept.
    pub fn show_config(&self, path: &str) -> Option<String> {
        let read = self.reader()?;
        read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Reads the root-relative `paths` at the base with one `git cat-file
    /// --batch` process, for [`Git::show_config`] and [`Git::reader`] to
    /// answer from: a diff that touches a hundred configs otherwise starts
    /// a hundred `git show` processes. What cannot be read this way
    /// (several merge bases, a path with a line break, a failed
    /// run) is left to the single reads.
    pub fn preload(&self, paths: &[&str]) {
        use std::io::Write as _;
        if self.bases.len() > 1 {
            return;
        }
        let wanted: Vec<&str> = match self.configs.lock() {
            Ok(c) => paths
                .iter()
                .copied()
                .filter(|p| !c.contains_key(*p) && !p.contains('\n'))
                .collect(),
            Err(_) => return,
        };
        if wanted.len() < 2 {
            return;
        }
        let input: String = wanted
            .iter()
            .map(|p| format!("{}:{}{p}\n", self.rev, self.prefix))
            .collect();
        let Ok(mut child) = Command::new("git")
            .arg("-C")
            .arg(&self.toplevel)
            .args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return;
        };
        // Written from another thread: git answers while it reads.
        let stdin = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(input.as_bytes());
            }
        });
        let out = child.wait_with_output();
        let _ = writer.join();
        let Ok(out) = out else {
            return;
        };
        // Per request, in order: `<object> <type> <size>\n<bytes>\n`, or `<request>
        // missing\n`.
        let mut read: Vec<(&str, Option<std::sync::Arc<[u8]>>)> = Vec::new();
        let mut rest = out.stdout.as_slice();
        for p in &wanted {
            let Some(end) = rest.iter().position(|&b| b == b'\n') else {
                break;
            };
            let header = String::from_utf8_lossy(&rest[..end]).into_owned();
            rest = &rest[end + 1..];
            if header.ends_with(" missing") {
                read.push((p, None));
                continue;
            }
            let mut fields = header.rsplitn(3, ' ');
            let (Some(size), Some(kind)) = (
                fields.next().and_then(|s| s.parse::<usize>().ok()),
                fields.next(),
            ) else {
                break;
            };
            if rest.len() < size + 1 {
                break;
            }
            // A directory at the base is no file.
            read.push((
                p,
                (kind == "blob").then(|| std::sync::Arc::from(&rest[..size])),
            ));
            rest = &rest[size + 1..];
        }
        if let Ok(mut c) = self.configs.lock() {
            for (p, bytes) in read {
                c.entry(p.to_owned()).or_insert(bytes);
            }
        }
    }

    /// Which of the root-relative `dirs` are directories in the base's tree,
    /// with one `git cat-file --batch-check` process. With several merge
    /// bases: in the tree of every one of them (the changes are the union
    /// of the diffs against each, and a directory that one base lacks was
    /// made since that base). `None` when that cannot be told (a failed
    /// run).
    pub fn dirs_at_base(&self, dirs: &[&str]) -> Option<Vec<bool>> {
        use std::io::Write as _;
        // (A name with a line break cannot be asked for this way: not known.)
        if dirs.iter().any(|d| d.contains('\n')) {
            return None;
        }
        // Base by base, the directories in order.
        let input: String = self
            .bases
            .iter()
            .flat_map(|base| {
                dirs.iter()
                    .map(move |d| format!("{base}:{}{d}\n", self.prefix))
            })
            .collect();
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.toplevel)
            .args(["cat-file", "--batch-check"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(input.as_bytes());
            }
        });
        let out = child.wait_with_output();
        let _ = writer.join();
        let out = out.ok().filter(|o| o.status.success())?;
        // Per request, in order: `<object> <type> <size>`, or `<request> missing`.
        let text = String::from_utf8_lossy(&out.stdout);
        let answers: Vec<bool> = text
            .lines()
            .map(|l| l.split(' ').nth(1) == Some("tree") && !l.ends_with(" missing"))
            .collect();
        if dirs.is_empty() || answers.len() != dirs.len() * self.bases.len() {
            return (dirs.is_empty() && answers.is_empty()).then(Vec::new);
        }
        // There at the base: at every base.
        Some(
            (0..dirs.len())
                .map(|i| answers.chunks(dirs.len()).all(|of_base| of_base[i]))
                .collect(),
        )
    }

    /// `git diff --raw --no-renames <base>`: tracked files changed between the
    /// base and the working tree, root-relative. A two-tree diff: no commit
    /// between them is looked at. With several merge bases, the union of
    /// the diffs against each (a path that one base has and another lacks
    /// is both modified and added).
    ///
    /// The raw format has each path's mode at the base: the ones that are
    /// symbolic links there are kept for [`Git::links_at_base`].
    pub fn tracked_changes(&self) -> anyhow::Result<Vec<Change>> {
        let mut out = Vec::new();
        let mut links = BTreeSet::new();
        for base in &self.bases {
            let diff = git(
                &self.toplevel,
                &["diff", "--raw", "--no-renames", "-z", base, "--"],
            )?;
            // Per path: `:<mode at the base> <mode now> <object> <object> <status>`, then
            // the path.
            let mut it = diff.split('\0').filter(|s| !s.is_empty());
            while let Some(meta) = it.next() {
                let Some(path) = it.next() else { break };
                let kind = match meta.rsplit(' ').next().and_then(|s| s.chars().next()) {
                    Some('A') => ChangeKind::Added,
                    Some('D') => ChangeKind::Deleted,
                    _ => ChangeKind::Modified,
                };
                if let Some(rel) = path.strip_prefix(&self.prefix) {
                    if meta.starts_with(":120000 ") {
                        links.insert(rel.to_owned());
                    }
                    out.push(Change::new(kind, rel));
                }
            }
        }
        if let Ok(mut l) = self.links.lock() {
            *l = links;
        }
        if self.bases.len() > 1 {
            out.sort();
            out.dedup();
        }
        Ok(out)
    }

    /// The changed paths (root-relative) that are symbolic links at the base,
    /// as the diff of [`Git::tracked_changes`] said: git stores a link as a
    /// file whose contents are where it leads ([`Git::show`] gives that
    /// text). Empty when no diff was computed (changes given as paths): a
    /// link of the base is then taken for a file, behind which nothing
    /// loads at the base.
    pub fn links_at_base(&self) -> Vec<String> {
        match self.links.lock() {
            Ok(l) => l.iter().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Sets the changed paths that are symbolic links at the base, when the
    /// changes come from a [`turborepo_trace_graph::ChangeSource`] instead
    /// of [`Git::tracked_changes`].
    pub fn set_links_at_base(&self, links: Vec<String>) {
        if let Ok(mut l) = self.links.lock() {
            *l = links.into_iter().collect();
        }
    }

    /// `git ls-files -z --full-name` in `root`: the tracked list
    /// [`Git::untracked_with`] takes, so that callers can run it ahead
    /// (concurrently with other work).
    pub fn tracked_files(root: &Path) -> anyhow::Result<std::collections::HashSet<String>> {
        let out = git(root, &["ls-files", "-z", "--full-name"])?;
        Ok(out
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Untracked files (not ignored), root-relative. With `walked` (the graph
    /// build's `.gitignore`-respecting walk of the root), they are the
    /// walked files git does not track, which avoids git's own untracked
    /// scan; otherwise `git ls-files --others`. `skip_dir` (root-relative,
    /// such as the graph directory `.turbotrace`) is excluded from
    /// git's scan.
    /// `tracked`: git's tracked list when already computed
    /// ([`Git::tracked_files`]).
    ///
    /// A symbolic link is an entry of the tree like a file (git tracks it as
    /// one): the walk passes it without following it and lists it apart
    /// (`links`, with `walked`), and an untracked one is an added path too.
    /// What was looked for through a new link to a directory is behind that
    /// path and nowhere else in the diff.
    pub fn untracked_with(
        &self,
        root: &Path,
        walked: Option<(&[String], &[String])>,
        skip_dir: Option<&str>,
        tracked: Option<anyhow::Result<std::collections::HashSet<String>>>,
    ) -> anyhow::Result<Vec<Change>> {
        let mut out = Vec::new();
        match walked {
            Some((walked, links)) => {
                let tracked = match tracked {
                    Some(t) => t?,
                    None => Self::tracked_files(root)?,
                };
                for f in walked.iter().chain(links) {
                    let known = if self.prefix.is_empty() {
                        tracked.contains(f.as_str())
                    } else {
                        tracked.contains(format!("{}{f}", self.prefix).as_str())
                    };
                    if !known {
                        out.push(Change::new(ChangeKind::Added, f.as_str()));
                    }
                }
            }
            None => {
                let mut args = vec![
                    "ls-files",
                    "--others",
                    "--exclude-standard",
                    "-z",
                    "--full-name",
                ];
                let exclude = skip_dir.map(|d| format!("--exclude=/{}/", d.trim_end_matches('/')));
                if let Some(x) = &exclude {
                    args.push(x);
                }
                let untracked = git(root, &args)?;
                for p in untracked.split('\0').filter(|s| !s.is_empty()) {
                    if let Some(rel) = p.strip_prefix(&self.prefix) {
                        out.push(Change::new(ChangeKind::Added, rel));
                    }
                }
            }
        }
        Ok(out)
    }
}

pub(crate) fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !o.status.success() {
        anyhow::bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// A package changed for some importers, with the reason.
#[derive(Clone, Debug)]
pub struct PackageChange {
    /// The graph package.
    pub package: PackageId,
    /// Importers it changed for.
    pub importers: Vec<PackageId>,
    /// The file that caused it.
    pub source: String,
    /// `lockfile diff`, `lockfile changed, no base`, `package.json range`.
    pub reason: &'static str,
}

/// Everything derived from lockfile and manifest changes.
#[derive(Debug, Default)]
pub struct Effects {
    /// Changed packages.
    pub packages: Vec<PackageChange>,
    /// File changes to drop (lockfiles diffed precisely).
    pub drop: BTreeSet<String>,
    /// Readable notes for `--format json` and `--stats`.
    pub notes: Vec<String>,
    /// Direct dependencies whose lockfile resolution changed for one importer
    /// (added, removed, pointing at other content, such as a workspace link
    /// that became a registry version, or at the same package with another
    /// dependency closure, such as another peer variant): the nodes of that
    /// importer that link the name are affected, also when the package they
    /// now link did not change itself.
    pub importer_deps: Vec<ImporterDep>,
}

/// A direct dependency whose resolution changed for one importer
/// ([`Effects::importer_deps`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImporterDep {
    /// Root of the importing first-party package (`""` for the repository
    /// root).
    pub importer: String,
    /// The dependency name.
    pub name: String,
    /// The lockfile that says so.
    pub source: String,
}

/// Ecosystem of a lockfile name, if it is one.
fn ecosystem(name: &str) -> Option<&'static str> {
    Some(match name {
        "pnpm-lock.yaml"
        | "package-lock.json"
        | "npm-shrinkwrap.json"
        | "yarn.lock"
        | "bun.lock" => "npm",
        "Cargo.lock" => "crates",
        "go.sum" | "go.mod" => "go",
        "uv.lock" | "poetry.lock" | "Pipfile.lock" => "pypi",
        _ if name.starts_with("requirements") && name.ends_with(".txt") => "pypi",
        _ => return None,
    })
}

fn split(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}

fn under(dir: &str, root: &str) -> bool {
    dir.is_empty() || root == dir || root.starts_with(&format!("{dir}/"))
}

fn canon(eco: &str, name: &str) -> String {
    if eco == "pypi" {
        name.to_ascii_lowercase().replace(['_', '.'], "-")
    } else {
        name.to_owned()
    }
}

/// `(ecosystem, name)` of a graph package name (`pypi:requests` → `("pypi",
/// "requests")`).
fn eco_of(name: &str) -> (&str, &str) {
    match name.split_once(':') {
        Some((e, n)) if !e.contains('/') && !e.starts_with('@') => (e, n),
        _ => ("npm", name),
    }
}

/// Parses one side of a lockfile change. For Go, `mod` and `sum` are the
/// module's `go.mod` and `go.sum`.
fn parse(name: &str, text: Option<&str>, go_sum: Option<&str>) -> Option<Lockfile> {
    let text = text?;
    if name == "go.mod" {
        return turborepo_trace_lockfile::gosum::parse(text, go_sum).ok();
    }
    turborepo_trace_lockfile::parse_named(name, text)
        .ok()
        .or_else(|| {
            // `requirements*.txt` names are detected by prefix; anything else falls
            // through.
            turborepo_trace_lockfile::detect(name, text)
                .ok()
                .filter(|k| *k == LockfileKind::Requirements)
                .and_then(|k| turborepo_trace_lockfile::parse(k, text).ok())
        })
}

/// Computes the package effects of `changes` (see the module docs).
/// Old and new sides of changed non-Go lockfiles, parsed ahead of [`effects`]
/// (they don't need the graph, so this overlaps with the graph refresh).
#[derive(Default)]
pub struct PreParsed(HashMap<String, (Option<Lockfile>, Option<Lockfile>)>);

/// Parses the old (`git show`) and new sides of every changed non-Go lockfile
/// in `changes`.
pub fn preparse(root: &Path, changes: &[Change], git: Option<&Git>) -> PreParsed {
    let todo: Vec<&Change> = changes
        .iter()
        .filter(|c| matches!(ecosystem(split(&c.path).1), Some(e) if e != "go"))
        .collect();
    let parsed = std::thread::scope(|s| {
        let hs: Vec<_> = todo
            .iter()
            .map(|c| {
                let name = split(&c.path).1;
                let old = s
                    .spawn(move || git.and_then(|g| parse(name, g.show(&c.path).as_deref(), None)));
                let new = s.spawn(move || {
                    let text = std::fs::read_to_string(root.join(&c.path)).ok();
                    parse(name, text.as_deref(), None)
                });
                (c.path.clone(), old, new)
            })
            .collect();
        hs.into_iter()
            .map(|(p, o, n)| {
                (
                    p,
                    (
                        o.join().unwrap_or_else(|p| std::panic::resume_unwind(p)),
                        n.join().unwrap_or_else(|p| std::panic::resume_unwind(p)),
                    ),
                )
            })
            .collect()
    });
    PreParsed(parsed)
}

// Invariant: `precise` holds only when `new` is Some.
#[allow(clippy::expect_used)]
pub fn effects(g: &Graph, changes: &[Change], git: Option<&Git>, mut pre: PreParsed) -> Effects {
    let mut fx = Effects::default();
    let root = Path::new(g.root());
    // First-party packages by root, external packages by (ecosystem, canonical
    // name).
    let mut first: Vec<(String, PackageId)> = Vec::new();
    let mut ext: HashMap<(String, String), Vec<(String, PackageId)>> = HashMap::new();
    for i in 0..g.package_count() {
        let p = PackageId(i);
        let info = g.package_info(p);
        match info.version {
            PackageVersion::Workspace => first.push((info.root.to_owned(), p)),
            PackageVersion::Version(v) => {
                let (eco, name) = eco_of(info.name);
                ext.entry((eco.to_owned(), canon(eco, name)))
                    .or_default()
                    .push((v.to_owned(), p));
            }
        }
    }
    // Lockfile directories per ecosystem (governance), from the graph and the
    // change set.
    let mut lock_dirs: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    let lockfiles = g.manifest().lockfiles.iter().map(|l| l.path.as_str());
    for p in lockfiles.chain(changes.iter().map(|c| c.path.as_str())) {
        let (dir, name) = split(p);
        if let Some(eco) = ecosystem(name) {
            lock_dirs.entry(eco).or_default().insert(dir.to_owned());
        }
    }
    let governed = |eco: &str, dir: &str| -> Vec<PackageId> {
        let deeper: Vec<&String> = lock_dirs
            .get(eco)
            .into_iter()
            .flatten()
            .filter(|d| d.as_str() != dir && under(dir, d))
            .collect();
        first
            .iter()
            .filter(|(r, _)| under(dir, r) && !deeper.iter().any(|d| under(d, r)))
            .map(|(_, p)| *p)
            .collect()
    };
    let read_new = |path: &str| std::fs::read_to_string(root.join(path)).ok();
    let mut js_lock_changed = false;
    let mut go_done: BTreeSet<String> = BTreeSet::new();

    for c in changes {
        let (dir, name) = split(&c.path);
        let Some(eco) = ecosystem(name) else { continue };
        if eco == "npm" {
            js_lock_changed = true;
        }
        let importers = governed(eco, dir);
        if importers.is_empty() {
            continue;
        }
        // Old and new sides.
        // For Go: whether `go.sum` lists the same module hashes on both sides.
        let mut go_sum_same = false;
        let (old, new, key_name) = if eco == "go" {
            if !go_done.insert(dir.to_owned()) {
                continue;
            }
            let m = |n: &str| {
                if dir.is_empty() {
                    n.to_owned()
                } else {
                    format!("{dir}/{n}")
                }
            };
            let (gm, gs) = (m("go.mod"), m("go.sum"));
            let old_sum = git.and_then(|g| g.show(&gs));
            let new_sum = read_new(&gs);
            go_sum_same = git.is_some()
                && crate::gosumdiff::same_lines(old_sum.as_deref(), new_sum.as_deref());
            let old = git.and_then(|g| {
                let om = g.show(&gm)?;
                parse("go.mod", Some(&om), old_sum.as_deref())
            });
            let new = read_new(&gm).and_then(|nm| parse("go.mod", Some(&nm), new_sum.as_deref()));
            (old, new, gs)
        } else {
            // Old and new sides in parallel (lockfiles can be megabytes).
            let (old, new) = pre.0.remove(&c.path).unwrap_or_else(|| {
                std::thread::scope(|s| {
                    let old =
                        s.spawn(|| git.and_then(|g| parse(name, g.show(&c.path).as_deref(), None)));
                    let new = parse(name, read_new(&c.path).as_deref(), None);
                    (
                        old.join().unwrap_or_else(|p| std::panic::resume_unwind(p)),
                        new,
                    )
                })
            });
            (old, new, c.path.clone())
        };
        let precise = matches!((&old, &new), (Some(_), Some(_)))
            || (c.kind == ChangeKind::Added && new.is_some() && git.is_some());
        if precise {
            let new = new.as_ref().expect("precise");
            let ids: Vec<String> = match &old {
                Some(o) => {
                    let d: LockDiff = turborepo_trace_lockfile::diff(o, new);
                    for imp in &d.importers {
                        let importer = match (dir, imp.path.as_str()) {
                            (d, "." | "") => d.to_owned(),
                            ("", p) => p.to_owned(),
                            (d, p) => format!("{d}/{p}"),
                        };
                        // `changed`: another package, added or removed. `affected`: the same
                        // package with another closure, also when no package changed at all
                        // (the importer moved to another peer variant of its test runner).
                        let names: BTreeSet<&str> = imp
                            .changed
                            .iter()
                            .map(|ch| ch.name.as_str())
                            .chain(imp.affected.iter().map(String::as_str))
                            .collect();
                        for name in names {
                            fx.importer_deps.push(ImporterDep {
                                importer: importer.clone(),
                                name: name.to_owned(),
                                source: c.path.clone(),
                            });
                        }
                    }
                    d.affected_packages()
                        .closure_changed
                        .iter()
                        .cloned()
                        .collect()
                }
                // A new lockfile: everything it locks is new.
                None => new.packages().iter().map(|p| p.id.clone()).collect(),
            };
            let mut n = 0usize;
            for id in &ids {
                let Some(lp) = new.package(id) else { continue };
                let Some(list) = ext.get(&(eco.to_owned(), canon(eco, &lp.name))) else {
                    continue;
                };
                for (v, p) in list {
                    if *v == lp.version
                        || v.trim_start_matches('v') == lp.version.trim_start_matches('v')
                    {
                        n += 1;
                        fx.packages.push(PackageChange {
                            package: *p,
                            importers: importers.clone(),
                            source: c.path.clone(),
                            reason: "lockfile diff",
                        });
                    }
                }
            }
            fx.notes.push(format!(
                "{}: {} package(s) with a changed closure, {} in the graph",
                c.path,
                ids.len(),
                n
            ));
            if eco == "pypi" && !ids.is_empty() {
                // A Python lockfile stays a file change when something it locks
                // changed. A Python import that no installed
                // distribution provides has the declarations
                // and lockfiles of its project as inputs (which distribution
                // provides a module is known only once it is installed), so a
                // bump reaches such a file through the lockfile
                // itself, not through a package node.
            } else if eco != "go" {
                fx.drop.insert(c.path.clone());
            } else if go_sum_same {
                // `go.mod` always stays a file change. `go.sum` stays one whenever a module
                // hash was added, removed or changed (see `gosumdiff`).
                fx.drop.insert(key_name);
            }
        } else {
            let mut n = 0usize;
            for ((e, _), list) in &ext {
                if e != eco {
                    continue;
                }
                for (_, p) in list {
                    n += 1;
                    fx.packages.push(PackageChange {
                        package: *p,
                        importers: importers.clone(),
                        source: c.path.clone(),
                        reason: "lockfile changed, no base",
                    });
                }
            }
            // With a base revision, the cause is a side that is missing or does not parse.
            let why = match (git.is_some(), old.is_some(), new.is_some()) {
                (false, _, _) => "changed without old contents (use --since)",
                (true, _, false) => "the new contents are missing or do not parse",
                (true, false, true) => "the old contents do not parse",
                (true, true, true) => "changed",
            };
            fx.notes.push(format!(
                "{}: {why}: all {n} {eco} package(s) changed for {} importer(s)",
                c.path,
                importers.len()
            ));
        }
    }

    // package.json dependency ranges.
    if let Some(git) = git
        && !js_lock_changed
    {
        for c in changes.iter().filter(|c| c.kind == ChangeKind::Modified) {
            let (dir, name) = split(&c.path);
            if name != "package.json" {
                continue;
            }
            let Some(&(_, importer)) = first.iter().find(|(r, _)| r == dir) else {
                continue;
            };
            let deps = |text: Option<String>| -> BTreeMap<String, String> {
                let mut m = BTreeMap::new();
                let Some(v) = text.and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                else {
                    return m;
                };
                for k in [
                    "dependencies",
                    "devDependencies",
                    "optionalDependencies",
                    "peerDependencies",
                ] {
                    if let Some(o) = v.get(k).and_then(|x| x.as_object()) {
                        for (n, r) in o {
                            m.insert(format!("{k}:{n}"), r.to_string());
                        }
                    }
                }
                m
            };
            let old = deps(git.show(&c.path));
            let new = deps(read_new(&c.path));
            let mut names: BTreeSet<&str> = BTreeSet::new();
            for (k, v) in old.iter().chain(new.iter()) {
                if old.get(k) != Some(v) || new.get(k) != Some(v) {
                    names.insert(k.split_once(':').map_or(k.as_str(), |x| x.1));
                }
            }
            for n in &names {
                if let Some(list) = ext.get(&("npm".to_owned(), (*n).to_owned())) {
                    for (_, p) in list {
                        fx.packages.push(PackageChange {
                            package: *p,
                            importers: vec![importer],
                            source: c.path.clone(),
                            reason: "package.json range",
                        });
                    }
                }
            }
            if !names.is_empty() {
                fx.notes.push(format!(
                    "{}: dependency range(s) changed without a lockfile change: {}",
                    c.path,
                    names.iter().copied().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    fx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(ecosystem("pnpm-lock.yaml"), Some("npm"));
        assert_eq!(ecosystem("requirements-dev.txt"), Some("pypi"));
        assert_eq!(ecosystem("go.sum"), Some("go"));
        assert_eq!(ecosystem("package.json"), None);
        assert!(under("", "apps/web"));
        assert!(under("apps", "apps/web"));
        assert!(!under("app", "apps/web"));
        assert_eq!(eco_of("pypi:requests"), ("pypi", "requests"));
        assert_eq!(eco_of("@scope/x"), ("npm", "@scope/x"));
        assert_eq!(canon("pypi", "Foo_Bar.baz"), "foo-bar-baz");
    }
}
