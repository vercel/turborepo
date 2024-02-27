#![deny(clippy::all)]
#![feature(assert_matches)]

use std::{
    fmt::{Debug, Display},
    future::IntoFuture,
    path::Path,
    result::Result,
    sync::Arc,
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
use tokio::sync::{broadcast, mpsc, watch::error::RecvError};
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathRelation};
#[cfg(feature = "manual_recursive_watch")]
use {
    notify::{
        event::{CreateKind, EventAttributes},
        ErrorKind,
    },
    std::io,
    tracing::trace,
    walkdir::WalkDir,
};

pub mod cookies;
#[cfg(target_os = "macos")]
mod fsevent;
pub mod globwatcher;
mod optional_watch;
pub mod package_changes_watcher;
pub mod package_watcher;

pub use optional_watch::OptionalWatch;

#[cfg(not(target_os = "macos"))]
type Backend = RecommendedWatcher;
#[cfg(target_os = "macos")]
type Backend = FsEventWatcher;

type EventResult = Result<Event, notify::Error>;

#[derive(Debug, Error)]
pub enum WatchError {
    #[error("filewatching backend error: {0}")]
    Notify(#[from] notify::Error),
    #[error("filewatching stopped")]
    Stopped(#[from] std::sync::mpsc::RecvError),
    #[error("enumerating recursive watch: {0}")]
    WalkDir(#[from] walkdir::Error),
    #[error("filewatching failed to start: {0}")]
    Setup(String),
}

// We want to broadcast the errors we get, but notify::Error does not implement
// Clone. We provide a wrapper that uses an Arc to implement Clone so that we
// can send errors on a broadcast channel.
#[derive(Clone, Debug, Error)]
pub struct NotifyError(Arc<notify::Error>);

impl From<notify::Error> for NotifyError {
    fn from(value: notify::Error) -> Self {
        Self(Arc::new(value))
    }
}

impl Display for NotifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct FileSystemWatcher {
    receiver: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_ch: tokio::sync::oneshot::Sender<()>,
    cookie_dir: AbsoluteSystemPathBuf,
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
                "Invalid cookie directory: {} does not contain {}",
                root, cookie_dir
            )));
        }

        let (file_events_receiver_tx, file_events_receiver_lazy) = OptionalWatch::new();
        let (send_file_events, mut recv_file_events) = mpsc::channel(1024);
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();

        tokio::task::spawn({
            let cookie_dir = cookie_dir.clone();
            let watch_root = root.to_owned();
            async move {
                // this task never yields, so run it in the blocking threadpool
                let watch_root_task = watch_root.clone();
                let cookie_dir_task = cookie_dir.clone();
                let task = tokio::task::spawn_blocking(move || {
                    setup_cookie_dir(&cookie_dir_task)?;
                    run_watcher(&watch_root_task, send_file_events)
                });

                let Ok(Ok(watcher)) = task.await else {
                    // if the watcher fails, just return. we don't set the event sender, and other
                    // services will never start
                    return;
                };

                // Ensure we are ready to receive new events, not events for existing state
                debug!("waiting for initial filesystem cookie");
                if let Err(e) = wait_for_cookie(&cookie_dir, &mut recv_file_events).await {
                    // if we can't get a cookie here, we should not make the file
                    // watching available to downstream services
                    warn!("failed to wait for initial filesystem cookie: {}", e);
                    return;
                }
                debug!("filewatching ready");

                let (sender, receiver) = broadcast::channel(1024);

                if file_events_receiver_tx.send(Some(receiver)).is_err() {
                    // if this fails, it means that nobody is listening (and
                    // nobody ever will) likely because the
                    // watcher has been dropped. We can just exit early.
                    tracing::debug!("no downstream listeners, exiting");
                    return;
                }

                watch_events(watcher, watch_root, recv_file_events, exit_signal, sender).await;
            }
        });

        Ok(Self {
            receiver: file_events_receiver_lazy,
            _exit_ch: exit_ch,
            cookie_dir,
        })
    }

    /// A convenience method around the sender watcher that waits for file
    /// watching to be ready and then returns a handle to the file stream.
    pub async fn subscribe(
        &self,
    ) -> Result<broadcast::Receiver<Result<Event, NotifyError>>, RecvError> {
        let mut receiver = self.receiver.clone();
        receiver.get().await.map(|r| r.resubscribe())
    }

    pub fn watch(&self) -> OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>> {
        self.receiver.clone()
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
            WatchError::Setup(format!("failed to clear cookie dir {}: {}", cookie_dir, e))
        })?;
    }
    cookie_dir.create_dir_all().map_err(|e| {
        WatchError::Setup(format!("failed to setup cookie dir {}: {}", cookie_dir, e))
    })?;
    Ok(())
}

