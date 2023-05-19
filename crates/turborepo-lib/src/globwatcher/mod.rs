use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use futures::{stream::iter, StreamExt};
use globwatch::{ConfigError, GlobWatcher, StopToken, WatchConfig, Watcher};
use itertools::Itertools;
use notify::RecommendedWatcher;
use tokio::time::timeout;
use tracing::{trace, warn};
use turbopath::AbsoluteSystemPathBuf;
use wax::{Glob as WaxGlob, Pattern};

// these aliases are for readability, but they're just strings. it may make
// sense to use a newtype wrapper for these types in the future.
type Glob = Arc<String>;
type Hash = Arc<String>;

/// timeout for flushing the watcher
const FLUSH_TIMEOUT: Duration = Duration::from_millis(500);

/// Tracks changes for a given hash. A hash is a unique identifier for a set of
/// files. Given a hash and a set of globs to track, this will watch for file
/// changes and allow the user to query for changes. Once all globs for a
/// particular hash have changed, that hash is no longer tracked.
#[derive(Clone)]
pub struct HashGlobWatcher<T: Watcher> {
    relative_to: PathBuf,

    /// maintains the list of <GlobSet> to watch for a given hash
    hash_globs: Arc<Mutex<HashMap<Hash, GlobSet>>>,

    /// maps a glob to the hashes for which this glob hasn't changed
    glob_statuses: Arc<Mutex<HashMap<Glob, HashSet<Hash>>>>,

    watcher: Arc<Mutex<Option<GlobWatcher>>>,
    config: WatchConfig<T>,
}

#[derive(Clone, Debug)]
pub struct GlobSet {
    include: HashSet<Glob>,
    exclude: HashSet<Glob>,
}

