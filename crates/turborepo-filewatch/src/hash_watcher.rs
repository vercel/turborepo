use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    sync::Arc,
};

use notify::Event;
use thiserror::Error;
use tokio::{
    select,
    sync::{
        broadcast, mpsc, oneshot,
        watch::{self, error::RecvError},
    },
};
use tracing::debug;
use turbopath::{
    AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf, PathRelation,
    RelativeUnixPathBuf,
};
use turborepo_repository::discovery::DiscoveryResponse;
use turborepo_scm::{package_deps::GitHashes, Error as SCMError, SCM};

use crate::{
    cookies::{CookieWatcher, CookiedRequest},
    globwatcher::GlobSet,
    package_watcher::DiscoveryData,
    NotifyError, OptionalWatch,
};

struct HashWatcher {
    _exit_tx: oneshot::Sender<()>,
    _handle: tokio::task::JoinHandle<()>,
    query_tx_lazy: OptionalWatch<mpsc::Sender<Query>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HashSpec {
    pub package_path: AnchoredSystemPathBuf,
    pub inputs: Option<GlobSet>,
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("package hashing encountered an error: {0}")]
    HashingError(String),
    #[error("file hashing is not available: {0}")]
    Unavailable(String),
}

// Communication errors that all funnel to Unavailable

impl From<watch::error::RecvError> for Error {
    fn from(e: watch::error::RecvError) -> Self {
        Self::Unavailable(e.to_string())
    }
}

impl From<oneshot::error::RecvError> for Error {
    fn from(e: oneshot::error::RecvError) -> Self {
        Self::Unavailable(e.to_string())
    }
}

impl<T> From<mpsc::error::SendError<T>> for Error {
    fn from(e: mpsc::error::SendError<T>) -> Self {
        Self::Unavailable(e.to_string())
    }
}

impl HashWatcher {
    fn new(
        repo_root: AbsoluteSystemPathBuf,
        package_discovery: watch::Receiver<Option<DiscoveryData>>,
        file_events: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
        scm: &SCM,
    ) -> Self {
        let (exit_tx, exit_rx) = oneshot::channel();
        //let (query_tx, query_rx) = mpsc::channel(16);
        let (query_tx_state, query_tx_lazy) = OptionalWatch::new();
        let process =
            HashWatchProcess::new(repo_root, package_discovery, scm.clone(), query_tx_state);
        let handle = tokio::spawn(process.watch(exit_rx, file_events));
        Self {
            _exit_tx: exit_tx,
            _handle: handle,
            query_tx_lazy,
        }
    }

    async fn get_hash_blocking(&self, hash_spec: HashSpec) -> Result<GitHashes, Error> {
        let (tx, rx) = oneshot::channel();
        let query_tx = self.query_tx_lazy.clone().get().await?.clone();
        query_tx.send(Query::GetHash(hash_spec, tx)).await?;
        let resp = rx.await?;
        resp.map_err(|e| Error::HashingError(e))
    }
}

struct HashWatchProcess {
    repo_root: AbsoluteSystemPathBuf,
    package_discovery: watch::Receiver<Option<DiscoveryData>>,
    query_tx_state: watch::Sender<Option<mpsc::Sender<Query>>>,
    scm: SCM,
}

enum Query {
    GetHash(HashSpec, oneshot::Sender<Result<GitHashes, String>>),
    //CookiedGetHash(CookiedRequest<(HashSpec, oneshot::Sender<Result<GitHashes, String>>)>)
}

// Version is a type that exists to stamp an asynchronous hash computation with
// a version so that we can ignore completion of outdated hash computations.
#[derive(Clone)]
struct Version(Arc<()>);

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Version {}

impl Version {
    fn new() -> Self {
        Self(Arc::new(()))
    }
}

enum HashState {
    Hashes(GitHashes),
    Pending(Version, Vec<oneshot::Sender<Result<GitHashes, String>>>),
    Unavailable(String),
}

struct HashUpdate {
    spec: HashSpec,
    version: Version,
    result: Result<GitHashes, SCMError>,
}

impl HashWatchProcess {
    fn new(
        repo_root: AbsoluteSystemPathBuf,
        package_discovery: watch::Receiver<Option<DiscoveryData>>,
        scm: SCM,
        query_tx_state: watch::Sender<Option<mpsc::Sender<Query>>>,
    ) -> Self {
        Self {
            repo_root,
            package_discovery,
            scm,
            query_tx_state,
        }
    }

