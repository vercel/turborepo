//! File watching for devtools.
//!
//! Watches the repository for changes to relevant files (package.json,
//! turbo.json, etc.) and emits events when changes are detected.

use std::{path::Path, time::Duration};

use notify::Event;
use thiserror::Error;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, trace, warn};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_filewatch::{FileSystemWatcher, NotifyError, OptionalWatch};

/// Errors that can occur during file watching
#[derive(Debug, Error)]
pub enum WatchError {
    #[error("Failed to initialize file watcher: {0}")]
    FileWatcher(#[from] turborepo_filewatch::WatchError),
    #[error("File watching stopped unexpectedly")]
    WatchingStopped,
}

/// Events emitted by the devtools watcher
#[derive(Clone, Debug)]
pub enum WatchEvent {
    /// Files changed that require a graph rebuild
    FilesChanged,
}

/// File names that trigger a rebuild when changed
const RELEVANT_FILES: &[&str] = &[
    "package.json",
    "turbo.json",
    "turbo.jsonc",
    "pnpm-workspace.yaml",
    "pnpm-workspace.yml",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
];

/// Directories to ignore entirely
const IGNORED_DIRS: &[&str] = &[".git", "node_modules", ".turbo", ".next", "dist", "build"];

/// Watches for file changes in the repository and emits events
pub struct DevtoolsWatcher {
    _exit_tx: oneshot::Sender<()>,
    event_rx: broadcast::Receiver<WatchEvent>,
    // Keep the file watcher alive for the lifetime of the DevtoolsWatcher
    _file_watcher: FileSystemWatcher,
}

impl DevtoolsWatcher {
    /// Creates a new devtools watcher for the given repository root.
    pub fn new(repo_root: AbsoluteSystemPathBuf) -> Result<Self, WatchError> {
        // Create file system watcher
        let file_watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root)?;

        // Set up channels
        let (exit_tx, exit_rx) = oneshot::channel();
        let (event_tx, event_rx) = broadcast::channel(16);

        // Spawn watcher task
        tokio::spawn(watch_loop(
            repo_root,
            file_watcher.watch(),
            event_tx,
            exit_rx,
        ));

        Ok(Self {
            _exit_tx: exit_tx,
            event_rx,
            _file_watcher: file_watcher,
        })
    }

    /// Subscribe to watch events
    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.event_rx.resubscribe()
    }
}

/// Check if a path is in an ignored directory
fn is_in_ignored_dir(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| IGNORED_DIRS.contains(&s))
            .unwrap_or(false)
    })
}

/// Check if a file is relevant for triggering a rebuild
fn is_relevant_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|name| RELEVANT_FILES.contains(&name))
        .unwrap_or(false)
}

/// Main watch loop that processes file events
async fn watch_loop(
    _repo_root: AbsoluteSystemPathBuf,
    mut file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    event_tx: broadcast::Sender<WatchEvent>,
    exit_rx: oneshot::Receiver<()>,
) {
    // Get the receiver and immediately resubscribe to drop the SomeRef
    // (which is not Send) before entering the select loop
    let Ok(mut file_events) = file_events_lazy.get().await.map(|r| r.resubscribe()) else {
        warn!("File watching not available");
        return;
    };
    let mut pending_rebuild = false;
    let mut debounce_interval = tokio::time::interval(Duration::from_millis(100));
    debounce_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tokio::pin!(exit_rx);

    loop {
        tokio::select! {
            biased;

            // Exit signal received
            _ = &mut exit_rx => {
                debug!("Devtools watcher shutting down");
                break;
            }

            // Debounce tick - send event if pending
            _ = debounce_interval.tick() => {
                if pending_rebuild {
                    pending_rebuild = false;
                    debug!("Sending FilesChanged event");
                    let _ = event_tx.send(WatchEvent::FilesChanged);
                }
            }

            // File event received
            result = file_events.recv() => {
                match result {
                    Ok(Ok(event)) => {
                        // Check if any of the changed files are relevant
                        let has_relevant_change = event.paths.iter().any(|path| {
                            // Skip ignored directories
                            if is_in_ignored_dir(path) {
                                return false;
                            }

                            // Check if it's a relevant file
                            if is_relevant_file(path) {
                                trace!("Relevant file changed: {:?}", path);
                                return true;
                            }

                            false
                        });

                        if has_relevant_change {
                            pending_rebuild = true;
                        }
                    }
                    Ok(Err(e)) => {
                        warn!("File watch error: {:?}", e);
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("File watcher lagged by {} events, triggering rebuild", n);
                        pending_rebuild = true;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("File event channel closed");
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_relevant_file() {
        assert!(is_relevant_file(Path::new("package.json")));
        assert!(is_relevant_file(Path::new("/some/path/package.json")));
        assert!(is_relevant_file(Path::new("turbo.json")));
        assert!(is_relevant_file(Path::new("turbo.jsonc")));
        assert!(is_relevant_file(Path::new("pnpm-workspace.yaml")));
        assert!(!is_relevant_file(Path::new("index.ts")));
        assert!(!is_relevant_file(Path::new("README.md")));
    }

    #[test]
    fn test_is_in_ignored_dir() {
        assert!(is_in_ignored_dir(Path::new(".git/config")));
        assert!(is_in_ignored_dir(Path::new(
            "node_modules/foo/package.json"
        )));
        assert!(is_in_ignored_dir(Path::new("/repo/.turbo/cache")));
        assert!(!is_in_ignored_dir(Path::new(
            "/repo/packages/app/package.json"
        )));
        assert!(!is_in_ignored_dir(Path::new("turbo.json")));
    }
}
