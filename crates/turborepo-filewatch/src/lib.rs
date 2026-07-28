//! File watching utilities for Turborepo. Includes a file watcher that is
//! designed to work across multiple platforms, with consistent behavior and
//! consistent ordering.
//!
//! Also includes watchers that take in file change events and produce derived
//! data like changed packages or the workspaces in a repository.
//!
//! ## Watcher Implementation
//! It's important to note that when implementing a watcher, you should aim to
//! make file change event processing as fast as possible. There should be
//! almost no slow code in the main event loop. Otherwise, the receiver will lag
//! behind, and return a `Lagged` event.
//!
//! A common pattern that we use to avoid lag is having a separate event thread
//! that processes events and accumulates them into a data structure, say a
//! `Trie` for changed files, or a `HashSet` for changed packages. From there, a
//! second thread is responsible for actually taking that accumulated data and
//! processing it. This second thread can do slower tasks like executing a run
//! or mapping files to changed packages. It can either be parked and awoken
//! using `tokio::sync::Notify` or it can run periodically using
//! `tokio::time::interval`.

#![deny(clippy::all)]
#![allow(clippy::mutable_key_type)]
#![allow(clippy::result_large_err)]

#[cfg(not(feature = "manual_recursive_watch"))]
use std::sync::atomic::AtomicBool;
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    future::IntoFuture,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

// windows -> no recursive watch, watch ancestors
// linux -> recursive watch, watch ancestors
// macos -> custom watcher impl in fsevents, no recursive watch, no watching ancestors
#[cfg(target_os = "macos")]
use fsevent::FsEventWatcher;
#[cfg(any(feature = "manual_recursive_watch", feature = "watch_ancestors"))]
use notify::event::EventKind;
#[cfg(not(target_os = "macos"))]
use notify::{Config, RecommendedWatcher};
use notify::{Event, EventHandler, RecursiveMode, Watcher};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{debug, error, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathRelation};
#[cfg(feature = "manual_recursive_watch")]
use {notify::event::CreateKind, tracing::trace, walkdir::WalkDir};

pub mod cookies;
mod debouncer;
#[cfg(target_os = "macos")]
mod fsevent;
pub mod globwatcher;
pub mod hash_watcher;
mod optional_watch;
pub mod package_watcher;
mod repository_ignore;
mod scm_resource;

pub use optional_watch::OptionalWatch;
pub use repository_ignore::RepositoryIgnore;

#[cfg(not(target_os = "macos"))]
type Backend = RecommendedWatcher;
#[cfg(target_os = "macos")]
type Backend = FsEventWatcher;

type EventResult = Result<Event, notify::Error>;

const EVENT_CHANNEL_CAPACITY: usize = 1024;

type EventFilter = dyn Fn(&mut Event) + Send + Sync;

#[derive(Default)]
struct PhysicalInterestState {
    paths: HashSet<PathBuf>,
    controls: Vec<mpsc::UnboundedSender<WatcherControl>>,
}

/// Mutable, explicit physical watch roots for paths that ordinary git-aware
/// recursion would omit. A root may be nonexistent; Linux watches its nearest
/// existing ancestor and extends the watch when the path is created.
#[derive(Clone, Default)]
pub struct WatchInterest(Arc<Mutex<PhysicalInterestState>>);

impl WatchInterest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace(&self, paths: impl IntoIterator<Item = PathBuf>) {
        let paths = paths.into_iter().collect();
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.paths == paths {
            return;
        }
        state.paths = paths;
        state
            .controls
            .retain(|control| control.send(WatcherControl::Refresh(None)).is_ok());
    }

    pub fn extend(&self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_len = state.paths.len();
        state.paths.extend(paths);
        if state.paths.len() == previous_len {
            return;
        }
        state
            .controls
            .retain(|control| control.send(WatcherControl::Refresh(None)).is_ok());
    }

    /// Waits until every attached Linux driver has applied the current roots.
    /// This is a no-op for detached interests and non-Linux backends.
    pub async fn flush(&self) {
        let controls = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .controls
            .clone();
        for control in controls {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if control.send(WatcherControl::Refresh(Some(tx))).is_ok() {
                let _ = rx.await;
            }
        }
    }

    fn bind(&self, control: mpsc::UnboundedSender<WatcherControl>) {
        #[cfg(not(feature = "manual_recursive_watch"))]
        let _ = control;
        #[cfg(feature = "manual_recursive_watch")]
        {
            let mut state = self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.controls.push(control.clone());
            let _ = control.send(WatcherControl::Refresh(None));
        }
    }

    #[cfg(feature = "manual_recursive_watch")]
    fn paths(&self) -> Vec<PathBuf> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .paths
            .iter()
            .cloned()
            .collect()
    }
}

#[derive(Debug)]
enum WatcherControl {
    Refresh(Option<tokio::sync::oneshot::Sender<()>>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceState {
    Starting,
    Ready,
    Closed,
}

#[derive(Debug, Error)]
pub enum SubscribeError {
    #[error("file watching closed before the subscription was ready")]
    Closed,
}

/// Declares which filesystem paths are relevant to a watcher consumer.
#[derive(Clone)]
pub struct WatchScope {
    filter: Arc<EventFilter>,
    physical: Option<WatchInterest>,
}

impl WatchScope {
    pub fn all() -> Self {
        Self {
            filter: Arc::new(|_| {}),
            physical: None,
        }
    }

    pub fn predicate(predicate: impl Fn(&Path) -> bool + Send + Sync + 'static) -> Self {
        Self {
            filter: Arc::new(move |event| event.paths.retain(|path| predicate(path))),
            physical: None,
        }
    }

    pub fn event_filter(filter: impl Fn(&mut Event) + Send + Sync + 'static) -> Self {
        Self {
            filter: Arc::new(filter),
            physical: None,
        }
    }

    /// Adds explicit physical coverage without changing event filtering.
    pub fn with_physical_interest(mut self, interest: WatchInterest) -> Self {
        self.physical = Some(interest);
        self
    }

    fn filter(&self, event: &mut Event) {
        (self.filter)(event);
    }
}

#[derive(Clone)]
struct SubscriptionEntry {
    scope: WatchScope,
    sender: broadcast::Sender<Result<Event, NotifyError>>,
}

struct SubscriptionRegistry {
    next_id: AtomicU64,
    state: Mutex<SubscriptionRegistryState>,
    control: mpsc::UnboundedSender<WatcherControl>,
}

#[derive(Default)]
struct SubscriptionRegistryState {
    closed: bool,
    entries: HashMap<u64, SubscriptionEntry>,
}

impl SubscriptionRegistry {
    fn new(control: mpsc::UnboundedSender<WatcherControl>) -> Self {
        Self {
            next_id: AtomicU64::new(0),
            state: Mutex::new(SubscriptionRegistryState::default()),
            control,
        }
    }

    fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.closed = true;
        state.entries.clear();
    }

    #[cfg(feature = "manual_recursive_watch")]
    fn physical_paths(&self) -> Vec<PathBuf> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .values()
            .filter_map(|entry| entry.scope.physical.as_ref())
            .flat_map(WatchInterest::paths)
            .collect()
    }
}

/// A demand-driven source of filesystem events. Each subscription receives only
/// paths matching its declared scope, so irrelevant bursts do not consume its
/// bounded channel capacity.
#[derive(Clone)]
pub struct WatchSource {
    ready: watch::Receiver<SourceState>,
    registry: Arc<SubscriptionRegistry>,
    repository_ignore: Option<RepositoryIgnore>,
}

impl WatchSource {
    #[doc(hidden)]
    pub fn channel() -> (WatchEventSender, Self) {
        let (control, _control_rx) = mpsc::unbounded_channel();
        let registry = Arc::new(SubscriptionRegistry::new(control));
        let (ready_tx, ready) = watch::channel(SourceState::Starting);
        let source = Self {
            ready,
            registry: registry.clone(),
            repository_ignore: None,
        };
        let _ = ready_tx.send(SourceState::Ready);
        (
            WatchEventSender {
                ready: ready_tx,
                registry,
                repository_ignore: None,
            },
            source,
        )
    }

    #[doc(hidden)]
    pub fn channel_for_root(root: &Path) -> (WatchEventSender, Self) {
        let (mut sender, mut source) = Self::channel();
        let repository_ignore = RepositoryIgnore::new(root);
        sender.repository_ignore = Some(repository_ignore.clone());
        source.repository_ignore = Some(repository_ignore);
        (sender, source)
    }

    pub fn repository_ignore(&self) -> Option<RepositoryIgnore> {
        self.repository_ignore.clone()
    }

    pub async fn ready(&self) -> Result<(), SubscribeError> {
        let mut ready = self.ready.clone();
        let state = ready
            .wait_for(|state| *state != SourceState::Starting)
            .await
            .map_err(|_| SubscribeError::Closed)?;
        if *state == SourceState::Closed {
            return Err(SubscribeError::Closed);
        }
        Ok(())
    }

    pub async fn subscribe(&self, scope: WatchScope) -> Result<WatchSubscription, SubscribeError> {
        let id = self.registry.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        {
            let mut state = self
                .registry
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.closed {
                return Err(SubscribeError::Closed);
            }
            state.entries.insert(
                id,
                SubscriptionEntry {
                    scope: scope.clone(),
                    sender,
                },
            );
        }

        // Registration must precede readiness: on Linux the initial physical
        // walk and readiness cookie can depend on this coverage.
        if let Some(interest) = &scope.physical {
            interest.bind(self.registry.control.clone());
        } else {
            let _ = self.registry.control.send(WatcherControl::Refresh(None));
        }

        if let Err(error) = self.ready().await {
            self.registry
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entries
                .remove(&id);
            return Err(error);
        }

        Ok(WatchSubscription {
            id,
            receiver,
            registry: self.registry.clone(),
        })
    }
}

pub struct WatchEventSender {
    ready: watch::Sender<SourceState>,
    registry: Arc<SubscriptionRegistry>,
    repository_ignore: Option<RepositoryIgnore>,
}

impl WatchEventSender {
    pub fn receiver_count(&self) -> usize {
        self.registry
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .len()
    }

