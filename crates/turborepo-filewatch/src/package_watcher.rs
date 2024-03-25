//! This module hosts the `PackageWatcher` type, which is used to watch the
//! filesystem for changes to packages.

use std::{collections::HashMap, path::Path, sync::Arc};

use futures::FutureExt;
use notify::Event;
use thiserror::Error;
use tokio::{
    join,
    sync::{
        broadcast::{self, error::RecvError},
        oneshot, watch,
    },
};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::{
    discovery::{
        self, DiscoveryResponse, LocalPackageDiscoveryBuilder, PackageDiscovery,
        PackageDiscoveryBuilder, WorkspaceData,
    },
    package_manager::{self, PackageManager, WorkspaceGlobs},
};

use crate::{
    cookies::{CookieError, CookieRegister, CookieWriter, CookiedOptionalWatch},
    optional_watch::OptionalWatch,
    NotifyError,
};

/// A package discovery strategy that watches the file system for changes. Basic
/// idea:
/// - Set up a watcher on file changes on the relevant workspace file for the
///   package manager
/// - When the workspace globs change, re-discover the workspace
/// - When a package.json changes, re-discover the workspace
/// - Keep an in-memory cache of the workspace
pub struct WatchingPackageDiscovery {
    /// file watching may not be ready yet so we store a watcher
    /// through which we can get the file watching stack
    watcher: Arc<PackageWatcher>,
}

impl WatchingPackageDiscovery {
    pub fn new(watcher: Arc<PackageWatcher>) -> Self {
        Self { watcher }
    }
}

impl PackageDiscovery for WatchingPackageDiscovery {
    async fn discover_packages(&self) -> Result<DiscoveryResponse, discovery::Error> {
        tracing::debug!("discovering packages using watcher implementation");

        // this can either not have a value ready, or the sender has been dropped. in
        // either case rust report that the value is unavailable
        let package_manager = self
            .watcher
            .get_package_manager()
            .await
            .and_then(Result::ok)
            .ok_or(discovery::Error::Unavailable)?;
        let workspaces = self
            .watcher
            .get_package_data()
            .await
            .and_then(Result::ok)
            .ok_or(discovery::Error::Unavailable)?;

        Ok(DiscoveryResponse {
            workspaces,
            package_manager,
        })
    }

    // if the event that either of the dependencies will never resolve,
    // this will still return unavailable
    async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, discovery::Error> {
        let package_manager = self
            .watcher
            .wait_for_package_manager()
            .await
            .map_err(|_| discovery::Error::Unavailable)?;

        let workspaces = self
            .watcher
            .wait_for_package_data()
            .await
            .map_err(|_| discovery::Error::Unavailable)?;

        Ok(DiscoveryResponse {
            workspaces,
            package_manager,
        })
    }
}

#[derive(Debug, Error)]
enum PackageWatcherError {
    #[error("filewatching not available, so package watching is not available")]
    Filewatching(watch::error::RecvError),
    #[error("filewatching closed, package watching no longer available")]
    FilewatchingClosed(broadcast::error::RecvError),
}

/// Watches the filesystem for changes to packages and package managers.
pub struct PackageWatcher {
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_tx: oneshot::Sender<()>,
    _handle: tokio::task::JoinHandle<()>,

    /// The current package data, if available.
    package_data: CookiedOptionalWatch<HashMap<AbsoluteSystemPathBuf, WorkspaceData>, ()>,

    /// The current package manager, if available.
    package_manager_lazy: CookiedOptionalWatch<PackageManager, ()>,
}

impl PackageWatcher {
    /// Creates a new package watcher whose current package data can be queried.
    /// `backup_discovery` is used to perform the initial discovery of packages,
    /// to populate the state before we can watch.
    pub fn new(
        root: AbsoluteSystemPathBuf,
        recv: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
        cookie_writer: CookieWriter,
    ) -> Result<Self, package_manager::Error> {
        let (exit_tx, exit_rx) = oneshot::channel();
        let subscriber = Subscriber::new(root, cookie_writer)?;
        let package_manager_lazy = subscriber.manager_receiver();
        let package_data = subscriber.package_data();
        let handle = tokio::spawn(subscriber.watch(exit_rx, recv));
        Ok(Self {
            _exit_tx: exit_tx,
            _handle: handle,
            package_data,
            package_manager_lazy,
        })
    }

