use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use globwatch::{GlobSender, GlobWatcher, StopToken, Watcher};
use itertools::Itertools;
use log::{trace, warn};
use notify::RecommendedWatcher;

/// Tracks changes for a given hash. A hash is a unique identifier for a set of
/// files. Given a hash and a set of globs to track, this will watch for file
/// changes and allow the user to query for changes.
#[derive(Clone)]
pub struct HashGlobWatcher<T: Watcher> {
    hash_globs: Arc<Mutex<HashMap<Arc<String>, Glob>>>,
    glob_statuses: Arc<Mutex<HashMap<Arc<String>, HashSet<Arc<String>>>>>,
    watcher: Arc<Mutex<Option<GlobWatcher<T>>>>,
    config: GlobSender,
}

#[derive(Clone)]
pub struct Glob {
    include: HashSet<Arc<String>>,
    exclude: HashSet<Arc<String>>,
}

impl HashGlobWatcher<RecommendedWatcher> {
    pub fn new(flush_folder: PathBuf) -> Result<Self, globwatch::Error> {
        let (watcher, config) = GlobWatcher::new(flush_folder)?;
        Ok(Self {
            hash_globs: Default::default(),
            glob_statuses: Default::default(),
            watcher: Arc::new(Mutex::new(Some(watcher))),
            config,
        })
    }
}

impl<T: Watcher> HashGlobWatcher<T> {
    /// Watches a given path, using the flush_folder as temporary storage to
    /// make sure that file events are handled in the appropriate order.
    pub async fn watch(&self, root_folder: PathBuf, token: StopToken) {
        let start_globs = {
            let lock = self.hash_globs.lock().expect("no panic");
            lock.iter()
                .flat_map(|(_, g)| &g.include)
                .cloned()
                .collect::<Vec<_>>()
        };

        let mut stream = match self.watcher.lock().expect("no panic").take() {
            Some(watcher) => watcher.into_stream(token),
            None => {
                warn!("watcher already consumed");
                return;
            }
        };

        // watch all the globs currently in the map
        for glob in start_globs {
            self.config.include(glob.to_string()).await.unwrap();
        }

        while let Some(Ok(event)) = stream.next().await {
            trace!("event: {:?}", event);

            let repo_relative_paths = event
                .paths
                .iter()
                .filter_map(|path| path.strip_prefix(&root_folder).ok());

            // put these in a block so we can drop the locks before we await
            let globs_to_exclude = {
                let glob_statuses = self.glob_statuses.lock().expect("ok");
                let hash_globs = self.hash_globs.lock().expect("ok");

                // hash globs is unlocked after this
                let (hash_globs_to_clear, globs_to_exclude) =
                    populate_hash_globs(&glob_statuses, repo_relative_paths, hash_globs);

                clear_hash_globs(glob_statuses, hash_globs_to_clear);

                globs_to_exclude
            };

            for glob in globs_to_exclude {
                self.config.exclude(glob.to_string()).await.unwrap();
            }
        }
    }

    pub async fn watch_globs<Iter: IntoIterator<Item = String>>(
        &self,
        hash: String,
        include: Iter,
        exclude: Iter,
    ) {
        self.config.flush().await.unwrap();

        let hash = Arc::new(hash);
        let include: HashSet<_> = include.into_iter().map(Arc::new).collect();
        let exclude = exclude.into_iter().map(Arc::new).collect();

        for glob in include.iter() {
            self.config.include(glob.to_string()).await.unwrap();
        }

        {
            let mut glob_status = self.glob_statuses.lock().expect("no panic");
            glob_status
                .entry(hash.clone())
                .or_default()
                .extend(include.clone());
        }

        {
            let mut hash_globs = self.hash_globs.lock().expect("no panic");
            hash_globs.insert(hash, Glob { include, exclude });
        }
    }

    /// Given a hash and a set of candidates, return the subset of candidates
    /// that have changed.
    pub async fn changed_globs(
        &self,
        hash: &str,
        mut candidates: HashSet<String>,
    ) -> HashSet<String> {
        self.config.flush().await.unwrap();

        let globs = self.hash_globs.lock().unwrap();
        match globs.get(&Arc::new(hash.to_string())) {
            Some(glob) => {
                candidates.retain(|c| glob.include.contains(c));
                candidates
            }
            None => candidates,
        }
    }
}

/// iterate each path-glob pair and stop tracking globs whose files have
/// changed
///
/// return a list of hash-glob pairs to clear, and a list of globs to exclude
fn populate_hash_globs<'a>(
    glob_statuses: &std::sync::MutexGuard<HashMap<Arc<String>, HashSet<Arc<String>>>>,
    repo_relative_paths: impl Iterator<Item = &'a Path> + Clone,
    mut hash_globs: std::sync::MutexGuard<HashMap<Arc<String>, Glob>>,
) -> (Vec<(Arc<String>, Arc<String>)>, Vec<Arc<String>>) {
    let mut clear_glob_status = vec![];
    let mut exclude_globs = vec![];

    for ((glob, hash_status), path) in glob_statuses
        .iter()
        .cartesian_product(repo_relative_paths)
        .filter(|((glob, _), path)| glob_match::glob_match(glob, path.to_str().unwrap()))
    {
        for hash in hash_status.iter() {
            let globs = match hash_globs.get_mut(hash).filter(|globs| {
                globs
                    .exclude
                    .iter()
                    .any(|f| glob_match::glob_match(f, path.to_str().unwrap()))
            }) {
                Some(globs) => globs,
                None => continue,
            };

            // we can stop tracking that glob
            globs.include.remove(glob);
            if globs.include.is_empty() {
                hash_globs.remove(hash);
            }

            // store the hash and glob so we can remove it from the glob_status
            exclude_globs.push(glob.to_owned());
            clear_glob_status.push((hash.clone(), glob.clone()));
        }
    }

    (clear_glob_status, exclude_globs)
}

/// given a list of hosh-glob pairs to stop tracking, remove them from the
/// map and remove the entry if the set of globs for that hash is empty
fn clear_hash_globs(
    mut glob_status: std::sync::MutexGuard<HashMap<Arc<String>, HashSet<Arc<String>>>>,
    hash_globs_to_clear: Vec<(Arc<String>, Arc<String>)>,
) {
    for (hash, glob) in hash_globs_to_clear {
        let empty = if let Some(globs) = glob_status.get_mut(&hash) {
            globs.remove(&glob);
            globs.is_empty()
        } else {
            false
        };

        if empty {
            glob_status.remove(&hash);
        }
    }
}
