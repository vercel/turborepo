use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use notify::Event;
use radix_trie::{Trie, TrieCommon};
use thiserror::Error;
use tokio::{
    select,
    sync::{broadcast, mpsc, oneshot, watch},
};
use tracing::{debug, trace};
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::discovery::DiscoveryResponse;
use turborepo_scm::{
    package_deps::{GitHashes, INPUT_INCLUDE_DEFAULT_FILES},
    Error as SCMError, SCM,
};

use crate::{
    debouncer::Debouncer,
    globwatcher::{GlobError, GlobSet},
    package_watcher::DiscoveryData,
    NotifyError, OptionalWatch,
};

pub struct HashWatcher {
    _exit_tx: oneshot::Sender<()>,
    _handle: tokio::task::JoinHandle<()>,
    query_tx: mpsc::Sender<Query>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InputGlobs {
    Default,
    DefaultWithExtras(GlobSet),
    Specific(GlobSet),
}

impl InputGlobs {
    pub fn from_raw(mut raw: Vec<String>) -> Result<Self, GlobError> {
        if raw.is_empty() {
            Ok(Self::Default)
        } else if let Some(default_pos) = raw.iter().position(|g| g == INPUT_INCLUDE_DEFAULT_FILES)
        {
            raw.remove(default_pos);
            Ok(Self::DefaultWithExtras(GlobSet::from_raw_unfiltered(raw)?))
        } else {
            Ok(Self::Specific(GlobSet::from_raw_unfiltered(raw)?))
        }
    }

    fn is_package_local(&self) -> bool {
        match self {
            InputGlobs::Default => true,
            InputGlobs::DefaultWithExtras(glob_set) => glob_set.is_package_local(),
            InputGlobs::Specific(glob_set) => glob_set.is_package_local(),
        }
    }