    async fn watch(
        mut self,
        mut exit_rx: oneshot::Receiver<()>,
        mut file_events: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    ) {
        debug!("starting file hash watcher");
        let mut file_events_recv = match file_events.get().await {
            Ok(r) => r.resubscribe(),
            Err(e) => {
                debug!("file hash watcher exited: {:?}", e);
                return;
            }
        };
        let (query_tx, mut query_rx) = mpsc::channel(16);
        let (hash_update_tx, mut hash_update_rx) = mpsc::channel::<HashUpdate>(16);
        let mut hashes: HashMap<HashSpec, HashState> = HashMap::new();

        // We need to wait for the first non-null (error is ok) update from the package
        // watcher before signalling ourselves as ready.
        let package_data = self.package_discovery.borrow().to_owned();
        let mut ready = package_data.is_some();
        self.handle_package_data_update(package_data, &mut hashes, &hash_update_tx);
        if ready {
            let _ = self.query_tx_state.send(Some(query_tx.clone()));
        }
        loop {
            select! {
                biased;
                _ = &mut exit_rx => {
                    debug!("file hash watcher exited");
                    return;
                },
                _ = self.package_discovery.changed() => {
                    let package_data = self.package_discovery.borrow().to_owned();
                    // If we weren't already ready, and this update is non-null, we are now ready
                    ready = !ready && package_data.is_some();
                    self.handle_package_data_update(package_data, &mut hashes, &hash_update_tx);
                    if ready {
                        let _ = self.query_tx_state.send(Some(query_tx.clone()));
                    }
                },
                file_event = file_events_recv.recv() => {
                    match file_event {
                        Ok(Ok(event)) => {
                            self.handle_file_event(event, &mut hashes);
                        },
                        Ok(Err(e)) => {
                            debug!("file watcher error: {:?}", e);
                        },
                        Err(broadcast::error::RecvError::Closed) => {
                            debug!("file watcher closed");
                            return;
                        },
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            debug!("file watcher lagged");
                        },
                    }
                },
                hash_update = hash_update_rx.recv() => {
                    if let Some(hash_update) = hash_update {
                        self.handle_hash_update(hash_update, &mut hashes);
                    } else {
                        // note that we only ever lend out hash_update_tx, so this should be impossible
                        unreachable!("hash update channel closed, but we have a live reference to it");
                    }
                },
                query = query_rx.recv() => {
                    if let Some(query) = query {
                        self.handle_query(query, &mut hashes);
                    }
                }
            }
        }
    }

    fn handle_query(&self, query: Query, hashes: &mut HashMap<HashSpec, HashState>) {
        match query {
            Query::GetHash(spec, tx) => {
                if let Some(state) = hashes.get_mut(&spec) {
                    match state {
                        HashState::Hashes(hashes) => {
                            tx.send(Ok(hashes.clone())).unwrap();
                        }
                        HashState::Pending(_, txs) => {
                            txs.push(tx);
                        }
                        HashState::Unavailable(e) => {
                            let _ = tx.send(Err(e.clone()));
                        }
                    }
                } else {
                    let _ = tx.send(Err(format!("package not found: {}", spec.package_path)));
                }
            }
        }
    }

    fn handle_hash_update(&self, update: HashUpdate, hashes: &mut HashMap<HashSpec, HashState>) {
        let HashUpdate {
            spec,
            version,
            result,
        } = update;
        // If we have a pending hash computation, update the state. If we don't, ignore
        // this update
        if let Some(state) = hashes.get_mut(&spec) {
            // If we have a pending hash computation, update the state. If we don't, ignore
            // this update
            if let HashState::Pending(existing_version, pending_queries) = state {
                if *existing_version == version {
                    match result {
                        Ok(hashes) => {
                            debug!("updating hash at {:?}", spec.package_path);
                            for pending_query in pending_queries.drain(..) {
                                // We don't care if the client has gone away
                                let _ = pending_query.send(Ok(hashes.clone()));
                            }
                            *state = HashState::Hashes(hashes);
                        }
                        Err(e) => {
                            let error = e.to_string();
                            for pending_query in pending_queries.drain(..) {
                                // We don't care if the client has gone away
                                let _ = pending_query.send(Err(error.clone()));
                            }
                            *state = HashState::Unavailable(error);
                        }
                    }
                }
            }
        }
    }

    fn queue_package_hash(
        &self,
        spec: &HashSpec,
        hash_update_tx: &mpsc::Sender<HashUpdate>,
    ) -> Version {
        let version = Version::new();
        let version_copy = version.clone();
        let tx = hash_update_tx.clone();
        let spec = spec.clone();
        let repo_root = self.repo_root.clone();
        let scm = self.scm.clone();
        tokio::task::spawn_blocking(move || {
            let telemetry = None;
            let inputs = spec.inputs.as_ref().map(|globs| globs.as_inputs());
            let result = scm.get_package_file_hashes(
                &repo_root,
                &spec.package_path,
                inputs
                    .as_ref()
                    .map(|inputs| inputs.as_slice())
                    .unwrap_or_default(),
                telemetry,
            );
            //let result = self.hash_package(&spec_copy);
            let _ = tx.blocking_send(HashUpdate {
                spec,
                version: version_copy,
                result,
            });
        });
        version
    }

    fn handle_file_event(&self, event: Event, hashes: &mut HashMap<HashSpec, HashState>) {
        let mut changed_packages: HashSet<HashSpec> = HashSet::new();
        'change_path: for path in event.paths {
            let path = AbsoluteSystemPathBuf::try_from(path).expect("event path is a valid path");
            let repo_relative_change_path = self
                .repo_root
                .anchor(&path)
                .expect("event path is in the repository");
            // TODO: better data structure to make this more efficient
            for hash_spec in changed_packages.iter() {
                if hash_spec
                    .package_path
                    .relation_to_path(&repo_relative_change_path)
                    == PathRelation::Parent
                {
                    // We've already seen a change in a parent package, no need to check this one
                    continue 'change_path;
                }
            }
            for hash_spec in hashes.keys() {
                if hash_spec
                    .package_path
                    .relation_to_path(&repo_relative_change_path)
                    == PathRelation::Parent
                {
                    changed_packages.insert(hash_spec.clone());
                }
            }
        }
        // let package_path = self.repo_root.anchor(&path).expect("event path is
        // in the repository"); let spec = HashSpec {
        //     package_path,
        //     inputs: None,
        // };
        // if let Some(state) = hashes.get_mut(&spec) {
        //     match state {
        //         HashState::Pending(count) => {
        //             *count += 1;
        //         },
        //         HashState::Hash(_) => {
        //             *state = HashState::Pending(1);
        //         },
        //     }
        // } else {
        //     hashes.insert(spec, HashState::Pending(1));
        // }
    }

    fn handle_package_data_update(
        &self,
        package_data: Option<Result<DiscoveryResponse, String>>,
        hashes: &mut HashMap<HashSpec, HashState>,
        hash_update_tx: &mpsc::Sender<HashUpdate>,
    ) {
        debug!("handling package data {:?}", package_data);
        match package_data {
            Some(Ok(data)) => {
                let package_paths: HashSet<AnchoredSystemPathBuf> =
                    HashSet::from_iter(data.workspaces.iter().map(|ws| {
                        self.repo_root
                            .anchor(
                                ws.package_json
                                    .parent()
                                    .expect("package.json is in a directory"),
                            )
                            .expect("package is in the repository")
                    }));
                // We have new package data. Drop any packages we don't need anymore, add any
                // new ones
                hashes.retain(|spec, _| package_paths.contains(&spec.package_path));
                for package_path in package_paths {
                    let spec = HashSpec {
                        package_path,
                        inputs: None,
                    };
                    if !hashes.contains_key(&spec) {
                        let version = self.queue_package_hash(&spec, hash_update_tx);
                        hashes.insert(spec, HashState::Pending(version, vec![]));
                    }
                }
                tracing::debug!("received package discovery data: {:?}", data);
            }
            None | Some(Err(_)) => {
                // package data invalidated, flush everything
                for (_, state) in hashes.drain() {
                    if let HashState::Pending(_, txs) = state {
                        for tx in txs {
                            let _ = tx.send(Err("package discovery is unavailable".to_string()));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use git2::Repository;
    use tempfile::{tempdir, TempDir};
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf};
    use turborepo_scm::{package_deps::GitHashes, SCM};

    use crate::{
        cookies::CookieWriter,
        hash_watcher::{HashSpec, HashWatcher},
        package_watcher::PackageWatcher,
        FileSystemWatcher,
    };

    fn commit_all(repo: &Repository) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        let tree_oid = index.write_tree().unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let previous_commit = repo.head().ok().map(|r| r.peel_to_commit().unwrap());
        repo.commit(
            Some("HEAD"),
            &repo.signature().unwrap(),
            &repo.signature().unwrap(),
            "Commit",
            &tree,
            previous_commit
                .as_ref()
                .as_ref()
                .map(std::slice::from_ref)
                .unwrap_or_default(),
        )
        .unwrap();
    }

    fn setup_fixture() -> (TempDir, Repository, AbsoluteSystemPathBuf) {
        let tmp = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        let repo = Repository::init(&repo_root).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        // Setup npm workspaces, .gitignore for dist/ and two packages, one with a
        // nested .gitignore
        //
        // <repo_root>
        // ├── .gitignore (ignore dist/)
        // ├── package.json
        // ├── package-lock.json
        // ├── packages
        // │   ├── foo
        // │   │   ├── .gitignore (ignore out/)
        // │   │   ├── package.json
        // │   │   ├── foo-file
        // │   │   ├── dist
        // │   │   └── out
        // |   |── bar
        // |   |   ├── package.json
        // │   │   ├── dist
        // │   │   └── bar-file
        repo_root
            .join_component(".gitignore")
            .create_with_contents("dist/\n")
            .unwrap();
        repo_root
            .join_component("package.json")
            .create_with_contents(r#"{"workspaces": ["packages/*"]}"#)
            .unwrap();
        repo_root
            .join_component("package-lock.json")
            .create_with_contents("{}")
            .unwrap();
        let packages = repo_root.join_component("packages");

        let foo_dir = packages.join_component("foo");
        foo_dir.join_component("out").create_dir_all().unwrap();
        foo_dir.join_component("dist").create_dir_all().unwrap();
        foo_dir
            .join_component(".gitignore")
            .create_with_contents("out/\n")
            .unwrap();
        foo_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "foo"}"#)
            .unwrap();
        foo_dir
            .join_component("foo-file")
            .create_with_contents("foo file contents")
            .unwrap();

        let bar_dir = packages.join_component("bar");
        bar_dir.join_component("dist").create_dir_all().unwrap();
        bar_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "bar"}"#)
            .unwrap();
        bar_dir
            .join_component("bar-file")
            .create_with_contents("bar file contents")
            .unwrap();
        commit_all(&repo);

        (tmp, repo, repo_root)
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_basic_file_changes() {
        let (_tmp, repo, repo_root) = setup_fixture();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();

        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(
            watcher.cookie_dir(),
            Duration::from_millis(100),
            recv.clone(),
        );

        let scm = SCM::new(&repo_root);
        assert!(!scm.is_manual());
        let package_watcher = PackageWatcher::new(repo_root.clone(), recv, cookie_writer).unwrap();
        let package_discovery = package_watcher.watch_discovery();
        let hash_watcher =
            HashWatcher::new(repo_root.clone(), package_discovery, watcher.watch(), &scm);

        let foo_path = repo_root.join_components(&["packages", "foo"]);

        let foo_hash = hash_watcher
            .get_hash_blocking(HashSpec {
                package_path: repo_root.anchor(&foo_path).unwrap(),
                inputs: None,
            })
            .await
            .unwrap();
        let expected = make_expected(vec![
            ("foo-file", "9317666a2e7b729b740c706ab79724952c97bde4"),
            ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
            (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
        ]);
        assert_eq!(foo_hash, expected);

        // update foo-file
        let foo_file_path = repo_root.join_components(&["packages", "foo", "foo-file"]);
        foo_file_path
            .create_with_contents("new foo-file contents")
            .unwrap();
        retry_get_hash(
            &hash_watcher,
            HashSpec {
                package_path: repo_root.anchor(&foo_path).unwrap(),
                inputs: None,
            },
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "new-hash"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
            ]),
        )
        .await;
    }

    // we don't have a signal for when hashing is complete after having made a file
    // change set a long timeout, but retry several times to try to hit the
    // success case quickly
    async fn retry_get_hash(
        hash_watcher: &HashWatcher,
        spec: HashSpec,
        timeout: Duration,
        expected: GitHashes,
    ) {
        let deadline = Instant::now() + timeout;
        let mut error = None;
        let mut last_value = None;
        while Instant::now() < deadline {
            match hash_watcher.get_hash_blocking(spec.clone()).await {
                Ok(hashes) => {
                    if hashes == expected {
                        return;
                    } else {
                        last_value = Some(hashes);
                    }
                }
                Err(e) => {
                    error = Some(e);
                }
            }
        }
        panic!(
            "failed to get expected hashes. Error {:?}, last hashes: {:?}",
            error, last_value
        );
    }

    fn make_expected(expected: Vec<(&str, &str)>) -> GitHashes {
        let mut map = GitHashes::new();
        for (path, hash) in expected {
            map.insert(RelativeUnixPathBuf::new(path).unwrap(), hash.to_string());
        }
        map
    }
}
