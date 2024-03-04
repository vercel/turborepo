use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    future::IntoFuture,
    str::FromStr,
    time::Duration,
};

use notify::Event;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPath};
use wax::{Any, Glob, Program};

use crate::{
    cookies::{CookieError, CookieWatcher, CookieWriter, CookiedRequest},
    NotifyError, OptionalWatch,
};

type Hash = String;

pub struct GlobSet {
    include: HashMap<String, wax::Glob<'static>>,
    exclude: Any<'static>,
    exclude_raw: Vec<String>,
}

impl std::fmt::Debug for GlobSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobSet")
            .field("include", &self.include.keys())
            .field("exclude", &self.exclude_raw)
            .finish()
    }
}

#[derive(Debug, Error)]
pub struct GlobError {
    // Boxed to minimize error size
    underlying: Box<wax::BuildError>,
    raw_glob: String,
}

impl Display for GlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.underlying, self.raw_glob)
    }
}

fn compile_glob(raw: &str) -> Result<Glob<'static>, GlobError> {
    Glob::from_str(raw)
        .map(|g| g.to_owned())
        .map_err(|e| GlobError {
            underlying: Box::new(e),
            raw_glob: raw.to_owned(),
        })
}

impl GlobSet {
    pub fn from_raw(
        raw_includes: Vec<String>,
        raw_excludes: Vec<String>,
    ) -> Result<Self, GlobError> {
        let include = raw_includes
            .into_iter()
            .map(|raw_glob| {
                let glob = compile_glob(&raw_glob)?;
                Ok((raw_glob, glob))
            })
            .collect::<Result<HashMap<_, _>, GlobError>>()?;
        let excludes = raw_excludes
            .clone()
            .iter()
            .map(|raw_glob| {
                let glob = compile_glob(raw_glob)?;
                Ok(glob)
            })
            .collect::<Result<Vec<_>, GlobError>>()?;
        let exclude = wax::any(excludes)
            .map_err(|e| GlobError {
                underlying: Box::new(e),
                raw_glob: format!("{{{}}}", raw_excludes.join(",")),
            })?
            .to_owned();
        Ok(Self {
            include,
            exclude,
            exclude_raw: raw_excludes,
        })
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    CookieError(#[from] CookieError),
    #[error("failed to send query to globwatcher: {0}")]
    SendError(#[from] mpsc::error::SendError<CookiedRequest<Query>>),
    #[error("globwatcher has closed")]
    Closed,
    #[error("globwatcher request timed out")]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("glob watching is unavailable")]
    Unavailable,
}

impl From<mpsc::error::SendError<Query>> for Error {
    fn from(_: mpsc::error::SendError<Query>) -> Self {
        Error::Closed
    }
}

impl From<oneshot::error::RecvError> for Error {
    fn from(_: oneshot::error::RecvError) -> Self {
        Error::Closed
    }
}

pub struct GlobWatcher {
    cookie_writer: CookieWriter,
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_ch: oneshot::Sender<()>,
    query_ch_lazy: OptionalWatch<mpsc::Sender<CookiedRequest<Query>>>,
}

#[derive(Debug)]
pub enum Query {
    WatchGlobs {
        hash: Hash,
        glob_set: GlobSet,
        resp: oneshot::Sender<Result<(), Error>>,
    },
    GetChangedGlobs {
        hash: Hash,
        candidates: HashSet<String>,
        resp: oneshot::Sender<Result<HashSet<String>, Error>>,
    },
}

struct GlobTracker {
    root: AbsoluteSystemPathBuf,

    /// maintains the list of <GlobSet> to watch for a given hash
    hash_globs: HashMap<Hash, GlobSet>,

    /// maps a string glob to the compiled glob and the hashes for which this
    /// glob hasn't changed
    glob_statuses: HashMap<String, (Glob<'static>, HashSet<Hash>)>,

    exit_signal: oneshot::Receiver<()>,

    recv: broadcast::Receiver<Result<Event, NotifyError>>,

    query_recv: mpsc::Receiver<CookiedRequest<Query>>,

    cookie_watcher: CookieWatcher<Query>,
}

impl GlobWatcher {
    pub fn new(
        root: AbsoluteSystemPathBuf,
        cookie_writer: CookieWriter,
        mut recv: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    ) -> Self {
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();
        let (query_ch_tx, query_ch_lazy) = OptionalWatch::new();
        let cookie_root = cookie_writer.root().to_owned();
        tokio::task::spawn(async move {
            let Ok(recv) = recv.get().await.map(|r| r.resubscribe()) else {
                // if this fails, it means that the filewatcher is not available
                // so starting the glob tracker is pointless
                return;
            };

            // if the receiver is closed, it means the glob watcher is closed and we
            // probably don't want to start the glob tracker
            let (query_ch, query_recv) = mpsc::channel(128);
            if query_ch_tx.send(Some(query_ch)).is_err() {
                tracing::debug!("no queryers for glob watcher, exiting");
                return;
            }

            GlobTracker::new(root, cookie_root, exit_signal, recv, query_recv)
                .watch()
                .await
        });
        Self {
            cookie_writer,
            _exit_ch: exit_ch,
            query_ch_lazy,
        }
    }

    /// Watch a set of globs for a given hash.
    ///
    /// This function will return `Error::Unavailable` if the globwatcher is not
    /// yet available.
    pub async fn watch_globs(
        &self,
        hash: Hash,
        globs: GlobSet,
        timeout: Duration,
    ) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        let req = Query::WatchGlobs {
            hash,
            glob_set: globs,
            resp: tx,
        };
        self.send_request(req).await?;
        tokio::time::timeout(timeout, rx).await??
    }

    /// Get the globs that have changed for a given hash.
    ///
    /// This function will return `Error::Unavailable` if the globwatcher is not
    /// yet available.
    pub async fn get_changed_globs(
        &self,
        hash: Hash,
        candidates: HashSet<String>,
        timeout: Duration,
    ) -> Result<HashSet<String>, Error> {
        let (tx, rx) = oneshot::channel();
        let req = Query::GetChangedGlobs {
            hash,
            candidates,
            resp: tx,
        };

        self.send_request(req).await?;
        tokio::time::timeout(timeout, rx).await??
    }

    async fn send_request(&self, req: Query) -> Result<(), Error> {
        let cookied_request = self.cookie_writer.cookie_request(req).await?;
        let mut query_ch = self.query_ch_lazy.clone();
        let query_ch = query_ch
            .get_immediate()
            .ok_or(Error::Unavailable)?
            .map(|ch| ch.clone())
            .map_err(|_| Error::Unavailable)?;

        query_ch.send(cookied_request).await?;
        Ok(())
    }
}

#[derive(Debug, Error)]
enum WatchError {
    #[error(transparent)]
    Recv(#[from] broadcast::error::RecvError),
    #[error(transparent)]
    Notify(#[from] NotifyError),
}

impl GlobTracker {
    fn new(
        root: AbsoluteSystemPathBuf,
        cookie_root: AbsoluteSystemPathBuf,
        exit_signal: oneshot::Receiver<()>,
        recv: broadcast::Receiver<Result<Event, NotifyError>>,
        query_recv: mpsc::Receiver<CookiedRequest<Query>>,
    ) -> Self {
        Self {
            root,
            hash_globs: HashMap::new(),
            glob_statuses: HashMap::new(),
            exit_signal,
            recv,
            query_recv,
            cookie_watcher: CookieWatcher::new(cookie_root),
        }
    }

    fn handle_cookied_query(&mut self, cookied_query: CookiedRequest<Query>) {
        if let Some(request) = self.cookie_watcher.check_request(cookied_query) {
            self.handle_query(request);
        }
    }

    fn handle_query(&mut self, query: Query) {
        match query {
            Query::WatchGlobs {
                hash,
                glob_set,
                resp,
            } => {
                debug!("watching globs {:?} for hash {}", glob_set, hash);
                // Assume cookie handling has happened external to this component.
                // Other tasks _could_ write to the
                // same output directories, however we are relying on task
                // execution dependencies to prevent that.
                for (glob_str, glob) in glob_set.include.iter() {
                    let glob_str = glob_str.to_owned();
                    let (_, hashes) = self
                        .glob_statuses
                        .entry(glob_str)
                        .or_insert_with(|| (glob.clone(), HashSet::new()));
                    hashes.insert(hash.clone());
                }
                self.hash_globs.insert(hash.clone(), glob_set);
                let _ = resp.send(Ok(()));
            }
            Query::GetChangedGlobs {
                hash,
                mut candidates,
                resp,
            } => {
                // Assume cookie handling has happened external to this component.
                // Build a set of candidate globs that *may* have changed.
                // An empty set translates to all globs have not changed.
                if let Some(unchanged_globs) = self.hash_globs.get(&hash) {
                    candidates.retain(|glob_str| {
                        // We are keeping the globs from candidates that
                        // we don't have a record of as unchanged.
                        // If we do have a record, drop it from candidates.
                        !unchanged_globs.include.contains_key(glob_str)
                    });
                }
                // If the client has gone away, we don't care about the error
                let _ = resp.send(Ok(candidates));
            }
        }
    }

    fn handle_file_event(
        &mut self,
        file_event: Result<Result<Event, NotifyError>, broadcast::error::RecvError>,
    ) {
        match file_event {
            Err(broadcast::error::RecvError::Closed) => (),
            Err(e @ broadcast::error::RecvError::Lagged(_)) => self.on_error(e.into()),
            Ok(Err(error)) => self.on_error(error.into()),
            Ok(Ok(file_event)) => {
                for path in file_event.paths {
                    let path = AbsoluteSystemPathBuf::try_from(path)
                        .expect("filewatching should produce absolute paths");
                    if let Some(queries) = self
                        .cookie_watcher
                        .pop_ready_requests(file_event.kind, &path)
                    {
                        for query in queries {
                            self.handle_query(query);
                        }
                        return;
                    }
                    let Ok(to_match) = self.root.anchor(path) else {
                        // irrelevant filesystem update
                        return;
                    };
                    self.handle_path_change(&to_match.to_unix());
                }
            }
        }
    }

    async fn watch(mut self) {
        loop {
            tokio::select! {
                _ = &mut self.exit_signal => return,
                Some(query) = self.query_recv.recv().into_future() => self.handle_cookied_query(query),
                file_event = self.recv.recv().into_future() => self.handle_file_event(file_event)
            }
        }
    }

    /// on_error takes the conservative approach of considering everything
    /// changed in the event of any error related to filewatching
    fn on_error(&mut self, err: WatchError) {
        warn!(
            "encountered filewatching error, flushing all globs: {}",
            err
        );
        self.hash_globs.clear();
        self.glob_statuses.clear();
    }

    fn handle_path_change(&mut self, path: &RelativeUnixPath) {
        self.glob_statuses
            .retain(|glob_str, (glob, hashes_for_glob)| {
                // If this is not a match, we aren't modifying this glob, bail early and mark
                // for retention.
                if !glob.is_match(path) {
                    return true;
                }
                // We have a match. Check which hashes need invalidation.
                hashes_for_glob.retain(|hash| {
                    let Some(glob_set) = self.hash_globs.get_mut(hash) else {
                        // This shouldn't ever happen, but if we aren't tracking this hash at
                        // all, we don't need to keep it in the set of hashes that are relevant
                        // for this glob.
                        debug_assert!(
                            false,
                            "A glob is referencing a hash that we are not tracking. This is most \
                             likely an internal bookkeeping error in globwatcher.rs"
                        );
                        return false;
                    };
                    // If we match an exclusion, don't invalidate this hash
                    if glob_set.exclude.is_match(path) {
                        return true;
                    }
                    // We didn't match an exclusion, we can remove this glob
                    debug!("file change at {} invalidated glob {}", path, glob_str);
                    glob_set.include.remove(glob_str);

                    // We removed the last include, we can stop tracking this hash
                    if glob_set.include.is_empty() {
                        self.hash_globs.remove(hash);
                    }

                    false
                });
                !hashes_for_glob.is_empty()
            });
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::{HashMap, HashSet},
        str::FromStr,
        time::Duration,
    };

    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
    use wax::{any, Glob};

    use crate::{
        cookies::CookieWriter,
        globwatcher::{GlobSet, GlobWatcher},
        FileSystemWatcher,
    };

    fn temp_dir() -> (AbsoluteSystemPathBuf, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (path, tmp)
    }

    fn setup(repo_root: &AbsoluteSystemPath) {
        // Directory layout:
        // <repo_root>/
        //   .git/
        //   my-pkg/
        //     irrelevant
        //     dist/
        //       dist-file
        //       distChild/
        //         child-file
        //     .next/
        //       next-file
        //       cache/
        repo_root.join_component(".git").create_dir_all().unwrap();
        let pkg_path = repo_root.join_component("my-pkg");
        pkg_path.create_dir_all().unwrap();
        pkg_path
            .join_component("irrelevant")
            .create_with_contents("")
            .unwrap();
        let dist_path = pkg_path.join_component("dist");
        dist_path.create_dir_all().unwrap();
        let dist_child_path = dist_path.join_component("distChild");
        dist_child_path.create_dir_all().unwrap();
        dist_child_path
            .join_component("child-file")
            .create_with_contents("")
            .unwrap();
        dist_path
            .join_component("dist-file")
            .create_with_contents("")
            .unwrap();
        let next_path = pkg_path.join_component(".next");
        next_path.create_dir_all().unwrap();
        next_path
            .join_component("next-file")
            .create_with_contents("")
            .unwrap();
        next_path.join_component("cache").create_dir_all().unwrap();
    }

    fn make_includes(raw: &[&str]) -> HashMap<String, Glob<'static>> {
        raw.iter()
            .map(|raw_glob| {
                (
                    raw_glob.to_string(),
                    Glob::from_str(raw_glob).unwrap().to_owned(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn test_track_outputs() {
        let timeout = Duration::from_secs(2);
        let (repo_root, _tmp_dir) = temp_dir();
        setup(&repo_root);
        let cookie_dir = repo_root.join_component(".git");

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, Duration::from_secs(2), recv.clone());
        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, recv);

        let raw_includes = &["my-pkg/dist/**", "my-pkg/.next/**"];
        let raw_excludes = ["my-pkg/.next/cache/**"];
        let exclude = wax::any(raw_excludes).unwrap().to_owned();
        let globs = GlobSet {
            include: make_includes(raw_includes),
            exclude,
            exclude_raw: raw_excludes.iter().map(|s| s.to_string()).collect(),
        };

        let hash = "the-hash".to_string();

        glob_watcher
            .watch_globs(hash.clone(), globs, timeout)
            .await
            .unwrap();

        let candidates = HashSet::from_iter(raw_includes.iter().map(|s| s.to_string()));
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        assert!(results.is_empty());

        // Make an irrelevant change
        repo_root
            .join_components(&["my-pkg", "irrelevant"])
            .create_with_contents("some bytes")
            .unwrap();
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        assert!(results.is_empty());

        // Make an excluded change
        repo_root
            .join_components(&["my-pkg", ".next", "cache", "foo"])
            .create_with_contents("some bytes")
            .unwrap();
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        assert!(results.is_empty());

        // Make a relevant change
        repo_root
            .join_components(&["my-pkg", "dist", "foo"])
            .create_with_contents("some bytes")
            .unwrap();
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        let expected = HashSet::from_iter(["my-pkg/dist/**".to_string()]);
        assert_eq!(results, expected);

        // Change a file matching the other glob
        repo_root
            .join_components(&["my-pkg", ".next", "foo"])
            .create_with_contents("some bytes")
            .unwrap();
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        let expected =
            HashSet::from_iter(["my-pkg/dist/**".to_string(), "my-pkg/.next/**".to_string()]);
        assert_eq!(results, expected);
    }

    #[tokio::test]
    async fn test_track_multiple_hashes() {
        let timeout = Duration::from_secs(2);
        let (repo_root, _tmp_dir) = temp_dir();
        setup(&repo_root);
        let cookie_dir = repo_root.join_component(".git");

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, Duration::from_secs(2), recv.clone());

        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, recv);

        let raw_includes = &["my-pkg/dist/**", "my-pkg/.next/**"];
        let raw_excludes: [&str; 0] = [];
        let globs = GlobSet {
            include: make_includes(raw_includes),
            exclude: any(raw_excludes).unwrap(),
            exclude_raw: raw_excludes.iter().map(|s| s.to_string()).collect(),
        };

        let hash = "the-hash".to_string();

        glob_watcher
            .watch_globs(hash.clone(), globs, timeout)
            .await
            .unwrap();

        let candidates = HashSet::from_iter(raw_includes.iter().map(|s| s.to_string()));
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        assert!(results.is_empty());

        let second_raw_includes = &["my-pkg/.next/**"];
        let second_raw_excludes = ["my-pkg/.next/cache/**"];
        let second_globs = GlobSet {
            include: make_includes(second_raw_includes),
            exclude: any(second_raw_excludes).unwrap(),
            exclude_raw: second_raw_excludes.iter().map(|s| s.to_string()).collect(),
        };
        let second_hash = "the-second-hash".to_string();
        glob_watcher
            .watch_globs(second_hash.clone(), second_globs, timeout)
            .await
            .unwrap();

        let second_candidates =
            HashSet::from_iter(second_raw_includes.iter().map(|s| s.to_string()));
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        assert!(results.is_empty());

        let results = glob_watcher
            .get_changed_globs(second_hash.clone(), second_candidates.clone(), timeout)
            .await
            .unwrap();
        assert!(results.is_empty());

        // Make a change that is excluded in one of the hashes but not in the other
        repo_root
            .join_components(&["my-pkg", ".next", "cache", "foo"])
            .create_with_contents("hello")
            .unwrap();
        // expect one changed glob for the first hash
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        let expected = HashSet::from_iter(["my-pkg/.next/**".to_string()]);
        assert_eq!(results, expected);

        // The second hash which excludes the change should still not have any changed
        // globs
        let results = glob_watcher
            .get_changed_globs(second_hash.clone(), second_candidates.clone(), timeout)
            .await
            .unwrap();
        assert!(results.is_empty());

        // Make a change for second_hash
        repo_root
            .join_components(&["my-pkg", ".next", "bar"])
            .create_with_contents("hello")
            .unwrap();
        let results = glob_watcher
            .get_changed_globs(second_hash.clone(), second_candidates.clone(), timeout)
            .await
            .unwrap();
        assert_eq!(results, second_candidates);
    }

    #[tokio::test]
    async fn test_watch_single_file() {
        let timeout = Duration::from_secs(2);
        let (repo_root, _tmp_dir) = temp_dir();
        setup(&repo_root);
        let cookie_dir = repo_root.join_component(".git");

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, Duration::from_secs(2), recv.clone());

        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, recv);

        // On windows, we expect different sanitization before the
        // globs are passed in, due to alternative data streams in files.
        #[cfg(windows)]
        let raw_includes = &["my-pkg/.next/next-file"];
        #[cfg(not(windows))]
        let raw_includes = &["my-pkg/.next/next-file\\:build"];
        let raw_excludes: [&str; 0] = [];
        let globs = GlobSet {
            include: make_includes(raw_includes),
            exclude: any(raw_excludes).unwrap(),
            exclude_raw: raw_excludes.iter().map(|s| s.to_string()).collect(),
        };

        let hash = "the-hash".to_string();

        glob_watcher
            .watch_globs(hash.clone(), globs, timeout)
            .await
            .unwrap();

        // A change to an irrelevant file
        repo_root
            .join_components(&["my-pkg", ".next", "foo"])
            .create_with_contents("hello")
            .unwrap();

        let candidates = HashSet::from_iter(raw_includes.iter().map(|s| s.to_string()));
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        assert!(results.is_empty());

        // Change the watched file
        let watched_file = repo_root.join_components(&["my-pkg", ".next", "next-file:build"]);
        watched_file.create_with_contents("hello").unwrap();
        let results = glob_watcher
            .get_changed_globs(hash.clone(), candidates.clone(), timeout)
            .await
            .unwrap();
        assert_eq!(results, candidates);
    }
}