    /// Get the package data. If the package data is not available, this will
    /// block until it is.
    pub async fn wait_for_package_data(&self) -> Result<Vec<WorkspaceData>, CookieError> {
        let mut recv = self.package_data.clone();
        recv.get().await.map(|v| v.values().cloned().collect())
    }

    /// A convenience wrapper around `FutureExt::now_or_never` to let you get
    /// the package data if it is immediately available.
    pub async fn get_package_data(
        &self,
    ) -> Option<Result<Vec<WorkspaceData>, watch::error::RecvError>> {
        let mut recv = self.package_data.clone();
        let data = if let Some(Ok(inner)) = recv.get_immediate().await {
            Some(Ok(inner.values().cloned().collect()))
        } else {
            None
        };
        data
    }

    /// Get the package manager. If the package manager is not available, this
    /// will block until it is.
    pub async fn wait_for_package_manager(&self) -> Result<PackageManager, CookieError> {
        let mut recv = self.package_manager_lazy.clone();
        recv.get().await.map(|s| s.to_owned())
    }

    /// A convenience wrapper around `FutureExt::now_or_never` to let you get
    /// the package manager if it is immediately available.
    pub async fn get_package_manager(&self) -> Option<Result<PackageManager, CookieError>> {
        let mut recv = self.package_manager_lazy.clone();
        // the borrow checker doesn't like returning immediately here so assign to a var
        #[allow(clippy::let_and_return)]
        let data = if let Some(Ok(inner)) = recv.get_immediate().await {
            Some(Ok(*inner))
        } else {
            None
        };
        data
    }

    pub fn watch(&self) -> CookiedOptionalWatch<HashMap<AbsoluteSystemPathBuf, WorkspaceData>, ()> {
        self.package_data.clone()
    }
}

/// The underlying task that listens to file system events and updates the
/// internal package state.
struct Subscriber {
    repo_root: AbsoluteSystemPathBuf,
    // This is the list of paths that will trigger rediscovering everything.
    invalidation_paths: Vec<AbsoluteSystemPathBuf>,

    // package manager data
    package_manager_tx: watch::Sender<Option<PackageManager>>,
    package_manager_lazy: CookiedOptionalWatch<PackageManager, ()>,
    package_data_tx: watch::Sender<Option<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>>,
    package_data_lazy: CookiedOptionalWatch<HashMap<AbsoluteSystemPathBuf, WorkspaceData>, ()>,
    cookie_tx: CookieRegister,
}

/// PackageWatcher state. We either don't have a valid package manager,
/// don't have valid globs, or we have both a package manager and globs
/// and some maybe-empty set of workspaces.
#[derive(Debug)]
enum State {
    NoPackageManager,
    InvalidGlobs(PackageManager),
    ValidWorkspaces {
        package_manager: PackageManager,
        filter: WorkspaceGlobs,
        workspaces: HashMap<AbsoluteSystemPathBuf, WorkspaceData>,
    },
}

// Because our package manager detection is coupled with the workspace globs, we
// need to recheck all workspaces any time any of these files change. A change
// in any of these might result in a different package manager being detected,
// or going from no package manager to some package manager.
const INVALIDATION_PATHS: &[&str] = &[
    "package.json",
    "pnpm-workspace.yaml",
    "pnpm-lock.yaml",
    "package-lock.json",
    "yarn.lock",
    "bun.lockb",
];

