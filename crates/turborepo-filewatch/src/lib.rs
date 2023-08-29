#![feature(assert_matches)]

use std::{fmt::Debug, future::IntoFuture, result::Result, sync::Arc, time::Duration};

use itertools::Itertools;
#[cfg(any(feature = "watch_recursively", feature = "watch_ancestors"))]
use notify::event::EventKind;
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{DebounceEventResult, DebouncedEvent, Debouncer, FileIdMap};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc};
// windows -> no recursive watch, watch ancestors
// linux -> recursive watch, watch ancestors
#[cfg(feature = "watch_ancestors")]
use turbopath::PathRelation;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
// macos -> custom watcher impl in fsevents, no recursive watch, no watching ancestors
#[cfg(target_os = "macos")]
use {
    fsevent::FsEventWatcher,
    notify_debouncer_full::{new_debouncer_opt, DebounceEventHandler},
};
#[cfg(feature = "watch_recursively")]
use {
    notify::event::{CreateKind, Event, EventAttributes},
    std::{path::Path, time::Instant},
    walkdir::WalkDir,
};
#[cfg(not(target_os = "macos"))]
use {notify::RecommendedWatcher, notify_debouncer_full::new_debouncer};

#[cfg(target_os = "macos")]
mod fsevent;

#[cfg(not(target_os = "macos"))]
type Backend = RecommendedWatcher;
#[cfg(target_os = "macos")]
type Backend = FsEventWatcher;

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
#[derive(Clone, Debug)]
pub struct NotifyError(Arc<notify::Error>);

impl From<notify::Error> for NotifyError {
    fn from(value: notify::Error) -> Self {
        Self(Arc::new(value))
    }
}

pub struct FileSystemWatcher {
    sender: broadcast::Sender<Result<DebouncedEvent, Vec<NotifyError>>>,
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_ch: tokio::sync::oneshot::Sender<()>,
}