    pub fn send(
        &self,
        event: Result<Event, NotifyError>,
    ) -> Result<(), broadcast::error::SendError<Result<Event, NotifyError>>> {
        let git_control_changed =
            if let (Some(repository_ignore), Ok(event)) = (&self.repository_ignore, &event) {
                let invalidates = event
                    .paths
                    .iter()
                    .any(|path| repository_ignore.invalidates_consumers(path));
                let refresh = event
                    .paths
                    .iter()
                    .any(|path| repository_ignore.should_refresh(path));
                invalidates || (refresh && repository_ignore.refresh())
            } else {
                false
            };
        if git_control_changed {
            route_event(
                &self.registry,
                Err(NotifyError::invalidation(
                    "Git index or exclude state changed",
                )),
            );
            return Ok(());
        }
        match &event {
            Ok(event) => route_event(&self.registry, Ok(event)),
            Err(error) => route_event(&self.registry, Err(error.clone())),
        }
        Ok(())
    }
}

impl Drop for WatchEventSender {
    fn drop(&mut self) {
        self.registry.close();
        let _ = self.ready.send(SourceState::Closed);
    }
}

pub struct WatchSubscription {
    id: u64,
    receiver: broadcast::Receiver<Result<Event, NotifyError>>,
    registry: Arc<SubscriptionRegistry>,
}

impl WatchSubscription {
    pub async fn recv(
        &mut self,
    ) -> Result<Result<Event, NotifyError>, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}

impl Drop for WatchSubscription {
    fn drop(&mut self) {
        self.registry
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .remove(&self.id);
        let _ = self.registry.control.send(WatcherControl::Refresh(None));
    }
}