impl Subscriber {
    /// Creates a new instance of PackageDiscovery. This will start a task that
    /// performs the initial discovery using the `backup_discovery` of your
    /// choice, and then listens to file system events to keep the package
    /// data up to date.
    fn new(
        repo_root: AbsoluteSystemPathBuf,
        writer: CookieWriter,
    ) -> Result<Self, package_manager::Error> {
        let (package_data_tx, cookie_tx, package_data_lazy) = CookiedOptionalWatch::new(writer);
        let (package_manager_tx, package_manager_lazy) = package_data_lazy.new_sibling();

        let invalidation_paths = INVALIDATION_PATHS
            .iter()
            .map(|p| repo_root.join_component(p))
            .collect();
        Ok(Self {
            repo_root,
            invalidation_paths,
            package_data_lazy,
            package_data_tx,
            package_manager_lazy,
            package_manager_tx,
            cookie_tx,
        })
    }

    async fn watch_process(
        mut self,
        mut recv: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    ) -> PackageWatcherError {
        tracing::debug!("starting package watcher");
        let mut recv = match recv.get().await {
            Ok(r) => r.resubscribe(),
            Err(e) => return PackageWatcherError::Filewatching(e),
        };

        // state represents our current understanding of the underlying filesystem, and
        // is expected to be mutated in place by handle_file_event. Both
        // rediscover_everything and handle_file_event are responsible for
        // broadcasting updates to state.
        let mut state = self.rediscover_everything().await;

        tracing::debug!("package watcher ready {:?}", state);
        loop {
            let file_event = recv.recv().await;
            match file_event {
                Ok(Ok(event)) => self.handle_file_event(&mut state, &event).await,
                // if we get an error, we need to re-discover the packages
                Ok(Err(_)) => state = self.rediscover_everything().await,
                Err(e @ RecvError::Closed) => return PackageWatcherError::FilewatchingClosed(e),
                // if we end up lagging, warn and rediscover packages
                Err(RecvError::Lagged(count)) => {
                    tracing::warn!("lagged behind {count} processing file watching events");
                    state = self.rediscover_everything().await;
                }
            }
            tracing::debug!("package watcher state: {:?}", state);
        }
    }

    async fn watch(
        self,
        exit_rx: oneshot::Receiver<()>,
        recv: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    ) {
        let process = tokio::spawn(self.watch_process(recv));
        tokio::select! {
            biased;
            _ = exit_rx => {
                tracing::debug!("exiting package watcher due to signal");
            },
            err = process => {
                if let Ok(err) = err {
                    tracing::debug!("exiting package watcher due to {err}");
                } else {
                    tracing::debug!("package watcher process exited");
                }
            }
        }
    }

    pub fn manager_receiver(&self) -> CookiedOptionalWatch<PackageManager, ()> {
        self.package_manager_lazy.clone()
    }

    pub fn package_data(
        &self,
    ) -> CookiedOptionalWatch<HashMap<AbsoluteSystemPathBuf, WorkspaceData>, ()> {
        self.package_data_lazy.clone()
    }

    fn path_invalidates_everything(&self, path: &Path) -> bool {
        self.invalidation_paths
            .iter()
            .any(|invalidation_path| path.eq(invalidation_path as &AbsoluteSystemPath))
    }

    /// Returns Err(()) if the package manager channel is closed, indicating
    /// that the entire watching task should exit.
    async fn handle_file_event(&mut self, state: &mut State, file_event: &Event) {
        tracing::trace!("file event: {:?} {:?}", file_event.kind, file_event.paths);

        if file_event
            .paths
            .iter()
            .any(|path| self.path_invalidates_everything(path))
        {
            // root package.json changed, rediscover everything
            *state = self.rediscover_everything().await;
        } else {
            tracing::trace!("handling non-root package.json change");
            self.handle_workspace_changes(state, file_event).await;
        }

        tracing::trace!("updating the cookies");

        // now that we have updated the state, we should bump the cookies so that
        // people waiting on downstream cookie watchers can get the new state
        self.cookie_tx.register(
            &file_event
                .paths
                .iter()
                .map(|p| AbsoluteSystemPath::from_std_path(p).expect("these paths are absolute"))
                .collect::<Vec<_>>(),
        );
    }