impl HashGlobWatcher<RecommendedWatcher> {
    #[tracing::instrument]
    pub fn new(
        relative_to: AbsoluteSystemPathBuf,
        flush_folder: PathBuf,
    ) -> Result<Self, globwatch::Error> {
        let (watcher, config) = GlobWatcher::new(flush_folder)?;
        Ok(Self {
            relative_to: relative_to.as_path().canonicalize()?,
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
    #[tracing::instrument(skip(self, token))]
    pub async fn watch(&self, token: StopToken) {
        let start_globs = {
            let lock = self.hash_globs.lock().expect("only fails if poisoned");
            lock.iter()
                .flat_map(|(_, g)| &g.include)
                .cloned()
                .collect::<Vec<_>>()
        };

        let mut stream = match self.watcher.lock().expect("only fails if poisoned").take() {
            Some(watcher) => watcher.into_stream(token),
            None => {
                warn!("watcher already consumed");
                return;
            }
        };

        // watch all the globs currently in the map
        for glob in start_globs {
            self.config.include(&self.relative_to, &glob).await.ok();
        }

        while let Some(Ok(event)) = stream.next().await {
            trace!("processing event: {:?}", event);

            let repo_relative_paths = event
                .paths
                .iter()
                .filter_map(|path| path.strip_prefix(&self.relative_to).ok());

            // put these in a block so we can drop the locks before we await
            let globs_to_exclude = {
                let glob_statuses = self.glob_statuses.lock().expect("only fails if poisoned");
                let hash_globs = self.hash_globs.lock().expect("only fails if poisoned");

                // hash globs is unlocked after this
                let (hash_globs_to_clear, globs_to_exclude) =
                    populate_hash_globs(&glob_statuses, repo_relative_paths, hash_globs);

                // glob_statuses is unlocked after this
                clear_hash_globs(glob_statuses, hash_globs_to_clear);

                globs_to_exclude
            };

            for glob in globs_to_exclude {
                self.config.exclude(&self.relative_to, &glob).await;
            }
        }
    }

    /// registers a hash with a set of globs to watch for changes
    pub async fn watch_globs<
        Iter: IntoIterator<Item = String>,
        Iter2: IntoIterator<Item = String>,
    >(
        &self,
        hash: Hash,
        include: Iter,
        exclude: Iter2,
    ) -> Result<(), ConfigError> {
        // wait for a the watcher to flush its events
        // that will ensure that we have seen all filesystem writes
        // *by the calling client*. Other tasks _could_ write to the
        // same output directories, however we are relying on task
        // execution dependencies to prevent that.
        //
        // this is a best effort, and times out after 500ms in
        // case there is a lot of activity on the filesystem
        match timeout(FLUSH_TIMEOUT, self.config.flush()).await {
            Ok(_) => {}
            Err(_) => {
                trace!("timed out waiting for flush");
            }
        }

        let include: HashSet<_> = include.into_iter().map(Arc::new).collect();
        let exclude = exclude.into_iter().map(Arc::new).collect();

        let result: Vec<(Glob, ConfigError)> = iter(include.iter())
            .then(|glob| async move {
                (
                    glob.clone(),
                    self.config.include(self.relative_to.as_path(), glob).await,
                )
            })
            .filter_map(|(glob, res)| async {
                match res {
                    Ok(_) => None,
                    Err(err) => Some((glob, err)),
                }
            })
            .collect()
            .await;

        {
            let mut glob_statuses = self.glob_statuses.lock().expect("only fails if poisoned");
            for glob in include.iter() {
                glob_statuses
                    .entry(glob.clone())
                    .or_default()
                    .insert(hash.clone());
            }
        }

        {
            let mut hash_globs = self.hash_globs.lock().expect("only fails if poisoned");
            hash_globs.insert(hash.clone(), GlobSet { include, exclude });
        }

        if !result.is_empty() {
            // we now 'undo' the failed watches if we encountered errors watching any
            // globs, and return an error

            let hash_globs_to_clear = result
                .iter()
                .map(|(glob, _)| (hash.clone(), glob.clone()))
                .collect();

            let glob_statuses = self.glob_statuses.lock().expect("only fails if poisoned");
            // mutex is consumedd here
            clear_hash_globs(glob_statuses, hash_globs_to_clear);

            use ConfigError::*;
            Err(result
                .into_iter()
                .fold(WatchError(vec![]), |acc, (_, err)| {
                    // accumulate any watch errors, but override if the server stopped
                    match (acc, err) {
                        (WatchError(_), ServerStopped) => ServerStopped,
                        (WatchError(files), WatchError(files2)) => {
                            WatchError(files.into_iter().chain(files2).collect())
                        }
                        (err, _) => err,
                    }
                }))
        } else {
            Ok(())
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
        //
        // this is a best effort, and times out after 500ms in
        // case there is a lot of activity on the filesystem
        match timeout(FLUSH_TIMEOUT, self.config.flush()).await {
            Ok(_) => {}
            Err(_) => {
                trace!("timed out waiting for flush");
            }
        }

        // hash_globs tracks all unchanged globs for a given hash.
        // if a hash is not in globs, then either everything has changed
        // or it was never registered. either way, we return all candidates
        let hash_globs = self.hash_globs.lock().expect("only fails if poisoned");
        match hash_globs.get(hash) {
            Some(glob) => {
                candidates.retain(|c| !glob.include.contains(c));
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

    // for every path, check to see if it matches any of the globs
    // if it does, then we need to stop watching that glob
    for ((glob, hash_status), path) in glob_statuses
        .iter()
        .cartesian_product(repo_relative_paths)
        .filter(|((glob, _), path)| {
            let glob = WaxGlob::new(glob).expect("only watch valid globs");
            glob.is_match(*path)
        })
    {
        let mut stop_watching = true;

        // for every hash that includes this glob, check to see if the glob
        // has changed for that hash. if it has, then we need to stop watching
        for hash in hash_status.iter() {
            let globs = match hash_globs.get_mut(hash).filter(|globs| {
                !globs.exclude.iter().any(|glob| {
                    let glob = WaxGlob::new(glob).expect("only watch valid globs");
                    glob.is_match(path)
                })
            }) {
                Some(globs) => globs,
                None => {
                    // if we get here, then the hash is excluded by a glob
                    // so we don't need to stop watching this glob
                    stop_watching = false;
                    continue;
                }
            };

            // if we get here, we know that the glob has changed for every hash that
            // included this glob and is not excluded by a hash's exclusion globs.
            // So, we can delete this glob from every hash tracking it as well as stop
            // watching this glob. To stop watching, we unref each of the
            // directories corresponding to this glob

            // we can stop tracking that glob
            globs.include.remove(glob);
            if globs.include.is_empty() {
                hash_globs.remove(hash);
            }

            clear_glob_status.push((hash.clone(), glob.clone()));
        }

        if stop_watching {
            // store the hash and glob so we can remove it from the glob_status
            exclude_globs.push(glob.to_owned());
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
        if let Entry::Occupied(mut o) = glob_status.entry(glob) {
            let val = o.get_mut();
            val.remove(&hash);
            if val.is_empty() {
                o.remove();
            }
        };
    }
}

#[cfg(test)]
mod test {
    use std::{fs::File, sync::Arc};

    use globwatch::StopSource;
    use turbopath::AbsoluteSystemPathBuf;

    fn setup() -> tempdir::TempDir {
        let tmp = tempdir::TempDir::new("globwatch").unwrap();

        let directories = ["my-pkg/dist/distChild", "my-pkg/.next/cache"];

        let files = [
            "my-pkg/dist/distChild/dist-file",
            "my-pkg/dist/dist-file",
            "my-pkg/.next/next-file",
            "my-pkg/irrelevant",
        ];

        for dir in directories.iter() {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }

        for file in files.iter() {
            std::fs::File::create(tmp.path().join(file)).unwrap();
        }

        tmp
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn track_outputs() {
        let dir = setup();
        let flush = tempdir::TempDir::new("globwatch-flush").unwrap();
        let watcher = Arc::new(
            super::HashGlobWatcher::new(
                AbsoluteSystemPathBuf::new(dir.path()).unwrap(),
                flush.path().to_path_buf(),
            )
            .unwrap(),
        );

        let stop = StopSource::new();

        let task_watcher = watcher.clone();
        let token = stop.token();

        // dropped when the test ends
        let _s = tokio::task::spawn(async move { task_watcher.watch(token).await });

        let hash = Arc::new("the-hash".to_string());
        let include = ["my-pkg/dist/**".to_string(), "my-pkg/.next/**".to_string()];
        let exclude = ["my-pkg/.next/cache/**".to_string()];

        println!("{:?} {:?}", include, exclude);

        watcher
            .watch_globs(
                hash.clone(),
                include.clone().into_iter(),
                exclude.clone().into_iter(),
            )
            .await
            .unwrap();

        let changed = watcher
            .changed_globs(&hash, include.clone().into_iter().collect())
            .await;

        assert!(
            changed.is_empty(),
            "expected no changed globs, got {:?}",
            changed
        );

        // change a file that is neither included nor excluded

        File::create(dir.path().join("my-pkg/irrelevant2")).unwrap();
        let changed = watcher
            .changed_globs(&hash, include.clone().into_iter().collect())
            .await;

        assert!(
            changed.is_empty(),
            "expected no changed globs, got {:?}",
            changed
        );

        // change a file that is excluded

        File::create(dir.path().join("my-pkg/.next/cache/next-file2")).unwrap();
        let changed = watcher
            .changed_globs(&hash, include.clone().into_iter().collect())
            .await;

        assert!(
            changed.is_empty(),
            "expected no changed globs, got {:?}",
            changed
        );

        // change a file that is included

        File::create(dir.path().join("my-pkg/dist/dist-file2")).unwrap();
        let changed = watcher
            .changed_globs(&hash, include.clone().into_iter().collect())
            .await;

        assert_eq!(
            changed,
            ["my-pkg/dist/**".to_string()].into_iter().collect(),
            "expected one of the globs to have changed"
        );

        // change a file that is included but with a subdirectory that is excluded
        // now both globs should be marked as changed

        File::create(dir.path().join("my-pkg/.next/next-file2")).unwrap();
        let changed = watcher
            .changed_globs(&hash, include.clone().into_iter().collect())
            .await;

        assert_eq!(
            changed,
            include.into_iter().collect(),
            "expected both globs to have changed"
        );

        assert!(
            watcher.hash_globs.lock().unwrap().is_empty(),
            "we should no longer be watching any hashes"
        );

        assert!(
            watcher.glob_statuses.lock().unwrap().is_empty(),
            "we should no longer be watching any globs: {:?}",
            watcher.glob_statuses.lock().unwrap()
        );
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_multiple_hashes() {
        let dir = setup();
        let flush = tempdir::TempDir::new("globwatch-flush").unwrap();
        let watcher = Arc::new(
            super::HashGlobWatcher::new(
                AbsoluteSystemPathBuf::new(dir.path()).unwrap(),
                flush.path().to_path_buf(),
            )
            .unwrap(),
        );

        let stop = StopSource::new();

        let task_watcher = watcher.clone();
        let token = stop.token();

        // dropped when the test ends
        let _s = tokio::task::spawn(async move { task_watcher.watch(token).await });

        let hash1 = Arc::new("the-hash-1".to_string());
        let hash2 = Arc::new("the-hash-2".to_string());

        let globs1_inclusion = ["my-pkg/dist/**".to_string(), "my-pkg/.next/**".to_string()];
        let globs2_inclusion = ["my-pkg/.next/**".to_string()];
        let globs2_exclusion = ["my-pkg/.next/cache/**".to_string()];

        watcher
            .watch_globs(
                hash1.clone(),
                globs1_inclusion.clone().into_iter(),
                vec![].into_iter(),
            )
            .await
            .unwrap();

        watcher
            .watch_globs(
                hash2.clone(),
                globs2_inclusion.clone().into_iter(),
                globs2_exclusion.clone().into_iter(),
            )
            .await
            .unwrap();

        let changed = watcher
            .changed_globs(&hash1, globs1_inclusion.clone().into_iter().collect())
            .await;

        assert!(
            changed.is_empty(),
            "expected no changed globs, got {:?}",
            changed
        );

        let changed = watcher
            .changed_globs(&hash2, globs2_inclusion.clone().into_iter().collect())
            .await;

        assert!(
            changed.is_empty(),
            "expected no changed globs, got {:?}",
            changed
        );

        // make a change excluded in only one of the hashes

        File::create(dir.path().join("my-pkg/.next/cache/next-file2")).unwrap();
        let changed = watcher
            .changed_globs(&hash1, globs1_inclusion.clone().into_iter().collect())
            .await;

        assert_eq!(
            changed,
            ["my-pkg/.next/**".to_string()].into_iter().collect(),
            "expected one of the globs to have changed"
        );

        let changed = watcher
            .changed_globs(&hash2, globs2_inclusion.clone().into_iter().collect())
            .await;

        assert!(
            changed.is_empty(),
            "expected no changed globs, got {:?}",
            changed
        );

        // make a change for the other hash

        File::create(dir.path().join("my-pkg/.next/next-file2")).unwrap();
        let changed = watcher
            .changed_globs(&hash2, globs2_inclusion.clone().into_iter().collect())
            .await;

        assert_eq!(
            changed,
            ["my-pkg/.next/**".to_string()].into_iter().collect(),
            "expected one of the globs to have changed"
        );

        assert_eq!(
            watcher.hash_globs.lock().unwrap().keys().len(),
            1,
            "we should be watching one hash, got {:?}",
            watcher.hash_globs.lock().unwrap()
        );

        assert_eq!(
            watcher.glob_statuses.lock().unwrap().keys().len(),
            1,
            "we should be watching one glob, got {:?}",
            watcher.glob_statuses.lock().unwrap()
        );
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn watch_single_file() {
        let dir = setup();
        let flush = tempdir::TempDir::new("globwatch-flush").unwrap();
        let watcher = Arc::new(
            super::HashGlobWatcher::new(
                AbsoluteSystemPathBuf::new(dir.path()).unwrap(),
                flush.path().to_path_buf(),
            )
            .unwrap(),
        );

        let stop = StopSource::new();

        let task_watcher = watcher.clone();
        let token = stop.token();

        // dropped when the test ends
        let _s = tokio::task::spawn(async move { task_watcher.watch(token).await });

        let hash = Arc::new("the-hash".to_string());
        let inclusions = ["my-pkg/.next/next-file".to_string()];

        watcher
            .watch_globs(
                hash.clone(),
                inclusions.clone().into_iter(),
                vec![].into_iter(),
            )
            .await
            .unwrap();

        File::create(dir.path().join("my-pkg/.next/irrelevant")).unwrap();
        let changed = watcher
            .changed_globs(&hash, inclusions.clone().into_iter().collect())
            .await;

        assert!(
            changed.is_empty(),
            "expected no changed globs, got {:?}",
            changed
        );

        File::create(dir.path().join("my-pkg/.next/next-file")).unwrap();
        let changed = watcher
            .changed_globs(&hash, inclusions.clone().into_iter().collect())
            .await;

        assert_eq!(
            changed,
            inclusions.clone().into_iter().collect(),
            "expected one of the globs to have changed"
        );

        assert!(
            watcher.hash_globs.lock().unwrap().is_empty(),
            "we should no longer be watching any hashes"
        );

        assert!(
            watcher.glob_statuses.lock().unwrap().is_empty(),
            "we should no longer be watching any globs: {:?}",
            watcher.glob_statuses.lock().unwrap()
        );
    }
}