impl FileSystemWatcher {
    pub fn new(root: &AbsoluteSystemPath) -> Result<Self, WatchError> {
        let (sender, _) = broadcast::channel(1024);
        let (send_file_events, mut recv_file_events) = mpsc::channel(1024);
        let watch_root = root.to_owned();
        let broadcast_sender = sender.clone();
        let debouncer = run_watcher(&watch_root, send_file_events).unwrap();
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();
        // Ensure we are ready to receive new events, not events for existing state
        futures::executor::block_on(wait_for_cookie(&watch_root, &mut recv_file_events))?;
        tokio::task::spawn(watch_events(
            debouncer,
            watch_root,
            recv_file_events,
            exit_signal,
            broadcast_sender,
        ));
        Ok(Self {
            sender,
            _exit_ch: exit_ch,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Result<DebouncedEvent, Vec<NotifyError>>> {
        self.sender.subscribe()
    }
}

#[cfg(not(any(feature = "watch_ancestors", feature = "watch_recursively")))]
async fn watch_events(
    _debouncer: Debouncer<Backend, FileIdMap>,
    _watch_root: AbsoluteSystemPathBuf,
    mut recv_file_events: mpsc::Receiver<DebounceEventResult>,
    exit_signal: tokio::sync::oneshot::Receiver<()>,
    broadcast_sender: broadcast::Sender<Result<DebouncedEvent, Vec<NotifyError>>>,
) -> Result<(), WatchError> {
    let mut exit_signal = exit_signal;
    'outer: loop {
        tokio::select! {
            _ = &mut exit_signal => break 'outer,
            Some(event) = recv_file_events.recv().into_future() => {
                match event {
                    Ok(events) => {
                        for event in events {
                            // we don't care if we fail to send, it just means no one is currently watching
                            let _ = broadcast_sender.send(Ok(event));
                        }
                    },
                    Err(errors) => {
                        // we don't care if we fail to send, it just means no one is currently watching
                        let _ = broadcast_sender.send(Err(errors.into_iter().map(NotifyError::from).collect()));
                    }
                }
            }
        }
    }
    Ok::<(), WatchError>(())
}

#[cfg(any(feature = "watch_ancestors", feature = "watch_recursively"))]
async fn watch_events(
    #[cfg(feature = "watch_recursively")] mut debouncer: Debouncer<Backend, FileIdMap>,
    #[cfg(not(feature = "watch_recursively"))] _debouncer: Debouncer<Backend, FileIdMap>,
    watch_root: AbsoluteSystemPathBuf,
    mut recv_file_events: mpsc::Receiver<DebounceEventResult>,
    exit_signal: tokio::sync::oneshot::Receiver<()>,
    broadcast_sender: broadcast::Sender<Result<DebouncedEvent, Vec<NotifyError>>>,
) -> Result<(), WatchError> {
    let mut exit_signal = exit_signal;
    'outer: loop {
        tokio::select! {
            _ = &mut exit_signal => break 'outer,
            Some(event) = recv_file_events.recv().into_future() => {
                match event {
                    Ok(events) => {
                        for mut event in events {
                            #[cfg(feature = "watch_recursively")]
                            {
                                let time = event.time;
                                if event.event.kind == EventKind::Create(CreateKind::Folder) {
                                    for new_path in &event.event.paths {
                                        watch_recursively(&new_path, debouncer.watcher(), Some((time, &broadcast_sender)))?;
                                    }
                                }
                            }
                            #[cfg(feature = "watch_ancestors")]
                            filter_relevant(&watch_root, &mut event);
                            // we don't care if we fail to send, it just means no one is currently watching
                            let _ = broadcast_sender.send(Ok(event));
                        }
                    },
                    Err(errors) => {
                        // we don't care if we fail to send, it just means no one is currently watching
                        let _ = broadcast_sender.send(Err(errors.into_iter().map(NotifyError::from).collect()));
                    }
                }
            }
        }
    }
    Ok::<(), WatchError>(())
}

// Since we're manually watching the parent directories, we need
// to handle both getting irrelevant events and getting ancestor
// events that translate to events at the root.
#[cfg(feature = "watch_ancestors")]
fn filter_relevant(root: &AbsoluteSystemPath, event: &mut DebouncedEvent) {
    // If path contains root && event type is modify, synthesize modify at root
    let is_modify_existing = matches!(event.kind, EventKind::Remove(_) | EventKind::Modify(_));

    event.paths.retain_mut(|path| {
        match root.relation_to_path(&path) {
            PathRelation::Incomparable => panic!("Non-absolute path from filewatching"),
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

#[cfg(feature = "watch_recursively")]
fn watch_recursively(
    root: &Path,
    watcher: &mut Backend,
    sender: Option<(
        Instant,
        &broadcast::Sender<Result<DebouncedEvent, Vec<NotifyError>>>,
    )>,
) -> Result<(), WatchError> {
    for dir in WalkDir::new(root).follow_links(false).into_iter() {
        let dir = dir?;
        if dir.file_type().is_dir() {
            watcher.watch(dir.path(), RecursiveMode::NonRecursive)?;
        }
        if let Some((instant, sender)) = sender.as_ref() {
            let create_kind = if dir.file_type().is_dir() {
                CreateKind::Folder
            } else {
                CreateKind::File
            };
            let event = DebouncedEvent {
                event: Event {
                    paths: vec![dir.path().to_owned()],
                    kind: EventKind::Create(create_kind),
                    attrs: EventAttributes::default(),
                },
                time: *instant,
            };
            // It's ok if we fail to send, it means we're shutting down
            let _ = sender.send(Ok(event));
        }
    }
    Ok(())
}

fn run_watcher(
    root: &AbsoluteSystemPath,
    sender: mpsc::Sender<DebounceEventResult>,
) -> Result<Debouncer<Backend, FileIdMap>, WatchError> {
    #[cfg(target_os = "macos")]
    let mut debouncer = make_debouncer(move |res| {
        let _ = sender.blocking_send(res);
    })?;
    #[cfg(not(target_os = "macos"))]
    let mut debouncer = new_debouncer(Duration::from_millis(1), None, move |res| {
        let _ = sender.blocking_send(res);
    })?;

    // Note that the "watch_recursively" feature corresponds to manual
    // recursive watching.
    #[cfg(not(feature = "watch_recursively"))]
    debouncer
        .watcher()
        .watch(&root.as_std_path(), RecursiveMode::Recursive)?;
    #[cfg(feature = "watch_recursively")]
    {
        // Don't synthesize initial events
        watch_recursively(root.as_std_path(), debouncer.watcher(), None)?;
    }
    #[cfg(feature = "watch_ancestors")]
    watch_parents(root, debouncer.watcher())?;
    Ok(debouncer)
}

#[cfg(target_os = "macos")]
fn make_debouncer<F: DebounceEventHandler>(
    event_handler: F,
) -> Result<Debouncer<Backend, FileIdMap>, notify::Error> {
    new_debouncer_opt::<F, FsEventWatcher, FileIdMap>(
        Duration::from_millis(1),
        None,
        event_handler,
        FileIdMap::new(),
        notify::Config::default(),
    )
}

/// wait_for_cookie performs a roundtrip through the filewatching mechanism.
/// This ensures that we are ready to receive *new* filesystem events, rather
/// than receiving events from existing state, which some backends can do.
async fn wait_for_cookie(
    root: &AbsoluteSystemPath,
    recv: &mut mpsc::Receiver<DebounceEventResult>,
) -> Result<(), WatchError> {
    let cookie_path = root.join_component(".turbo-cookie");
    cookie_path.create_with_contents("cookie").map_err(|e| {
        WatchError::Setup(format!("failed to write cookie to {}: {}", cookie_path, e))
    })?;
    loop {
        let events = tokio::time::timeout(Duration::from_millis(2000), recv.recv())
            .await
            .map_err(|_| WatchError::Setup("waiting for cookie timed out".to_string()))?
            .ok_or_else(|| {
                WatchError::Setup(
                    "filewatching closed before cookie file  was observed".to_string(),
                )
            })?
            .map_err(|errs| {
                WatchError::Setup(format!(
                    "initial watch encountered errors: {}",
                    errs.into_iter().map(|e| e.to_string()).join(", ")
                ))
            })?;
        for event in events {
            for path in &event.paths {
                if path == (&cookie_path as &AbsoluteSystemPath) {
                    let _ = cookie_path.remove();
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, sync::atomic::AtomicUsize, time::Duration};

    #[cfg(not(target_os = "windows"))]
    use notify::event::RenameMode;
    use notify::{event::ModifyKind, EventKind};
    use notify_debouncer_full::DebouncedEvent;
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
                println!("event {:?}", event);
                for path in event.event.paths {
                    if path == (&$expected_path as &AbsoluteSystemPath)
                        && matches!(event.event.kind, $pattern)
                    {
                        break 'outer;
                    }
                }
            }
        };
    }

    static WATCH_COUNT: AtomicUsize = AtomicUsize::new(0);

    async fn expect_watching(
        recv: &mut broadcast::Receiver<Result<DebouncedEvent, Vec<NotifyError>>>,
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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();

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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();

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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();
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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();
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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();
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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();
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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();
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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();
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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();
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

        let watcher = FileSystemWatcher::new(&repo_root).unwrap();
        let mut recv = watcher.subscribe();
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
            let watcher = FileSystemWatcher::new(&repo_root).unwrap();
            watcher.subscribe()
        };

        assert_matches!(recv.recv().await, Err(broadcast::error::RecvError::Closed));
    }
}
