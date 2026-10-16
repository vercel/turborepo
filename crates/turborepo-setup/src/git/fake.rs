//! In-memory [`Git`] for unit tests. Answers are configured per test; nothing
//! is spawned. Unless an ignored set is configured, `check_ignore` models only
//! a root `.gitignore` line that ignores `/.turbo/`, so fixtures can still
//! remove that rule to exercise setup's ignore guards.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use super::{Git, Reply, git_spawn_error};

/// An injected Git failure, applied to every query until cleared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// `git` could not be spawned because it is not on PATH.
    Spawn,
    /// Git exited with status 128 and this stderr (for example, a broken
    /// `.git`).
    Fatal(String),
}

struct State {
    toplevels: BTreeMap<PathBuf, PathBuf>,
    ignored: Option<BTreeSet<String>>,
    tracked: BTreeSet<String>,
    failure: Option<Failure>,
    queries: usize,
}

/// Stateless tests can share one `static` instance.
pub struct FakeGit {
    state: Mutex<State>,
    hook: Mutex<Option<Hook>>,
}

type Hook = Arc<dyn Fn() + Send + Sync>;

impl Default for FakeGit {
    fn default() -> Self {
        Self::new()
    }
}

fn key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

fn covers(entry: &str, path: &str) -> bool {
    entry == path || (entry.ends_with('/') && path.starts_with(entry))
}

fn lines<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
    paths
        .into_iter()
        .flat_map(|path| [path.as_bytes(), b"\n"].concat())
        .collect()
}

impl FakeGit {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                toplevels: BTreeMap::new(),
                ignored: None,
                tracked: BTreeSet::new(),
                failure: None,
                queries: 0,
            }),
            hook: Mutex::new(None),
        }
    }

    /// A fresh fake for a caller that stores `Arc<dyn Git>`.
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Answer `toplevel(dir)` with `toplevel` instead of `dir` itself. The
    /// path is reported as given, so a symlink keeps its spelling.
    pub fn set_toplevel(&self, dir: &Path, toplevel: &Path) {
        self.state().toplevels.insert(key(dir), toplevel.to_owned());
    }

    /// Report exactly these root-relative paths (or `dir/` prefixes) as
    /// ignored, replacing the `.gitignore` model.
    pub fn set_ignored<'a>(&self, paths: impl IntoIterator<Item = &'a str>) {
        self.state().ignored = Some(paths.into_iter().map(str::to_owned).collect());
    }

    /// Record a root-relative path as tracked, like `git add --force`.
    pub fn track(&self, path: &str) {
        self.state().tracked.insert(path.to_owned());
    }

    /// Fail every later query, or stop failing with `None`.
    pub fn fail(&self, failure: Option<Failure>) {
        self.state().failure = failure;
    }

    /// Run `hook` before answering each later query, so a test can change
    /// repository state at the exact point setup asks Git again.
    pub fn on_query(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self
            .hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(hook));
    }

    /// The number of queries answered so far, including failed ones.
    pub fn queries(&self) -> usize {
        self.state().queries
    }

    fn begin(&self) -> io::Result<Option<Reply>> {
        let hook = self
            .hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook();
        }
        let mut state = self.state();
        state.queries += 1;
        match &state.failure {
            None => Ok(None),
            Some(Failure::Spawn) => Err(git_spawn_error(io::ErrorKind::NotFound.into())),
            Some(Failure::Fatal(stderr)) => Ok(Some(Reply {
                code: Some(128),
                stdout: Vec::new(),
                stderr: stderr.as_bytes().to_vec(),
            })),
        }
    }
}

fn ignore_rule(root: &Path) -> io::Result<bool> {
    match fs::read_to_string(root.join(".gitignore")) {
        Ok(text) => Ok(text
            .lines()
            .any(|line| matches!(line.trim(), "/.turbo/" | ".turbo/" | "/.turbo" | ".turbo"))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

impl Git for FakeGit {
    fn toplevel(&self, dir: &Path) -> io::Result<Reply> {
        if let Some(reply) = self.begin()? {
            return Ok(reply);
        }
        let toplevel = self
            .state()
            .toplevels
            .get(&key(dir))
            .cloned()
            .unwrap_or_else(|| dir.to_owned());
        Ok(Reply {
            code: Some(0),
            stdout: format!("{}\n", toplevel.display()).into_bytes(),
            stderr: Vec::new(),
        })
    }

    fn check_ignore(&self, root: &Path, paths: &[&str]) -> io::Result<Reply> {
        if let Some(reply) = self.begin()? {
            return Ok(reply);
        }
        let configured = self.state().ignored.clone();
        let rule = configured.is_none() && ignore_rule(root)?;
        let state = self.state();
        // Like Git, tracked paths, and directories holding them, are never
        // reported as ignored.
        let ignored: Vec<&str> = paths
            .iter()
            .copied()
            .filter(|path| !state.tracked.iter().any(|file| covers(path, file)))
            .filter(|path| match &configured {
                Some(set) => set.iter().any(|entry| covers(entry, path)),
                None => rule && path.starts_with(".turbo/"),
            })
            .collect();
        Ok(Reply {
            code: Some(if ignored.is_empty() { 1 } else { 0 }),
            stdout: lines(ignored),
            stderr: Vec::new(),
        })
    }

    fn ls_files(&self, _root: &Path, paths: &[&str]) -> io::Result<Reply> {
        if let Some(reply) = self.begin()? {
            return Ok(reply);
        }
        let state = self.state();
        let tracked = state
            .tracked
            .iter()
            .filter(|file| paths.iter().any(|path| covers(path, file)))
            .map(String::as_str);
        Ok(Reply {
            code: Some(0),
            stdout: lines(tracked),
            stderr: Vec::new(),
        })
    }
}

/// The `.git` directory entries setup's filesystem checks read (`HEAD`,
/// `config`), without running `git init`. Pair it with a [`FakeGit`].
pub fn fake_repository(root: &Path) -> io::Result<()> {
    let git = root.join(".git");
    fs::create_dir_all(git.join("objects"))?;
    fs::create_dir_all(git.join("refs/heads"))?;
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n")?;
    fs::write(
        git.join("config"),
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n",
    )
}
