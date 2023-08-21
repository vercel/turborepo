use std::{
    result::Result,
    sync::{Arc, Mutex},
    path::{Path, PathBuf},
    fmt::Debug,
    thread,
    time::{Duration, Instant}, future::IntoFuture
};
use notify::event::{CreateKind, EventAttributes};
//use notify::{watcher, DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{notify::*, new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, FileIdMap};
use thiserror::Error;
use tokio::{sync::{broadcast, mpsc}, task::JoinHandle};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use walkdir::WalkDir;

#[derive(Default)]
struct DiskWatcher {
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// Keeps track of which directories are currently watched. This is only
    /// used on a OS that doesn't support recursive watching.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    watching: dashmap::DashSet<PathBuf>,
}

#[derive(Debug, Error)]
enum WatchError {
    #[error("filewatching backend error: {0}")]
    Notify(#[from] notify::Error),
    #[error("filewatching stopped")]
    Stopped(#[from] std::sync::mpsc::RecvError),
    #[error("enumerating recursive watch: {0}")]
    WalkDir(#[from] walkdir::Error)
}

// impl DiskWatcher {
//     #[cfg(not(any(target_os = "macos", target_os = "windows")))]
//     fn restore_if_watching(&self, dir_path: &Path, root_path: &Path) -> Result<()> {
//         if self.watching.contains(dir_path) {
//             let mut watcher = self.watcher.lock().unwrap();
//             self.start_watching(&mut watcher, dir_path, root_path)?;
//         }
//         Ok(())
//     }

//     #[cfg(not(any(target_os = "macos", target_os = "windows")))]
//     fn ensure_watching(&self, dir_path: &Path, root_path: &Path) -> Result<()> {
//         if self.watching.contains(dir_path) {
//             return Ok(());
//         }
//         let mut watcher = self.watcher.lock().unwrap();
//         if self.watching.insert(dir_path.to_path_buf()) {
//             self.start_watching(&mut watcher, dir_path, root_path)?;
//         }
//         Ok(())
//     }

//     #[cfg(not(any(target_os = "macos", target_os = "windows")))]
//     fn start_watching(
//         &self,
//         watcher: &mut std::sync::MutexGuard<Option<RecommendedWatcher>>,
//         dir_path: &Path,
//         root_path: &Path,
//     ) -> Result<()> {
//         if let Some(watcher) = watcher.as_mut() {
//             let mut path = dir_path;
//             while let Err(err) = watcher.watch(path, RecursiveMode::NonRecursive) {
//                 if path == root_path {
//                     return Err(err).context(format!(
//                         "Unable to watch {} (tried up to {})",
//                         dir_path.display(),
//                         path.display()
//                     ));
//                 }
//                 let Some(parent_path) = path.parent() else {
//                     return Err(err).context(format!(
//                         "Unable to watch {} (tried up to {})",
//                         dir_path.display(),
//                         path.display()
//                     ));
//                 };
//                 path = parent_path;
//             }
//         }
//         Ok(())
//     }
// }

struct FileSystemWatcher {
    sender: broadcast::Sender<DebouncedEvent>,
    exit_ch: tokio::sync::oneshot::Sender<()>
    //watcher: Arc<Mutex<RecommendedWatcher>>,
}

impl FileSystemWatcher {
    pub fn new(root: &AbsoluteSystemPath) -> Self {

        let (sender, _) = broadcast::channel(1024);
        let (send_file_events, mut recv_file_events) = mpsc::channel(1024);
        let watch_root = root.to_owned();
        let broadcast_sender = sender.clone();
        let debouncer = run_watcher(&watch_root, send_file_events).unwrap();
        println!("watching {}", &watch_root);
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();
        tokio::task::spawn(async move {
            let mut debouncer = debouncer;
            let mut exit_signal = exit_signal;
            loop {
                tokio::select! {
                    _ = &mut exit_signal => { println!("exit ch dropped"); return Ok::<(), WatchError>(()); },
                    Some(event) = recv_file_events.recv().into_future() => {
                        //let event = recv_file_events.recv()?;
                        match event {
                            Ok(events) => {
                                for event in events {
                                    let time = event.time;
                                    println!("event {:?}", event);
                                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                                    if event.event.kind == EventKind::Create(CreateKind::Folder) {
                                        for new_path in &event.event.paths {
                                            println!("new {}", new_path.display());
                                            //debouncer.watcher().watch(&new_path, RecursiveMode::Recursive)?;
                                            watch_recursively(&new_path, debouncer.watcher(), Some((time, &broadcast_sender)))?;
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
        });
        Self {
            sender,
            exit_ch
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DebouncedEvent> {
        self.sender.subscribe()
    }
}

fn watch_recursively(root: &Path, watcher: &mut RecommendedWatcher, sender: Option<(Instant, &broadcast::Sender<DebouncedEvent>)>) -> Result<(), WatchError> {
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
                    attrs: EventAttributes::default()
                },
                time: *instant
            };
            // It's ok if we fail to send, it means we're shutting down
            let _ = sender.send(event);
        }
        println!("ADD {}", dir.path().display());
    }
    Ok(())
}

fn run_watcher(root: &AbsoluteSystemPath, sender: mpsc::Sender<DebounceEventResult>) -> Result<Debouncer<RecommendedWatcher, FileIdMap>, WatchError> {
    //let (tx, recv) = mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(1), None, move |res| {
        futures::executor::block_on(async {
            // It's ok if we fail to send, it means we're shutting down
            let _ = sender.send(res).await;
        })
    })?;

    //let mut watcher = watcher(sender, Duration::from_millis(1))?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    debouncer.watcher().watch(&root_path, RecursiveMode::Recursive)?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    //debouncer.watcher().watch(root.as_std_path(), RecursiveMode::Recursive)?;
    // Don't synthesize initial events
    watch_recursively(root.as_std_path(), debouncer.watcher(), None)?;
    Ok(debouncer)
}

#[cfg(test)]
mod test {
    use std::{time::Duration, sync::atomic::AtomicUsize};

    use notify::{EventKind, event::CreateKind};
    use notify_debouncer_full::DebouncedEvent;
    use tokio::sync::broadcast;
    use turbopath::{AbsoluteSystemPathBuf, AbsoluteSystemPath};

    use crate::FileSystemWatcher;

    #[tokio::test]
    async fn test_hello() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();

        let watcher = FileSystemWatcher::new(&root);
        let mut events_channel = watcher.subscribe();

        println!("writing");
        root.join_component("foo").create_with_contents("hello world").unwrap();

        let event = tokio::time::timeout(Duration::from_millis(2000), events_channel.recv()).await.unwrap();
        println!("test event {:?}", event);
    }

    fn temp_dir() -> (AbsoluteSystemPathBuf, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        (path, tmp)
    }

    async fn expect_filesystem_event(recv: &mut broadcast::Receiver<DebouncedEvent>, expected_path: &AbsoluteSystemPath, expected_event: EventKind) {
        println!("WAIT FOR {}", expected_path);
        'outer: loop {
            let event = tokio::time::timeout(Duration::from_millis(1000), recv.recv())
                .await
                .expect("timed out waiting for filesystem event")
                .expect("sender was dropped");
            for path in event.event.paths {
                if path == expected_path && event.event.kind == expected_event {
                    break 'outer;
                } else {
                    println!("{}, {:?}", path.display(), event.event.kind);
                }
            }
        }
    }

    static WATCH_COUNT: AtomicUsize = AtomicUsize::new(0);

    async fn expect_watching(recv: &mut broadcast::Receiver<DebouncedEvent>, dirs: &[&AbsoluteSystemPath]) {
        for dir in dirs {
            let count = WATCH_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let filename = dir.join_component(format!("test-{}", count).as_str());
            println!("WRITING {}", filename);
            filename.create_with_contents("hello").unwrap();

            expect_filesystem_event(recv, &filename, EventKind::Create(CreateKind::File))
                .await;
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

        let watcher = FileSystemWatcher::new(&repo_root);
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
        expect_filesystem_event(
            &mut recv,
            &deep_path,
            EventKind::Create(CreateKind::Folder),
        )
        .await;
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
}
