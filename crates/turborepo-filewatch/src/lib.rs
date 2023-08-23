use std::{
    fmt::Debug,
    future::IntoFuture,
    path::Path,
    result::Result,
    time::{Duration, Instant},
};

use fsevent::FsEventWatcher;
use itertools::Itertools;
use notify::{
    event::{CreateKind, EventAttributes},
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use notify_debouncer_full::{
    new_debouncer, new_debouncer_opt, DebounceEventHandler, DebounceEventResult, DebouncedEvent,
    Debouncer, FileIdMap,
};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc};
use turbopath::AbsoluteSystemPath;
use walkdir::WalkDir;

#[cfg(target_os = "macos")]
mod fsevent;

#[cfg(not(target_os = "macos"))]
type Backend = RecommendedWatcher;
#[cfg(target_os = "macos")]
type Backend = FsEventWatcher;

#[derive(Debug, Error)]
enum WatchError {
    #[error("filewatching backend error: {0}")]
    Notify(#[from] notify::Error),
    #[error("filewatching stopped")]
    Stopped(#[from] std::sync::mpsc::RecvError),
    #[error("enumerating recursive watch: {0}")]
    WalkDir(#[from] walkdir::Error),
    #[error("filewatching failed to start: {0}")]
    Setup(String),
}

struct FileSystemWatcher {
    sender: broadcast::Sender<DebouncedEvent>,
    exit_ch: tokio::sync::oneshot::Sender<()>, //watcher: Arc<Mutex<RecommendedWatcher>>,
}

impl FileSystemWatcher {
    pub fn new(root: &AbsoluteSystemPath) -> Result<Self, WatchError> {
        let (sender, _) = broadcast::channel(1024);
        let (send_file_events, mut recv_file_events) = mpsc::channel(1024);
        let watch_root = root.to_owned();
        let broadcast_sender = sender.clone();
        let debouncer = run_watcher(&watch_root, send_file_events).unwrap();
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();
        #[cfg(target_os = "macos")]
        futures::executor::block_on(async {
            wait_for_cookie(&watch_root, &mut recv_file_events).await
        })?;
        tokio::task::spawn(async move {
            // Ensure we move the debouncer to our task
            #[cfg(target_os = "linux")]
            let mut debouncer = debouncer;
            #[cfg(not(target_os = "linux"))]
            let _debouncer = debouncer;

            let mut exit_signal = exit_signal;
            'outer: loop {
                tokio::select! {
                    _ = &mut exit_signal => { println!("exit ch dropped"); break 'outer; },
                    Some(event) = recv_file_events.recv().into_future() => {
                        match event {
                            Ok(events) => {
                                for event in events {
                                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                                    {
                                        let time = event.time;
                                        if event.event.kind == EventKind::Create(CreateKind::Folder) {
                                            for new_path in &event.event.paths {
                                                watch_recursively(&new_path, debouncer.watcher(), Some((time, &broadcast_sender)))?;
                                            }
                                        }
                                    }
                                    // we don't care if we fail to send, it just means no one is currently watching
                                    let _ = broadcast_sender.send(event);
                                }
                            },
                            Err(errors) => {
                                println!("errors {:?}", errors);
                                panic!("uh oh")
                            }
                        }
                    }
                }
            }
            println!("DONE");
            Ok::<(), WatchError>(())
        });
        Ok(Self { sender, exit_ch })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DebouncedEvent> {
        self.sender.subscribe()
    }
}

fn watch_recursively(
    root: &Path,
    watcher: &mut RecommendedWatcher,
    sender: Option<(Instant, &broadcast::Sender<DebouncedEvent>)>,
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
            let _ = sender.send(event);
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

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    debouncer
        .watcher()
        .watch(&root.as_std_path(), RecursiveMode::Recursive)?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    // Don't synthesize initial events
    watch_recursively(root.as_std_path(), debouncer.watcher(), None)?;
    Ok(debouncer)
}

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

#[cfg(target_os = "macos")]
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
                if path == (&cookie_path as &AbsoluteSystemPath)
                /* && event.kind == EventKind::Create(CreateKind::File) */
                {
                    let _ = cookie_path.remove();
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{sync::atomic::AtomicUsize, time::Duration};

    use notify::{
        event::{CreateKind, ModifyKind, RemoveKind, RenameMode},
        EventKind,
    };
    use notify_debouncer_full::DebouncedEvent;
    use tokio::sync::broadcast;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use crate::FileSystemWatcher;

    #[tokio::test]
    async fn test_hello() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let watcher = FileSystemWatcher::new(&root).unwrap();
        let mut events_channel = watcher.subscribe();

        root.join_component("foo")
            .create_with_contents("hello world")
            .unwrap();

        let event = tokio::time::timeout(Duration::from_millis(2000), events_channel.recv())
            .await
            .unwrap();
    }

    fn temp_dir() -> (AbsoluteSystemPathBuf, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        (path, tmp)
    }

    async fn expect_filesystem_event(
        recv: &mut broadcast::Receiver<DebouncedEvent>,
        expected_path: &AbsoluteSystemPath,
        expected_event: EventKind,
    ) {
        'outer: loop {
            let event = tokio::time::timeout(Duration::from_millis(3000), recv.recv())
                .await
                .expect("timed out waiting for filesystem event")
                .expect("sender was dropped");
            println!("event {:?}", event);
            for path in event.event.paths {
                if path == expected_path && event.event.kind == expected_event {
                    break 'outer;
                }
            }
        }
    }

    static WATCH_COUNT: AtomicUsize = AtomicUsize::new(0);

    async fn expect_watching(
        recv: &mut broadcast::Receiver<DebouncedEvent>,
        dirs: &[&AbsoluteSystemPath],
    ) {
        for dir in dirs {
            let count = WATCH_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let filename = dir.join_component(format!("test-{}", count).as_str());
            filename.create_with_contents("hello").unwrap();

            expect_filesystem_event(recv, &filename, EventKind::Create(CreateKind::File)).await;
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
        expect_filesystem_event(&mut recv, &foo_path, EventKind::Create(CreateKind::File)).await;

        let deep_path = sibling_path.join_components(&["deep", "path"]);
        deep_path.create_dir_all().unwrap();
        expect_filesystem_event(
            &mut recv,
            &sibling_path.join_component("deep"),
            EventKind::Create(CreateKind::Folder),
        )
        .await;
        expect_filesystem_event(&mut recv, &deep_path, EventKind::Create(CreateKind::Folder)).await;
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
        expect_filesystem_event(
            &mut recv,
            &parent_path,
            EventKind::Remove(RemoveKind::Folder),
        )
        .await;

        // Ensure we get events when creating file in deleted directory
        child_path.create_dir_all().unwrap();
        expect_filesystem_event(
            &mut recv,
            &parent_path,
            EventKind::Create(CreateKind::Folder),
        )
        .await;
        expect_filesystem_event(
            &mut recv,
            &child_path,
            EventKind::Create(CreateKind::Folder),
        )
        .await;

        let foo_path = child_path.join_component("foo");
        foo_path.create_with_contents("hello").unwrap();
        expect_filesystem_event(&mut recv, &foo_path, EventKind::Create(CreateKind::File)).await;
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
        expect_filesystem_event(&mut recv, &repo_root, EventKind::Remove(RemoveKind::Folder)).await;
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

        expect_filesystem_event(
            &mut recv,
            &new_parent,
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        )
        .await;
    }
}