#[cfg(not(any(feature = "watch_ancestors", feature = "manual_recursive_watch")))]
async fn watch_events(
    _watcher: Backend,
    _watch_root: AbsoluteSystemPathBuf,
    mut recv_file_events: mpsc::Receiver<EventResult>,
    exit_signal: tokio::sync::oneshot::Receiver<()>,
    broadcast_sender: broadcast::Sender<Result<Event, NotifyError>>,
) {
    let mut exit_signal = exit_signal;
    'outer: loop {
        tokio::select! {
            _ = &mut exit_signal => break 'outer,
            Some(event) = recv_file_events.recv().into_future() => {
                // we don't care if we fail to send, it just means no one is currently watching
                let _ = broadcast_sender.send(event.map_err(NotifyError::from));
            }
        }
    }
}

#[cfg(any(feature = "watch_ancestors", feature = "manual_recursive_watch"))]
async fn watch_events(
    #[cfg(feature = "manual_recursive_watch")] mut watcher: Backend,
    #[cfg(not(feature = "manual_recursive_watch"))] _watcher: Backend,
    watch_root: AbsoluteSystemPathBuf,
    mut recv_file_events: mpsc::Receiver<EventResult>,
    exit_signal: tokio::sync::oneshot::Receiver<()>,
    broadcast_sender: broadcast::Sender<Result<Event, NotifyError>>,
) {
    let mut exit_signal = exit_signal;
    'outer: loop {
        tokio::select! {
            _ = &mut exit_signal => break 'outer,
            Some(event) = recv_file_events.recv().into_future() => {
                match event {
                    Ok(mut event) => {
                        // Note that we need to filter relevant events
                        // before doing manual recursive watching so that
                        // we don't try to add watches to siblings of the
                        // directories on our path to the root.
                        #[cfg(feature = "watch_ancestors")]
                        filter_relevant(&watch_root, &mut event);

                        #[cfg(feature = "manual_recursive_watch")]
                        {
                            if event.kind == EventKind::Create(CreateKind::Folder) {
                                for new_path in &event.paths {
                                    if let Err(err) = manually_add_recursive_watches(new_path, &mut watcher, Some(&broadcast_sender)) {
                                        warn!("encountered error watching filesystem {}", err);
                                        break 'outer;
                                    }
                                }
                            }
                        }
                        // we don't care if we fail to send, it just means no one is currently watching
                        let _ = broadcast_sender.send(Ok(event));
                    },
                    Err(error) => {
                        // we don't care if we fail to send, it just means no one is currently watching
                        let _ = broadcast_sender.send(Err(NotifyError::from(error)));
                    }
                }
            }
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
        let abs_path: &AbsoluteSystemPath = path
            .as_path()
            .try_into()
            .expect("Non-absolute path from filewatching");
        match root.relation_to_path(abs_path) {
            // An irrelevant path, probably from a non-recursive watch of a parent directory
            PathRelation::Divergent => false,
            // A path contained in the root
            PathRelation::Parent => true,
            PathRelation::Child => {
                // If we're modifying something along the path to the
                // root, move the event to the root
                if is_modify_existing {
                    *path = root.as_std_path().to_owned();
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
fn watch_recursively(root: &AbsoluteSystemPath, watcher: &mut Backend) -> Result<(), WatchError> {
    watcher.watch(root.as_std_path(), RecursiveMode::Recursive)?;
    Ok(())
}

#[cfg(feature = "manual_recursive_watch")]
fn is_not_found(err: &notify::Error) -> bool {
    if let ErrorKind::Io(ref io_err) = err.kind {
        io_err.kind() == io::ErrorKind::NotFound
    } else {
        false
    }
}

#[cfg(feature = "manual_recursive_watch")]
fn watch_recursively(root: &AbsoluteSystemPath, watcher: &mut Backend) -> Result<(), WatchError> {
    // Don't synthesize initial events
    manually_add_recursive_watches(root.as_std_path(), watcher, None)
}

#[cfg(feature = "manual_recursive_watch")]
fn manually_add_recursive_watches(
    root: &Path,
    watcher: &mut Backend,
    sender: Option<&broadcast::Sender<Result<Event, NotifyError>>>,
) -> Result<(), WatchError> {
    // Note that WalkDir yields the root as well as doing the walk.
    for dir in WalkDir::new(root).follow_links(false).into_iter() {
        let dir = dir?;
        if dir.file_type().is_dir() {
            trace!("manually watching {}", dir.path().display());
            match watcher.watch(dir.path(), RecursiveMode::NonRecursive) {
                Ok(()) => {}
                // If we try to watch a non-existent path, we can just skip
                // it.
                Err(e) if is_not_found(&e) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        if let Some(sender) = sender.as_ref() {
            let create_kind = if dir.file_type().is_dir() {
                CreateKind::Folder
            } else {
                CreateKind::File
            };
            let event = Event {
                paths: vec![dir.path().to_owned()],
                kind: EventKind::Create(create_kind),
                attrs: EventAttributes::default(),
            };
            // It's ok if we fail to send, it means we're shutting down
            let _ = sender.send(Ok(event));
        }
    }
    Ok(())
}

fn run_watcher(
    root: &AbsoluteSystemPath,
    sender: mpsc::Sender<EventResult>,
) -> Result<Backend, WatchError> {
    let mut watcher = make_watcher(move |res| {
        let _ = sender.blocking_send(res);
    })?;

    watch_recursively(root, &mut watcher)?;

    #[cfg(feature = "watch_ancestors")]
    watch_parents(root, &mut watcher)?;
    Ok(watcher)
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
    recv: &mut mpsc::Receiver<EventResult>,
) -> Result<(), WatchError> {
    // TODO: should this be passed in? Currently the caller guarantees that the
    // directory is empty, but it could be the responsibility of the
    // filewatcher...
    let cookie_path = cookie_dir.join_component(".turbo-cookie");
    cookie_path.create_with_contents("cookie").map_err(|e| {
        WatchError::Setup(format!("failed to write cookie to {}: {}", cookie_path, e))
    })?;
    loop {
        let event = tokio::time::timeout(Duration::from_millis(2000), recv.recv())
            .await
            .map_err(|e| WatchError::Setup(format!("waiting for cookie timed out: {}", e)))?
            .ok_or_else(|| {
                WatchError::Setup(
                    "filewatching closed before cookie file  was observed".to_string(),
                )
            })?
            .map_err(|err| {
                WatchError::Setup(format!("initial watch encountered errors: {}", err))
            })?;
        if event.paths.iter().any(|path| {
            let path: &Path = path;
            path == (&cookie_path as &AbsoluteSystemPath)
        }) {
            // We don't need to stop everything if we failed to remove the cookie file
            // for some reason. We can warn about it though.
            if let Err(e) = cookie_path.remove() {
                warn!("failed to remove cookie file {}", e);
            }
            return Ok(());
        }
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, sync::atomic::AtomicUsize, time::Duration};

    #[cfg(not(target_os = "windows"))]
    use notify::event::RenameMode;
    use notify::{event::ModifyKind, Event, EventKind};
    use tokio::sync::broadcast;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use crate::{FileSystemWatcher, NotifyError};

    fn temp_dir() -> (AbsoluteSystemPathBuf, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        (path, tmp)
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

    async fn expect_watching(
        recv: &mut broadcast::Receiver<Result<Event, NotifyError>>,
        dirs: &[&AbsoluteSystemPath],
    ) {
        for dir in dirs {
            let count = WATCH_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let filename = dir.join_component(format!("test-{}", count).as_str());
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

        // TODO: implement default filtering (.git, node_modules)
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
}
