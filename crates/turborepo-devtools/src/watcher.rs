//! File watching for devtools.
//!
//! Watches the repository for changes to relevant files (package.json,
//! turbo.json, etc.) and emits events when changes are detected.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use thiserror::Error;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, trace, warn};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_filewatch::{FileSystemWatcher, WatchInterest, WatchScope, WatchSource};

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
    "Cargo.toml",
    "Cargo.lock",
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
        Self::new_with_paths(repo_root, Vec::new())
    }

    /// Creates a watcher with exact paths that bypass filename and ignored
    /// directory filtering.
    pub fn new_with_paths(
        repo_root: AbsoluteSystemPathBuf,
        exact_paths: Vec<AbsoluteSystemPathBuf>,
    ) -> Result<Self, WatchError> {
        // Create file system watcher
        let file_watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root)?;
        let exact_paths = exact_watch_paths(&repo_root, exact_paths);

        // Set up channels
        let (exit_tx, exit_rx) = oneshot::channel();
        let (event_tx, event_rx) = broadcast::channel(16);

        // Spawn watcher task
        tokio::spawn(watch_loop(
            repo_root,
            file_watcher.source(),
            exact_paths,
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

fn exact_watch_paths(
    repo_root: &AbsoluteSystemPathBuf,
    paths: Vec<AbsoluteSystemPathBuf>,
) -> HashSet<PathBuf> {
    let mut paths: HashSet<PathBuf> = paths
        .into_iter()
        .map(|path| path.as_std_path().to_owned())
        .collect();
    paths.insert(
        repo_root
            .join_components(&[".turbo", "config.json"])
            .as_std_path()
            .to_owned(),
    );
    paths
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

fn should_watch_path(path: &Path, exact_paths: &HashSet<PathBuf>) -> bool {
    exact_paths.contains(path) || (!is_in_ignored_dir(path) && is_relevant_file(path))
}

/// Main watch loop that processes file events
async fn watch_loop(
    _repo_root: AbsoluteSystemPathBuf,
    file_events: WatchSource,
    exact_paths: HashSet<PathBuf>,
    event_tx: broadcast::Sender<WatchEvent>,
    exit_rx: oneshot::Receiver<()>,
) {
    let physical_interest = WatchInterest::new();
    physical_interest.replace(exact_paths.iter().cloned());
    let exact_paths = Arc::new(exact_paths);
    let scope = WatchScope::predicate(move |path| should_watch_path(path, &exact_paths))
        .with_physical_interest(physical_interest);
    let Ok(mut file_events) = file_events.subscribe(scope).await else {
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
                        let has_relevant_change = !event.paths.is_empty();

                        if has_relevant_change {
                            trace!(paths = ?event.paths, "Relevant files changed");
                            pending_rebuild = true;
                        }
                    }
                    Ok(Err(e)) => {
                        warn!("File watch error: {:?}", e);
                        pending_rebuild = true;
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
        assert!(is_relevant_file(Path::new("crates/app/Cargo.toml")));
        assert!(is_relevant_file(Path::new("Cargo.lock")));
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

    #[test]
    fn exact_config_paths_bypass_normal_ignore_and_filename_rules() {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let repo_root = AbsoluteSystemPathBuf::new(tempdir.path().to_string_lossy().to_string())
            .expect("absolute root");
        let custom = repo_root.join_components(&[".turbo", "custom.config"]);
        let arbitrary = repo_root.join_components(&["config", "devtools.conf"]);
        let local_config = repo_root.join_components(&[".turbo", "config.json"]);
        let unrelated = repo_root.join_components(&[".turbo", "unrelated.json"]);
        let turbo_json = repo_root.join_components(&[".turbo", "turbo.json"]);
        let exact_paths = exact_watch_paths(&repo_root, vec![custom.clone(), arbitrary.clone()]);

        assert!(should_watch_path(custom.as_std_path(), &exact_paths));
        assert!(should_watch_path(arbitrary.as_std_path(), &exact_paths));
        assert!(should_watch_path(local_config.as_std_path(), &exact_paths));
        assert!(!should_watch_path(unrelated.as_std_path(), &exact_paths));
        assert!(!should_watch_path(turbo_json.as_std_path(), &exact_paths));
    }
}
