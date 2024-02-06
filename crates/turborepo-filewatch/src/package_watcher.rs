//! This module hosts the `PackageWatcher` type, which is used to watch the
//! filesystem for changes to packages.

use std::{
    collections::HashMap,
    future::IntoFuture,
    sync::{Arc, Mutex},
};

use notify::Event;
use tokio::{
    join,
    sync::{
        broadcast::{self, error::RecvError},
        oneshot, watch, Mutex as AsyncMutex,
    },
};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::{
    discovery::{PackageDiscovery, WorkspaceData},
    package_manager::{self, Error, PackageManager, WorkspaceGlobs},
};

use crate::NotifyError;

/// Watches the filesystem for changes to packages and package managers.
pub struct PackageWatcher {
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_tx: oneshot::Sender<()>,
    _handle: tokio::task::JoinHandle<()>,

    package_data: Arc<Mutex<Option<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>>>,
    manager_rx: watch::Receiver<Option<PackageManagerState>>,
}

impl PackageWatcher {
    /// Creates a new package watcher whose current package data can be queried.
    pub async fn new<T: PackageDiscovery + Send + 'static>(
        root: AbsoluteSystemPathBuf,
        recv: broadcast::Receiver<Result<Event, NotifyError>>,
        backup_discovery: T,
    ) -> Result<Self, package_manager::Error> {
        let (exit_tx, exit_rx) = oneshot::channel();
        let subscriber = Subscriber::new(exit_rx, root, recv, backup_discovery).await?;
        let manager_rx = subscriber.manager_receiver();
        let package_data = subscriber.package_data();
        let handle = tokio::spawn(subscriber.watch());
        Ok(Self {
            _exit_tx: exit_tx,
            _handle: handle,
            package_data,
            manager_rx,
        })
    }

    pub async fn get_package_data(&self) -> Option<Vec<WorkspaceData>> {
        self.package_data
            .lock()
            .expect("not poisoned")
            .as_ref()
            .map(|inner| inner.values().cloned().collect())
    }

    pub async fn get_package_manager(&self) -> Option<PackageManager> {
        self.manager_rx.borrow().as_ref().map(|s| s.manager)
    }
}

/// The underlying task that listens to file system events and updates the
/// internal package state.
struct Subscriber<T: PackageDiscovery> {
    exit_rx: oneshot::Receiver<()>,
    recv: broadcast::Receiver<Result<Event, NotifyError>>,
    backup_discovery: Arc<AsyncMutex<T>>,

    repo_root: AbsoluteSystemPathBuf,
    package_json_path: AbsoluteSystemPathBuf,

