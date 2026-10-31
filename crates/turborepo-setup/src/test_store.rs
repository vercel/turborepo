//! A fixture tool store for adapter tests: a private store per test, and the
//! setup session `turbo setup` would hold over it for one repository.
#![allow(clippy::unwrap_used)]
// Only Unix adapter tests exercise force, check and publication helpers.
#![cfg_attr(windows, allow(dead_code))]

use std::{
    cell::RefCell,
    ops::Deref,
    path::{Path, PathBuf},
};

use turborepo_tool_install::{Error, Reuse, Selection, SetupSession, Tool, UserStore};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Unchanged,
    Replaced,
}

/// The repository's current environment.
pub(crate) struct Current {
    pub tools: Vec<Tool>,
    pub bin: PathBuf,
}

pub(crate) struct TestStore {
    session: SetupSession<'static>,
    store: &'static UserStore,
    repo: PathBuf,
    current: RefCell<Option<Selection>>,
    _home: tempfile::TempDir,
}

impl TestStore {
    pub(crate) fn open(repo: &Path) -> Self {
        let home = tempfile::tempdir().unwrap();
        // Each test owns one tiny store handle for its whole run.
        let store: &'static UserStore = Box::leak(Box::new(
            UserStore::at(&home.path().join("store"), repo).unwrap(),
        ));
        Self {
            session: store.begin_setup(repo, Reuse::Sealed).unwrap(),
            store,
            repo: repo.canonicalize().unwrap(),
            current: RefCell::default(),
            _home: home,
        }
    }

    pub(crate) fn store(&self) -> &UserStore {
        self.store
    }

    /// Converges the repository onto `desired`, like setup. `Replaced` when
    /// a tree was staged or the repository moved to another environment.
    pub(crate) fn reconcile(
        &self,
        desired: &[Tool],
        stage: impl FnMut(&Tool, &Path) -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        self.ensure(&self.session, desired, stage)
    }

    pub(crate) fn reconcile_checked(
        &self,
        desired: &[Tool],
        stage: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        before: impl FnOnce() -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        before()?;
        self.reconcile(desired, stage)
    }

    pub(crate) fn force_reconcile_checked(
        &self,
        desired: &[Tool],
        stage: impl FnMut(&Tool, &Path) -> Result<(), Error>,
        before: impl FnOnce() -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        before()?;
        self.force_reconcile(desired, stage)
    }

    /// `turbo setup --force`: re-verify every tree and replace what changed.
    pub(crate) fn force_reconcile(
        &self,
        desired: &[Tool],
        stage: impl FnMut(&Tool, &Path) -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        let session = self.store.begin_setup(&self.repo, Reuse::Verified)?;
        self.ensure(&session, desired, stage)
    }

    fn ensure(
        &self,
        session: &SetupSession<'_>,
        desired: &[Tool],
        stage: impl FnMut(&Tool, &Path) -> Result<(), Error>,
    ) -> Result<Outcome, Error> {
        let selection = Selection::new(desired.to_vec())?;
        let ensured = session.ensure(&selection, stage, |_| Ok(()))?;
        let changed = !ensured.installed.is_empty()
            || self.current.borrow().as_ref().map(Selection::key) != Some(&ensured.env);
        *self.current.borrow_mut() = Some(selection);
        Ok(if changed {
            Outcome::Replaced
        } else {
            Outcome::Unchanged
        })
    }

    /// The current environment's `bin`, as run activates it.
    pub(crate) fn bin(&self) -> PathBuf {
        self.current().unwrap().expect("nothing reconciled yet").bin
    }

    /// The environment the repository was last converged onto, if any.
    pub(crate) fn current(&self) -> Result<Option<Current>, Error> {
        let current = self.current.borrow();
        let Some(selection) = current.as_ref() else {
            return Ok(None);
        };
        let activation = self.store.activate(&self.repo, selection.key())?;
        Ok(Some(Current {
            tools: selection.tools().to_vec(),
            bin: activation.bin().to_owned(),
        }))
    }

    pub(crate) fn is_current(&self, desired: &[Tool]) -> Result<bool, Error> {
        let desired = Selection::new(desired.to_vec())?;
        Ok(self.current.borrow().as_ref() == Some(&desired))
    }

    /// The installed tree of `tool`.
    pub(crate) fn tree_of(&self, tool: &Tool) -> PathBuf {
        self.session
            .tree(tool)
            .unwrap()
            .expect("tool is not installed")
    }

    /// Every tree and environment in the store, so tests can see that a
    /// failed setup published nothing.
    pub(crate) fn published(&self) -> Vec<String> {
        let mut names = Vec::new();
        for dir in ["trees", "envs"] {
            for entry in std::fs::read_dir(self.store.root().join(dir)).unwrap() {
                let name = entry.unwrap().file_name().into_string().unwrap();
                names.push(format!("{dir}/{name}"));
            }
        }
        names.sort();
        names
    }

    /// `turbo setup --check` for `desired`.
    pub(crate) fn check(&self, desired: &[Tool]) -> Result<(), Error> {
        self.store.check(&Selection::new(desired.to_vec())?)
    }
}

impl Deref for TestStore {
    type Target = SetupSession<'static>;

    fn deref(&self) -> &Self::Target {
        &self.session
    }
}
