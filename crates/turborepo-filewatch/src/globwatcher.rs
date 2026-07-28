use std::{
    collections::{BTreeSet, HashMap, HashSet},
    future::IntoFuture,
    path::PathBuf,
    str::FromStr,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use notify::Event;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPathBuf, PathRelation, RelativeUnixPath};
use wax::{Any, Glob, Program};

use crate::{
    NotifyError, OptionalWatch, WatchInterest, WatchScope, WatchSource, WatchSubscription,
    cookies::{CookieError, CookieWatcher, CookieWriter, CookiedRequest},
};

type Hash = String;
type RegistrationId = u64;
type SharedGlobState = Arc<RwLock<GlobState>>;

#[derive(Clone, Default)]
struct RouteNode {
    children: HashMap<String, RouteNode>,
    glob_sets: HashSet<usize>,
}

/// An immutable snapshot of all active and pending glob sets. Literal prefixes
/// route an event to a small set of possible matches; only those sets pay the
/// cost of exact include/exclude matching.
#[derive(Clone, Default)]
struct GlobRoutingIndex {
    root: RouteNode,
    root_only: HashSet<usize>,
    unprefixed: HashSet<usize>,
    glob_sets: Vec<Option<GlobSet>>,
    glob_set_ids: HashMap<GlobSet, usize>,
    references: Vec<usize>,
    free_ids: Vec<usize>,
}

impl GlobRoutingIndex {
    #[cfg(test)]
    fn from_glob_sets(glob_sets: impl IntoIterator<Item = GlobSet>) -> Self {
        let mut index = Self::default();
        for glob_set in glob_sets {
            index.insert(glob_set);
        }
        index
    }

    /// Adds one logical registration without rebuilding existing routes. The
    /// returned count is useful for asserting that registration work is bounded
    /// by the new glob's size rather than the number of existing registrations.
    fn insert(&mut self, glob_set: GlobSet) -> usize {
        if let Some(&id) = self.glob_set_ids.get(&glob_set) {
            self.references[id] += 1;
            return 0;
        }

        let id = self.free_ids.pop().unwrap_or(self.glob_sets.len());
        if id == self.glob_sets.len() {
            self.glob_sets.push(None);
            self.references.push(0);
        }
        self.glob_sets[id] = Some(glob_set.clone());
        self.references[id] = 1;
        self.glob_set_ids.insert(glob_set.clone(), id);

        let mut work = 0;
        for raw in glob_set.include.keys() {
            work += 1;
            let prefix = literal_prefix(raw);
            if prefix.as_os_str().is_empty() {
                // A component glob cannot cross a separator. In contrast, a
                // leading glob with a separator (notably **/...) can match at
                // arbitrary depth and must remain a repository-wide fallback.
                let routes = if raw.contains('/') {
                    &mut self.unprefixed
                } else {
                    &mut self.root_only
                };
                routes.insert(id);
                continue;
            }

            let mut node = &mut self.root;
            for component in prefix.iter() {
                work += 1;
                node = node
                    .children
                    .entry(component.to_string_lossy().into_owned())
                    .or_default();
            }
            node.glob_sets.insert(id);
        }
        work
    }

    fn remove(&mut self, glob_set: &GlobSet) {
        let Some(&id) = self.glob_set_ids.get(glob_set) else {
            return;
        };
        self.references[id] -= 1;
        if self.references[id] != 0 {
            return;
        }

        for raw in glob_set.include.keys() {
            let prefix = literal_prefix(raw);
            if prefix.as_os_str().is_empty() {
                if raw.contains('/') {
                    self.unprefixed.remove(&id);
                } else {
                    self.root_only.remove(&id);
                }
            } else {
                remove_route(&mut self.root, &mut prefix.iter(), id);
            }
        }
        self.glob_set_ids.remove(glob_set);
        self.glob_sets[id] = None;
        self.free_ids.push(id);
    }

    fn matches(&self, path: &RelativeUnixPath) -> bool {
        self.candidates(path).into_iter().any(|id| {
            self.glob_sets[id]
                .as_ref()
                .is_some_and(|set| set.matches(path))
        })
    }

    fn candidates(&self, path: &RelativeUnixPath) -> Vec<usize> {
        let mut candidates = self.unprefixed.clone();

        let components = path.as_str().split('/').collect::<Vec<_>>();
        if components.len() == 1 {
            for &id in &self.root_only {
                candidates.insert(id);
            }
        }

        let mut node = &self.root;
        for component in components {
            let Some(child) = node.children.get(component) else {
                break;
            };
            node = child;
            for &id in &node.glob_sets {
                candidates.insert(id);
            }
        }

        candidates.into_iter().collect()
    }
}

fn remove_route<'a>(
    node: &mut RouteNode,
    components: &mut impl Iterator<Item = &'a std::ffi::OsStr>,
    id: usize,
) -> bool {
    if let Some(component) = components.next() {
        let component = component.to_string_lossy();
        if let Some(child) = node.children.get_mut(component.as_ref())
            && remove_route(child, components, id)
        {
            node.children.remove(component.as_ref());
        }
    } else {
        node.glob_sets.remove(&id);
    }
    node.children.is_empty() && node.glob_sets.is_empty()
}