    // package manager data
    manager_tx: Arc<watch::Sender<Option<PackageManagerState>>>,
    manager_rx: watch::Receiver<Option<PackageManagerState>>,
    package_data: Arc<Mutex<Option<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>>>,
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

impl<T: PackageDiscovery + Send + 'static> Subscriber<T> {
    async fn new(
        exit_rx: oneshot::Receiver<()>,
        repo_root: AbsoluteSystemPathBuf,
        recv: broadcast::Receiver<Result<Event, NotifyError>>,
        backup_discovery: T,
    ) -> Result<Self, Error> {
        let package_data = Arc::new(Mutex::new(None));
        let (manager_tx, manager_rx) = watch::channel(None);
        let manager_tx = Arc::new(manager_tx);

        let backup_discovery = Arc::new(AsyncMutex::new(backup_discovery));

        let package_json_path = repo_root.join_component("package.json");

        let _task = tokio::spawn({
            let package_data = package_data.clone();
            let manager_tx = manager_tx.clone();
            let backup_discovery = backup_discovery.clone();
            let repo_root = repo_root.clone();
            let package_json_path = package_json_path.clone();
            async move {
                let initial_discovery = backup_discovery.lock().await.discover_packages().await;

                let Ok(initial_discovery) = initial_discovery else {
                    // if initial discovery fails, there is nothing we can do. we should just report
                    // that the package watcher is not available
                    // NOTE: in the future, if we decide to differentiate between 'not ready' and
                    // unavailable,       we MUST update the status here to
                    // unavailable or the client will hang
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

                manager_tx.send(Some(state)).ok();
                *package_data.lock().expect("not poisoned") = Some(
                    initial_discovery
                        .workspaces
                        .into_iter()
                        .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                        .collect(),
                );
            }
        });

        Ok(Self {
            exit_rx,
            recv,
            backup_discovery,
            repo_root,
            package_json_path,
            package_data,
            manager_rx,
            manager_tx,
        })
    }

    async fn watch(mut self) {
        // initialize the contents
        self.rediscover_packages().await;

        // respond to changes
        loop {
            tokio::select! {
                biased;
                _ = &mut self.exit_rx => {
                    tracing::info!("exiting package watcher");
                    return
                },
                file_event = self.recv.recv().into_future() => match file_event {
                    Ok(Ok(event)) => self.handle_file_event(event).await,
                    // if we get an error, we need to re-discover the packages
                    Ok(Err(_)) => self.rediscover_packages().await,
                    Err(RecvError::Closed) => return,
                    // if we end up lagging, warn and rediscover packages
                    Err(RecvError::Lagged(count)) => {
                        tracing::warn!("lagged behind {count} processing file watching events");
                        self.rediscover_packages().await;
                    },
                }
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

    pub fn manager_receiver(&self) -> watch::Receiver<Option<PackageManagerState>> {
        self.manager_rx.clone()
    }

    pub fn package_data(
        &self,
    ) -> Arc<Mutex<Option<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>>> {
        self.package_data.clone()
    }

    async fn handle_file_event(&mut self, file_event: Event) {
        tracing::trace!("file event: {:?}", file_event);

        if file_event
            .paths
            .iter()
            .any(|p| self.package_json_path.as_std_path().eq(p))
        {
            // if the package.json changed, we need to re-infer the package manager
            // and update the glob list
            tracing::debug!("package.json changed");

            let resp = match self.backup_discovery.lock().await.discover_packages().await {
                Ok(pm) => pm,
                Err(e) => {
                    tracing::error!("error discovering package manager: {}", e);
                    return;
                }
            };

            let new_manager = Self::update_package_manager(
                &resp.package_manager,
                &self.repo_root,
                &self.package_json_path,
            )
            .map(|(a, b)| (resp, a, b));

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

                    // if this fails, we are closing anyways so ignore
                    self.manager_tx.send(Some(state)).ok();
                    {
                        let mut data = self.package_data.lock().unwrap();
                        *data = Some(
                            new_manager
                                .workspaces
                                .into_iter()
                                .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                                .collect(),
                        );
                    }
                }
                Err(e) => {
                    // a change in the package json does not necessarily mean
                    // that the package manager has changed, so continue with
                    // best effort
                    tracing::error!("error getting package manager: {}", e);
                }
            }
        }

        // if it is the package manager, update the glob list
        let changed = {
            // here, we can only update if we have a valid package state
            let Ok(state) = self.manager_rx.wait_for(|v| v.is_some()).await else {
                // the channel is closed, so there is no state to write into, return
                return;
            };

            let state = state.as_ref().expect("validated above");

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

                let changed = state.filter != new_filter;
                if changed {
                    let mut state = state.to_owned();
                    state.filter = new_filter;
                    self.manager_tx.send(Some(state)).ok();
                }
                changed
            } else {
                false
            }
        };

        if changed {
            // if the glob list has changed, do a recursive walk and replace
            self.rediscover_packages().await;
        } else {
            // here, we can only update if we have a valid package state
            let state = {
                let Ok(state) = self.manager_rx.wait_for(|v| v.is_some()).await else {
                    // the channel is closed, so there is no state to write into, return
                    return;
                };

                state.to_owned().expect("validated above")
            };

            // if a path is not a valid utf8 string, it is not a valid path, so ignore
            for path in file_event
                .paths
                .iter()
                .filter_map(|p| p.as_os_str().to_str())
            {
                let path_file =
                    AbsoluteSystemPathBuf::new(path).expect("watched paths are absolute");

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

                    let mut data = self.package_data.lock().expect("not poisoned");
                    if let Some(data) = data.as_mut() {
                        if let Ok(true) = package_exists {
                            data.insert(
                                path_workspace,
                                WorkspaceData {
                                    package_json,
                                    turbo_json: turbo_exists
                                        .unwrap_or_default()
                                        .then_some(turbo_json),
                                },
                            );
                        } else {
                            data.remove(&path_workspace);
                        }
                    }
                }
            }
        }
    }

    async fn rediscover_packages(&mut self) {
        tracing::debug!("rediscovering packages");
        {
            // make sure package data is unavailable while we are updating
            let mut data = self.package_data.lock().expect("not poisoned");
            *data = None;
        }

        if let Ok(data) = self.backup_discovery.lock().await.discover_packages().await {
            let workspace = data
                .workspaces
                .into_iter()
                .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                .collect();
            let mut data = self.package_data.lock().expect("not poisoned");
            *data = Some(workspace);
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
    use tokio::sync::broadcast;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{
        discovery::{self, DiscoveryResponse, WorkspaceData},
        package_manager::PackageManager,
    };

    use super::Subscriber;

    #[derive(Debug)]
    struct MockDiscovery {
        pub manager: PackageManager,
        pub package_data: Arc<Mutex<Vec<WorkspaceData>>>,
    }

    impl super::PackageDiscovery for MockDiscovery {
        async fn discover_packages(&mut self) -> Result<DiscoveryResponse, discovery::Error> {
            Ok(DiscoveryResponse {
                package_manager: self.manager,
                workspaces: self.package_data.lock().unwrap().clone(),
            })
        }
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn subscriber_test() {
        let tmp = tempfile::tempdir().unwrap();

        let (tx, rx) = broadcast::channel(10);
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

        let subscriber = Subscriber::new(exit_rx, root.clone(), rx, mock_discovery)
            .await
            .unwrap();

        let package_data = subscriber.package_data();

        let _handle = tokio::spawn(subscriber.watch());

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Create(notify::event::CreateKind::File),
            paths: vec![root.join_component("package.json").as_std_path().to_owned()],
            ..Default::default()
        }))
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(
            package_data
                .lock()
                .unwrap()
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

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(
            package_data
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect::<Vec<_>>(),
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

        let subscriber = Subscriber::new(exit_rx, root.clone(), rx, mock_discovery)
            .await
            .unwrap();

        let package_data = subscriber.package_data();

        let _handle = tokio::spawn(subscriber.watch());

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(
            package_data
                .lock()
                .unwrap()
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

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(
            package_data
                .lock()
                .unwrap()
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
