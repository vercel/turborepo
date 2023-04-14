use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use futures::StreamExt;
use globwatch::{GlobSender, GlobWatcher, StopToken, Watcher};
use itertools::Itertools;
use log::{trace, warn};
use notify::RecommendedWatcher;

// these aliases are for readability, but they're just strings. it may make
// sense to use a newtype wrapper for these types in the future.
type Glob = Arc<String>;
type Hash = Arc<String>;

/// Tracks changes for a given hash. A hash is a unique identifier for a set of
/// files. Given a hash and a set of globs to track, this will watch for file
/// changes and allow the user to query for changes. Once all globs for a
/// particular hash have changed, that hash is no longer tracked.
#[derive(Clone)]
pub struct HashGlobWatcher<T: Watcher> {
    /// maintains the list of <GlobSet> to watch for a given hash
    hash_globs: Arc<Mutex<HashMap<Hash, GlobSet>>>,

    /// maps a glob to the hashes for which this glob hasn't changed
    glob_statuses: Arc<Mutex<HashMap<Glob, HashSet<Hash>>>>,

    watcher: Arc<Mutex<Option<GlobWatcher<T>>>>,
    config: GlobSender,
}

#[derive(Clone)]
pub struct GlobSet {
    include: HashSet<Glob>,
    exclude: HashSet<Glob>,
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

                // glob_statuses is unlocked after this
                clear_hash_globs(glob_statuses, hash_globs_to_clear);

                globs_to_exclude
            };

            for glob in globs_to_exclude {
                self.config.exclude(glob.to_string()).await.unwrap();
            }
        }
    }

    /// registers a hash with a set of globs to watch for changes
    pub async fn watch_globs<Iter: IntoIterator<Item = String>>(
        &self,
        hash: Hash,
        include: Iter,
        exclude: Iter,
    ) {
        // wait for a the watcher to flush its events
        // that will ensure that we have seen all filesystem writes
        // *by the calling client*. Other tasks _could_ write to the
        // same output directories, however we are relying on task
        // execution dependencies to prevent that.
        self.config.flush().await.unwrap();

        let include: HashSet<_> = include.into_iter().map(Arc::new).collect();
        let exclude = exclude.into_iter().map(Arc::new).collect();

        for glob in include.iter() {
            self.config.include(glob.to_string()).await.unwrap();
        }

        {
            let mut glob_status = self.glob_statuses.lock().expect("only fails if poisoned");
            glob_status
                .entry(hash.clone())
                .or_default()
                .extend(include.clone());
        }

        {
            let mut hash_globs = self.hash_globs.lock().expect("only fails if poisoned");
            hash_globs.insert(hash, GlobSet { include, exclude });
        }
    }

    /// given a hash and a set of candidates, return the subset of candidates
    /// that have changed.
    pub async fn changed_globs(
        &self,
        hash: &Hash,
        mut candidates: HashSet<String>,
    ) -> HashSet<String> {
        // wait for a the watcher to flush its events
        // that will ensure that we have seen all filesystem writes
        // *by the calling client*. Other tasks _could_ write to the
        // same output directories, however we are relying on task
        // execution dependencies to prevent that.
        self.config.flush().await.unwrap();

        let globs = self.hash_globs.lock().unwrap();
        match globs.get(hash) {
            Some(glob) => {
                candidates.retain(|c| glob.include.contains(c));
                candidates
            }
            None => candidates,
        }
    }
}

/// iterate each path-glob pair and stop tracking globs whose files have
/// changed. if a path is not a valid utf8 string, it is ignored. this is
/// okay, because we don't register any paths that are not valid utf8,
/// since the source globs are valid utf8
///
/// returns a list of hash-glob pairs to clear, and a list of globs to exclude
///
/// note: we take a mutex guard to make sure that the mutex is dropped
///       when the function returns
fn populate_hash_globs<'a>(
    glob_statuses: &MutexGuard<HashMap<Glob, HashSet<Hash>>>,
    repo_relative_paths: impl Iterator<Item = &'a Path> + Clone,
    mut hash_globs: MutexGuard<HashMap<Hash, GlobSet>>,
) -> (Vec<(Arc<String>, Arc<String>)>, Vec<Arc<String>>) {
    let mut clear_glob_status = vec![];
    let mut exclude_globs = vec![];

    for ((glob, hash_status), path) in glob_statuses
        .iter()
        .cartesian_product(repo_relative_paths)
        .filter(|((glob, _), path)| {
            // ignore paths that don't match the glob, or are not valid utf8
            path.to_str()
                .map(|s| glob_match::glob_match(glob, s))
                .unwrap_or(false)
        })
    {
        // if we get here, we know that the glob has changed for every hash that
        // included this glob and is not excluded by a hash's exclusion globs.
        // So, we can delete this glob from every hash tracking it as well as stop
        // watching this glob. To stop watching, we unref each of the
        // directories corresponding to this glob.

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

/// given a list of hash-glob pairs to stop tracking, remove them from the
/// map and remove the entry if the set of globs for that hash is empty
///
/// note: we take a mutex guard to make sure that the mutex is dropped
///       when the function returns
fn clear_hash_globs(
    mut glob_status: MutexGuard<HashMap<Glob, HashSet<Hash>>>,
    hash_globs_to_clear: Vec<(Hash, Glob)>,
) {
    for (hash, glob) in hash_globs_to_clear {
        if let Entry::Occupied(mut o) = glob_status.entry(hash) {
            let val = o.get_mut();
            val.remove(&glob);
            if val.is_empty() {
                o.remove();
            }
        };
    }
}
