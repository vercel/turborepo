//! This module hosts the `PackageWatcher` type, which is used to watch the
//! filesystem for changes to packages.

use std::{collections::HashMap, sync::Arc, time::Duration};

use notify::Event;
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
    package_manager::{self, Error, PackageManager, WorkspaceGlobs},
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
    ) -> Result<Self, package_manager::Error> {
        let (exit_tx, exit_rx) = oneshot::channel();
        let subscriber = Subscriber::new(root, recv, backup_discovery)?;
        let package_manager_lazy = subscriber.manager_receiver();
        let package_data = subscriber.package_data();
        let handle = tokio::spawn(subscriber.watch(exit_rx));
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
    /// we need to hold on to this. dropping it will close the downstream
    /// data dependencies
    #[allow(clippy::type_complexity)]
    #[allow(dead_code)]
    file_event_receiver_tx:
        Arc<watch::Sender<Option<broadcast::Receiver<Result<Event, NotifyError>>>>>,

    file_event_receiver_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    backup_discovery: Arc<T>,

    repo_root: AbsoluteSystemPathBuf,
    root_package_json_path: AbsoluteSystemPathBuf,

    // package manager data
    package_manager_tx: Arc<watch::Sender<Option<PackageManagerState>>>,
    package_manager_lazy: CookiedOptionalWatch<PackageManagerState, ()>,
    package_data_tx: Arc<watch::Sender<Option<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>>>,
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
        mut recv: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
        backup_discovery: T,
    ) -> Result<Self, Error> {
        let writer = CookieWriter::new(&repo_root, Duration::from_secs(1), recv.clone());
        let (package_data_tx, cookie_tx, package_data_lazy) = CookiedOptionalWatch::new(writer);
        let package_data_tx = Arc::new(package_data_tx);
        let (package_manager_tx, package_manager_lazy) = package_data_lazy.new_sibling();
        let package_manager_tx = Arc::new(package_manager_tx);

        // we create a second optional watch here so that we can ensure it is ready and
        // pass it down stream after the initial discovery, otherwise our package
        // discovery watcher will consume events before we have our initial state
        let (file_event_receiver_tx, file_event_receiver_lazy) = OptionalWatch::new();
        let file_event_receiver_tx = Arc::new(file_event_receiver_tx);

        let backup_discovery = Arc::new(backup_discovery);

        let package_json_path = repo_root.join_component("package.json");

        let _task = tokio::spawn({
            let package_data_tx = package_data_tx.clone();
            let manager_tx = package_manager_tx.clone();
            let backup_discovery = backup_discovery.clone();
            let repo_root = repo_root.clone();
            let package_json_path = package_json_path.clone();
            let recv_tx = file_event_receiver_tx.clone();
            async move {
                // wait for the watcher, so we can process events that happen during discovery
                let Ok(recv) = recv.get().await.map(|r| r.resubscribe()) else {
                    // if we get here, it means that file watching has not started, so we should
                    // just report that the package watcher is not available
                    tracing::debug!("file watching shut down, package watcher not available");
                    return;
                };

                let initial_discovery = backup_discovery.discover_packages().await;

                let Ok(initial_discovery) = initial_discovery else {
                    // if initial discovery fails, there is nothing we can do. we should just report
                    // that the package watcher is not available
                    //
                    // NOTE: in the future, if we decide to differentiate between 'not ready' and
                    // unavailable, we MUST update the status here to unavailable or the client will
                    // hang
                    return;
                };

                let Ok((workspace_config_path, filter)) = Self::update_package_manager(
                    &initial_discovery.package_manager,
                    &repo_root,
                    &package_json_path,
                ) else {
                    // similar story here, if the package manager cannot be read, we should just
                    // report that the package watcher is not available
                    return;
                };

                // now that the two pieces of data are available, we can send the package
                // manager and set the packages

                let state = PackageManagerState {
                    manager: initial_discovery.package_manager,
                    filter: Arc::new(filter),
                    workspace_config_path,
                };

                // if either of these fail, it means that there are no more subscribers and we
                // should just ignore it, since we are likely closing
                let manager_listeners = if manager_tx.send(Some(state)).is_err() {
                    tracing::debug!("no listeners for package manager");
                    false
                } else {
                    true
                };

                let package_data_listeners = if package_data_tx
                    .send(Some(
                        initial_discovery
                            .workspaces
                            .into_iter()
                            .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                            .collect::<HashMap<_, _>>(),
                    ))
                    .is_err()
                {
                    tracing::debug!("no listeners for package data");
                    false
                } else {
                    true
                };

                // if we have no listeners for either, we should just exit
                if manager_listeners || package_data_listeners {
                    _ = recv_tx.send(Some(recv));
                } else {
                    tracing::debug!("no listeners for file events, exiting");
                }
            }
        });

        Ok(Self {
            file_event_receiver_tx,
            file_event_receiver_lazy,
            backup_discovery,
            repo_root,
            root_package_json_path: package_json_path,
            package_data_lazy,
            package_data_tx,
            package_manager_lazy,
            package_manager_tx,
            cookie_tx,
        })
    }

    async fn watch(mut self, exit_rx: oneshot::Receiver<()>) {
        let process = async move {
            tracing::debug!("starting package watcher");
            let Ok(mut recv) = self
                .file_event_receiver_lazy
                .get()
                .await
                .map(|r| r.resubscribe())
            else {
                // if the channel is closed, we should just exit
                tracing::debug!("file watcher shut down, exiting");
                return;
            };

            tracing::debug!("package watcher ready");
            loop {
                let file_event = recv.recv().await;
                match file_event {
                    Ok(Ok(event)) => match self.handle_file_event(&event).await {
                        Ok(()) => {}
                        Err(()) => {
                            tracing::debug!("package watching is closing, exiting");
                            return;
                        }
                    },
                    // if we get an error, we need to re-discover the packages
                    Ok(Err(_)) => self.rediscover_packages().await,
                    Err(RecvError::Closed) => return,
                    // if we end up lagging, warn and rediscover packages
                    Err(RecvError::Lagged(count)) => {
                        tracing::warn!("lagged behind {count} processing file watching events");
                        self.rediscover_packages().await;
                    }
                }
            }
        };

        // respond to changes

        tokio::select! {
            biased;
            _ = exit_rx => {
                tracing::debug!("exiting package watcher due to signal");
            },
            _ = process => {
                tracing::debug!("exiting package watcher due to process end");
            }
        }
    }

    fn update_package_manager(
        manager: &PackageManager,
        repo_root: &AbsoluteSystemPath,
        package_json_path: &AbsoluteSystemPath,
    ) -> Result<(AbsoluteSystemPathBuf, WorkspaceGlobs), Error> {
        let workspace_config_path = manager.workspace_configuration_path().map_or_else(
            || package_json_path.to_owned(),
            |p| repo_root.join_component(p),
        );
        let filter = manager.get_workspace_globs(repo_root)?;

        Ok((workspace_config_path, filter))
    }

    pub fn manager_receiver(&self) -> CookiedOptionalWatch<PackageManagerState, ()> {
        self.package_manager_lazy.clone()
    }

    pub fn package_data(
        &self,
    ) -> CookiedOptionalWatch<HashMap<AbsoluteSystemPathBuf, WorkspaceData>, ()> {
        self.package_data_lazy.clone()
    }

    /// Returns Err(()) if the package manager channel is closed, indicating
    /// that the entire watching task should exit.
    async fn handle_file_event(&mut self, file_event: &Event) -> Result<(), ()> {
        tracing::trace!("file event: {:?} {:?}", file_event.kind, file_event.paths);

        if file_event
            .paths
            .iter()
            .any(|p| self.root_package_json_path.as_std_path().eq(p))
        {
            if let Err(e) = self.handle_root_package_json_change().await {
                tracing::error!("error discovering package manager: {}", e);
            }
        }

        let out = match self.have_workspace_globs_changed(file_event).await {
            Ok(true) => {
                self.rediscover_packages().await;
                Ok(())
            }
            Ok(false) => {
                // it is the end of the function so we are going to return regardless
                self.handle_package_json_change(file_event).await
            }
            Err(()) => Err(()),
        };

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

        out
    }

    /// Returns Err(()) if the package manager channel is closed, indicating
    /// that the entire watching task should exit.
    async fn handle_package_json_change(&mut self, file_event: &Event) -> Result<(), ()> {
        let Ok(state) = self
            .package_manager_lazy
            .get_raw("this is called from the file event loop")
            .await
            .map(|x| x.to_owned())
        else {
            // the channel is closed, so there is no state to write into, return
            return Err(());
        };

        // here, we can only update if we have a valid package state

        // if a path is not a valid utf8 string, it is not a valid path, so ignore
        for path in file_event
            .paths
            .iter()
            .filter_map(|p| p.as_os_str().to_str())
        {
            let path_file = AbsoluteSystemPathBuf::new(path).expect("watched paths are absolute");

            // the path to the workspace this file is in is the parent
            let path_workspace = path_file
                .parent()
                .expect("watched paths will not be at the root")
                .to_owned();

            let is_workspace = match state
                .filter
                .target_is_workspace(&self.repo_root, &path_workspace)
            {
                Ok(is_workspace) => is_workspace,
                Err(e) => {
                    // this will only error if `repo_root` is not an anchor of `path_workspace`.
                    // if we hit this case, we can safely ignore it
                    tracing::debug!("yielded path not in workspace: {:?}", e);
                    continue;
                }
            };

            if is_workspace {
                tracing::debug!("tracing file in package: {:?}", path_file);
                let package_json = path_workspace.join_component("package.json");
                let turbo_json = path_workspace.join_component("turbo.json");

                let (package_exists, turbo_exists) = join!(
                    tokio::fs::try_exists(&package_json),
                    tokio::fs::try_exists(&turbo_json)
                );

                self.package_data_tx
                    .send_modify(|mut data| match (&mut data, package_exists) {
                        (Some(data), Ok(true)) => {
                            data.insert(
                                path_workspace,
                                WorkspaceData {
                                    package_json,
                                    turbo_json: turbo_exists
                                        .unwrap_or_default()
                                        .then_some(turbo_json),
                                },
                            );
                        }
                        (Some(data), Ok(false)) => {
                            data.remove(&path_workspace);
                        }
                        (None, Ok(true)) => {
                            let mut map = HashMap::new();
                            map.insert(
                                path_workspace,
                                WorkspaceData {
                                    package_json,
                                    turbo_json: turbo_exists
                                        .unwrap_or_default()
                                        .then_some(turbo_json),
                                },
                            );
                            *data = Some(map);
                        }
                        (None, Ok(false)) => {} // do nothing
                        (_, Err(_)) => todo!(),
                    });
            }
        }

        Ok(())
    }

    /// A change to the workspace config path could mean a change to the package
    /// glob list. If this happens, we need to re-walk the packages.
    ///
    /// Returns Err(()) if the package manager channel is closed, indicating
    /// that the entire watching task should exit.
    async fn have_workspace_globs_changed(&mut self, file_event: &Event) -> Result<bool, ()> {
        // here, we can only update if we have a valid package state
        let Ok(state) = self
            .package_manager_lazy
            .get_raw("this is called from the file event loop")
            .await
            .map(|s| s.to_owned())
        else {
            // we can only fail receiving if the channel is closed,
            // which indicated that the entire watching task should exit
            return Err(());
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

            Ok(self.package_manager_tx.send_if_modified(|f| match f {
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
            }))
        } else {
            Ok(false)
        }
    }

    /// A change to the root package json means we need to re-infer the package
    /// manager, update the glob list, and re-walk the packages.
    ///
    /// todo: we can probably improve the uptime here by splitting the package
    ///       manager out of the package discovery. if the package manager has
    ///       not changed, we probably do not need to re-walk the packages
    async fn handle_root_package_json_change(&mut self) -> Result<(), discovery::Error> {
        {
            // clear all data
            self.package_manager_tx.send(None).ok();
            self.package_data_tx.send(None).ok();
        }
        tracing::debug!("root package.json changed, refreshing package manager and globs");
        let resp = self.backup_discovery.discover_packages().await?;
        let new_manager = Self::update_package_manager(
            &resp.package_manager,
            &self.repo_root,
            &self.root_package_json_path,
        )
        .map(move |(a, b)| (resp, a, b));

        // if the package.json changed, we need to re-infer the package manager
        // and update the glob list

        match new_manager {
            Ok((new_manager, workspace_config_path, filter)) => {
                tracing::debug!(
                    "new package manager data: {:?}, {:?}",
                    new_manager.package_manager,
                    filter
                );

                let state = PackageManagerState {
                    manager: new_manager.package_manager,
                    filter: Arc::new(filter),
                    workspace_config_path,
                };

                {
                    // if this fails, we are closing anyways so ignore
                    self.package_manager_tx.send(Some(state)).ok();
                    self.package_data_tx.send_modify(move |mut d| {
                        let new_data = new_manager
                            .workspaces
                            .into_iter()
                            .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                            .collect::<HashMap<_, _>>();
                        if let Some(data) = &mut d {
                            *data = new_data;
                        } else {
                            *d = Some(new_data);
                        }
                    });
                }
            }
            Err(e) => {
                // if we cannot update the package manager, we should just leave
                // the package manager as None and make the package data unavailable
                tracing::error!("error getting package manager: {}", e);
            }
        }

        Ok(())
    }

    async fn rediscover_packages(&mut self) {
        tracing::debug!("rediscovering packages");

        // make sure package data is unavailable while we are updating
        // if this fails, we have no subscribers so we can just exit
        if self.package_data_tx.send(None).is_err() {
            return;
        }

        if let Ok(response) = self.backup_discovery.discover_packages().await {
            self.package_data_tx.send_modify(|d| {
                let new_data = response
                    .workspaces
                    .into_iter()
                    .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                    .collect::<HashMap<_, _>>();
                if let Some(data) = d {
                    *data = new_data;
                } else {
                    *d = Some(new_data);
                }
            });
        } else {
            // package data stays unavailable
            tracing::error!("error discovering packages");
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::{Arc, Mutex};

    use itertools::Itertools;
    use tokio::{join, sync::broadcast};
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{
        discovery::{self, DiscoveryResponse, WorkspaceData},
        package_manager::PackageManager,
    };

    use super::Subscriber;
    use crate::OptionalWatch;

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
                package_json: root
                    .join_component("packages")
                    .join_component("foo")
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
            r#"{"workspaces":["packages/*"]}"#,
        )
        .await
        .unwrap();

        let mock_discovery = MockDiscovery {
            manager,
            package_data: Arc::new(Mutex::new(package_data)),
        };

        let subscriber = Subscriber::new(root.clone(), rx, mock_discovery).unwrap();

        let mut package_data = subscriber.package_data();

        let _handle = tokio::spawn(subscriber.watch(exit_rx));

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

                    let path = root.join_component("1.cookie").as_std_path().to_owned();
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
            ]
        );

        tracing::info!("removing subpackage");

        // delete package.json in foo
        tokio::fs::remove_file(
            root.join_component("packages")
                .join_component("foo")
                .join_component("package.json"),
        )
        .await
        .unwrap();

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![root
                .join_component("packages")
                .join_component("foo")
                .join_component("package.json")
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

                    let path = root.join_component("2.cookie").as_std_path().to_owned();
                    tracing::info!("writing cookie at {}", path.to_string_lossy());
                    tx.send(Ok(notify::Event {
                        kind: notify::EventKind::Create(notify::event::CreateKind::File),
                        paths: vec![path],
                        ..Default::default()
                    })).unwrap();
                }
        };

        assert_eq!(
            data.unwrap().values().cloned().collect::<Vec<_>>(),
            vec![WorkspaceData {
                package_json: root.join_component("package.json"),
                turbo_json: None,
            },]
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

        let subscriber = Subscriber::new(root.clone(), rx, mock_discovery).unwrap();

        let mut package_data = subscriber.package_data();

        let _handle = tokio::spawn(subscriber.watch(exit_rx));

        let (data, _) = join! {
                package_data.get(),
                async {
                    // simulate fs round trip
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    let path = root.join_component("1.cookie").as_std_path().to_owned();
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

                    let path = root.join_component("2.cookie").as_std_path().to_owned();
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
}
