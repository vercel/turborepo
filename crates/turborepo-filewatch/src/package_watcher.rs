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
    discovery::{self, DiscoveryResponse, PackageDiscovery, WorkspaceData},
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
    #[error("failed to resolve package manager {0}")]
    PackageManager(#[from] package_manager::Error),
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
    package_manager_lazy: CookiedOptionalWatch<PackageManagerState, ()>,
}

impl PackageWatcher {
    /// Creates a new package watcher whose current package data can be queried.
    /// `backup_discovery` is used to perform the initial discovery of packages,
    /// to populate the state before we can watch.
    pub fn new<T: PackageDiscovery + Send + Sync + 'static>(
        root: AbsoluteSystemPathBuf,
        recv: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
        backup_discovery: T,
        cookie_writer: CookieWriter,
    ) -> Result<Self, package_manager::Error> {
        let (exit_tx, exit_rx) = oneshot::channel();
        let subscriber = Subscriber::new(root, backup_discovery, cookie_writer)?;
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
        recv.get().await.map(|s| s.manager)
    }

    /// A convenience wrapper around `FutureExt::now_or_never` to let you get
    /// the package manager if it is immediately available.
    pub async fn get_package_manager(&self) -> Option<Result<PackageManager, CookieError>> {
        let mut recv = self.package_manager_lazy.clone();
        // the borrow checker doesn't like returning immediately here so assign to a var
        #[allow(clippy::let_and_return)]
        let data = if let Some(Ok(inner)) = recv.get_immediate().await {
            Some(Ok(inner.manager))
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
struct Subscriber<T: PackageDiscovery> {
    backup_discovery: T,

    repo_root: AbsoluteSystemPathBuf,
    root_package_json_path: AbsoluteSystemPathBuf,
    // pnpm workspace file is handled specifically, the rest of the package managers
    // use package.json for workspaces. We need to invalidate everything on a workspace glob
    // change because we couple package manager detection with valid workspace globs.
    pnpm_workspace_path: AbsoluteSystemPathBuf,

    // package manager data
    package_manager_tx: watch::Sender<Option<PackageManagerState>>,
    package_manager_lazy: CookiedOptionalWatch<PackageManagerState, ()>,
    package_data_tx: watch::Sender<Option<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>>,
    package_data_lazy: CookiedOptionalWatch<HashMap<AbsoluteSystemPathBuf, WorkspaceData>, ()>,
    cookie_tx: CookieRegister,
}

/// A collection of state inferred from a package manager. All this data will
/// change if the package manager changes.
#[derive(Clone)]
struct PackageManagerState {
    manager: PackageManager,
    // we need to wrap in Arc to make it send / sync
    filter: Arc<WorkspaceGlobs>,
    workspace_config_path: AbsoluteSystemPathBuf,
}

impl<T: PackageDiscovery + Send + Sync + 'static> Subscriber<T> {
    /// Creates a new instance of PackageDiscovery. This will start a task that
    /// performs the initial discovery using the `backup_discovery` of your
    /// choice, and then listens to file system events to keep the package
    /// data up to date.
    fn new(
        repo_root: AbsoluteSystemPathBuf,
        backup_discovery: T,
        writer: CookieWriter,
    ) -> Result<Self, package_manager::Error> {
        let (package_data_tx, cookie_tx, package_data_lazy) = CookiedOptionalWatch::new(writer);
        let (package_manager_tx, package_manager_lazy) = package_data_lazy.new_sibling();

        // we create a second optional watch here so that we can ensure it is ready and
        // pass it down stream after the initial discovery, otherwise our package
        // discovery watcher will consume events before we have our initial state
        let package_json_path = repo_root.join_component("package.json");
        let pnpm_workspace_path = repo_root.join_component("pnpm-workspace.yaml");
        Ok(Self {
            backup_discovery,
            repo_root,
            pnpm_workspace_path,
            root_package_json_path: package_json_path,
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

        self.rediscover_everything().await;

        tracing::debug!("package watcher ready");
        loop {
            let file_event = recv.recv().await;
            match file_event {
                Ok(Ok(event)) => {
                    if let Err(e) = self.handle_file_event(&event).await {
                        tracing::debug!("package watching is closing, exiting");
                        return e;
                    }
                }
                // if we get an error, we need to re-discover the packages
                Ok(Err(_)) => self.rediscover_everything().await,
                Err(e @ RecvError::Closed) => return PackageWatcherError::FilewatchingClosed(e),
                // if we end up lagging, warn and rediscover packages
                Err(RecvError::Lagged(count)) => {
                    tracing::warn!("lagged behind {count} processing file watching events");
                    self.rediscover_everything().await;
                }
            }
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

    pub fn manager_receiver(&self) -> CookiedOptionalWatch<PackageManagerState, ()> {
        self.package_manager_lazy.clone()
    }

    pub fn package_data(
        &self,
    ) -> CookiedOptionalWatch<HashMap<AbsoluteSystemPathBuf, WorkspaceData>, ()> {
        self.package_data_lazy.clone()
    }

    fn path_invalidates_everything(&self, path: &Path) -> bool {
        path.eq(&self.root_package_json_path as &AbsoluteSystemPath)
            || path.eq(&self.pnpm_workspace_path as &AbsoluteSystemPath)
    }

    /// Returns Err(()) if the package manager channel is closed, indicating
    /// that the entire watching task should exit.
    async fn handle_file_event(&mut self, file_event: &Event) -> Result<(), PackageWatcherError> {
        tracing::trace!("file event: {:?} {:?}", file_event.kind, file_event.paths);

        if file_event
            .paths
            .iter()
            .any(|path| self.path_invalidates_everything(path))
        {
            // root package.json changed, rediscover everything
            self.rediscover_everything().await;
        } else {
            tracing::trace!("handling non-root package.json change");
            let globs_have_changed = self.have_workspace_globs_changed(file_event).await;
            if globs_have_changed {
                tracing::trace!("glob change?");
                self.rediscover_packages().await;
            } else {
                tracing::trace!("checking for package.json change");
                self.handle_package_json_change(file_event).await?;
            }
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

        Ok(())
    }

    /// Returns Err(()) if the package manager channel is closed, indicating
    /// that the entire watching task should exit.
    async fn handle_package_json_change(
        &mut self,
        file_event: &Event,
    ) -> Result<(), PackageWatcherError> {
        // we can only fail receiving if the channel is closed,
        // which indicated that the entire watching task should exit
        let state = {
            let package_manager_state = self
                .package_manager_lazy
                .get_immediate_raw("this is called from the file event loop")
                .await;
            match package_manager_state {
                // We don't have a package manager, no sense trying to handle workspace changes.
                None => return Ok(()),
                Some(state) => state.map(|s| s.to_owned()).expect(
                    "this is called from the file event loop, and only when we have a package \
                     manager",
                ),
            }
        };

        // here, we can only update if we have a valid package state

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
                    if state
                        .filter
                        .target_is_workspace(&self.repo_root, path_parent)
                        .unwrap_or(false)
                    {
                        path_parent
                    } else {
                        // irrelevant package.json file update, it's not in a directory
                        // matching workspace globs
                        continue;
                    }
                } else if state
                    .filter
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

            self.package_data_tx
                .send_modify(|mut data| match (&mut data, package_exists) {
                    // We have initial data, and this workspace exists
                    (Some(data), true) => {
                        data.insert(
                            path_workspace.to_owned(),
                            WorkspaceData {
                                package_json,
                                turbo_json: turbo_exists.unwrap_or_default().then_some(turbo_json),
                            },
                        );
                    }
                    // we have initial data, and this workspace does not exist
                    (Some(data), false) => {
                        data.remove(path_workspace);
                    }
                    // this is our first workspace, and it exists
                    (None, true) => {
                        let mut map = HashMap::new();
                        map.insert(
                            path_workspace.to_owned(),
                            WorkspaceData {
                                package_json,
                                turbo_json: turbo_exists.unwrap_or_default().then_some(turbo_json),
                            },
                        );
                        *data = Some(map);
                    }
                    // we have no workspaces, and this one does not exist,
                    // so there's nothing to do.
                    (None, false) => {}
                });
        }

        Ok(())
    }

    /// A change to the workspace config path could mean a change to the package
    /// glob list. If this happens, we need to re-walk the packages.
    ///
    /// Returns Err(()) if the package manager channel is closed, indicating
    /// that the entire watching task should exit.
    async fn have_workspace_globs_changed(&mut self, file_event: &Event) -> bool {
        // here, we can only update if we have a valid package state
        // we can only fail receiving if the channel is closed,
        // which indicated that the entire watching task should exit
        let package_manager_state = self
            .package_manager_lazy
            .get_immediate_raw("this is called from the file event loop")
            .await;
        let state = match package_manager_state {
            // We don't have a package manager, no sense trying to parse globs
            None => return false,
            Some(state) => state.map(|s| s.to_owned()).expect(
                "this is called from the file event loop, and only when we have a package manager",
            ),
        };

        if file_event
            .paths
            .iter()
            .any(|p| state.workspace_config_path.as_std_path().eq(p))
        {
            let new_filter = state
                .manager
                .get_workspace_globs(&self.repo_root)
                .map(Arc::new)
                // under some saving strategies a file can be totally empty for a moment
                // during a save. these strategies emit multiple events and so we can
                // a previous or subsequent event in the 'cluster' will still trigger
                .unwrap_or_else(|_| state.filter.clone());

            self.package_manager_tx.send_if_modified(|f| match f {
                Some(state) if state.filter == new_filter => false,
                Some(state) => {
                    tracing::debug!("workspace globs changed: {:?}", new_filter);
                    state.filter = new_filter;
                    true
                }
                // if we haven't got a valid manager, then it probably means
                // that we are currently calcuating one, so we should just
                // ignore this event
                None => false,
            })
        } else {
            false
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

    async fn rediscover_everything(&mut self) {
        // If we're rediscovering the package manager, clear all data
        self.reset_package_manager();
        self.reset_workspaces();
        let initial_discovery = match self.backup_discovery.discover_packages().await {
            Ok(discovery) => discovery,
            // If we failed the discovery, that's fine, we've reset the values, leave them as None
            Err(e) => {
                tracing::debug!("failed to rediscover packages: {}", e);
                return;
            }
        };
        tracing::debug!("rediscovered packages: {:?}", initial_discovery);

        let workspace_config_path = initial_discovery
            .package_manager
            .workspace_configuration_path()
            .map_or_else(
                || self.root_package_json_path.to_owned(),
                |p| self.repo_root.join_component(p),
            );
        let filter = match initial_discovery
            .package_manager
            .get_workspace_globs(&self.repo_root)
        {
            Ok(filter) => Arc::new(filter),
            Err(e) => {
                // If the globs are invalid, leave everything set to None
                tracing::debug!("failed to get workspace globs: {}", e);
                return;
            }
        };

        let state = PackageManagerState {
            manager: initial_discovery.package_manager,
            filter,
            workspace_config_path,
        };

        // if either of these fail, it means that there are no more subscribers and we
        // should just ignore it, since we are likely closing
        let _ = self.package_manager_tx.send(Some(state));
        let _ = self.package_data_tx.send(Some(
            initial_discovery
                .workspaces
                .into_iter()
                .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                .collect::<HashMap<_, _>>(),
        ));
    }

    async fn rediscover_packages(&mut self) {
        tracing::debug!("rediscovering packages");

        // make sure package data is unavailable while we are updating
        self.reset_workspaces();

        let response = match self.backup_discovery.discover_packages().await {
            Ok(discovery) => discovery,
            // If we failed the discovery, that's fine, we've reset the values, leave them as None
            Err(e) => {
                tracing::debug!("failed to rediscover packages: {}", e);
                return;
            }
        };
        self.package_data_tx.send_modify(|d| {
            let new_data = response
                .workspaces
                .into_iter()
                .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                .collect::<HashMap<_, _>>();
            let _ = d.insert(new_data);
        });
    }
}

#[cfg(test)]
mod test {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use itertools::Itertools;
    use tokio::{join, sync::broadcast};
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{
        discovery::{
            self, DiscoveryResponse, LocalPackageDiscoveryBuilder, PackageDiscoveryBuilder,
            WorkspaceData,
        },
        package_manager::PackageManager,
    };

    use super::Subscriber;
    use crate::{
        cookies::CookieWriter, package_watcher::PackageWatcher, FileSystemWatcher, OptionalWatch,
    };

    #[derive(Debug)]
    struct MockDiscovery {
        pub manager: PackageManager,
        pub package_data: Arc<Mutex<Vec<WorkspaceData>>>,
    }

    impl super::PackageDiscovery for MockDiscovery {
        async fn discover_packages(&self) -> Result<DiscoveryResponse, discovery::Error> {
            Ok(DiscoveryResponse {
                package_manager: self.manager,
                workspaces: self.package_data.lock().unwrap().clone(),
            })
        }

        async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, discovery::Error> {
            self.discover_packages().await
        }
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn subscriber_test() {
        let tmp = tempfile::tempdir().unwrap();

        let (tx, rx) = broadcast::channel(10);
        let rx = OptionalWatch::once(rx);
        let (_exit_tx, exit_rx) = tokio::sync::oneshot::channel();

        let root: AbsoluteSystemPathBuf = tmp.path().try_into().unwrap();
        let manager = PackageManager::Yarn;

        let package_data = vec![
            WorkspaceData {
                package_json: root.join_component("package.json"),
                turbo_json: None,
            },
            WorkspaceData {
                package_json: root.join_components(&["packages", "foo", "package.json"]),
                turbo_json: None,
            },
            WorkspaceData {
                package_json: root.join_components(&["packages", "bar", "package.json"]),
                turbo_json: None,
            },
        ];

        // create folders and files
        for data in &package_data {
            data.package_json.ensure_dir().unwrap();
            data.package_json.create_with_contents("{}").unwrap();
        }

        // write workspaces to root
        root.join_component("package.json")
            .create_with_contents(r#"{"workspaces":["packages/*"]}"#)
            .unwrap();

        let mock_discovery = MockDiscovery {
            manager,
            package_data: Arc::new(Mutex::new(package_data)),
        };
        let cookie_writer =
            CookieWriter::new_with_default_cookie_dir(&root, Duration::from_secs(2), rx.clone());

        let subscriber =
            Subscriber::new(root.clone(), mock_discovery, cookie_writer.clone()).unwrap();

        let mut package_data = subscriber.package_data();
        let _handle = tokio::spawn(subscriber.watch(exit_rx, rx));

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Create(notify::event::CreateKind::File),
            paths: vec![root.join_component("package.json").as_std_path().to_owned()],
            ..Default::default()
        }))
        .unwrap();

        let (data, _) = join! {
                package_data.get(),
                async {
                    // simulate fs round trip
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    let path = cookie_writer.cookie_dir().join_component("1.cookie").as_std_path().to_owned();
                    tracing::info!("writing cookie at {}", path.to_string_lossy());
                    tx.send(Ok(notify::Event {
                        kind: notify::EventKind::Create(notify::event::CreateKind::File),
                        paths: vec![path],
                        ..Default::default()
                    })).unwrap();
                }
        };

        assert_eq!(
            data.unwrap()
                .values()
                .cloned()
                .sorted_by_key(|f| f.package_json.clone())
                .collect::<Vec<_>>(),
            vec![
                WorkspaceData {
                    package_json: root.join_component("package.json"),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: root.join_components(&["packages", "bar", "package.json",]),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: root.join_components(&["packages", "foo", "package.json",]),
                    turbo_json: None,
                },
            ]
        );

        tracing::info!("removing subpackage");

        // delete package.json in foo
        root.join_components(&["packages", "foo", "package.json"])
            .remove_file()
            .unwrap();

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![root
                .join_components(&["packages", "foo", "package.json"])
                .as_std_path()
                .to_owned()],
            ..Default::default()
        }))
        .unwrap();

        let (data, _) = join! {
                package_data.get(),
                async {
                    // simulate fs round trip
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    let path = cookie_writer.cookie_dir().join_component("2.cookie").as_std_path().to_owned();
                    tracing::info!("writing cookie at {}", path.to_string_lossy());
                    tx.send(Ok(notify::Event {
                        kind: notify::EventKind::Create(notify::event::CreateKind::File),
                        paths: vec![path],
                        ..Default::default()
                    })).unwrap();
                }
        };

        assert_eq!(
            data.unwrap()
                .values()
                .cloned()
                .sorted_by_key(|f| f.package_json.clone())
                .collect::<Vec<_>>(),
            vec![
                WorkspaceData {
                    package_json: root.join_component("package.json"),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: root.join_components(&["packages", "bar", "package.json"]),
                    turbo_json: None,
                }
            ]
        );

        // move package bar
        root.join_components(&["packages", "bar"])
            .rename(&root.join_component("bar"))
            .unwrap();

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![root
                .join_components(&["packages", "bar"])
                .as_std_path()
                .to_owned()],
            ..Default::default()
        }))
        .unwrap();

        let (data, _) = join! {
                package_data.get(),
                async {
                    // simulate fs round trip
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    let path = cookie_writer.cookie_dir().join_component("3.cookie").as_std_path().to_owned();
                    tracing::info!("writing cookie at {}", path.to_string_lossy());
                    tx.send(Ok(notify::Event {
                        kind: notify::EventKind::Create(notify::event::CreateKind::File),
                        paths: vec![path],
                        ..Default::default()
                    })).unwrap();
                }
        };

        assert_eq!(
            data.unwrap()
                .values()
                .cloned()
                .sorted_by_key(|f| f.package_json.clone())
                .collect::<Vec<_>>(),
            vec![WorkspaceData {
                package_json: root.join_component("package.json"),
                turbo_json: None,
            }]
        );
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn subscriber_update_workspaces() {
        let tmp = tempfile::tempdir().unwrap();

        let (tx, rx) = broadcast::channel(10);
        let rx = OptionalWatch::once(rx);
        let (_exit_tx, exit_rx) = tokio::sync::oneshot::channel();

        let root = AbsoluteSystemPathBuf::new(tmp.path().to_string_lossy()).unwrap();
        let manager = PackageManager::Yarn;

        let package_data = vec![
            WorkspaceData {
                package_json: root.join_component("package.json"),
                turbo_json: None,
            },
            WorkspaceData {
                package_json: root
                    .join_component("packages")
                    .join_component("foo")
                    .join_component("package.json"),
                turbo_json: None,
            },
            WorkspaceData {
                package_json: root
                    .join_component("packages2")
                    .join_component("bar")
                    .join_component("package.json"),
                turbo_json: None,
            },
        ];

        // create folders and files
        for data in &package_data {
            tokio::fs::create_dir_all(data.package_json.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(&data.package_json, b"{}").await.unwrap();
        }

        // write workspaces to root
        tokio::fs::write(
            root.join_component("package.json"),
            r#"{"workspaces":["packages/*", "packages2/*"]}"#,
        )
        .await
        .unwrap();

        let package_data_raw = Arc::new(Mutex::new(package_data));

        let mock_discovery = MockDiscovery {
            manager,
            package_data: package_data_raw.clone(),
        };

        let cookie_writer =
            CookieWriter::new_with_default_cookie_dir(&root, Duration::from_secs(2), rx.clone());
        let subscriber =
            Subscriber::new(root.clone(), mock_discovery, cookie_writer.clone()).unwrap();

        let mut package_data = subscriber.package_data();

        let _handle = tokio::spawn(subscriber.watch(exit_rx, rx));

        let (data, _) = join! {
                package_data.get(),
                async {
                    // simulate fs round trip
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    let path = cookie_writer.cookie_dir().join_component("1.cookie").as_std_path().to_owned();
                    tracing::info!("writing cookie at {}", path.to_string_lossy());
                    tx.send(Ok(notify::Event {
                        kind: notify::EventKind::Create(notify::event::CreateKind::File),
                        paths: vec![path],
                        ..Default::default()
                    })).unwrap();
                }
        };

        assert_eq!(
            data.unwrap()
                .values()
                .cloned()
                .sorted_by_key(|f| f.package_json.clone())
                .collect::<Vec<_>>(),
            vec![
                WorkspaceData {
                    package_json: root.join_component("package.json"),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: root
                        .join_component("packages")
                        .join_component("foo")
                        .join_component("package.json"),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: root
                        .join_component("packages2")
                        .join_component("bar")
                        .join_component("package.json"),
                    turbo_json: None,
                },
            ]
        );

        // update workspaces
        tracing::info!("updating workspaces");
        *package_data_raw.lock().unwrap() = vec![
            WorkspaceData {
                package_json: root.join_component("package.json"),
                turbo_json: None,
            },
            WorkspaceData {
                package_json: root
                    .join_component("packages")
                    .join_component("foo")
                    .join_component("package.json"),
                turbo_json: None,
            },
        ];
        tokio::fs::write(
            root.join_component("package.json"),
            r#"{"workspaces":["packages/*"]}"#,
        )
        .await
        .unwrap();

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![root.join_component("package.json").as_std_path().to_owned()],
            ..Default::default()
        }))
        .unwrap();

        let (data, _) = join! {
                package_data.get(),
                async {
                    // simulate fs round trip
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    let path = cookie_writer.cookie_dir().join_component("2.cookie").as_std_path().to_owned();
                    tracing::info!("writing cookie at {}", path.to_string_lossy());
                    tx.send(Ok(notify::Event {
                        kind: notify::EventKind::Create(notify::event::CreateKind::File),
                        paths: vec![path],
                        ..Default::default()
                    })).unwrap();
                }
        };

        assert_eq!(
            data.unwrap()
                .values()
                .cloned()
                .sorted_by_key(|f| f.package_json.clone())
                .collect::<Vec<_>>(),
            vec![
                WorkspaceData {
                    package_json: root.join_component("package.json"),
                    turbo_json: None,
                },
                WorkspaceData {
                    package_json: root
                        .join_component("packages")
                        .join_component("foo")
                        .join_component("package.json"),
                    turbo_json: None,
                }
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

        let package_watcher = PackageWatcher::new(
            repo_root.clone(),
            recv,
            LocalPackageDiscoveryBuilder::new(repo_root.clone(), None, None)
                .build()
                .unwrap(),
            cookie_writer,
        )
        .unwrap();

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

        let package_manager = tokio::time::timeout(
            Duration::from_millis(200),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap()
        .unwrap();
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

        let package_watcher = PackageWatcher::new(
            repo_root.clone(),
            recv,
            LocalPackageDiscoveryBuilder::new(repo_root.clone(), None, None)
                .build()
                .unwrap(),
            cookie_writer,
        )
        .unwrap();

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

        let package_manager = tokio::time::timeout(
            Duration::from_millis(200),
            package_watcher.wait_for_package_manager(),
        )
        .await
        .unwrap()
        .unwrap();
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
}