    // checks if the file event contains any changes to package.json files, or
    // directories that would map to a workspace.
    async fn handle_workspace_changes(&mut self, state: &mut State, file_event: &Event) {
        // If we don't have a valid package manager and workspace globs, nothing to be
        // done here
        let State::ValidWorkspaces {
            filter, workspaces, ..
        } = state
        else {
            return;
        };

        // here, we can only update if we have a valid package state
        let mut changed = false;
        // if a path is not a valid utf8 string, it is not a valid path, so ignore
        for path in file_event
            .paths
            .iter()
            .filter_map(|p| p.as_os_str().to_str())
        {
            let path_file = AbsoluteSystemPathBuf::new(path).expect("watched paths are absolute");
            let path_workspace: &AbsoluteSystemPath =
                if path_file.file_name() == Some("package.json") {
                    // The file event is for a package.json file. Check if the parent is a workspace
                    let path_parent = path_file
                        .parent()
                        .expect("watched paths will not be at the root");
                    if filter
                        .target_is_workspace(&self.repo_root, path_parent)
                        .unwrap_or(false)
                    {
                        path_parent
                    } else {
                        // irrelevant package.json file update, it's not in a directory
                        // matching workspace globs
                        continue;
                    }
                } else if filter
                    .target_is_workspace(&self.repo_root, &path_file)
                    .unwrap_or(false)
                {
                    // The file event is for a workspace directory itself
                    &path_file
                } else {
                    // irrelevant file update, it's not a package.json file or a workspace directory
                    continue;
                };

            tracing::debug!("handling change to workspace {path_workspace}");
            let package_json = path_workspace.join_component("package.json");
            let turbo_json = path_workspace.join_component("turbo.json");

            let (package_exists, turbo_exists) = join!(
                // It's possible that an IO error could occur other than the file not existing, but
                // we will treat it like the file doesn't exist. It's possible we'll need to
                // revisit this, depending on what kind of errors occur.
                tokio::fs::try_exists(&package_json).map(|result| result.unwrap_or(false)),
                tokio::fs::try_exists(&turbo_json)
            );

            changed |= if package_exists {
                workspaces
                    .insert(
                        path_workspace.to_owned(),
                        WorkspaceData {
                            package_json,
                            turbo_json: turbo_exists.unwrap_or_default().then_some(turbo_json),
                        },
                    )
                    .is_none()
            } else {
                workspaces.remove(path_workspace).is_some()
            }
        }

        if changed {
            self.write_workspace_state(workspaces);
        }
    }

    fn reset_package_manager(&self) {
        self.package_manager_tx.send_if_modified(|package_manager| {
            if package_manager.is_some() {
                *package_manager = None;
                true
            } else {
                false
            }
        });
    }

    fn reset_workspaces(&self) {
        self.package_data_tx.send_if_modified(|package_data| {
            if package_data.is_some() {
                *package_data = None;
                true
            } else {
                false
            }
        });
    }

    async fn rediscover_everything(&mut self) -> State {
        // If we're rediscovering the package manager, clear all data
        self.reset_package_manager();
        self.reset_workspaces();
        // If we're rediscovering everything, we need to rediscover the package manager.
        // It may have changed if a lockfile changed or package.json changed.
        let discovery =
            match LocalPackageDiscoveryBuilder::new(self.repo_root.clone(), None, None).build() {
                Ok(discovery) => discovery,
                Err(_) => return State::NoPackageManager,
            };
        let initial_discovery = match discovery.discover_packages().await {
            Ok(discovery) => discovery,
            // If we failed the discovery, that's fine, we've reset the values, leave them as None
            Err(e) => {
                tracing::debug!("failed to rediscover packages: {}", e);
                return State::NoPackageManager;
            }
        };

        tracing::debug!("rediscovered packages: {:?}", initial_discovery);
        let filter = match initial_discovery
            .package_manager
            .get_workspace_globs(&self.repo_root)
        {
            Ok(filter) => filter,
            Err(e) => {
                // If the globs are invalid, leave everything set to None
                tracing::debug!("failed to get workspace globs: {}", e);
                return State::InvalidGlobs(initial_discovery.package_manager);
            }
        };

        let workspaces = initial_discovery
            .workspaces
            .into_iter()
            .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
            .collect::<HashMap<_, _>>();
        let state = State::ValidWorkspaces {
            package_manager: initial_discovery.package_manager,
            filter,
            workspaces,
        };
        self.write_state(&state);
        state
    }