fn literal_prefix(raw: &str) -> PathBuf {
    let mut prefix = PathBuf::new();
    for component in raw.split('/') {
        if component
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | ']' | '{' | '}'))
        {
            break;
        }
        if !component.is_empty() && component != "." {
            prefix.push(component.replace("\\:", ":"));
        }
    }
    prefix
}

#[derive(Clone)]
pub struct GlobSet {
    include: HashMap<String, wax::Glob<'static>>,
    exclude: Any<'static>,
    // Note that these globs do not include the leading '!' character
    exclude_raw: BTreeSet<String>,
}

impl GlobSet {
    pub fn as_inputs(&self) -> Vec<String> {
        let mut inputs: Vec<String> = self.include.keys().cloned().collect();
        inputs.extend(self.exclude_raw.iter().map(|s| format!("!{s}")));
        inputs
    }

    pub fn matches(&self, input: &RelativeUnixPath) -> bool {
        self.include.values().any(|glob| glob.is_match(input)) && !self.exclude.is_match(input)
    }
}

impl std::fmt::Debug for GlobSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobSet")
            .field("include", &self.include.keys())
            .field("exclude", &self.exclude_raw)
            .finish()
    }
}

impl PartialEq for GlobSet {
    fn eq(&self, other: &Self) -> bool {
        self.include.keys().collect::<HashSet<_>>() == other.include.keys().collect::<HashSet<_>>()
            && self.exclude_raw == other.exclude_raw
    }
}

impl Eq for GlobSet {}

impl std::hash::Hash for GlobSet {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.include.keys().collect::<BTreeSet<_>>().hash(state);
        self.exclude_raw.hash(state);
    }
}