    fn as_inputs(&self) -> Vec<String> {
        match self {
            InputGlobs::Default => Vec::new(),
            InputGlobs::DefaultWithExtras(glob_set) => {
                let mut inputs = glob_set.as_inputs();
                inputs.push(INPUT_INCLUDE_DEFAULT_FILES.to_string());
                inputs
            }
            InputGlobs::Specific(glob_set) => glob_set.as_inputs(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HashSpec {
    pub package_path: AnchoredSystemPathBuf,
    pub inputs: InputGlobs,
}

impl HashSpec {
    fn is_package_local(&self) -> bool {
        self.inputs.is_package_local()
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("package hashing encountered an error: {0}")]
    HashingError(String),
    #[error("file hashing is not available: {0}")]
    Unavailable(String),
    #[error("package not found: {} {:?}", .0.package_path, .0.inputs)]
    UnknownPackage(HashSpec),
    #[error("unsupported: glob traverses out of the package")]
    UnsupportedGlob,
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
    pub fn new(
        repo_root: AbsoluteSystemPathBuf,
        package_discovery: watch::Receiver<Option<DiscoveryData>>,
        file_events: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
        scm: SCM,
    ) -> Self {
        let (exit_tx, exit_rx) = oneshot::channel();
        let (query_tx, query_rx) = mpsc::channel(16);
        let subscriber = Subscriber::new(repo_root, package_discovery, scm, query_rx);
        let handle = tokio::spawn(subscriber.watch(exit_rx, file_events));
        Self {
            _exit_tx: exit_tx,
            _handle: handle,
            query_tx,
        }
    }

    // Note that this does not wait for any sort of ready signal. The watching
    // process won't respond until filewatching is ready, but there is no
    // guarantee that package data or file hashing will be done before
    // responding. Both package discovery and file hashing can fail depending on the
    // state of the filesystem, so clients will need to be robust to receiving
    // errors.
    pub async fn get_file_hashes(&self, hash_spec: HashSpec) -> Result<GitHashes, Error> {
        let (tx, rx) = oneshot::channel();
        self.query_tx.send(Query::GetHash(hash_spec, tx)).await?;
        rx.await?
    }
}

struct Subscriber {
    repo_root: AbsoluteSystemPathBuf,
    package_discovery: watch::Receiver<Option<DiscoveryData>>,
    query_rx: mpsc::Receiver<Query>,
    scm: SCM,
    next_version: AtomicUsize,
}

#[derive(Debug)]
enum Query {
    GetHash(HashSpec, oneshot::Sender<Result<GitHashes, Error>>),
}

// Version is a type that exists to stamp an asynchronous hash computation
// with a version so that we can ignore completion of outdated hash
// computations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Version(usize);

enum HashState {
    Hashes(GitHashes),
    Pending(
        Version,
        Arc<Debouncer>,
        Vec<oneshot::Sender<Result<GitHashes, Error>>>,
    ),
    Unavailable(String),
}
// We use a radix_trie to store hashes so that we can quickly match a file path
// to a package without having to iterate over the set of all packages. We
// expect file changes to be the highest volume of events that this service
// handles, so we want to ensure we're efficient in deciding if a given change
// is relevant or not.
//
// Our Trie keys off of a String because of the orphan rule. Keys are required
// to be TrieKey, but this crate doesn't own TrieKey or AnchoredSystemPathBuf.
// We *could* implement TrieKey in AnchoredSystemPathBuf and avoid the String
// conversion, if we decide we want to add the radix_trie dependency to
// turbopath.
struct FileHashes(Trie<String, HashMap<InputGlobs, HashState>>);

impl FileHashes {
    fn new() -> Self {
        Self(Trie::new())
    }

    fn drop_matching<F>(&mut self, mut f: F, reason: &str)
    where
        F: FnMut(&AnchoredSystemPath) -> bool,
    {
        let mut previous = std::mem::take(&mut self.0);

        // radix_trie doesn't have an into_iter() implementation, so we have a slightly
        // inefficient method for removing matching values. Fortunately, we only
        // need to do this when the package layout changes. It's O(n) in the
        // number of packages, on top of the trie internals.
        let keys = previous.keys().map(|k| k.to_owned()).collect::<Vec<_>>();
        for key in keys {
            let previous_value = previous
                .remove(&key)
                .expect("this key was pulled from previous");
            let path_key =
                AnchoredSystemPath::new(&key).expect("keys are valid AnchoredSystemPaths");
            if !f(path_key) {
                // keep it, we didn't match the key.
                self.0.insert(key, previous_value);
            } else {
                for state in previous_value.into_values() {
                    if let HashState::Pending(_, _, txs) = state {
                        for tx in txs {
                            let _ = tx.send(Err(Error::Unavailable(reason.to_string())));
                        }
                    }
                }
            }
        }
    }

    fn get_changed_specs(&self, file_path: &AnchoredSystemPath) -> HashSet<HashSpec> {
        self.0
            .get_ancestor(file_path.as_str())
            // verify we have a key
            .and_then(|subtrie| subtrie.key().map(|key| (key, subtrie)))
            // convert key to AnchoredSystemPath, and verify we have a value
            .and_then(|(package_path, subtrie)| {
                let package_path = AnchoredSystemPath::new(package_path)
                    .expect("keys are valid AnchoredSystemPaths");
                // handle scenarios where even though we've found an ancestor, it might be a
                // sibling file or directory that starts with the same prefix,
                // e,g an update to apps/foo_decoy when the package path is
                // apps/foo.
                if let Some(package_path_to_file) = file_path.strip_prefix(package_path) {
                    // Pass along the path to the package, the path _within_ the package to this
                    // change, in unix format, and the set of input specs that
                    // we're tracking.
                    subtrie
                        .value()
                        .map(|specs| (package_path, package_path_to_file.to_unix(), specs))
                } else {
                    None
                }
            })
            // now that we have a path and a set of specs, filter the specs to the relevant ones
            .map(|(package_path, change_in_package, input_globs)| {
                input_globs
                    .keys()
                    .filter_map(|input_globs| match input_globs {
                        InputGlobs::Default => Some(HashSpec {
                            package_path: package_path.to_owned(),
                            inputs: InputGlobs::Default,
                        }),
                        inputs @ InputGlobs::DefaultWithExtras(_) => Some(HashSpec {
                            package_path: package_path.to_owned(),
                            inputs: inputs.clone(),
                        }),
                        inputs @ InputGlobs::Specific(glob_set)
                            if glob_set.matches(&change_in_package) =>
                        {
                            Some(HashSpec {
                                package_path: package_path.to_owned(),
                                inputs: inputs.clone(),
                            })
                        }
                        _ => None,
                    })
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default()
    }

    fn drain(&mut self, reason: &str) {
        // funnel through drop_matching even though we could just swap with a new trie.
        // We want to ensure we respond to any pending queries.
        self.drop_matching(|_| true, reason);
    }

    fn contains_key(&self, key: &HashSpec) -> bool {
        self.0
            .get(key.package_path.as_str())
            .and_then(|states| states.get(&key.inputs))
            .is_some()
    }

    fn insert(&mut self, key: HashSpec, value: HashState) {
        if let Some(states) = self.0.get_mut(key.package_path.as_str()) {
            states.insert(key.inputs, value);
        } else {
            let mut states = HashMap::new();
            states.insert(key.inputs, value);
            self.0.insert(key.package_path.as_str().to_owned(), states);
        }
    }

    fn get_mut(&mut self, key: &HashSpec) -> Option<&mut HashState> {
        self.0
            .get_mut(key.package_path.as_str())
            .and_then(|states| states.get_mut(&key.inputs))
    }
}

struct HashUpdate {
    spec: HashSpec,
    version: Version,
    result: Result<GitHashes, SCMError>,
}

impl Subscriber {
    fn new(
        repo_root: AbsoluteSystemPathBuf,
        package_discovery: watch::Receiver<Option<DiscoveryData>>,
        scm: SCM,
        query_rx: mpsc::Receiver<Query>,
    ) -> Self {
        Self {
            repo_root,
            package_discovery,
            scm,
            query_rx,
            next_version: AtomicUsize::new(0),
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
        let (hash_update_tx, mut hash_update_rx) = mpsc::channel::<HashUpdate>(16);
        let mut hashes = FileHashes::new();

        let mut package_data = self.package_discovery.borrow().to_owned();
        self.handle_package_data_update(&package_data, &mut hashes, &hash_update_tx);
        // We've gotten the ready signal from filewatching, and *some* state from
        // package discovery, but there is no guarantee that package discovery
        // is ready. This means that initial queries may be returned with errors
        // until we've completed package discovery and then hashing.
        //
        // This is the main event loop for the hash watcher. It receives file events,
        // updates to the package discovery state, and queries for hashes. It does
        // not use filesystem cookies, as it is expected that the client will
        // synchronize itself first before issuing a series of queries, one per
        // task that in the task graph for a run, and we don't want to block on
        // the filesystem for each query. This is analogous to running without
        // the daemon, where we assume a static filesystem for the duration of
        // generating task hashes.
        loop {
            select! {
                biased;
                _ = &mut exit_rx => {
                    debug!("file hash watcher exited");
                    return;
                },
                _ = self.package_discovery.changed() => {
                    self.package_discovery.borrow().clone_into(&mut package_data);
                    self.handle_package_data_update(&package_data, &mut hashes, &hash_update_tx);
                },
                file_event = file_events_recv.recv() => {
                    match file_event {
                        Ok(Ok(event)) => {
                            self.handle_file_event(event, &mut hashes, &hash_update_tx);
                        },
                        Ok(Err(e)) => {
                            debug!("file watcher error: {:?}", e);
                            self.flush_and_rehash(&mut hashes, &hash_update_tx, &package_data, &format!("file watcher error: {e}"));
                        },
                        Err(broadcast::error::RecvError::Closed) => {
                            debug!("file watcher closed");
                            hashes.drain("file watcher closed");
                            return;
                        },
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            debug!("file watcher lagged");
                            self.flush_and_rehash(&mut hashes, &hash_update_tx, &package_data, "file watcher lagged");
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
                Some(query) = self.query_rx.recv() => {
                    self.handle_query(query, &mut hashes, &hash_update_tx);
                }
            }
        }
    }

    fn flush_and_rehash(
        &self,
        hashes: &mut FileHashes,
        hash_update_tx: &mpsc::Sender<HashUpdate>,
        package_data: &Option<Result<DiscoveryResponse, String>>,
        reason: &str,
    ) {
        // We need to send errors to any RPCs that are pending, and having an empty set
        // of hashes will cause handle_package_data_update to consider all
        // packages as new and rehash them.
        hashes.drain(reason);
        self.handle_package_data_update(package_data, hashes, hash_update_tx);
    }

    // We currently only support a single query, getting hashes for a given
    // HashSpec.
    fn handle_query(
        &self,
        query: Query,
        hashes: &mut FileHashes,
        hash_update_tx: &mpsc::Sender<HashUpdate>,
    ) {
        //trace!("handling query {query:?}");
        match query {
            Query::GetHash(spec, tx) => {
                // We don't currently support inputs that are not package-local. Adding this
                // support would require tracking arbitrary file paths and
                // mapping them back to packages. It is doable if we want to
                // attempt it in the future.
                if !spec.is_package_local() {
                    let _ = tx.send(Err(Error::UnsupportedGlob));
                    trace!("unsupported glob in query {:?}", spec);
                    return;
                }
                if let Some(state) = hashes.get_mut(&spec) {
                    match state {
                        HashState::Hashes(hashes) => {
                            tx.send(Ok(hashes.clone())).unwrap();
                        }
                        HashState::Pending(_, _, txs) => {
                            txs.push(tx);
                        }
                        HashState::Unavailable(e) => {
                            let _ = tx.send(Err(Error::HashingError(e.clone())));
                        }
                    }
                } else if !matches!(spec.inputs, InputGlobs::Default)
                    && hashes.contains_key(&HashSpec {
                        package_path: spec.package_path.clone(),
                        inputs: InputGlobs::Default,
                    })
                {
                    // in this scenario, we know the package exists, but we aren't tracking these
                    // particular inputs. Queue a hash request for them.
                    let (version, debouncer) = self.queue_package_hash(&spec, hash_update_tx, true);
                    // this request will likely time out. However, if the client has asked for
                    // this spec once, they might ask again, and we can start tracking it.
                    hashes.insert(spec, HashState::Pending(version, debouncer, vec![tx]));
                } else {
                    // We don't know anything about this package.
                    let _ = tx.send(Err(Error::UnknownPackage(spec)));
                }
            }
        }
    }

    fn handle_hash_update(&self, update: HashUpdate, hashes: &mut FileHashes) {
        let HashUpdate {
            spec,
            version,
            result,
        } = update;
        // If we have a pending hash computation, update the state. If we don't, ignore
        // this update
        if let Some(state) = hashes.get_mut(&spec) {
            // We need mutable access to 'state' to update it, as well as being able to
            // extract the pending state, so we need two separate if statements
            // to pull the value apart.
            if let HashState::Pending(existing_version, _, pending_queries) = state {
                if *existing_version == version {
                    match result {
                        Ok(hashes) => {
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
                                let _ = pending_query.send(Err(Error::HashingError(error.clone())));
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
        immediate: bool,
    ) -> (Version, Arc<Debouncer>) {
        let version = Version(self.next_version.fetch_add(1, Ordering::SeqCst));
        let tx = hash_update_tx.clone();
        let spec = spec.clone();
        let repo_root = self.repo_root.clone();
        let scm = self.scm.clone();
        let debouncer = if immediate {
            Debouncer::new(Duration::from_millis(0))
        } else {
            Debouncer::default()
        };
        let debouncer = Arc::new(debouncer);
        let debouncer_copy = debouncer.clone();
        tokio::task::spawn(async move {
            debouncer_copy.debounce().await;
            // Package hashing involves blocking IO calls, so run on a blocking thread.
            tokio::task::spawn_blocking(move || {
                let telemetry = None;
                let inputs = spec.inputs.as_inputs();
                let result =
                    scm.get_package_file_hashes(&repo_root, &spec.package_path, &inputs, telemetry);
                trace!("hashing complete for {:?}", spec);
                let _ = tx.blocking_send(HashUpdate {
                    spec,
                    version,
                    result,
                });
            });
        });
        (version, debouncer)
    }

    fn handle_file_event(
        &self,
        event: Event,
        hashes: &mut FileHashes,
        hash_update_tx: &mpsc::Sender<HashUpdate>,
    ) {
        let mut changed_specs: HashSet<HashSpec> = HashSet::new();
        for path in event.paths {
            let path = AbsoluteSystemPathBuf::try_from(path).expect("event path is a valid path");
            let repo_relative_change_path = self
                .repo_root
                .anchor(&path)
                .expect("event path is in the repository");
            // If this change is not relevant to a package, ignore it
            trace!("file change at {:?}", repo_relative_change_path);
            let changed_specs_for_path = hashes.get_changed_specs(&repo_relative_change_path);
            if !changed_specs_for_path.is_empty() {
                // We have a file change in a package, and we haven't seen this package yet.
                // Queue it for rehashing.
                // TODO: further qualification. Which sets of inputs? Is this file .gitignored?
                // We are somewhat saved here by deferring to the SCM to do the hashing. A
                // change to a gitignored file will trigger a re-hash, but won't
                // actually affect what the hash is.
                trace!("specs changed: {:?}", changed_specs_for_path);
                //changed_specs.insert(package_path.to_owned());
                changed_specs.extend(changed_specs_for_path.into_iter());
            } else {
                trace!("Ignoring change to {repo_relative_change_path}");
            }
        }
        // TODO: handle different sets of inputs

        // Any rehashing we do was triggered by a file event, so don't do it
        // immediately. Wait for the debouncer to time out instead.
        let immediate = false;
        for spec in changed_specs {
            match hashes.get_mut(&spec) {
                // Technically this shouldn't happen, the package_paths are sourced from keys in
                // hashes.
                None => {
                    let (version, debouncer) =
                        self.queue_package_hash(&spec, hash_update_tx, immediate);
                    hashes.insert(spec, HashState::Pending(version, debouncer, vec![]));
                }
                Some(entry) => {
                    if let HashState::Pending(_, debouncer, txs) = entry {
                        if !debouncer.bump() {
                            // we failed to bump the debouncer, the hash must already be in
                            // progress. Drop this calculation and start
                            // a new one
                            let (version, debouncer) =
                                self.queue_package_hash(&spec, hash_update_tx, immediate);
                            let mut swap_target = vec![];
                            std::mem::swap(txs, &mut swap_target);
                            *entry = HashState::Pending(version, debouncer, swap_target);
                        }
                    } else {
                        // it's not a pending hash calculation, overwrite the entry with a new
                        // pending calculation
                        let (version, debouncer) =
                            self.queue_package_hash(&spec, hash_update_tx, immediate);
                        *entry = HashState::Pending(version, debouncer, vec![]);
                    }
                }
            }
        }
    }

    fn handle_package_data_update(
        &self,
        package_data: &Option<Result<DiscoveryResponse, String>>,
        hashes: &mut FileHashes,
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
                hashes.drop_matching(
                    |package_path| !package_paths.contains(package_path),
                    "package was removed",
                );
                // package data updates are triggered by file events, so don't immediately
                // start rehashing, use the debouncer to wait for a quiet period.
                let immediate = false;
                for package_path in package_paths {
                    let spec = HashSpec {
                        package_path,
                        inputs: InputGlobs::Default,
                    };
                    if !hashes.contains_key(&spec) {
                        let (version, debouncer) =
                            self.queue_package_hash(&spec, hash_update_tx, immediate);
                        hashes.insert(spec, HashState::Pending(version, debouncer, vec![]));
                    }
                }
                tracing::debug!("received package discovery data: {:?}", data);
            }
            None | Some(Err(_)) => {
                // package data invalidated, flush everything
                hashes.drain("package discovery is unavailable");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        assert_matches::assert_matches,
        time::{Duration, Instant},
    };

    use git2::Repository;
    use tempfile::{tempdir, TempDir};
    use turbopath::{
        AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf,
    };
    use turborepo_scm::{package_deps::GitHashes, SCM};

    use super::{FileHashes, HashState};
    use crate::{
        cookies::CookieWriter,
        globwatcher::GlobSet,
        hash_watcher::{HashSpec, HashWatcher, InputGlobs},
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
            .create_with_contents(
                r#"{"workspaces": ["packages/*"], "packageManager": "npm@10.0.0"}"#,
            )
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

    fn create_fixture_branch(repo: &Repository, repo_root: &AbsoluteSystemPath) {
        // create a branch that deletes bar-file and adds baz-file to the bar package
        let bar_dir = repo_root.join_components(&["packages", "bar"]);
        bar_dir.join_component("bar-file").remove().unwrap();
        bar_dir
            .join_component("baz-file")
            .create_with_contents("baz file contents")
            .unwrap();
        let current_commit = repo
            .head()
            .ok()
            .map(|r| r.peel_to_commit().unwrap())
            .unwrap();
        repo.branch("test-branch", &current_commit, false).unwrap();
        repo.set_head("refs/heads/test-branch").unwrap();
        commit_all(repo);
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_basic_file_changes() {
        let (_tmp, _repo, repo_root) = setup_fixture();

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
            HashWatcher::new(repo_root.clone(), package_discovery, watcher.watch(), scm);

        let foo_path = repo_root.join_components(&["packages", "foo"]);
        // We need to give filewatching time to do the initial scan,
        // but this should resolve in short order to the expected value.
        retry_get_hash(
            &hash_watcher,
            HashSpec {
                package_path: repo_root.anchor(&foo_path).unwrap(),
                inputs: InputGlobs::Default,
            },
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "9317666a2e7b729b740c706ab79724952c97bde4"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
            ]),
        )
        .await;

        // update foo-file
        let foo_file_path = repo_root.join_components(&["packages", "foo", "foo-file"]);
        foo_file_path
            .create_with_contents("new foo-file contents")
            .unwrap();
        retry_get_hash(
            &hash_watcher,
            HashSpec {
                package_path: repo_root.anchor(&foo_path).unwrap(),
                inputs: InputGlobs::Default,
            },
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "5f6796bbd23dcdc9d30d07a2d8a4817c34b7f1e7"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
            ]),
        )
        .await;

        // update files in dist/ and out/ and foo-file
        // verify we don't get hashes for the gitignored files
        repo_root
            .join_components(&["packages", "foo", "out", "some-file"])
            .create_with_contents("an ignored file")
            .unwrap();
        repo_root
            .join_components(&["packages", "foo", "dist", "some-other-file"])
            .create_with_contents("an ignored file")
            .unwrap();
        foo_file_path
            .create_with_contents("even more foo-file contents")
            .unwrap();
        retry_get_hash(
            &hash_watcher,
            HashSpec {
                package_path: repo_root.anchor(&foo_path).unwrap(),
                inputs: InputGlobs::Default,
            },
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "0cb73634538618658f092cd7a3a373c243513a6a"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
            ]),
        )
        .await;
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_switch_branch() {
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
            HashWatcher::new(repo_root.clone(), package_discovery, watcher.watch(), scm);

        let bar_path = repo_root.join_components(&["packages", "bar"]);
        let bar_spec = HashSpec {
            package_path: repo_root.anchor(&bar_path).unwrap(),
            inputs: InputGlobs::Default,
        };

        // We need to give filewatching time to do the initial scan,
        // but this should resolve in short order to the expected value.
        retry_get_hash(
            &hash_watcher,
            bar_spec.clone(),
            Duration::from_secs(2),
            make_expected(vec![
                ("bar-file", "b9bdb1e4875f7397b3f68c104bc249de0ecd3f8e"),
                ("package.json", "b39117e03f0dbe217b957f58a2ad78b993055088"),
            ]),
        )
        .await;

        create_fixture_branch(&repo, &repo_root);

        retry_get_hash(
            &hash_watcher,
            bar_spec,
            Duration::from_secs(2),
            make_expected(vec![
                ("baz-file", "a5395ccf1b8966f3ea805aff0851eac13acb3540"),
                ("package.json", "b39117e03f0dbe217b957f58a2ad78b993055088"),
            ]),
        )
        .await;
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_non_existent_package() {
        let (_tmp, _repo, repo_root) = setup_fixture();

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
            HashWatcher::new(repo_root.clone(), package_discovery, watcher.watch(), scm);

        // Ensure everything is up and running so we can verify the correct error for a
        // non-existing package
        let foo_path = repo_root.join_components(&["packages", "foo"]);
        // We need to give filewatching time to do the initial scan,
        // but this should resolve in short order to the expected value.
        retry_get_hash(
            &hash_watcher,
            HashSpec {
                package_path: repo_root.anchor(&foo_path).unwrap(),
                inputs: InputGlobs::Default,
            },
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "9317666a2e7b729b740c706ab79724952c97bde4"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
            ]),
        )
        .await;

        let non_existent_path = repo_root.join_components(&["packages", "non-existent"]);
        let relative_non_existent_path = repo_root.anchor(&non_existent_path).unwrap();
        let result = hash_watcher
            .get_file_hashes(HashSpec {
                package_path: relative_non_existent_path.clone(),
                inputs: InputGlobs::Default,
            })
            .await;
        assert_matches!(result, Err(crate::hash_watcher::Error::UnknownPackage(unknown_spec)) if unknown_spec.package_path == relative_non_existent_path);
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
            match hash_watcher.get_file_hashes(spec.clone()).await {
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
            tokio::time::sleep(Duration::from_millis(200)).await;
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

    #[test]
    fn test_file_hashes_ancestor() {
        let mut hashes = FileHashes::new();

        let root = AnchoredSystemPathBuf::try_from("").unwrap();
        let foo_path = root.join_components(&["apps", "foo"]);
        let foo_spec = HashSpec {
            package_path: foo_path.clone(),
            inputs: InputGlobs::Default,
        };
        hashes.insert(foo_spec, HashState::Hashes(GitHashes::new()));
        let foo_bar_path = root.join_components(&["apps", "foobar"]);
        let foo_bar_spec = HashSpec {
            package_path: foo_bar_path.clone(),
            inputs: InputGlobs::Default,
        };
        hashes.insert(foo_bar_spec, HashState::Hashes(GitHashes::new()));

        let foo_candidate = foo_path.join_component("README.txt");
        let result = hashes.get_changed_specs(&foo_candidate);
        assert_eq!(result.len(), 1);
        assert_eq!(result.into_iter().next().unwrap().package_path, foo_path);

        let foo_bar_candidate = foo_bar_path.join_component("README.txt");
        let result = hashes.get_changed_specs(&foo_bar_candidate);
        assert_eq!(
            result.into_iter().next().unwrap().package_path,
            foo_bar_path
        );

        // try a path that is a *sibling* of a package, but not itself a package
        let sibling = root.join_components(&["apps", "sibling"]);
        let result = hashes.get_changed_specs(&sibling);
        assert!(result.is_empty());

        // try a path that is a *sibling* of a package, but not itself a package, but
        // starts with the prefix of a package
        let decoy = root.join_components(&["apps", "foodecoy"]);
        let result = hashes.get_changed_specs(&decoy);
        assert!(result.is_empty());
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_basic_file_changes_with_inputs() {
        let (_tmp, _repo, repo_root) = setup_fixture();

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
            HashWatcher::new(repo_root.clone(), package_discovery, watcher.watch(), scm);

        let foo_path = repo_root.join_components(&["packages", "foo"]);
        let foo_inputs = GlobSet::from_raw(vec!["*-file".to_string()], vec![]).unwrap();
        let foo_spec = HashSpec {
            package_path: repo_root.anchor(&foo_path).unwrap(),
            inputs: InputGlobs::Specific(foo_inputs),
        };
        // package.json is always included, whether it matches your inputs or not.
        retry_get_hash(
            &hash_watcher,
            foo_spec.clone(),
            Duration::from_secs(2),
            make_expected(vec![
                // Note that without inputs, we'd also get the .gitignore file
                ("foo-file", "9317666a2e7b729b740c706ab79724952c97bde4"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
            ]),
        )
        .await;

        // update foo-file
        let foo_file_path = repo_root.join_components(&["packages", "foo", "foo-file"]);
        foo_file_path
            .create_with_contents("new foo-file contents")
            .unwrap();
        retry_get_hash(
            &hash_watcher,
            foo_spec.clone(),
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "5f6796bbd23dcdc9d30d07a2d8a4817c34b7f1e7"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
            ]),
        )
        .await;
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_switch_branch_with_inputs() {
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
            HashWatcher::new(repo_root.clone(), package_discovery, watcher.watch(), scm);

        let bar_path = repo_root.join_components(&["packages", "bar"]);

        let bar_inputs = GlobSet::from_raw(vec!["*z-file".to_string()], vec![]).unwrap();
        let bar_spec = HashSpec {
            package_path: repo_root.anchor(&bar_path).unwrap(),
            inputs: InputGlobs::Specific(bar_inputs),
        };

        // package.json is always included, whether it matches your inputs or not.
        retry_get_hash(
            &hash_watcher,
            bar_spec.clone(),
            Duration::from_secs(2),
            make_expected(vec![(
                "package.json",
                "b39117e03f0dbe217b957f58a2ad78b993055088",
            )]),
        )
        .await;

        create_fixture_branch(&repo, &repo_root);

        retry_get_hash(
            &hash_watcher,
            bar_spec,
            Duration::from_secs(2),
            make_expected(vec![
                ("baz-file", "a5395ccf1b8966f3ea805aff0851eac13acb3540"),
                ("package.json", "b39117e03f0dbe217b957f58a2ad78b993055088"),
            ]),
        )
        .await;
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_inputs_with_turbo_defaults() {
        let (_tmp, _repo, repo_root) = setup_fixture();
        // Add an ignored file
        let foo_path = repo_root.join_components(&["packages", "foo"]);
        let ignored_file_path = foo_path.join_components(&["out", "ignored-file"]);
        ignored_file_path.ensure_dir().unwrap();
        ignored_file_path
            .create_with_contents("included in inputs")
            .unwrap();

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
            HashWatcher::new(repo_root.clone(), package_discovery, watcher.watch(), scm);

        let extra_foo_inputs = GlobSet::from_raw(vec!["out/*-file".to_string()], vec![]).unwrap();
        let foo_spec = HashSpec {
            package_path: repo_root.anchor(&foo_path).unwrap(),
            inputs: InputGlobs::DefaultWithExtras(extra_foo_inputs),
        };

        retry_get_hash(
            &hash_watcher,
            foo_spec.clone(),
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "9317666a2e7b729b740c706ab79724952c97bde4"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
                (
                    "out/ignored-file",
                    "e77845e6da275119a0a5a38dbb824773a45f66b3",
                ),
            ]),
        )
        .await;

        // update ignored file
        ignored_file_path
            .create_with_contents("included in inputs again")
            .unwrap();

        retry_get_hash(
            &hash_watcher,
            foo_spec.clone(),
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "9317666a2e7b729b740c706ab79724952c97bde4"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
                (
                    "out/ignored-file",
                    "9fdccf172d999222f3b2103d99a8658de7b21fc6",
                ),
            ]),
        )
        .await;

        // update foo-file
        let foo_file_path = repo_root.join_components(&["packages", "foo", "foo-file"]);
        foo_file_path
            .create_with_contents("new foo-file contents")
            .unwrap();
        retry_get_hash(
            &hash_watcher,
            foo_spec,
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "5f6796bbd23dcdc9d30d07a2d8a4817c34b7f1e7"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (".gitignore", "89f9ac04aac6c8ee66e158853e7d0439b3ec782d"),
                (
                    "out/ignored-file",
                    "9fdccf172d999222f3b2103d99a8658de7b21fc6",
                ),
            ]),
        )
        .await;
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_negative_inputs() {
        let (_tmp, _repo, repo_root) = setup_fixture();

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
            HashWatcher::new(repo_root.clone(), package_discovery, watcher.watch(), scm);

        let foo_path = repo_root.join_components(&["packages", "foo"]);
        let dist_path = foo_path.join_component("dist");
        dist_path
            .join_component("some-dist-file")
            .create_with_contents("dist file")
            .unwrap();
        dist_path
            .join_component("extra-file")
            .create_with_contents("extra file")
            .unwrap();
        let foo_inputs = GlobSet::from_raw_unfiltered(vec![
            "!dist/extra-file".to_string(),
            "**/*-file".to_string(),
        ])
        .unwrap();
        let foo_spec = HashSpec {
            package_path: repo_root.anchor(&foo_path).unwrap(),
            inputs: InputGlobs::Specific(foo_inputs),
        };

        retry_get_hash(
            &hash_watcher,
            foo_spec.clone(),
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "9317666a2e7b729b740c706ab79724952c97bde4"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (
                    "dist/some-dist-file",
                    "21aa527e5ea52d11bf53f493df0dbe6d659b6a30",
                ),
            ]),
        )
        .await;

        dist_path
            .join_component("some-dist-file")
            .create_with_contents("new dist file contents")
            .unwrap();
        retry_get_hash(
            &hash_watcher,
            foo_spec.clone(),
            Duration::from_secs(2),
            make_expected(vec![
                ("foo-file", "9317666a2e7b729b740c706ab79724952c97bde4"),
                ("package.json", "395351bdd7167f351af3396d3225ebe97a7a4d13"),
                (
                    "dist/some-dist-file",
                    "03d4fc427f0bccc1ca7053fc889fa73e54a402fa",
                ),
            ]),
        )
        .await;
    }
}