    fn write_state(&self, state: &State) {
        match state {
            State::NoPackageManager => {} /* no state to write, we've already reset when */
            // invalidating
            State::InvalidGlobs(package_manager) => self.write_package_manager(package_manager),
            State::ValidWorkspaces {
                package_manager,
                workspaces,
                ..
            } => {
                self.write_package_manager(package_manager);
                self.write_workspace_state(workspaces);
            }
        }
    }

    fn write_package_manager(&self, package_manager: &PackageManager) {
        let _ = self
            .package_manager_tx
            .send_if_modified(|existing| match existing {
                Some(existing) if existing == package_manager => false,
                Some(_) | None => {
                    *existing = Some(*package_manager);
                    true
                }
            });
    }

    fn write_workspace_state(&self, workspaces: &HashMap<AbsoluteSystemPathBuf, WorkspaceData>) {
        let _ = self
            .package_data_tx
            .send_if_modified(|existing| match existing {
                Some(existing) if existing == workspaces => false,
                Some(_) | None => {
                    *existing = Some(workspaces.clone());
                    true
                }
            });
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{discovery::WorkspaceData, package_manager::PackageManager};

    use crate::{cookies::CookieWriter, package_watcher::PackageWatcher, FileSystemWatcher};

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn subscriber_test() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let package_data = vec![
            WorkspaceData {
                package_json: repo_root.join_components(&["packages", "foo", "package.json"]),
                turbo_json: None,
            },
            WorkspaceData {
                package_json: repo_root.join_components(&["packages", "bar", "package.json"]),
                turbo_json: None,
            },
        ];

        // create folders and files
        for data in &package_data {
            data.package_json.ensure_dir().unwrap();
            let name = data.package_json.parent().unwrap().file_name().unwrap();
            data.package_json
                .create_with_contents(format!("{{\"name\": \"{name}\"}}"))
                .unwrap();
        }
        repo_root
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        // write workspaces to root
        repo_root
            .join_component("package.json")
            .create_with_contents(r#"{"workspaces":["packages/*"]}"#)
            .unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(
            watcher.cookie_dir(),
            Duration::from_millis(100),
            recv.clone(),
        );

        let package_watcher = PackageWatcher::new(repo_root.clone(), recv, cookie_writer).unwrap();

        let mut data = package_watcher.wait_for_package_data().await.unwrap();
        data.sort_by_key(|workspace| workspace.package_json.clone());
        assert_eq!(
            data,
            vec![
                WorkspaceData {
                    package_json: repo_root.join_components(&["packages", "bar", "package.json",]),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: repo_root.join_components(&["packages", "foo", "package.json",]),
                    turbo_json: None,
                },
            ]
        );

        tracing::info!("removing subpackage");

        // delete package.json in foo
        repo_root
            .join_components(&["packages", "foo", "package.json"])
            .remove_file()
            .unwrap();

        let mut data = package_watcher.wait_for_package_data().await.unwrap();
        data.sort_by_key(|workspace| workspace.package_json.clone());
        assert_eq!(
            data,
            vec![WorkspaceData {
                package_json: repo_root.join_components(&["packages", "bar", "package.json"]),
                turbo_json: None,
            }]
        );

        // move package bar
        repo_root
            .join_components(&["packages", "bar"])
            .rename(&repo_root.join_component("bar"))
            .unwrap();

        let mut data = package_watcher.wait_for_package_data().await.unwrap();
        data.sort_by_key(|workspace| workspace.package_json.clone());
        assert_eq!(data, vec![]);
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn subscriber_update_workspaces() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let package_data = vec![
            WorkspaceData {
                package_json: repo_root
                    .join_component("packages")
                    .join_component("foo")
                    .join_component("package.json"),
                turbo_json: None,
            },
            WorkspaceData {
                package_json: repo_root
                    .join_component("packages2")
                    .join_component("bar")
                    .join_component("package.json"),
                turbo_json: None,
            },
        ];

        // create folders and files
        for data in &package_data {
            data.package_json.ensure_dir().unwrap();
            let name = data.package_json.parent().unwrap().file_name().unwrap();
            data.package_json
                .create_with_contents(format!("{{\"name\": \"{name}\"}}"))
                .unwrap();
        }
        repo_root
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        // write workspaces to root
        repo_root
            .join_component("package.json")
            .create_with_contents(r#"{"workspaces":["packages/*", "packages2/*"]}"#)
            .unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(
            watcher.cookie_dir(),
            Duration::from_millis(100),
            recv.clone(),
        );

        let package_watcher = PackageWatcher::new(repo_root.clone(), recv, cookie_writer).unwrap();

        let mut data = package_watcher.wait_for_package_data().await.unwrap();
        data.sort_by_key(|workspace| workspace.package_json.clone());

        assert_eq!(
            data,
            vec![
                WorkspaceData {
                    package_json: repo_root
                        .join_component("packages")
                        .join_component("foo")
                        .join_component("package.json"),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: repo_root
                        .join_component("packages2")
                        .join_component("bar")
                        .join_component("package.json"),
                    turbo_json: None,
                },
            ]
        );

        // update workspaces to no longer cover packages2
        repo_root
            .join_component("package.json")
            .create_with_contents(r#"{"workspaces":["packages/*"]}"#)
            .unwrap();

        let mut data = package_watcher.wait_for_package_data().await.unwrap();
        data.sort_by_key(|workspace| workspace.package_json.clone());

        assert_eq!(
            data,
            vec![WorkspaceData {
                package_json: repo_root
                    .join_component("packages")
                    .join_component("foo")
                    .join_component("package.json"),
                turbo_json: None,
            }]
        );

        // move the packages2 workspace into package
        repo_root
            .join_components(&["packages2", "bar"])
            .rename(&repo_root.join_components(&["packages", "bar"]))
            .unwrap();
        let mut data = package_watcher.wait_for_package_data().await.unwrap();
        data.sort_by_key(|workspace| workspace.package_json.clone());
        assert_eq!(
            data,
            vec![
                WorkspaceData {
                    package_json: repo_root
                        .join_component("packages")
                        .join_component("bar")
                        .join_component("package.json"),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: repo_root
                        .join_component("packages")
                        .join_component("foo")
                        .join_component("package.json"),
                    turbo_json: None,
                },
            ]
        );
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn pnpm_invalid_states_test() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let workspaces_path = repo_root.join_component("pnpm-workspace.yaml");
        // Currently we require valid state to start the daemon
        let root_package_json_path = repo_root.join_component("package.json");
        // Start with no workspace glob
        root_package_json_path
            .create_with_contents(r#"{"packageManager": "pnpm@7.0"}"#)
            .unwrap();
        repo_root
            .join_component("pnpm-lock.yaml")
            .create_with_contents("")
            .unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(
            watcher.cookie_dir(),
            Duration::from_millis(100),
            recv.clone(),
        );

        let package_watcher = PackageWatcher::new(repo_root.clone(), recv, cookie_writer).unwrap();

        // TODO: change this to expect either an empty result, or a package manager
        // without globs
        tokio::time::timeout(
            Duration::from_millis(50),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap_err();

        workspaces_path
            .create_with_contents(r#"packages: ["foo/*"]"#)
            .unwrap();

        let package_manager = package_watcher.wait_for_package_manager().await.unwrap();
        assert_eq!(package_manager, PackageManager::Pnpm);

        // Remove workspaces file, verify we get a timeout
        workspaces_path.remove_file().unwrap();
        // TODO: this should eventually be an empty result
        tokio::time::timeout(
            Duration::from_millis(50),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap_err();

        // Create an invalid workspace glob
        workspaces_path
            .create_with_contents(r#"packages: ["foo/***"]"#)
            .unwrap();

        // TODO: this should eventually be an empty result
        tokio::time::timeout(
            Duration::from_millis(50),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap_err();

        // Set it back to valid, ensure we recover
        workspaces_path
            .create_with_contents(r#"packages: ["foo/*"]"#)
            .unwrap();

        let package_manager = tokio::time::timeout(
            Duration::from_millis(200),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(package_manager, PackageManager::Pnpm);
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn npm_invalid_states_test() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        // Currently we require valid state to start the daemon
        let root_package_json_path = repo_root.join_component("package.json");
        // Start with no workspace glob
        root_package_json_path
            .create_with_contents(r#"{"packageManager": "npm@7.0"}"#)
            .unwrap();
        repo_root
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(
            watcher.cookie_dir(),
            Duration::from_millis(100),
            recv.clone(),
        );

        let package_watcher = PackageWatcher::new(repo_root.clone(), recv, cookie_writer).unwrap();

        // TODO: change this to expect either an empty result, or a package manager
        // without globs
        tokio::time::timeout(
            Duration::from_millis(50),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap_err();

        root_package_json_path
            .create_with_contents(r#"{"packageManager": "pnpm@7.0", "workspaces": ["foo/*"]}"#)
            .unwrap();

        let package_manager = package_watcher.wait_for_package_manager().await.unwrap();
        assert_eq!(package_manager, PackageManager::Npm);

        // Remove workspaces file, verify we get a timeout
        root_package_json_path.remove_file().unwrap();
        // TODO: this should eventually be an empty result
        tokio::time::timeout(
            Duration::from_millis(50),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap_err();

        // Create an invalid workspace glob
        root_package_json_path
            .create_with_contents(r#"{"packageManager": "pnpm@7.0", "workspaces": ["foo/***"]}"#)
            .unwrap();

        // TODO: this should eventually be an empty result
        tokio::time::timeout(
            Duration::from_millis(50),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap_err();

        // Set it back to valid, ensure we recover
        root_package_json_path
            .create_with_contents(r#"{"packageManager": "pnpm@7.0", "workspaces": ["foo/*"]}"#)
            .unwrap();
        let package_manager = tokio::time::timeout(
            Duration::from_millis(200),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(package_manager, PackageManager::Npm);
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_change_package_manager() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let workspaces_path = repo_root.join_component("pnpm-workspace.yaml");
        workspaces_path
            .create_with_contents(r#"packages: ["foo/*"]"#)
            .unwrap();
        // Currently we require valid state to start the daemon
        let root_package_json_path = repo_root.join_component("package.json");
        // Start with no workspace glob
        root_package_json_path
            .create_with_contents(r#"{"packageManager": "pnpm@7.0"}"#)
            .unwrap();
        let pnpm_lock_file = repo_root.join_component("pnpm-lock.yaml");
        pnpm_lock_file.create_with_contents("").unwrap();

        let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(
            watcher.cookie_dir(),
            Duration::from_millis(100),
            recv.clone(),
        );

        let package_watcher = PackageWatcher::new(repo_root.clone(), recv, cookie_writer).unwrap();

        let package_manager = package_watcher.wait_for_package_manager().await.unwrap();
        assert_eq!(package_manager, PackageManager::Pnpm);

        pnpm_lock_file.remove_file().unwrap();
        // No more lock file, verify we're in an invalid state
        tokio::time::timeout(
            Duration::from_millis(50),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap_err();

        let npm_lock_file = repo_root.join_component("package-lock.json");
        npm_lock_file.create_with_contents("").unwrap();
        // now we have an npm lockfile, but we don't have workspaces. Still invalid
        tokio::time::timeout(
            Duration::from_millis(50),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap_err();

        // update package.json to complete the transition
        root_package_json_path
            .create_with_contents(r#"{"packageManager": "npm@7.0", "workspaces": ["foo/*"]}"#)
            .unwrap();
        let package_manager = tokio::time::timeout(
            Duration::from_millis(200),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(package_manager, PackageManager::Npm);
    }
}
