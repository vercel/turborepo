//! In-memory [`Git`] for unit tests. Answers are configured per test; nothing
//! is spawned.

use std::{
    collections::BTreeMap,
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

impl FakeGit {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                toplevels: BTreeMap::new(),
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