#[derive(Debug, Error)]
#[error("{underlying}: {raw_glob}")]
pub struct GlobError {
    // Boxed to minimize error size
    underlying: Box<wax::BuildError>,
    raw_glob: String,
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
            .iter()
            .cloned()
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
            exclude_raw: BTreeSet::from_iter(raw_excludes),
        })
    }

    // delegates to from_raw, but filters the globs into inclusions and exclusions
    // first
    pub fn from_raw_unfiltered(raw: Vec<String>) -> Result<Self, GlobError> {
        let (includes, excludes): (Vec<_>, Vec<_>) = {
            let mut includes = vec![];
            let mut excludes = vec![];
            for pattern in raw {
                if let Some(exclude) = pattern.strip_prefix('!') {
                    excludes.push(exclude.to_string());
                } else {
                    includes.push(pattern);
                }
            }
            (includes, excludes)
        };
        Self::from_raw(includes, excludes)
    }

    pub fn is_package_local(&self) -> bool {
        self.include
            .keys()
            .all(|raw_glob| !raw_glob.starts_with("../"))
            && self
                .exclude_raw
                .iter()
                .all(|raw_glob| !raw_glob.starts_with("../"))
    }

    pub(crate) fn literal_prefixes(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.include.keys().map(|raw| literal_prefix(raw))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    CookieError(#[from] CookieError),
    #[error("Failed to send query to glob watcher: {0}")]
    SendError(#[from] mpsc::error::SendError<CookiedRequest<Query>>),
    #[error("Glob watcher has closed.")]
    Closed,
    #[error("Glob watcher request timed out.")]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("Glob watching is unavailable.")]
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
    root: AbsoluteSystemPathBuf,
    cookie_writer: CookieWriter,
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_ch: oneshot::Sender<()>,
    query_ch_lazy: OptionalWatch<mpsc::Sender<CookiedRequest<Query>>>,
    state: SharedGlobState,
    physical_interest: WatchInterest,
    next_registration_id: AtomicU64,
    control_ch: mpsc::UnboundedSender<Control>,
}

#[derive(Clone, Debug)]
pub struct Registration {
    id: RegistrationId,
    hash: Hash,
    glob_set: GlobSet,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone)]
struct ActiveRegistration {
    id: RegistrationId,
    glob_set: GlobSet,
}

#[derive(Default)]
struct GlobState {
    pending: HashMap<RegistrationId, Registration>,
    active: HashMap<Hash, ActiveRegistration>,
    active_ids: HashMap<RegistrationId, Hash>,
    routing_index: GlobRoutingIndex,
    physical_prefixes: HashMap<PathBuf, usize>,
}

enum Control {
    Cancel {
        id: RegistrationId,
        done: oneshot::Sender<()>,
    },
}

#[derive(Debug)]
pub enum Query {
    WatchGlobs {
        registration: Registration,
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
    state: SharedGlobState,

    /// maps a string glob to the compiled glob and the hashes for which this
    /// glob hasn't changed
    glob_statuses: HashMap<String, (Glob<'static>, HashSet<Hash>)>,

    exit_signal: oneshot::Receiver<()>,

    recv: WatchSubscription,

    query_recv: mpsc::Receiver<CookiedRequest<Query>>,

    control_recv: mpsc::UnboundedReceiver<Control>,

    cookie_watcher: CookieWatcher<Query>,

    physical_interest: WatchInterest,
}

impl GlobWatcher {
    pub fn new(
        root: AbsoluteSystemPathBuf,
        cookie_writer: CookieWriter,
        source: impl Into<WatchSource>,
    ) -> Self {
        let source = source.into();
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();
        let (query_ch_tx, query_ch_lazy) = OptionalWatch::new();
        let (control_ch, control_recv) = mpsc::unbounded_channel();
        let cookie_root = cookie_writer.root().to_owned();
        let state = Arc::new(RwLock::new(GlobState::default()));
        let physical_interest = WatchInterest::new();
        physical_interest.extend([cookie_root.as_std_path().to_owned()]);
        let scope = dynamic_watch_scope(root.clone(), cookie_root.clone(), state.clone())
            .with_physical_interest(physical_interest.clone());
        let tracker_state = state.clone();
        let tracker_physical_interest = physical_interest.clone();
        let watcher_root = root.clone();
        tokio::task::spawn(async move {
            let Ok(recv) = source.subscribe(scope).await else {
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

            GlobTracker::new(
                root,
                cookie_root,
                exit_signal,
                recv,
                (query_recv, control_recv),
                (tracker_state, tracker_physical_interest),
            )
            .watch()
            .await
        });
        Self {
            root: watcher_root,
            cookie_writer,
            _exit_ch: exit_ch,
            query_ch_lazy,
            state,
            physical_interest,
            next_registration_id: AtomicU64::new(0),
            control_ch,
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
        let registration = Registration {
            id: self.next_registration_id.fetch_add(1, Ordering::Relaxed),
            hash,
            glob_set: globs,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let added_paths = self.add_pending(registration.clone());
        self.physical_interest.extend(added_paths);
        self.physical_interest.flush().await;
        let req = Query::WatchGlobs {
            registration: registration.clone(),
            resp: tx,
        };
        if let Err(error) = self.send_request(req).await {
            registration.cancelled.store(true, Ordering::Release);
            self.remove_registration_direct(registration.id);
            return Err(error);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                self.cancel_registration(&registration).await;
                Err(error.into())
            }
            Err(error) => {
                // Mark first, then rendezvous with the single tracker task. If
                // commit is in progress, the cancellation is processed after it
                // and removes precisely this generation before Timeout escapes.
                registration.cancelled.store(true, Ordering::Release);
                self.cancel_registration(&registration).await;
                Err(error.into())
            }
        }
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

    fn add_pending(&self, registration: Registration) -> Vec<PathBuf> {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.routing_index.insert(registration.glob_set.clone());
        let added_paths = add_physical_prefixes(&mut state, &self.root, &registration.glob_set);
        state.pending.insert(registration.id, registration);
        added_paths
    }

    async fn cancel_registration(&self, registration: &Registration) {
        registration.cancelled.store(true, Ordering::Release);
        let (done, completed) = oneshot::channel();
        if self
            .control_ch
            .send(Control::Cancel {
                id: registration.id,
                done,
            })
            .is_ok()
        {
            let _ = completed.await;
        } else {
            // The tracker is gone, so no commit can race this direct cleanup.
            self.remove_registration_direct(registration.id);
        }
    }

    fn remove_registration_direct(&self, id: RegistrationId) {
        let removed_paths = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            remove_registration_state(&mut state, id).is_some_and(|removed| removed.removed_path)
        };
        if removed_paths {
            replace_physical_interest(
                &self.state,
                &self.physical_interest,
                &self.root,
                self.cookie_writer.root(),
            );
        }
    }
}

fn add_physical_prefixes(
    state: &mut GlobState,
    root: &AbsoluteSystemPathBuf,
    glob_set: &GlobSet,
) -> Vec<PathBuf> {
    let mut added = Vec::new();
    for prefix in glob_set.literal_prefixes() {
        let count = state.physical_prefixes.entry(prefix.clone()).or_default();
        if *count == 0 {
            added.push(root.as_std_path().join(&prefix));
        }
        *count += 1;
    }
    added
}

fn remove_physical_prefixes(state: &mut GlobState, glob_set: &GlobSet) -> bool {
    let mut removed = false;
    for prefix in glob_set.literal_prefixes() {
        let Some(count) = state.physical_prefixes.get_mut(&prefix) else {
            continue;
        };
        *count -= 1;
        if *count == 0 {
            state.physical_prefixes.remove(&prefix);
            removed = true;
        }
    }
    removed
}

fn replace_physical_interest(
    state: &SharedGlobState,
    physical_interest: &WatchInterest,
    root: &AbsoluteSystemPathBuf,
    cookie_root: &turbopath::AbsoluteSystemPath,
) {
    let paths = state
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .physical_prefixes
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    physical_interest.replace(
        paths
            .into_iter()
            .map(|prefix| root.as_std_path().join(prefix))
            .chain(std::iter::once(cookie_root.as_std_path().to_owned())),
    );
}

struct RemovedRegistration {
    hash: Option<Hash>,
    glob_set: GlobSet,
    removed_path: bool,
}

fn commit_registration_state(
    state: &mut GlobState,
    registration: &Registration,
) -> Option<(Option<ActiveRegistration>, bool)> {
    state.pending.remove(&registration.id)?;
    let previous = state.active.remove(&registration.hash);
    let removed_path = if let Some(previous) = &previous {
        state.active_ids.remove(&previous.id);
        state.routing_index.remove(&previous.glob_set);
        remove_physical_prefixes(state, &previous.glob_set)
    } else {
        false
    };
    state.active.insert(
        registration.hash.clone(),
        ActiveRegistration {
            id: registration.id,
            glob_set: registration.glob_set.clone(),
        },
    );
    state
        .active_ids
        .insert(registration.id, registration.hash.clone());
    Some((previous, removed_path))
}

fn remove_registration_state(
    state: &mut GlobState,
    id: RegistrationId,
) -> Option<RemovedRegistration> {
    let registration = state
        .pending
        .remove(&id)
        .map(|pending| (None, pending.glob_set))
        .or_else(|| {
            let hash = state.active_ids.remove(&id)?;
            state
                .active
                .remove(&hash)
                .map(|active| (Some(hash), active.glob_set))
        });
    let (hash, glob_set) = registration?;
    state.routing_index.remove(&glob_set);
    let removed_path = remove_physical_prefixes(state, &glob_set);
    Some(RemovedRegistration {
        hash,
        glob_set,
        removed_path,
    })
}

fn dynamic_watch_scope(
    root: AbsoluteSystemPathBuf,
    cookie_root: AbsoluteSystemPathBuf,
    state: SharedGlobState,
) -> WatchScope {
    WatchScope::predicate(move |path| {
        let Ok(path) = AbsoluteSystemPathBuf::try_from(path.to_owned()) else {
            return false;
        };

        // Queries are ordered by cookie events, so cookies must remain in scope even
        // when no output globs have been registered yet.
        if cookie_root.relation_to_path(&path) == PathRelation::Parent {
            return true;
        }

        let Ok(relative_path) = root.anchor(&path) else {
            return false;
        };
        let relative_path = relative_path.to_unix();
        let state = state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.routing_index.matches(&relative_path)
    })
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
        recv: WatchSubscription,
        receivers: (
            mpsc::Receiver<CookiedRequest<Query>>,
            mpsc::UnboundedReceiver<Control>,
        ),
        shared: (SharedGlobState, WatchInterest),
    ) -> Self {
        let (query_recv, control_recv) = receivers;
        let (state, physical_interest) = shared;
        Self {
            root,
            state,
            glob_statuses: HashMap::new(),
            exit_signal,
            recv,
            query_recv,
            control_recv,
            cookie_watcher: CookieWatcher::new(cookie_root),
            physical_interest,
        }
    }

    fn handle_cookied_query(&mut self, cookied_query: CookiedRequest<Query>) {
        if let Some(request) = self.cookie_watcher.check_request(cookied_query) {
            self.handle_query(request);
        }
    }

    fn handle_query(&mut self, query: Query) {
        match query {
            Query::WatchGlobs { registration, resp } => {
                if resp.is_closed() || registration.cancelled.load(Ordering::Acquire) {
                    self.remove_registration(registration.id);
                    return;
                }
                let hash = registration.hash.clone();
                let glob_set = registration.glob_set.clone();
                debug!("watching globs {:?} for hash {}", glob_set, hash);
                // Assume cookie handling has happened external to this component.
                // Other tasks _could_ write to the
                // same output directories, however we are relying on task
                // execution dependencies to prevent that.
                let Some((previous, removed_path)) = ({
                    let mut state = self
                        .state
                        .write()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    commit_registration_state(&mut state, &registration)
                }) else {
                    return;
                };
                if let Some(previous) = previous {
                    for glob_str in previous.glob_set.include.keys() {
                        if let Some((_, hashes)) = self.glob_statuses.get_mut(glob_str) {
                            hashes.remove(&hash);
                        }
                    }
                    self.glob_statuses
                        .retain(|_, (_, hashes)| !hashes.is_empty());
                }
                for (glob_str, glob) in &glob_set.include {
                    let glob_str = glob_str.to_owned();
                    let (_, hashes) = self
                        .glob_statuses
                        .entry(glob_str)
                        .or_insert_with(|| (glob.clone(), HashSet::new()));
                    hashes.insert(hash.clone());
                }
                if removed_path {
                    replace_physical_interest(
                        &self.state,
                        &self.physical_interest,
                        &self.root,
                        self.cookie_watcher.root(),
                    );
                }
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
                let state = self
                    .state
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(active) = state.active.get(&hash) {
                    candidates.retain(|glob_str| {
                        // We are keeping the globs from candidates that
                        // we don't have a record of as unchanged.
                        // If we do have a record, drop it from candidates.
                        !active.glob_set.include.contains_key(glob_str)
                    });
                }
                // If the client has gone away, we don't care about the error
                let _ = resp.send(Ok(candidates));
            }
        }
    }

    fn handle_control(&mut self, control: Control) {
        match control {
            Control::Cancel { id, done } => {
                self.remove_registration(id);
                let _ = done.send(());
            }
        }
    }

    fn remove_registration(&mut self, id: RegistrationId) {
        let removed = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut state = removed;
        let removed = remove_registration_state(&mut state, id);
        drop(state);
        let Some(removed) = removed else {
            return;
        };
        if let Some(hash) = removed.hash {
            for glob_str in removed.glob_set.include.keys() {
                if let Some((_, hashes)) = self.glob_statuses.get_mut(glob_str) {
                    hashes.remove(&hash);
                }
            }
            self.glob_statuses
                .retain(|_, (_, hashes)| !hashes.is_empty());
        }
        if removed.removed_path {
            replace_physical_interest(
                &self.state,
                &self.physical_interest,
                &self.root,
                self.cookie_watcher.root(),
            );
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
                    let Ok(path) = AbsoluteSystemPathBuf::try_from(path) else {
                        continue;
                    };
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
                biased;
                _ = &mut self.exit_signal => return,
                Some(control) = self.control_recv.recv() => self.handle_control(control),
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
        {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let active = std::mem::take(&mut state.active);
            state.active_ids.clear();
            for registration in active.into_values() {
                state.routing_index.remove(&registration.glob_set);
                remove_physical_prefixes(&mut state, &registration.glob_set);
            }
        }
        self.glob_statuses.clear();
        replace_physical_interest(
            &self.state,
            &self.physical_interest,
            &self.root,
            self.cookie_watcher.root(),
        );
    }

    fn handle_path_change(&mut self, path: &RelativeUnixPath) {
        let (removed_path, added_paths) = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (_, removed_path, added_paths) =
                invalidate_path_candidates(&mut state, &mut self.glob_statuses, path, &self.root);
            (removed_path, added_paths)
        };
        if removed_path {
            replace_physical_interest(
                &self.state,
                &self.physical_interest,
                &self.root,
                self.cookie_watcher.root(),
            );
        } else {
            self.physical_interest.extend(added_paths);
        }
    }
}

fn invalidate_path_candidates(
    state: &mut GlobState,
    glob_statuses: &mut HashMap<String, (Glob<'static>, HashSet<Hash>)>,
    path: &RelativeUnixPath,
    root: &AbsoluteSystemPathBuf,
) -> (usize, bool, Vec<PathBuf>) {
    let candidate_globs = state
        .routing_index
        .candidates(path)
        .into_iter()
        .flat_map(|id| {
            state.routing_index.glob_sets[id]
                .iter()
                .flat_map(|glob_set| glob_set.include.keys().cloned())
        })
        .collect::<HashSet<_>>();
    let mut inspected = 0;
    let mut changes = Vec::new();

    for glob_str in &candidate_globs {
        let remove_status = {
            let Some((glob, hashes_for_glob)) = glob_statuses.get_mut(glob_str) else {
                continue;
            };
            inspected += 1;
            if !glob.is_match(path) {
                continue;
            }

            hashes_for_glob.retain(|hash| {
                let Some(active) = state.active.get_mut(hash) else {
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
                // If we match an exclusion, don't invalidate this hash.
                if active.glob_set.exclude.is_match(path) {
                    return true;
                }

                debug!("file change at {} invalidated glob {}", path, glob_str);
                let previous = active.glob_set.clone();
                active.glob_set.include.remove(glob_str);
                let next = (!active.glob_set.include.is_empty()).then(|| active.glob_set.clone());
                changes.push((hash.clone(), previous, next));

                false
            });
            hashes_for_glob.is_empty()
        };

        if remove_status {
            glob_statuses.remove(glob_str);
        }
    }

    let mut removed_path = false;
    let mut added_paths = Vec::new();
    for (hash, previous, next) in changes {
        state.routing_index.remove(&previous);
        removed_path |= remove_physical_prefixes(state, &previous);
        if let Some(next) = next {
            state.routing_index.insert(next.clone());
            added_paths.extend(add_physical_prefixes(state, root, &next));
        } else {
            if let Some(active) = state.active.remove(&hash) {
                state.active_ids.remove(&active.id);
            }
        }
    }

    (inspected, removed_path, added_paths)
}

#[cfg(test)]
mod test {
    use std::{
        collections::{HashMap, HashSet},
        str::FromStr,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use notify::{Event, EventKind, event::CreateKind};
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPath};
    use wax::{Glob, any};

    use crate::{
        FileSystemWatcher, WatchSource,
        cookies::CookieWriter,
        globwatcher::{
            ActiveRegistration, GlobRoutingIndex, GlobSet, GlobState, GlobWatcher, Registration,
            add_physical_prefixes, commit_registration_state, invalidate_path_candidates,
            remove_registration_state,
        },
    };

    fn relative(path: &str) -> &RelativeUnixPath {
        RelativeUnixPath::new(path).unwrap()
    }

    #[test]
    fn routing_index_rejects_unrelated_literal_prefixes_without_candidates() {
        let index = GlobRoutingIndex::from_glob_sets([
            GlobSet::from_raw(vec!["apps/web/dist/**".to_string()], vec![]).unwrap(),
            GlobSet::from_raw(vec!["packages/api/generated/**".to_string()], vec![]).unwrap(),
        ]);

        assert_eq!(index.candidates(relative("docs/unrelated.md")).len(), 0);
        assert!(!index.matches(relative("docs/unrelated.md")));
        assert!(index.matches(relative("apps/web/dist/output.js")));
    }

    #[test]
    fn routing_index_distinguishes_root_only_and_recursive_wildcards() {
        let root_only =
            GlobRoutingIndex::from_glob_sets([
                GlobSet::from_raw(vec!["*.js".to_string()], vec![]).unwrap()
            ]);
        assert!(root_only.matches(relative("output.js")));
        assert!(
            root_only
                .candidates(relative("nested/output.js"))
                .is_empty()
        );
        assert!(!root_only.matches(relative("nested/output.js")));

        let recursive = GlobRoutingIndex::from_glob_sets([GlobSet::from_raw(
            vec!["**/*.js".to_string()],
            vec!["vendor/**".to_string()],
        )
        .unwrap()]);
        assert!(recursive.matches(relative("nested/output.js")));
        assert!(!recursive.matches(relative("vendor/output.js")));
    }

    fn state_from_globs(hash_globs: HashMap<String, GlobSet>) -> GlobState {
        let mut state = GlobState::default();
        for (id, (hash, glob_set)) in hash_globs.into_iter().enumerate() {
            let id = id as u64;
            state.routing_index.insert(glob_set.clone());
            state.active_ids.insert(id, hash.clone());
            state
                .active
                .insert(hash, ActiveRegistration { id, glob_set });
        }
        state
    }

    #[test]
    fn registration_index_updates_are_linear_at_scale() {
        let mut index = GlobRoutingIndex::default();
        let mut work = 0;
        for registration in 0..5_000 {
            work += index.insert(
                GlobSet::from_raw(vec![format!("apps/app-{registration}/dist/**")], vec![])
                    .unwrap(),
            );
        }

        // One include plus three literal components per registration. This is
        // independent of how many routes were already registered.
        assert_eq!(work, 20_000);
        assert_eq!(index.glob_set_ids.len(), 5_000);
    }

    #[test]
    fn incremental_state_preserves_shared_routes_and_physical_interests() {
        let (root, _tmp) = temp_dir();
        let glob_set = GlobSet::from_raw(vec!["shared/dist/**".to_string()], vec![]).unwrap();
        let mut state = GlobState::default();
        for id in [1, 2] {
            let registration = Registration {
                id,
                hash: format!("hash-{id}"),
                glob_set: glob_set.clone(),
                cancelled: Arc::new(AtomicBool::new(false)),
            };
            state.routing_index.insert(glob_set.clone());
            add_physical_prefixes(&mut state, &root, &glob_set);
            state.pending.insert(id, registration);
        }

        assert!(state.routing_index.matches(relative("shared/dist/file")));
        assert_eq!(
            state
                .physical_prefixes
                .values()
                .copied()
                .collect::<Vec<_>>(),
            [2]
        );

        remove_registration_state(&mut state, 1).unwrap();
        assert!(state.routing_index.matches(relative("shared/dist/file")));
        assert_eq!(
            state
                .physical_prefixes
                .values()
                .copied()
                .collect::<Vec<_>>(),
            [1]
        );

        remove_registration_state(&mut state, 2).unwrap();
        assert!(!state.routing_index.matches(relative("shared/dist/file")));
        assert!(state.physical_prefixes.is_empty());
    }

    #[test]
    fn timeout_at_commit_cancels_exact_registration_generation() {
        let glob_set = GlobSet::from_raw(vec!["dist/**".to_string()], vec![]).unwrap();
        let registration = Registration {
            id: 41,
            hash: "hash".to_string(),
            glob_set: glob_set.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let mut state = GlobState::default();
        state.routing_index.insert(glob_set);
        state.pending.insert(registration.id, registration.clone());

        // Deterministically model the boundary that used to race: state was
        // committed, but the response had not yet been observed at the deadline.
        assert!(commit_registration_state(&mut state, &registration).is_some());
        registration.cancelled.store(true, Ordering::Release);
        assert!(remove_registration_state(&mut state, registration.id).is_some());
        assert!(!state.active.contains_key("hash"));

        // A delayed cancellation for generation 41 must not remove a newer
        // registration for the same hash.
        state.active.insert(
            "hash".to_string(),
            ActiveRegistration {
                id: 42,
                glob_set: registration.glob_set,
            },
        );
        state.active_ids.insert(42, "hash".to_string());
        assert!(remove_registration_state(&mut state, 41).is_none());
        assert_eq!(state.active["hash"].id, 42);
    }

    #[test]
    fn path_invalidation_inspects_only_routing_candidates_at_scale() {
        let mut hash_globs = HashMap::new();
        let mut glob_statuses = HashMap::new();

        for index in 0..1_000 {
            let hash = format!("hash-{index}");
            let raw_glob = format!("apps/app-{index}/dist/**");
            let glob_set = GlobSet::from_raw(vec![raw_glob.clone()], vec![]).unwrap();
            hash_globs.insert(hash.clone(), glob_set);
            glob_statuses.insert(
                raw_glob.clone(),
                (
                    Glob::from_str(&raw_glob).unwrap().to_owned(),
                    HashSet::from([hash]),
                ),
            );
        }

        let (root, _tmp) = temp_dir();
        let mut state = state_from_globs(hash_globs);
        let (inspected, _, _) = invalidate_path_candidates(
            &mut state,
            &mut glob_statuses,
            relative("docs/unrelated.md"),
            &root,
        );
        assert_eq!(inspected, 0);
        assert_eq!(state.active.len(), 1_000);
        assert_eq!(glob_statuses.len(), 1_000);

        let (inspected, _, _) = invalidate_path_candidates(
            &mut state,
            &mut glob_statuses,
            relative("apps/app-517/dist/output.js"),
            &root,
        );
        assert_eq!(inspected, 1);
        assert!(!state.active.contains_key("hash-517"));
        assert!(!glob_statuses.contains_key("apps/app-517/dist/**"));
        assert_eq!(state.active.len(), 999);
        assert_eq!(glob_statuses.len(), 999);
    }

    #[test]
    fn candidate_invalidation_preserves_shared_glob_exclusions_and_cleanup() {
        let raw_glob = "packages/shared/dist/**".to_string();
        let excluded_hash = "excluded".to_string();
        let invalidated_hash = "invalidated".to_string();
        let excluded_set = GlobSet::from_raw(
            vec![raw_glob.clone()],
            vec!["packages/shared/dist/cache/**".to_string()],
        )
        .unwrap();
        let invalidated_set = GlobSet::from_raw(vec![raw_glob.clone()], vec![]).unwrap();
        let hash_globs = HashMap::from([
            (excluded_hash.clone(), excluded_set.clone()),
            (invalidated_hash.clone(), invalidated_set.clone()),
        ]);
        let mut glob_statuses = HashMap::from([(
            raw_glob.clone(),
            (
                Glob::from_str(&raw_glob).unwrap().to_owned(),
                HashSet::from([excluded_hash.clone(), invalidated_hash.clone()]),
            ),
        )]);
        let (root, _tmp) = temp_dir();
        let mut state = state_from_globs(hash_globs);

        assert_eq!(
            invalidate_path_candidates(
                &mut state,
                &mut glob_statuses,
                relative("packages/shared/dist/cache/item"),
                &root,
            )
            .0,
            1
        );
        assert!(state.active.contains_key(&excluded_hash));
        assert!(!state.active.contains_key(&invalidated_hash));
        assert_eq!(
            glob_statuses[&raw_glob].1,
            HashSet::from([excluded_hash.clone()])
        );

        assert_eq!(
            invalidate_path_candidates(
                &mut state,
                &mut glob_statuses,
                relative("packages/shared/dist/output"),
                &root,
            )
            .0,
            1
        );
        assert!(state.active.is_empty());
        assert!(glob_statuses.is_empty());
    }

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

    #[tokio::test]
    async fn timed_out_registration_removes_pending_interest() {
        let (repo_root, _tmp_dir) = temp_dir();
        let (event_sender, source) = WatchSource::channel();
        let cookie_root = repo_root.join_components(&[".turbo", "cookies"]);
        let cookie_writer = CookieWriter::new(&cookie_root, Duration::from_secs(1), source.clone());
        let watcher = GlobWatcher::new(repo_root, cookie_writer, source);
        let globs = GlobSet::from_raw(vec!["dist/**".to_string()], vec![]).unwrap();

        assert!(
            watcher
                .watch_globs("hash".to_string(), globs, Duration::from_millis(20))
                .await
                .is_err()
        );
        assert!(
            watcher
                .state
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pending
                .is_empty()
        );
        assert!(
            watcher
                .state
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .routing_index
                .glob_sets
                .iter()
                .all(Option::is_none)
        );

        event_sender
            .send(Ok(Event::new(EventKind::Create(CreateKind::File))
                .add_path(
                    cookie_root
                        .join_component("1.cookie")
                        .as_std_path()
                        .to_owned(),
                )))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !watcher
                .state
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .active
                .contains_key("hash")
        );
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

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let cookie_dir = watcher.cookie_dir().to_owned();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, Duration::from_secs(2), recv.clone());
        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, watcher.source());

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

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let cookie_dir = watcher.cookie_dir().to_owned();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, Duration::from_secs(2), recv.clone());

        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, watcher.source());

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

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let cookie_dir = watcher.cookie_dir().to_owned();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, Duration::from_secs(2), recv.clone());

        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, watcher.source());

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

    // -----------------------------------------------------------------------
    // Regression tests for OutputWatcher trait compatibility
    //
    // These tests verify that GlobWatcher's API satisfies the contract
    // required by the OutputWatcher trait (defined in turborepo-run-cache).
    // When we replace the daemon with in-process file watching, an
    // InProcessOutputWatcher will delegate to GlobWatcher. These tests
    // ensure the delegation is correct.
    // -----------------------------------------------------------------------

    /// Verifies that watch_globs + get_changed_globs round-trips correctly
    /// when used with the string-based API that OutputWatcher will use.
    /// This mirrors what the daemon server does in its NotifyOutputsWritten
    /// and GetChangedOutputs RPC handlers.
    #[tokio::test]
    async fn test_output_watcher_delegation_unchanged() {
        let timeout = Duration::from_secs(2);
        let (repo_root, _tmp_dir) = temp_dir();
        setup(&repo_root);

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let cookie_dir = watcher.cookie_dir().to_owned();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, timeout, recv.clone());
        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, watcher.source());

        // Simulate notify_outputs_written: construct GlobSet from string slices
        // (the same conversion InProcessOutputWatcher will do)
        let include_globs = vec!["my-pkg/dist/**".to_string()];
        let exclude_globs: Vec<String> = vec![];
        let glob_set = GlobSet::from_raw(include_globs.clone(), exclude_globs).unwrap();
        let hash = "test-hash".to_string();

        glob_watcher
            .watch_globs(hash.clone(), glob_set, timeout)
            .await
            .unwrap();

        // Simulate get_changed_outputs: pass include globs as candidates
        let candidates: HashSet<String> = include_globs.into_iter().collect();
        let changed = glob_watcher
            .get_changed_globs(hash.clone(), candidates, timeout)
            .await
            .unwrap();

        // No files were modified, so nothing should have changed
        assert!(
            changed.is_empty(),
            "expected no changed outputs when no files were modified, got: {changed:?}"
        );
    }

    /// Verifies that after registering globs via watch_globs and then
    /// modifying a file that matches, get_changed_globs reports the change.
    #[tokio::test]
    async fn test_output_watcher_delegation_with_change() {
        let timeout = Duration::from_secs(2);
        let (repo_root, _tmp_dir) = temp_dir();
        setup(&repo_root);

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let cookie_dir = watcher.cookie_dir().to_owned();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, timeout, recv.clone());
        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, watcher.source());

        let include_globs = vec!["my-pkg/dist/**".to_string()];
        let exclude_globs: Vec<String> = vec![];
        let glob_set = GlobSet::from_raw(include_globs.clone(), exclude_globs).unwrap();
        let hash = "test-hash".to_string();

        glob_watcher
            .watch_globs(hash.clone(), glob_set, timeout)
            .await
            .unwrap();

        // Modify a file matching the glob
        repo_root
            .join_components(&["my-pkg", "dist", "new-output"])
            .create_with_contents("build output")
            .unwrap();

        let candidates: HashSet<String> = include_globs.into_iter().collect();
        let changed = glob_watcher
            .get_changed_globs(hash.clone(), candidates, timeout)
            .await
            .unwrap();

        assert!(
            changed.contains("my-pkg/dist/**"),
            "expected dist glob to be reported as changed after file write, got: {changed:?}"
        );
    }

    /// Verifies that exclusion globs are properly respected when constructing
    /// a GlobSet from raw strings (the path InProcessOutputWatcher will use).
    #[tokio::test]
    async fn test_output_watcher_delegation_exclusions() {
        let timeout = Duration::from_secs(2);
        let (repo_root, _tmp_dir) = temp_dir();
        setup(&repo_root);

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let cookie_dir = watcher.cookie_dir().to_owned();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(&cookie_dir, timeout, recv.clone());
        let glob_watcher = GlobWatcher::new(repo_root.clone(), cookie_writer, watcher.source());

        let include_globs = vec!["my-pkg/.next/**".to_string()];
        let exclude_globs = vec!["my-pkg/.next/cache/**".to_string()];
        let glob_set = GlobSet::from_raw(include_globs.clone(), exclude_globs).unwrap();
        let hash = "test-hash".to_string();

        glob_watcher
            .watch_globs(hash.clone(), glob_set, timeout)
            .await
            .unwrap();

        // Write a file that matches the EXCLUSION glob — should not trigger change
        repo_root
            .join_components(&["my-pkg", ".next", "cache", "cached-file"])
            .create_with_contents("cached data")
            .unwrap();

        let candidates: HashSet<String> = include_globs.into_iter().collect();
        let changed = glob_watcher
            .get_changed_globs(hash.clone(), candidates, timeout)
            .await
            .unwrap();

        assert!(
            changed.is_empty(),
            "files matching exclusion globs should not trigger changes, got: {changed:?}"
        );
    }
}
