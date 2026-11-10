//! The place of the checkout as an input of a project's test discovery.
//!
//! jest matches its test patterns against the absolute path of a file
//! ([`turborepo_trace_frameworks::location`]): which files are tests is a fact
//! about where the repository is checked out whenever a pattern can match the
//! directories above the root. For almost every project it cannot, and then
//! nothing about the place is part of a graph: the same tree gives the same
//! shards under the same keys wherever it is.
//!
//! So each project whose runner matches absolute paths leaves a record in the
//! shards of the files its config is consulted for
//! ([`crate::shard::LocatedRec`]): its discovery as the config writes it,
//! without the place, and its **state**, which is empty when the place decides
//! nothing and the absolute root when it does (the project is
//! location-dependent there, or its discovery cannot be kept without the
//! place). The state is part of the shard's key like the state of every other
//! observation: a refresh evaluates the stored discovery at the root it runs at
//! ([`Places::state`]), so a graph built or packed at one place and refreshed
//! or restored at another keeps a shard exactly when the place decides nothing
//! at both, and traces it again otherwise. A shard that holds a root is also
//! one that `graph pack` lists as naming its checkout
//! ([`crate::artifact::Located`]).
//!
//! The evaluation is decided from the patterns alone for an ordinary project
//! (no file is matched), and memoized per discovery and build.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use turborepo_trace_frameworks::location::{self, Tree};

/// The states of the location records of one build.
pub(crate) struct Places {
    repo: PathBuf,
    /// The walked files (repo-relative, sorted).
    files: Arc<Vec<String>>,
    /// Whether the tree holds the checkout's own path again
    /// ([`Tree::sharing`]).
    chain: Arc<OnceLock<bool>>,
    memo: dashmap::DashMap<String, Arc<str>>,
}

impl Places {
    pub(crate) fn new(repo: &Path, files: Arc<Vec<String>>) -> Self {
        Places {
            repo: repo.to_path_buf(),
            files,
            chain: Arc::default(),
            memo: Default::default(),
        }
    }

    /// The state of the discovery `spec` ([`location::ProjectSpec::to_text`])
    /// in this checkout: empty when the place decides nothing, else the
    /// absolute root.
    pub(crate) fn state(&self, spec: &str) -> Arc<str> {
        if let Some(s) = self.memo.get(spec) {
            return Arc::clone(&s);
        }
        let tree = Tree::sharing(&self.files, &self.chain);
        let state: Arc<str> = Arc::from(location::state(spec, &self.repo, &tree));
        self.memo.insert(spec.to_owned(), Arc::clone(&state));
        state
    }
}