#[derive(Debug, Error)]
pub enum WatchError {
    #[error("filewatching backend error: {0}")]
    Notify(#[from] notify::Error),
    #[error("filewatching stopped")]
    Stopped(#[from] std::sync::mpsc::RecvError),
    #[error("enumerating recursive watch: {0}")]
    WalkDir(#[from] walkdir::Error),
    #[error("enumerating git-aware recursive watch: {0}")]
    Ignore(#[from] ignore::Error),
    #[error("filewatching failed to start: {0}")]
    Setup(String),
}

// We want to broadcast the errors we get, but notify::Error does not implement
// Clone. We provide a wrapper that uses an Arc to implement Clone so that we
// can send errors on a broadcast channel.
#[derive(Clone, Debug, Error)]
#[error("{error}")]
pub struct NotifyError {
    error: Arc<notify::Error>,
    invalidation: bool,
}

impl NotifyError {
    fn invalidation(message: &str) -> Self {
        Self {
            error: Arc::new(notify::Error::generic(message)),
            invalidation: true,
        }
    }

    fn fatal(message: String) -> Self {
        Self {
            error: Arc::new(notify::Error::generic(&message)),
            invalidation: false,
        }
    }

    pub fn is_invalidation(&self) -> bool {
        self.invalidation
    }
}

impl From<notify::Error> for NotifyError {
    fn from(value: notify::Error) -> Self {
        Self {
            error: Arc::new(value),
            invalidation: false,
        }
    }
}

pub struct FileSystemWatcher {
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_ch: tokio::sync::oneshot::Sender<()>,
    cookie_dir: AbsoluteSystemPathBuf,
    source: WatchSource,
}

impl FileSystemWatcher {
    pub fn new_with_default_cookie_dir(root: &AbsoluteSystemPath) -> Result<Self, WatchError> {
        // We already store logs in .turbo and recommend it be gitignore'd.
        // Watchman uses .git, but we can't guarantee that git is present _or_
        // that the turbo root is the same as the git root.
        Self::new(root, root.join_components(&[".turbo", "cookies"]))
    }

    pub fn new(
        root: &AbsoluteSystemPath,
        cookie_dir: AbsoluteSystemPathBuf,
    ) -> Result<Self, WatchError> {
        tracing::debug!("initing file-system watcher");

        if root.relation_to_path(&cookie_dir) != PathRelation::Parent {
            return Err(WatchError::Setup(format!(
                "Invalid cookie directory: {root} does not contain {cookie_dir}"
            )));
        }

        let (send_file_events, mut recv_file_events) = mpsc::unbounded_channel();
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();
        let (ready_tx, ready_rx) = watch::channel(SourceState::Starting);
        let (watch_control_tx, watch_control_rx) = mpsc::unbounded_channel();
        let registry = Arc::new(SubscriptionRegistry::new(watch_control_tx));
        let repository_ignore = RepositoryIgnore::new(root.as_std_path());
        let source_repository_ignore = repository_ignore.clone();
        #[cfg(not(feature = "manual_recursive_watch"))]
        let backend_ready = Arc::new(AtomicBool::new(false));
        #[cfg(not(feature = "manual_recursive_watch"))]
        let ordered_driver_delivery = Arc::new(AtomicBool::new(false));

        tokio::task::spawn({
            let cookie_dir = cookie_dir.clone();
            let watch_root = root.to_owned();
            let registry = registry.clone();
            let repository_ignore = repository_ignore.clone();
            #[cfg(not(feature = "manual_recursive_watch"))]
            let backend_ready = backend_ready.clone();
            #[cfg(not(feature = "manual_recursive_watch"))]
            let ordered_driver_delivery = ordered_driver_delivery.clone();
            async move {
                // this task never yields, so run it in the blocking threadpool
                let watch_root_task = watch_root.clone();
                let cookie_dir_task = cookie_dir.clone();
                let task_repository_ignore = repository_ignore.clone();
                #[cfg(not(feature = "manual_recursive_watch"))]
                let task_backend_ready = backend_ready.clone();
                #[cfg(not(feature = "manual_recursive_watch"))]
                let task_ordered_driver_delivery = ordered_driver_delivery.clone();
                #[cfg(not(feature = "manual_recursive_watch"))]
                let task_registry = registry.clone();
                let task = tokio::task::spawn_blocking(move || {
                    setup_cookie_dir(&cookie_dir_task)?;
                    run_watcher(
                        &watch_root_task,
                        &cookie_dir_task,
                        send_file_events,
                        &task_repository_ignore,
                        #[cfg(not(feature = "manual_recursive_watch"))]
                        task_backend_ready,
                        #[cfg(not(feature = "manual_recursive_watch"))]
                        task_ordered_driver_delivery,
                        #[cfg(not(feature = "manual_recursive_watch"))]
                        task_registry,
                    )
                });

                let Ok(Ok((mut watcher, watched))) = task.await else {
                    // if the watcher fails, just return. we don't set the event sender, and other
                    // services will never start
                    error!(
                        "file watcher failed to start. watch mode and other daemon-dependent \
                         features will not work"
                    );
                    registry.close();
                    let _ = ready_tx.send(SourceState::Closed);
                    return;
                };

                // Ensure we are ready to receive new events, not events for existing state
                debug!("waiting for initial filesystem cookie");
                let mut watch_control_rx = watch_control_rx;
                let initial_watched = match wait_for_cookie(
                    &cookie_dir,
                    &watch_root,
                    &mut watcher,
                    &mut recv_file_events,
                    &registry,
                    &mut watch_control_rx,
                    watched,
                )
                .await
                {
                    Ok(watched) => watched,
                    Err(e) => {
                        // if we can't get a cookie here, we should not make the file
                        // watching available to downstream services
                        error!(
                            "failed to wait for initial filesystem cookie: {}. This means the \
                             file system event backend (e.g. FSEvents on macOS) is not delivering \
                             events. watch mode will not work. Try running `turbo daemon clean` \
                             and retrying.",
                            e
                        );
                        registry.close();
                        let _ = ready_tx.send(SourceState::Closed);
                        return;
                    }
                };
                debug!("filewatching ready");

                // Keep the bounded channel as the startup handoff (and as the
                // Linux mutation queue), but route directly after the cookie
                // has established readiness on non-mutating backends.
                #[cfg(not(feature = "manual_recursive_watch"))]
                backend_ready.store(true, Ordering::Release);

                if ready_tx.send(SourceState::Ready).is_err() {
                    tracing::debug!("no scoped downstream listeners");
                }

                watch_events(
                    watcher,
                    watch_root,
                    cookie_dir,
                    recv_file_events,
                    exit_signal,
                    registry.clone(),
                    watch_control_rx,
                    initial_watched,
                    repository_ignore,
                )
                .await;
                registry.close();
                let _ = ready_tx.send(SourceState::Closed);
            }
        });

        Ok(Self {
            _exit_ch: exit_ch,
            cookie_dir,
            source: WatchSource {
                ready: ready_rx,
                registry,
                repository_ignore: Some(source_repository_ignore),
            },
        })
    }

    /// A convenience method around the sender watcher that waits for file
    /// watching to be ready and then returns a handle to the file stream.
    pub async fn subscribe(&self) -> Result<WatchSubscription, SubscribeError> {
        self.source.subscribe(WatchScope::all()).await
    }

    pub fn watch(&self) -> WatchSource {
        self.source()
    }

    pub fn source(&self) -> WatchSource {
        self.source.clone()
    }

    pub fn cookie_dir(&self) -> &AbsoluteSystemPath {
        &self.cookie_dir
    }
}

fn setup_cookie_dir(cookie_dir: &AbsoluteSystemPath) -> Result<(), WatchError> {
    // We need to ensure that the cookie directory is cleared out first so
    // that we can start over with cookies.
    tracing::debug!("setting up the cookie dir");

    if cookie_dir.exists() {
        cookie_dir.remove_dir_all().map_err(|e| {
            WatchError::Setup(format!("failed to clear cookie dir {cookie_dir}: {e}"))
        })?;
    }
    cookie_dir
        .create_dir_all()
        .map_err(|e| WatchError::Setup(format!("failed to setup cookie dir {cookie_dir}: {e}")))?;
    Ok(())
}

#[cfg(not(any(feature = "watch_ancestors", feature = "manual_recursive_watch")))]
#[allow(clippy::too_many_arguments)]
async fn watch_events(
    mut watcher: Backend,
    watch_root: AbsoluteSystemPathBuf,
    _cookie_dir: AbsoluteSystemPathBuf,
    mut recv_file_events: mpsc::UnboundedReceiver<EventResult>,
    exit_signal: tokio::sync::oneshot::Receiver<()>,
    registry: Arc<SubscriptionRegistry>,
    _watch_control_rx: mpsc::UnboundedReceiver<WatcherControl>,
    mut watched: HashSet<PathBuf>,
    repository_ignore: RepositoryIgnore,
) {
    let mut exit_signal = exit_signal;
    'outer: loop {
        tokio::select! {
            _ = &mut exit_signal => break 'outer,
            Some(event) = recv_file_events.recv().into_future() => {
                if let Err(error) = route_non_mutating_backend_event(
                    &watch_root,
                    &registry,
                    &repository_ignore,
                    &mut watcher,
                    &mut watched,
                    event,
                ) {
                    route_fatal_driver_error(
                        &registry,
                        "failed to refresh Git control watches",
                        &error,
                    );
                    break 'outer;
                }
            }
        }
    }
}

#[cfg(any(feature = "watch_ancestors", feature = "manual_recursive_watch"))]
#[allow(clippy::too_many_arguments)]
async fn watch_events(
    #[cfg(feature = "manual_recursive_watch")] mut watcher: Backend,
    #[cfg(not(feature = "manual_recursive_watch"))] mut watcher: Backend,
    watch_root: AbsoluteSystemPathBuf,
    #[cfg(feature = "manual_recursive_watch")] cookie_dir: AbsoluteSystemPathBuf,
    #[cfg(not(feature = "manual_recursive_watch"))] _cookie_dir: AbsoluteSystemPathBuf,
    mut recv_file_events: mpsc::UnboundedReceiver<EventResult>,
    exit_signal: tokio::sync::oneshot::Receiver<()>,
    registry: Arc<SubscriptionRegistry>,
    #[cfg(feature = "manual_recursive_watch")] mut watch_control_rx: mpsc::UnboundedReceiver<
        WatcherControl,
    >,
    #[cfg(not(feature = "manual_recursive_watch"))] _watch_control_rx: mpsc::UnboundedReceiver<
        WatcherControl,
    >,
    #[cfg(feature = "manual_recursive_watch")] mut watched: HashSet<PathBuf>,
    #[cfg(not(feature = "manual_recursive_watch"))] mut watched: HashSet<PathBuf>,
    repository_ignore: RepositoryIgnore,
) {
    let mut exit_signal = exit_signal;
    #[cfg(feature = "manual_recursive_watch")]
    let mut explicit_watched =
        match explicit_watch_paths(watch_root.as_std_path(), &registry.physical_paths()) {
            Ok(watched) => watched,
            Err(error) => {
                warn!("failed to initialize explicit filesystem watches: {error}");
                route_fatal_driver_error(
                    &registry,
                    "failed to initialize explicit filesystem watches",
                    &error,
                );
                return;
            }
        };
    'outer: loop {
        tokio::select! {
            _ = &mut exit_signal => break 'outer,
            _control = async {
                #[cfg(feature = "manual_recursive_watch")]
                {
                    watch_control_rx.recv().await
                }
                #[cfg(not(feature = "manual_recursive_watch"))]
                std::future::pending::<Option<WatcherControl>>().await
            } => {
                #[cfg(feature = "manual_recursive_watch")]
                if let Some(WatcherControl::Refresh(ack)) = _control {
                    let mut acknowledgements = ack.into_iter().collect::<Vec<_>>();
                    while let Ok(WatcherControl::Refresh(ack)) = watch_control_rx.try_recv() {
                        acknowledgements.extend(ack);
                    }
                    let explicit = registry.physical_paths();
                    if let Err(error) = refresh_explicit_watches(
                        watch_root.as_std_path(),
                        cookie_dir.as_std_path(),
                        &explicit,
                        &mut watcher,
                        &mut watched,
                        &mut explicit_watched,
                        &repository_ignore,
                    ) {
                        warn!("failed to refresh explicit filesystem watches: {error}");
                        route_fatal_driver_error(
                            &registry,
                            "failed to refresh explicit filesystem watches",
                            &error,
                        );
                        break 'outer;
                    }
                    for acknowledgement in acknowledgements {
                        let _ = acknowledgement.send(());
                    }
                }
            }
            Some(event) = recv_file_events.recv().into_future() => {
                #[cfg(not(feature = "manual_recursive_watch"))]
                {
                    if let Err(error) = route_non_mutating_backend_event(
                        &watch_root,
                        &registry,
                        &repository_ignore,
                        &mut watcher,
                        &mut watched,
                        event,
                    ) {
                        route_fatal_driver_error(
                            &registry,
                            "failed to refresh Git control watches",
                            &error,
                        );
                        break 'outer;
                    }
                    continue;
                }
                #[cfg(feature = "manual_recursive_watch")]
                match event {
                    #[allow(unused_mut)]
                    Ok(mut event) => {
                        let repository_state_changed = event
                            .paths
                            .iter()
                            .any(|path| repository_ignore.should_refresh(path));
                        let previous_controls = repository_state_changed
                            .then(|| repository_ignore.control_paths());
                        let mut git_control_changed = event
                            .paths
                            .iter()
                            .any(|path| repository_ignore.invalidates_consumers(path));
                        if repository_state_changed {
                            git_control_changed |= repository_ignore.refresh();
                        }
                        // Note that we need to filter relevant events
                        // before doing manual recursive watching so that
                        // we don't try to add watches to siblings of the
                        // directories on our path to the root.
                        #[cfg(feature = "watch_ancestors")]
                        filter_relevant(&watch_root, &mut event);

                        #[cfg(feature = "manual_recursive_watch")]
                        {
                            if matches!(event.kind, EventKind::Remove(_)) {
                                for removed in &event.paths {
                                    watched.retain(|path| !path.starts_with(removed));
                                    explicit_watched.retain(|path| !path.starts_with(removed));
                                }
                            }
                            if repository_state_changed
                                && let Err(error) = add_control_watches(
                                    watch_root.as_std_path(),
                                    &repository_ignore.control_paths(),
                                    &mut watcher,
                                    &mut watched,
                                )
                            {
                                route_fatal_driver_error(
                                    &registry,
                                    "failed to refresh Git control watches",
                                    &error,
                                );
                                break 'outer;
                            }
                            if let Some(previous_controls) = previous_controls.as_deref()
                                && let Err(error) = reconcile_control_watches(
                                    watch_root.as_std_path(),
                                    previous_controls,
                                    &repository_ignore.control_paths(),
                                    &mut watcher,
                                    &mut watched,
                                    |path| {
                                        repository_ignore.is_relevant(path, true)
                                            || explicit_watched.contains(path)
                                            || cookie_dir.as_std_path().starts_with(path)
                                            || path.starts_with(cookie_dir.as_std_path())
                                    },
                                )
                            {
                                route_fatal_driver_error(
                                    &registry,
                                    "failed to reconcile Git control watches",
                                    &error,
                                );
                                break 'outer;
                            }
                            if repository_state_changed
                                && let Err(error) = reconcile_ordinary_watches(
                                watch_root.as_std_path(),
                                cookie_dir.as_std_path(),
                                &registry.physical_paths(),
                                &mut watcher,
                                &mut watched,
                                &repository_ignore,
                            ) {
                                warn!("failed to reconcile git-aware filesystem watches: {error}");
                                route_fatal_driver_error(
                                    &registry,
                                    "failed to reconcile git-aware filesystem watches",
                                    &error,
                                );
                                break 'outer;
                            }
                            if event.kind == EventKind::Create(CreateKind::Folder) {
                                for new_path in &event.paths {
                                    let explicit = registry.physical_paths();
                                    if explicit.iter().any(|interest| {
                                        interest.starts_with(new_path) || new_path.starts_with(interest)
                                    }) {
                                        if let Err(error) = refresh_explicit_watches(
                                            watch_root.as_std_path(),
                                            cookie_dir.as_std_path(),
                                            &explicit,
                                            &mut watcher,
                                            &mut watched,
                                            &mut explicit_watched,
                                            &repository_ignore,
                                        ) {
                                            warn!("failed to refresh materialized explicit filesystem watches: {error}");
                                            route_fatal_driver_error(
                                                &registry,
                                                "failed to refresh materialized explicit filesystem watches",
                                                &error,
                                            );
                                            break 'outer;
                                        }
                                        route_materialized_interests(
                                            &registry,
                                            &explicit,
                                            new_path,
                                        );
                                    } else {
                                        let result = add_ordinary_watches(
                                            watch_root.as_std_path(),
                                            new_path,
                                            &mut watcher,
                                            &mut watched,
                                            Some(&registry),
                                            &repository_ignore,
                                        );
                                        if let Err(err) = result {
                                            match err {
                                                WatchError::WalkDir(err) => {
                                                    // Likely the path no longer exists
                                                    debug!("encountered error watching filesystem {}", err);
                                                    continue;
                                                },
                                                _ => {
                                                    warn!("encountered error watching filesystem {}", err);
                                                    route_fatal_driver_error(
                                                        &registry,
                                                        "failed to extend recursive filesystem watches",
                                                        &err,
                                                    );
                                                    break 'outer;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if git_control_changed {
                            route_event(
                                &registry,
                                Err(NotifyError::invalidation(
                                    "Git index or exclude state changed",
                                )),
                            );
                        } else {
                            route_event(&registry, Ok(&event));
                        }
                    },
                    Err(error) => {
                        let error = NotifyError::from(error);
                        route_event(&registry, Err(error.clone()));
                    }
                }
            }
        }
    }
}

#[cfg(not(feature = "manual_recursive_watch"))]
fn route_non_mutating_backend_event(
    watch_root: &AbsoluteSystemPath,
    registry: &SubscriptionRegistry,
    repository_ignore: &RepositoryIgnore,
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
    event: EventResult,
) -> Result<(), WatchError> {
    match event {
        #[allow(unused_mut)]
        Ok(mut event) => {
            let repository_state_changed = event
                .paths
                .iter()
                .any(|path| repository_ignore.should_refresh(path));
            let mut git_control_changed = event
                .paths
                .iter()
                .any(|path| repository_ignore.invalidates_consumers(path));
            if repository_state_changed {
                let previous_controls = repository_ignore.control_paths();
                git_control_changed |= repository_ignore.refresh();
                reconcile_control_watches(
                    watch_root.as_std_path(),
                    &previous_controls,
                    &repository_ignore.control_paths(),
                    watcher,
                    watched,
                    |path| non_mutating_ordinary_owner(watch_root.as_std_path(), path),
                )?;
            }

            #[cfg(feature = "watch_ancestors")]
            filter_relevant(watch_root, &mut event);
            #[cfg(not(feature = "watch_ancestors"))]
            let _ = watch_root;

            if git_control_changed {
                route_event(
                    registry,
                    Err(NotifyError::invalidation(
                        "Git index or exclude state changed",
                    )),
                );
            } else {
                route_event(registry, Ok(&event));
            }
        }
        Err(error) => route_event(registry, Err(NotifyError::from(error))),
    }
    Ok(())
}

fn route_fatal_driver_error(registry: &SubscriptionRegistry, context: &str, error: &WatchError) {
    route_event(
        registry,
        Err(NotifyError::fatal(format!("{context}: {error}"))),
    );
}

fn route_event(registry: &SubscriptionRegistry, event: Result<&Event, NotifyError>) {
    let event = match event {
        Ok(event) if event.need_rescan() => Err(NotifyError::invalidation(
            "file watching backend requires a full rescan",
        )),
        event => event,
    };
    let entries: Vec<_> = registry
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entries
        .iter()
        .map(|(id, entry)| (*id, entry.clone()))
        .collect();
    let mut closed = Vec::new();
    for (id, entry) in entries {
        let scoped_event = match &event {
            Ok(event) => {
                let mut event = (*event).clone();
                entry.scope.filter(&mut event);
                if event.paths.is_empty() {
                    continue;
                }
                Ok(event)
            }
            Err(error) => Err((*error).clone()),
        };
        if entry.sender.send(scoped_event).is_err() {
            closed.push(id);
        }
    }
    if !closed.is_empty() {
        let mut entries = registry
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for id in closed {
            entries.entries.remove(&id);
        }
    }
}

// Since we're manually watching the parent directories, we need
// to handle both getting irrelevant events and getting ancestor
// events that translate to events at the root.
#[cfg(feature = "watch_ancestors")]
fn filter_relevant(root: &AbsoluteSystemPath, event: &mut Event) {
    // If path contains root && event type is modify, synthesize modify at root
    let is_modify_existing = matches!(event.kind, EventKind::Remove(_) | EventKind::Modify(_));

    event.paths.retain_mut(|path| {
        let Ok(abs_path) = <&AbsoluteSystemPath>::try_from(path.as_path()) else {
            return false;
        };
        match root.relation_to_path(abs_path) {
            // An irrelevant path, probably from a non-recursive watch of a parent directory
            PathRelation::Divergent => false,
            // A path contained in the root
            PathRelation::Parent => true,
            PathRelation::Child => {
                // If we're modifying something along the path to the
                // root, move the event to the root
                if is_modify_existing {
                    root.as_std_path().clone_into(path);
                }
                true
            }
        }
    })
}

#[cfg(feature = "watch_ancestors")]
fn is_permission_denied(result: &Result<(), notify::Error>) -> bool {
    if let Err(err) = result {
        if let notify::ErrorKind::Io(io_err) = &err.kind {
            matches!(io_err.kind(), std::io::ErrorKind::PermissionDenied)
        } else {
            false
        }
    } else {
        false
    }
}

#[cfg(feature = "watch_ancestors")]
fn watch_parents(root: &AbsoluteSystemPath, watcher: &mut Backend) -> Result<(), WatchError> {
    let mut current = root;
    while let Some(parent) = current.parent() {
        current = parent;
        let watch_result = watcher.watch(current.as_std_path(), RecursiveMode::NonRecursive);
        if is_permission_denied(&watch_result) {
            // It is expected we hit a permission denied error at some point. We won't
            // get notifications if someone e.g. deletes all of /home
            break;
        } else {
            watch_result?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "manual_recursive_watch"))]
fn watch_recursively(
    root: &AbsoluteSystemPath,
    watcher: &mut Backend,
    _watched: &mut HashSet<PathBuf>,
    _repository_ignore: &RepositoryIgnore,
) -> Result<(), WatchError> {
    watcher.watch(root.as_std_path(), RecursiveMode::Recursive)?;
    Ok(())
}

fn is_not_found(err: &notify::Error) -> bool {
    if let notify::ErrorKind::Io(ref io_err) = err.kind {
        io_err.kind() == std::io::ErrorKind::NotFound
    } else {
        false
    }
}

#[cfg(feature = "manual_recursive_watch")]
fn watch_recursively(
    root: &AbsoluteSystemPath,
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
    repository_ignore: &RepositoryIgnore,
) -> Result<(), WatchError> {
    add_ordinary_watches(
        root.as_std_path(),
        root.as_std_path(),
        watcher,
        watched,
        None,
        repository_ignore,
    )
}

#[cfg(feature = "manual_recursive_watch")]
fn add_unignored_recursive_watches(
    root: &Path,
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
    _registry: Option<&SubscriptionRegistry>,
) -> Result<(), WatchError> {
    // Note that WalkDir yields the root as well as doing the walk.
    // filter_entry prunes entire subtrees so we never descend into
    // node_modules or .git, which avoids exhausting inotify watches
    // on Linux in large monorepos.
    for dir in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name();
            name != ".git" && name != "node_modules"
        })
    {
        let dir = dir?;
        if dir.file_type().is_dir() {
            if !watched.insert(dir.path().to_owned()) {
                continue;
            }
            trace!("manually watching {}", dir.path().display());
            match watcher.watch(dir.path(), RecursiveMode::NonRecursive) {
                Ok(()) => {}
                // If we try to watch a non-existent path, we can just skip
                // it.
                Err(e) if is_not_found(&e) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(())
}

#[cfg(feature = "manual_recursive_watch")]
fn add_ordinary_watches(
    repo_root: &Path,
    subtree: &Path,
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
    registry: Option<&SubscriptionRegistry>,
    repository_ignore: &RepositoryIgnore,
) -> Result<(), WatchError> {
    let mut paths: Vec<_> = ordinary_watch_paths(repo_root, subtree, repository_ignore)?
        .into_iter()
        .collect();
    paths.sort_by_key(|path| path.components().count());
    for path in paths {
        if !watched.insert(path.clone()) {
            continue;
        }
        watcher.watch(&path, RecursiveMode::NonRecursive)?;
        if let Some(registry) = registry {
            let event = Event {
                paths: vec![path],
                kind: EventKind::Create(CreateKind::Folder),
                attrs: Default::default(),
            };
            route_event(registry, Ok(&event));
        }
    }
    Ok(())
}

#[cfg(feature = "manual_recursive_watch")]
fn ordinary_watch_paths(
    repo_root: &Path,
    subtree: &Path,
    repository_ignore: &RepositoryIgnore,
) -> Result<HashSet<PathBuf>, WatchError> {
    let subtree = subtree.to_owned();
    let mut paths = HashSet::new();
    for entry in WalkDir::new(repo_root).into_iter().filter_entry(|entry| {
        let path = entry.path();
        (subtree.starts_with(path) || path.starts_with(&subtree))
            && !matches!(entry.file_name().to_str(), Some(".git" | "node_modules"))
            && (path == repo_root || repository_ignore.is_relevant(path, true))
    }) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            paths.insert(entry.into_path());
        }
    }
    Ok(paths)
}

#[cfg(feature = "manual_recursive_watch")]
fn reconcile_ordinary_watches(
    repo_root: &Path,
    cookie_dir: &Path,
    explicit: &[PathBuf],
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
    repository_ignore: &RepositoryIgnore,
) -> Result<(), WatchError> {
    let desired = ordinary_watch_paths(repo_root, repo_root, repository_ignore)?;
    for path in &desired {
        if watched.insert(path.clone()) {
            watcher.watch(path, RecursiveMode::NonRecursive)?;
        }
    }

    let stale: Vec<_> = watched
        .iter()
        .filter(|path| {
            !desired.contains(*path)
                && !cookie_dir.starts_with(path)
                && !path.starts_with(cookie_dir)
                && !explicit
                    .iter()
                    .any(|interest| interest.starts_with(path) || path.starts_with(interest))
                && !repository_ignore
                    .control_paths()
                    .iter()
                    .any(|control| control.starts_with(path) || path.starts_with(control))
        })
        .cloned()
        .collect();
    for path in stale {
        match watcher.unwatch(&path) {
            Ok(()) => {
                watched.remove(&path);
            }
            Err(error) if is_not_found(&error) => {
                watched.remove(&path);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(feature = "manual_recursive_watch")]
fn add_explicit_watches(
    repo_root: &Path,
    interests: &[PathBuf],
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
) -> Result<(), WatchError> {
    for path in explicit_watch_paths(repo_root, interests)? {
        if watched.insert(path.clone()) {
            watcher.watch(&path, RecursiveMode::NonRecursive)?;
        }
    }
    Ok(())
}

#[cfg(feature = "manual_recursive_watch")]
fn explicit_watch_paths(
    repo_root: &Path,
    interests: &[PathBuf],
) -> Result<HashSet<PathBuf>, WatchError> {
    let mut paths = HashSet::new();
    for interest in interests.iter().filter(|path| path.starts_with(repo_root)) {
        let mut guard = interest.parent().unwrap_or(repo_root);
        while !guard.is_dir() {
            let Some(parent) = guard.parent() else {
                break;
            };
            guard = parent;
        }
        if guard.starts_with(repo_root) {
            paths.insert(guard.to_owned());
        }

        let target_is_dir = interest.is_dir();
        let mut existing = if target_is_dir {
            interest.as_path()
        } else {
            interest.parent().unwrap_or(repo_root)
        };
        while !existing.is_dir() {
            let Some(parent) = existing.parent() else {
                break;
            };
            existing = parent;
        }
        if existing.starts_with(repo_root) {
            if target_is_dir && existing == interest {
                for entry in WalkDir::new(existing)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|entry| {
                        !matches!(entry.file_name().to_str(), Some(".git" | "node_modules"))
                    })
                {
                    let entry = entry?;
                    if entry.file_type().is_dir() {
                        paths.insert(entry.into_path());
                    }
                }
            } else {
                paths.insert(existing.to_owned());
            }
        }
    }
    Ok(paths)
}

#[cfg(feature = "manual_recursive_watch")]
fn refresh_explicit_watches(
    repo_root: &Path,
    cookie_dir: &Path,
    interests: &[PathBuf],
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
    explicit_watched: &mut HashSet<PathBuf>,
    repository_ignore: &RepositoryIgnore,
) -> Result<(), WatchError> {
    let desired = explicit_watch_paths(repo_root, interests)?;
    for path in desired.difference(explicit_watched) {
        if watched.insert(path.clone()) {
            watcher.watch(path, RecursiveMode::NonRecursive)?;
        }
    }

    let stale: Vec<_> = explicit_watched.difference(&desired).cloned().collect();
    let controls = repository_ignore.control_paths();
    for path in stale {
        if !repository_ignore.is_relevant(&path, true)
            && !cookie_dir.starts_with(&path)
            && !path.starts_with(cookie_dir)
            && !controls
                .iter()
                .any(|control| control.starts_with(&path) || path.starts_with(control))
        {
            match watcher.unwatch(&path) {
                Ok(()) => {
                    watched.remove(&path);
                }
                Err(error) if is_not_found(&error) => {
                    watched.remove(&path);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    *explicit_watched = desired;
    Ok(())
}

#[cfg(feature = "manual_recursive_watch")]
fn route_materialized_interests(
    registry: &SubscriptionRegistry,
    interests: &[PathBuf],
    created: &Path,
) {
    let paths: Vec<_> = interests
        .iter()
        .filter(|interest| interest.starts_with(created) || created.starts_with(interest))
        .filter(|interest| interest.exists())
        .flat_map(|interest| {
            if interest.is_dir() {
                WalkDir::new(interest)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(Result::ok)
                    .map(|entry| entry.into_path())
                    .collect()
            } else {
                vec![interest.clone()]
            }
        })
        .collect();
    if paths.is_empty() {
        return;
    }
    let event = Event {
        paths,
        kind: EventKind::Create(CreateKind::Any),
        attrs: Default::default(),
    };
    route_event(registry, Ok(&event));
}

fn run_watcher(
    root: &AbsoluteSystemPath,
    cookie_dir: &AbsoluteSystemPath,
    sender: mpsc::UnboundedSender<EventResult>,
    repository_ignore: &RepositoryIgnore,
    #[cfg(not(feature = "manual_recursive_watch"))] backend_ready: Arc<AtomicBool>,
    #[cfg(not(feature = "manual_recursive_watch"))] ordered_driver_delivery: Arc<AtomicBool>,
    #[cfg(not(feature = "manual_recursive_watch"))] registry: Arc<SubscriptionRegistry>,
) -> Result<(Backend, HashSet<PathBuf>), WatchError> {
    #[cfg(feature = "manual_recursive_watch")]
    let mut watcher = make_watcher(move |event| {
        let _ = sender.send(event);
    })?;
    #[cfg(not(feature = "manual_recursive_watch"))]
    let mut watcher = {
        let watch_root = root.to_owned();
        let readiness_cookie = cookie_dir.join_component(".turbo-cookie");
        let repository_ignore = repository_ignore.clone();
        make_watcher(move |event| {
            dispatch_non_mutating_backend_event(
                &backend_ready,
                &ordered_driver_delivery,
                &sender,
                &readiness_cookie,
                &watch_root,
                &registry,
                &repository_ignore,
                event,
            );
        })?
    };

    let mut watched = HashSet::new();
    watch_recursively(root, &mut watcher, &mut watched, repository_ignore)?;

    add_control_watches(
        root.as_std_path(),
        &repository_ignore.control_paths(),
        &mut watcher,
        &mut watched,
    )?;

    #[cfg(feature = "manual_recursive_watch")]
    add_unignored_recursive_watches(cookie_dir.as_std_path(), &mut watcher, &mut watched, None)?;

    #[cfg(feature = "watch_ancestors")]
    watch_parents(root, &mut watcher)?;
    Ok((watcher, watched))
}

#[cfg(not(feature = "manual_recursive_watch"))]
#[allow(clippy::too_many_arguments)]
fn dispatch_non_mutating_backend_event(
    backend_ready: &AtomicBool,
    ordered_driver_delivery: &AtomicBool,
    startup_sender: &mpsc::UnboundedSender<EventResult>,
    readiness_cookie: &AbsoluteSystemPath,
    watch_root: &AbsoluteSystemPath,
    registry: &SubscriptionRegistry,
    repository_ignore: &RepositoryIgnore,
    event: EventResult,
) {
    if backend_ready.load(Ordering::Acquire) && !ordered_driver_delivery.load(Ordering::Acquire) {
        let requires_driver = event.as_ref().is_ok_and(|event| {
            event
                .paths
                .iter()
                .any(|path| repository_ignore.should_refresh(path))
        });
        if requires_driver {
            // Refreshing RepositoryIgnore can spawn Git and walk the worktree.
            // Backend callback threads only classify control events and hand
            // them to the async driver; ordinary bursts remain direct.
            ordered_driver_delivery.store(true, Ordering::Release);
            let _ = startup_sender.send(event);
        } else {
            route_direct_non_mutating_event(watch_root, registry, event);
        }
    } else {
        let establishes_readiness = event.as_ref().is_ok_and(|event| {
            event
                .paths
                .iter()
                .any(|path| path == readiness_cookie.as_std_path())
        });
        if startup_sender.send(event).is_ok() && establishes_readiness {
            // Event handlers are invoked in backend order. Publishing only
            // after the cookie is queued keeps pre-ready events on the startup
            // channel while allowing the next callback to route directly.
            backend_ready.store(true, Ordering::Release);
        }
    }
}

#[cfg(not(feature = "manual_recursive_watch"))]
fn route_direct_non_mutating_event(
    watch_root: &AbsoluteSystemPath,
    registry: &SubscriptionRegistry,
    event: EventResult,
) {
    match event {
        #[allow(unused_mut)]
        Ok(mut event) => {
            #[cfg(feature = "watch_ancestors")]
            filter_relevant(watch_root, &mut event);
            #[cfg(not(feature = "watch_ancestors"))]
            let _ = watch_root;
            route_event(registry, Ok(&event));
        }
        Err(error) => route_event(registry, Err(NotifyError::from(error))),
    }
}

fn add_control_watches(
    repo_root: &Path,
    controls: &[PathBuf],
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
) -> Result<(), WatchError> {
    for parent in control_watch_parents(repo_root, controls) {
        if parent.is_dir() && watched.insert(parent.to_owned()) {
            watcher.watch(&parent, RecursiveMode::NonRecursive)?;
        }
    }
    Ok(())
}

fn reconcile_control_watches(
    repo_root: &Path,
    previous_controls: &[PathBuf],
    current_controls: &[PathBuf],
    watcher: &mut Backend,
    watched: &mut HashSet<PathBuf>,
    is_owned_elsewhere: impl Fn(&Path) -> bool,
) -> Result<(), WatchError> {
    let (additions, removals) = control_watch_changes(
        repo_root,
        previous_controls,
        current_controls,
        watched,
        is_owned_elsewhere,
    );

    for parent in additions {
        watcher.watch(&parent, RecursiveMode::NonRecursive)?;
        watched.insert(parent);
    }
    for stale in removals {
        match watcher.unwatch(&stale) {
            Ok(()) => {
                watched.remove(&stale);
            }
            Err(error) if is_not_found(&error) => {
                watched.remove(&stale);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn control_watch_changes(
    repo_root: &Path,
    previous_controls: &[PathBuf],
    current_controls: &[PathBuf],
    watched: &HashSet<PathBuf>,
    is_owned_elsewhere: impl Fn(&Path) -> bool,
) -> (HashSet<PathBuf>, HashSet<PathBuf>) {
    let previous = control_watch_parents(repo_root, previous_controls);
    let current = control_watch_parents(repo_root, current_controls);
    let additions = current
        .iter()
        .filter(|parent| parent.is_dir() && !watched.contains(*parent))
        .cloned()
        .collect();
    let removals = previous
        .difference(&current)
        .filter(|stale| watched.contains(*stale) && !is_owned_elsewhere(stale))
        .cloned()
        .collect();
    (additions, removals)
}

fn control_watch_parents(repo_root: &Path, controls: &[PathBuf]) -> HashSet<PathBuf> {
    #[cfg(not(target_os = "macos"))]
    let _ = repo_root;
    controls
        .iter()
        .filter_map(|control| {
            let parent = control.parent()?;
            if !cfg!(feature = "manual_recursive_watch") && parent.starts_with(repo_root) {
                return None;
            }
            #[cfg(target_os = "macos")]
            {
                use std::os::unix::fs::MetadataExt;

                let root_device = std::fs::metadata(repo_root).map(|metadata| metadata.dev());
                let parent_device = std::fs::metadata(parent).map(|metadata| metadata.dev());
                if matches!((root_device, parent_device), (Ok(root), Ok(parent)) if root != parent)
                {
                    debug!(
                        path = %control.display(),
                        "skipping cross-device Git control watch"
                    );
                    return None;
                }
            }
            Some(parent.to_owned())
        })
        .collect()
}

#[cfg(not(feature = "manual_recursive_watch"))]
fn non_mutating_ordinary_owner(repo_root: &Path, path: &Path) -> bool {
    if path == repo_root {
        return true;
    }
    #[cfg(feature = "watch_ancestors")]
    if repo_root.starts_with(path) {
        return true;
    }
    false
}

#[cfg(not(target_os = "macos"))]
fn make_watcher<F: EventHandler>(event_handler: F) -> Result<Backend, notify::Error> {
    RecommendedWatcher::new(event_handler, Config::default())
}

#[cfg(target_os = "macos")]
fn make_watcher<F: EventHandler>(event_handler: F) -> Result<Backend, notify::Error> {
    FsEventWatcher::new(event_handler, notify::Config::default())
}

/// wait_for_cookie performs a roundtrip through the filewatching mechanism.
/// This ensures that we are ready to receive *new* filesystem events, rather
/// than receiving events from existing state, which some backends can do.
async fn wait_for_cookie(
    cookie_dir: &AbsoluteSystemPath,
    _watch_root: &AbsoluteSystemPath,
    _watcher: &mut Backend,
    recv: &mut mpsc::UnboundedReceiver<EventResult>,
    _registry: &SubscriptionRegistry,
    watch_control_rx: &mut mpsc::UnboundedReceiver<WatcherControl>,
    #[allow(unused_mut)] mut watched: HashSet<PathBuf>,
) -> Result<HashSet<PathBuf>, WatchError> {
    // TODO: should this be passed in? Currently the caller guarantees that the
    // directory is empty, but it could be the responsibility of the
    // filewatcher...
    let cookie_path = cookie_dir.join_component(".turbo-cookie");
    cookie_path
        .create_with_contents("cookie")
        .map_err(|e| WatchError::Setup(format!("failed to write cookie to {cookie_path}: {e}")))?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
    loop {
        let event = tokio::select! {
            biased;
            control = watch_control_rx.recv() => {
                if let Some(WatcherControl::Refresh(ack)) = control {
                    let mut acknowledgements = ack.into_iter().collect::<Vec<_>>();
                    while let Ok(WatcherControl::Refresh(ack)) = watch_control_rx.try_recv() {
                        acknowledgements.extend(ack);
                    }
                    #[cfg(feature = "manual_recursive_watch")]
                    add_explicit_watches(
                        _watch_root.as_std_path(),
                        &_registry.physical_paths(),
                        _watcher,
                        &mut watched,
                    )?;
                    for acknowledgement in acknowledgements {
                        let _ = acknowledgement.send(());
                    }
                }
                continue;
            }
            event = tokio::time::timeout_at(deadline, recv.recv()) => {
                event
                    .map_err(|e| WatchError::Setup(format!("waiting for cookie timed out: {e}")))?
                    .ok_or_else(|| WatchError::Setup(
                        "filewatching closed before cookie file was observed".to_string(),
                    ))?
                    .map_err(|err| WatchError::Setup(format!("initial watch encountered errors: {err}")))?
            }
        };
        if event.paths.iter().any(|path| {
            let path: &Path = path;
            path == (&cookie_path as &AbsoluteSystemPath)
        }) {
            // We don't need to stop everything if we failed to remove the cookie file
            // for some reason. We can warn about it though.
            if let Err(e) = cookie_path.remove() {
                warn!("failed to remove cookie file {e}");
            }
            return Ok(watched);
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        assert_matches,
        path::Path,
        sync::{Arc, atomic::AtomicUsize},
        time::Duration,
    };

    #[cfg(not(target_os = "windows"))]
    use notify::event::RenameMode;
    use notify::{Event, EventKind, event::ModifyKind};
    use tokio::sync::{broadcast, mpsc};
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use crate::FileSystemWatcher;

    fn temp_dir() -> (AbsoluteSystemPathBuf, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        (path, tmp)
    }

    #[test]
    fn control_watch_changes_add_new_and_preserve_other_owners() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let added = temp.path().join("added");
        let removed = temp.path().join("removed");
        let ordinary = root.clone();
        let explicit = temp.path().join("explicit");
        for directory in [&root, &added, &removed, &explicit] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let previous = [
            removed.join("control"),
            ordinary.join("control"),
            explicit.join("control"),
        ];
        let current = [added.join("control")];
        let watched = [removed.clone(), ordinary.clone(), explicit.clone()]
            .into_iter()
            .collect();

        let (additions, removals) =
            super::control_watch_changes(&root, &previous, &current, &watched, |path| {
                path == ordinary || path == explicit
            });

        assert_eq!(additions, [added].into_iter().collect());
        assert_eq!(removals, [removed].into_iter().collect());
    }

    #[tokio::test]
    async fn scoped_subscription_drops_irrelevant_burst_before_channel() {
        let (control, _control_rx) = mpsc::unbounded_channel();
        let registry = Arc::new(super::SubscriptionRegistry::new(control));
        let (_ready_tx, ready) = tokio::sync::watch::channel(super::SourceState::Ready);
        let source = super::WatchSource {
            ready,
            registry: registry.clone(),
            repository_ignore: None,
        };
        let mut subscription = source
            .subscribe(super::WatchScope::predicate(|path| {
                !path
                    .components()
                    .any(|component| component.as_os_str() == ".cache")
            }))
            .await
            .unwrap();

        for index in 0..30_000 {
            let event = Event {
                paths: vec![Path::new("repo/.cache").join(format!("output-{index}"))],
                kind: EventKind::Create(notify::event::CreateKind::File),
                attrs: Default::default(),
            };
            super::route_event(&registry, Ok(&event));
        }

        let source_path = Path::new("repo/src/index.ts").to_path_buf();
        let event = Event {
            paths: vec![source_path.clone()],
            kind: EventKind::Create(notify::event::CreateKind::File),
            attrs: Default::default(),
        };
        super::route_event(&registry, Ok(&event));

        let received = subscription.recv().await.unwrap().unwrap();
        assert_eq!(received.paths, vec![source_path]);
    }

    #[cfg(not(feature = "manual_recursive_watch"))]
    #[tokio::test]
    async fn post_ready_backend_events_route_without_bounded_channel() {
        let (repo_root, _tmp) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();
        let (control, _control_rx) = mpsc::unbounded_channel();
        let registry = Arc::new(super::SubscriptionRegistry::new(control));
        let (_ready_tx, ready) = tokio::sync::watch::channel(super::SourceState::Ready);
        let source = super::WatchSource {
            ready,
            registry: registry.clone(),
            repository_ignore: None,
        };
        let expected = repo_root.join_components(&["src", "index.ts"]);
        let mut subscription = source
            .subscribe(super::WatchScope::predicate({
                let expected = expected.clone();
                move |path| path == expected.as_std_path()
            }))
            .await
            .unwrap();
        let (startup_sender, mut startup_receiver) = mpsc::unbounded_channel();
        let backend_ready = std::sync::atomic::AtomicBool::new(true);
        let ordered_driver_delivery = std::sync::atomic::AtomicBool::new(false);
        let repository_ignore = super::RepositoryIgnore::new(repo_root.as_std_path());

        for index in 0..30_000 {
            super::dispatch_non_mutating_backend_event(
                &backend_ready,
                &ordered_driver_delivery,
                &startup_sender,
                &repo_root.join_components(&[".turbo", "cookies", ".turbo-cookie"]),
                &repo_root,
                &registry,
                &repository_ignore,
                Ok(
                    Event::new(EventKind::Create(notify::event::CreateKind::File)).add_path(
                        repo_root
                            .join_components(&[".cache", &format!("output-{index}")])
                            .as_std_path()
                            .to_owned(),
                    ),
                ),
            );
        }
        super::dispatch_non_mutating_backend_event(
            &backend_ready,
            &ordered_driver_delivery,
            &startup_sender,
            &repo_root.join_components(&[".turbo", "cookies", ".turbo-cookie"]),
            &repo_root,
            &registry,
            &repository_ignore,
            Ok(
                Event::new(EventKind::Create(notify::event::CreateKind::File))
                    .add_path(expected.as_std_path().to_owned()),
            ),
        );

        assert!(startup_receiver.try_recv().is_err());
        let event = subscription.recv().await.unwrap().unwrap();
        assert_eq!(event.paths, vec![expected.as_std_path().to_owned()]);
    }

    #[cfg(not(feature = "manual_recursive_watch"))]
    #[test]
    fn post_ready_ignore_events_are_deferred_to_driver() {
        let (repo_root, _tmp) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();
        let (control, _control_rx) = mpsc::unbounded_channel();
        let registry = Arc::new(super::SubscriptionRegistry::new(control));
        let (startup_sender, mut startup_receiver) = mpsc::unbounded_channel();
        let backend_ready = std::sync::atomic::AtomicBool::new(true);
        let ordered_driver_delivery = std::sync::atomic::AtomicBool::new(false);
        let repository_ignore = super::RepositoryIgnore::new(repo_root.as_std_path());
        let gitignore = repo_root.join_component(".gitignore");

        super::dispatch_non_mutating_backend_event(
            &backend_ready,
            &ordered_driver_delivery,
            &startup_sender,
            &repo_root.join_components(&[".turbo", "cookies", ".turbo-cookie"]),
            &repo_root,
            &registry,
            &repository_ignore,
            Ok(Event::new(EventKind::Modify(ModifyKind::Any))
                .add_path(gitignore.as_std_path().to_owned())),
        );

        let queued = startup_receiver.try_recv().unwrap().unwrap();
        assert_eq!(queued.paths, vec![gitignore.as_std_path().to_owned()]);

        let source = repo_root.join_components(&["src", "index.ts"]);
        super::dispatch_non_mutating_backend_event(
            &backend_ready,
            &ordered_driver_delivery,
            &startup_sender,
            &repo_root.join_components(&[".turbo", "cookies", ".turbo-cookie"]),
            &repo_root,
            &registry,
            &repository_ignore,
            Ok(Event::new(EventKind::Modify(ModifyKind::Any))
                .add_path(source.as_std_path().to_owned())),
        );
        let queued = startup_receiver.try_recv().unwrap().unwrap();
        assert_eq!(queued.paths, vec![source.as_std_path().to_owned()]);
    }

    #[cfg(feature = "manual_recursive_watch")]
    #[tokio::test]
    async fn fatal_driver_failure_reaches_subscriber_before_close() {
        let (control, _control_rx) = mpsc::unbounded_channel();
        let registry = Arc::new(super::SubscriptionRegistry::new(control));
        let (_ready_tx, ready) = tokio::sync::watch::channel(super::SourceState::Ready);
        let source = super::WatchSource {
            ready,
            registry: registry.clone(),
            repository_ignore: None,
        };
        let mut subscription = source.subscribe(super::WatchScope::all()).await.unwrap();

        super::route_fatal_driver_error(
            &registry,
            "failed to reconcile filesystem watches",
            &super::WatchError::Setup("injected failure".to_string()),
        );
        registry.close();

        let error = subscription.recv().await.unwrap().unwrap_err();
        assert!(!error.is_invalidation());
        assert!(error.to_string().contains("injected failure"));
    }

    #[tokio::test]
    async fn rescan_events_bypass_subscription_scopes() {
        let (sender, source) = super::WatchSource::channel();
        let mut first = source
            .subscribe(super::WatchScope::predicate(|path| path.ends_with("first")))
            .await
            .unwrap();
        let mut second = source
            .subscribe(super::WatchScope::predicate(|path| {
                path.ends_with("second")
            }))
            .await
            .unwrap();
        sender
            .send(Ok(Event::new(EventKind::Other)
                .set_flag(notify::event::Flag::Rescan)
                .add_path(Path::new("outside-both-scopes").to_path_buf())))
            .unwrap();

        assert!(first.recv().await.unwrap().is_err());
        assert!(second.recv().await.unwrap().is_err());
    }

    #[tokio::test]
    async fn dropping_scoped_subscription_unregisters_it() {
        let (control, _control_rx) = mpsc::unbounded_channel();
        let registry = Arc::new(super::SubscriptionRegistry::new(control));
        let (_ready_tx, ready) = tokio::sync::watch::channel(super::SourceState::Ready);
        let source = super::WatchSource {
            ready,
            registry: registry.clone(),
            repository_ignore: None,
        };
        let subscription = source.subscribe(super::WatchScope::all()).await.unwrap();
        assert_eq!(
            registry
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entries
                .len(),
            1
        );

        drop(subscription);

        assert!(
            registry
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entries
                .is_empty()
        );
    }

    macro_rules! expect_filesystem_event {
        ($recv:ident, $expected_path:expr, $pattern:pat) => {
            'outer: loop {
                let event = tokio::time::timeout(Duration::from_millis(3000), $recv.recv())
                    .await
                    .expect("timed out waiting for filesystem event")
                    .expect("sender was dropped")
                    .expect("filewatching error");
                for path in event.paths {
                    if path == (&$expected_path as &AbsoluteSystemPath)
                        && matches!(event.kind, $pattern)
                    {
                        break 'outer;
                    }
                }
            }
        };
    }

    static WATCH_COUNT: AtomicUsize = AtomicUsize::new(0);

    async fn expect_watching(recv: &mut super::WatchSubscription, dirs: &[&AbsoluteSystemPath]) {
        for dir in dirs {
            let count = WATCH_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let filename = dir.join_component(format!("test-{count}").as_str());
            filename.create_with_contents("hello").unwrap();

            expect_filesystem_event!(recv, filename, EventKind::Create(_));
        }
    }

    #[tokio::test]
    async fn test_file_watching() {
        // Directory layout:
        // <repoRoot>/
        //	 .git/
        //   node_modules/
        //     some-dep/
        //   parent/
        //     child/
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();
        let sibling_path = parent_path.join_component("sibling");
        sibling_path.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();

        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;
        let foo_path = child_path.join_component("foo");
        foo_path.create_with_contents("hello").unwrap();
        expect_filesystem_event!(recv, foo_path, EventKind::Create(_));

        let deep_path = sibling_path.join_components(&["deep", "path"]);
        deep_path.create_dir_all().unwrap();
        expect_filesystem_event!(
            recv,
            sibling_path.join_component("deep"),
            EventKind::Create(_)
        );
        expect_filesystem_event!(recv, deep_path, EventKind::Create(_));
        expect_watching(
            &mut recv,
            &[
                &repo_root,
                &parent_path,
                &child_path,
                &deep_path,
                &sibling_path.join_component("deep"),
            ],
        )
        .await;

        let test_file_path = repo_root.join_component("test-file");
        test_file_path
            .create_with_contents("test contents")
            .unwrap();
        expect_filesystem_event!(recv, test_file_path, EventKind::Create(_));
    }

    #[tokio::test]
    async fn test_file_watching_subfolder_deletion() {
        // Directory layout:
        // <repoRoot>/
        //	 .git/
        //   node_modules/
        //     some-dep/
        //   parent/
        //     child/
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();

        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        // Delete parent folder during file watching
        parent_path.remove_dir_all().unwrap();
        expect_filesystem_event!(recv, parent_path, EventKind::Remove(_));

        // Ensure we get events when creating file in deleted directory
        child_path.create_dir_all().unwrap();
        expect_filesystem_event!(recv, parent_path, EventKind::Create(_));
        expect_filesystem_event!(recv, child_path, EventKind::Create(_));

        let foo_path = child_path.join_component("foo");
        foo_path.create_with_contents("hello").unwrap();
        expect_filesystem_event!(recv, foo_path, EventKind::Create(_));
        // We cannot guarantee no more events, windows sends multiple delete
        // events
    }

    #[tokio::test]
    async fn test_file_watching_root_deletion() {
        // Directory layout:
        // <repoRoot>/
        //	 .git/
        //   node_modules/
        //     some-dep/
        //   parent/
        //     child/
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        repo_root.remove_dir_all().unwrap();
        expect_filesystem_event!(recv, repo_root, EventKind::Remove(_));
    }

    #[tokio::test]
    async fn test_file_watching_subfolder_rename() {
        // Directory layout:
        // <repoRoot>/
        //	 .git/
        //   node_modules/
        //     some-dep/
        //   parent/
        //     child/
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        let new_parent = repo_root.join_component("new_parent");
        parent_path.rename(&new_parent).unwrap();

        expect_filesystem_event!(recv, new_parent, EventKind::Modify(ModifyKind::Name(_)));
    }

    #[tokio::test]
    async fn test_file_watching_root_rename() {
        // Directory layout:
        // <repoRoot>/
        //	 .git/
        //   node_modules/
        //     some-dep/
        //   parent/
        //     child/
        let (tmp_root, _tmp_repo_root) = temp_dir();
        let tmp_root = tmp_root.to_realpath().unwrap();
        let repo_root = tmp_root.join_component("repo_root");

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        let new_repo_root = repo_root.parent().unwrap().join_component("new_repo_root");
        repo_root.rename(&new_repo_root).unwrap();

        expect_filesystem_event!(recv, repo_root, EventKind::Modify(ModifyKind::Name(_)));
    }

    #[tokio::test]
    async fn test_file_watching_symlink_create() {
        // Directory layout:
        // <repoRoot>/
        //	 .git/
        //   node_modules/
        //     some-dep/
        //   parent/
        //     child/
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        // Create symlink during file watching
        let symlink_path = repo_root.join_component("symlink");
        symlink_path.symlink_to_dir(child_path.as_str()).unwrap();
        expect_filesystem_event!(recv, symlink_path, EventKind::Create(_));

        // we expect that events in the symlinked directory will be raised with the
        // original path
        let symlink_subfile = symlink_path.join_component("symlink_subfile");
        symlink_subfile.create_with_contents("hello").unwrap();
        let expected_subfile_path = child_path.join_component("symlink_subfile");
        expect_filesystem_event!(recv, expected_subfile_path, EventKind::Create(_));
    }

    #[tokio::test]
    async fn test_file_watching_symlink_delete() {
        // Directory layout:
        // <repoRoot>/
        //	 .git/
        //   node_modules/
        //     some-dep/
        //   parent/
        //     child/
        //   symlink -> parent/child
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();
        let symlink_path = repo_root.join_component("symlink");
        symlink_path.symlink_to_dir(child_path.as_str()).unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        // Delete symlink during file watching
        // Note that on Windows, to remove a symlink to a directory
        // remove_dir is required.
        #[cfg(windows)]
        symlink_path.remove_dir().unwrap();
        #[cfg(not(windows))]
        symlink_path.remove().unwrap();
        expect_filesystem_event!(recv, symlink_path, EventKind::Remove(_));
    }

    #[tokio::test]
    async fn test_file_watching_symlink_rename() {
        // Directory layout:
        // <repoRoot>/
        //	 .git/
        //   node_modules/
        //     some-dep/
        //   parent/
        //     child/
        //   symlink -> parent/child

        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();
        let symlink_path = repo_root.join_component("symlink");
        symlink_path.symlink_to_dir(child_path.as_str()).unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        // Delete symlink during file watching
        let new_symlink_path = repo_root.join_component("new_symlink");
        symlink_path.rename(&new_symlink_path).unwrap();
        expect_filesystem_event!(
            recv,
            new_symlink_path,
            EventKind::Modify(ModifyKind::Name(_))
        );
    }

    // Watching a directory on windows locks it, so we cannot rename it.
    // Since we are recursively watching parents, we also cannot rename parents.
    // Note the contrast to the root of our watch, which we don't lock,
    // but instead rely on watching the parent directory. This means we
    // have permission to rename or delete the repo root, but not anything
    // else in the path.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn test_file_watching_root_parent_rename() {
        // Directory layout:
        // repo_parent/
        //   repo_root/
        //     .git/
        //     node_modules/
        //       some-dep/
        //     parent/
        //       child/
        let (tmp_root, _tmp_repo_root) = temp_dir();
        let tmp_root = tmp_root.to_realpath().unwrap().join_component("layer");
        let repo_root = tmp_root.join_components(&["repo_parent", "repo_root"]);

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        let repo_parent = repo_root.parent().unwrap();
        let new_parent = tmp_root.join_component("new_parent");
        repo_parent.rename(&new_parent).unwrap();

        expect_filesystem_event!(
            recv,
            repo_root,
            EventKind::Modify(ModifyKind::Name(RenameMode::From))
        );
    }

    #[tokio::test]
    async fn test_file_watching_root_parent_delete() {
        // Directory layout:
        // repo_parent/
        //   repo_root/
        //     .git/
        //     node_modules/
        //       some-dep/
        //     parent/
        //       child/
        let (tmp_root, _tmp_repo_root) = temp_dir();
        let tmp_root = tmp_root.to_realpath().unwrap();
        let repo_root = tmp_root.join_components(&["repo_parent", "repo_root"]);

        repo_root.join_component(".git").create_dir_all().unwrap();
        repo_root
            .join_components(&["node_modules", "some-dep"])
            .create_dir_all()
            .unwrap();
        let parent_path = repo_root.join_component("parent");
        let child_path = parent_path.join_component("child");
        child_path.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        expect_watching(&mut recv, &[&repo_root, &parent_path, &child_path]).await;

        let repo_parent = repo_root.parent().unwrap();
        repo_parent.remove_dir_all().unwrap();
        expect_filesystem_event!(
            recv,
            repo_root,
            EventKind::Modify(ModifyKind::Name(_)) | EventKind::Remove(_)
        );
    }

    // Verify that node_modules and .git are excluded from inotify watch
    // registration on Linux to avoid exhausting the OS watch limit in large
    // monorepos. No .gitignore is needed — these are hardcoded exclusions.
    #[cfg(all(feature = "manual_recursive_watch", not(target_os = "macos")))]
    #[tokio::test]
    async fn test_file_watching_hardcoded_exclusions() {
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        let node_modules = repo_root.join_component("node_modules");
        node_modules
            .join_components(&["some-dep", "lib"])
            .create_dir_all()
            .unwrap();

        let git_dir = repo_root.join_component(".git");
        git_dir.create_dir_all().unwrap();

        let src_dir = repo_root.join_component("src");
        src_dir.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();

        expect_watching(&mut recv, &[&repo_root, &src_dir]).await;

        // Write to node_modules and .git — should NOT generate watch events
        // because these subtrees are never registered with inotify.
        node_modules
            .join_components(&["some-dep", "lib", "index.js"])
            .create_with_contents("module.exports = {}")
            .unwrap();
        git_dir
            .join_component("COMMIT_EDITMSG")
            .create_with_contents("initial commit")
            .unwrap();

        // Write a sentinel to a watched directory so we can drain
        // the event stream and confirm no node_modules/.git events arrived.
        let sentinel = src_dir.join_component("sentinel.js");
        sentinel.create_with_contents("export {}").unwrap();

        let ignored_dirs = [&node_modules, &git_dir];
        loop {
            let event = tokio::time::timeout(Duration::from_millis(3000), recv.recv())
                .await
                .expect("timed out waiting for sentinel event")
                .expect("sender was dropped")
                .expect("filewatching error");
            for path in &event.paths {
                for ignored in &ignored_dirs {
                    assert!(
                        !path.starts_with(ignored.as_std_path()),
                        "received unexpected event from excluded dir {ignored}: {path:?}"
                    );
                }
            }
            if event.paths.iter().any(|p| {
                let p: &std::path::Path = p;
                p == (&sentinel as &AbsoluteSystemPath)
            }) {
                break;
            }
        }
    }

    #[cfg(all(feature = "manual_recursive_watch", not(target_os = "macos")))]
    async fn assert_unseen_before_sentinel(
        recv: &mut super::WatchSubscription,
        unseen: &AbsoluteSystemPath,
        sentinel: &AbsoluteSystemPath,
    ) {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(3), recv.recv())
                .await
                .expect("timed out waiting for sentinel")
                .expect("watch source closed")
                .expect("filewatching error");
            assert!(
                event.paths.iter().all(|path| !path.starts_with(unseen)),
                "received event from physically pruned tree: {:?}",
                event.paths
            );
            if event.paths.iter().any(|path| path == sentinel) {
                return;
            }
        }
    }

    #[cfg(all(feature = "manual_recursive_watch", not(target_os = "macos")))]
    #[tokio::test]
    async fn ignored_startup_tree_is_not_watched() {
        let (repo_root, _tmp) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();
        repo_root
            .join_component(".gitignore")
            .create_with_contents("ignored/\n")
            .unwrap();
        let ignored = repo_root.join_components(&["ignored", "nested"]);
        ignored.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();
        let unseen = ignored.join_component("file");
        unseen.create_with_contents("ignored").unwrap();
        let sentinel = repo_root.join_component("sentinel");
        sentinel.create_with_contents("seen").unwrap();
        assert_unseen_before_sentinel(&mut recv, &unseen, &sentinel).await;
    }

    #[cfg(all(feature = "manual_recursive_watch", not(target_os = "macos")))]
    #[tokio::test]
    async fn explicit_ignored_interest_is_watched() {
        let (repo_root, _tmp) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();
        repo_root
            .join_component(".gitignore")
            .create_with_contents("ignored/\n")
            .unwrap();
        let ignored = repo_root.join_component("ignored");
        ignored.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let interest = super::WatchInterest::new();
        interest.replace([ignored.as_std_path().to_owned()]);
        let mut recv = watcher
            .source()
            .subscribe(super::WatchScope::all().with_physical_interest(interest.clone()))
            .await
            .unwrap();
        interest.flush().await;

        let file = ignored.join_component("output");
        file.create_with_contents("watched").unwrap();
        expect_filesystem_event!(recv, file, EventKind::Create(_));
    }

    #[cfg(all(feature = "manual_recursive_watch", not(target_os = "macos")))]
    #[tokio::test]
    async fn nonexistent_explicit_interest_is_watched_after_creation() {
        let (repo_root, _tmp) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();
        repo_root
            .join_component(".gitignore")
            .create_with_contents("ignored/\n")
            .unwrap();
        let future_dir = repo_root.join_components(&["ignored", "generated", "deep"]);

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let interest = super::WatchInterest::new();
        interest.replace([future_dir.as_std_path().to_owned()]);
        let mut recv = watcher
            .source()
            .subscribe(super::WatchScope::all().with_physical_interest(interest.clone()))
            .await
            .unwrap();
        interest.flush().await;

        future_dir.create_dir_all().unwrap();
        let file = future_dir.join_component("input");
        file.create_with_contents("watched").unwrap();
        expect_filesystem_event!(recv, file, EventKind::Create(_));
    }

    #[cfg(all(feature = "manual_recursive_watch", not(target_os = "macos")))]
    #[tokio::test]
    async fn existing_ignored_interest_is_watched_after_recreation() {
        let (repo_root, _tmp) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();
        repo_root
            .join_component(".gitignore")
            .create_with_contents("ignored/\n")
            .unwrap();
        let ignored = repo_root.join_component("ignored");
        ignored.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let interest = super::WatchInterest::new();
        interest.replace([ignored.as_std_path().to_owned()]);
        let mut recv = watcher
            .source()
            .subscribe(super::WatchScope::all().with_physical_interest(interest.clone()))
            .await
            .unwrap();
        interest.flush().await;

        ignored.remove_dir_all().unwrap();
        expect_filesystem_event!(recv, ignored, EventKind::Remove(_));
        ignored.create_dir_all().unwrap();
        let file = ignored.join_component("recreated-output");
        file.create_with_contents("watched").unwrap();
        expect_filesystem_event!(recv, file, EventKind::Create(_));
    }

    #[cfg(all(feature = "manual_recursive_watch", not(target_os = "macos")))]
    #[tokio::test]
    async fn newly_created_uninterested_ignored_dir_is_not_recursed() {
        let (repo_root, _tmp) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();
        repo_root
            .join_component(".gitignore")
            .create_with_contents("ignored/\n")
            .unwrap();
        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();

        let ignored = repo_root.join_components(&["ignored", "nested"]);
        ignored.create_dir_all().unwrap();
        let unseen = ignored.join_component("file");
        unseen.create_with_contents("ignored").unwrap();
        let sentinel = repo_root.join_component("sentinel");
        sentinel.create_with_contents("seen").unwrap();
        assert_unseen_before_sentinel(&mut recv, &unseen, &sentinel).await;
    }

    #[cfg(all(feature = "manual_recursive_watch", not(target_os = "macos")))]
    #[tokio::test]
    async fn gitignore_changes_reconcile_physical_watches() {
        let (repo_root, _tmp) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();
        let gitignore = repo_root.join_component(".gitignore");
        gitignore.create_with_contents("ignored/\n").unwrap();
        let ignored = repo_root.join_component("ignored");
        ignored.create_dir_all().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let mut recv = watcher.subscribe().await.unwrap();

        gitignore.create_with_contents("").unwrap();
        expect_filesystem_event!(recv, gitignore, EventKind::Modify(_));
        let newly_visible = ignored.join_component("visible");
        newly_visible.create_with_contents("visible").unwrap();
        expect_filesystem_event!(recv, newly_visible, EventKind::Create(_));

        gitignore.create_with_contents("ignored/\n").unwrap();
        expect_filesystem_event!(recv, gitignore, EventKind::Modify(_));
        let newly_hidden = ignored.join_component("hidden");
        newly_hidden.create_with_contents("hidden").unwrap();
        let sentinel = repo_root.join_component("sentinel-after-reignore");
        sentinel.create_with_contents("seen").unwrap();
        assert_unseen_before_sentinel(&mut recv, &newly_hidden, &sentinel).await;
    }

    #[tokio::test]
    async fn test_close() {
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        let mut recv = {
            // create and immediately drop the watcher, which should trigger the exit
            // channel
            let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
            watcher.subscribe().await.unwrap()
        };

        // There may be spurious events, but we should expect a close in short order
        tokio::time::timeout(Duration::from_millis(100), async move {
            loop {
                if let Err(e) = recv.recv().await {
                    assert_matches!(e, broadcast::error::RecvError::Closed);
                    return;
                }
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn scoped_subscription_closes_with_watcher() {
        let (repo_root, _tmp_repo_root) = temp_dir();
        let repo_root = repo_root.to_realpath().unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let source = watcher.source();
        let mut subscription = source.subscribe(super::WatchScope::all()).await.unwrap();
        drop(watcher);

        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if matches!(
                    subscription.recv().await,
                    Err(broadcast::error::RecvError::Closed)
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            source.subscribe(super::WatchScope::all()).await,
            Err(super::SubscribeError::Closed)
        ));
    }
}
