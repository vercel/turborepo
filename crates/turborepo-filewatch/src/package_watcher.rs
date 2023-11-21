//! This module hosts the `PackageWatcher` type, which is used to watch the
//! filesystem for changes to packages.

use std::{
    collections::HashMap,
    future::IntoFuture,
    path::PathBuf,
    sync::{Arc, Mutex},
};

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

    package_data: Arc<Mutex<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>>,
    manager_rx: watch::Receiver<PackageManager>,
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

    pub async fn get_package_data(&self) -> Vec<WorkspaceData> {
        self.package_data
            .lock()
            .expect("not poisoned")
            .clone()
            .into_values()
            .collect()
    }

    pub async fn get_package_manager(&self) -> PackageManager {
        *self.manager_rx.borrow()
    }
}

/// The underlying task that listens to file system events and updates the
/// internal package state.
struct Subscriber<T: PackageDiscovery> {
    exit_rx: oneshot::Receiver<()>,
    filter: WorkspaceGlobs,
    recv: broadcast::Receiver<Result<Event, NotifyError>>,
    repo_root: AbsoluteSystemPathBuf,
    backup_discovery: T,

    // package manager data
    package_data: Arc<Mutex<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>>,
    manager_rx: watch::Receiver<PackageManager>,
    manager_tx: watch::Sender<PackageManager>,

    // stored as PathBuf to avoid processing later.
    // if package_json changes, we need to re-infer
    // the package manager
    package_json_path: std::path::PathBuf,
    workspace_config_path: std::path::PathBuf,
}

impl<T: PackageDiscovery + Send + 'static> Subscriber<T> {
    async fn new(
        exit_rx: oneshot::Receiver<()>,
        repo_root: AbsoluteSystemPathBuf,
        recv: broadcast::Receiver<Result<Event, NotifyError>>,
        mut discovery: T,
    ) -> Result<Self, Error> {
        let manager = discovery.discover_packages().await.unwrap().package_manager;

        let (package_json_path, workspace_config_path, filter) =
            Self::update_package_manager(&manager, &repo_root)?;

        let (manager_tx, manager_rx) = watch::channel(manager);

        Ok(Self {
            exit_rx,
            filter,
            package_data: Default::default(),
            recv,
            manager_rx,
            manager_tx,
            repo_root,
            backup_discovery: discovery,

            package_json_path,
            workspace_config_path,
        })
    }

    async fn watch(mut self) {
        // initialize the contents
        self.rediscover_packages().await;

        // respond to changes
        loop {
            tokio::select! {
                _ = &mut self.exit_rx => return,
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
    ) -> Result<(PathBuf, PathBuf, WorkspaceGlobs), Error> {
        let package_json_path = repo_root
            .join_component("package.json")
            .as_std_path()
            .to_owned();

        let workspace_config_path = manager
            .workspace_configuration_path()
            .map_or(package_json_path.clone(), |p| {
                repo_root.join_component(p).as_std_path().to_owned()
            });
        let filter = manager.get_workspace_globs(repo_root)?;

        Ok((package_json_path, workspace_config_path, filter))
    }

    pub fn manager_receiver(&self) -> watch::Receiver<PackageManager> {
        self.manager_rx.clone()
    }

    pub fn package_data(&self) -> Arc<Mutex<HashMap<AbsoluteSystemPathBuf, WorkspaceData>>> {
        self.package_data.clone()
    }

    fn manager(&self) -> PackageManager {
        *self.manager_rx.borrow()
    }

    async fn handle_file_event(&mut self, file_event: Event) {
        tracing::trace!("file event: {:?}", file_event);

        if file_event
            .paths
            .iter()
            .any(|p| self.package_json_path.eq(p))
        {
            tracing::debug!("package.json changed");
            // if the package.json changed, we need to re-infer the package manager
            // and update the glob list
            let new_manager =
                PackageManager::get_package_manager(&self.repo_root, None).and_then(|manager| {
                    Self::update_package_manager(&manager, &self.repo_root)
                        .map(|(a, b, c)| (manager, a, b, c))
                });

            match new_manager {
                Ok((new_manager, package_json_path, workspace_config_path, filter)) => {
                    // if this fails, we are closing anyways so ignore
                    self.manager_tx.send(new_manager).ok();
                    self.package_json_path = package_json_path;
                    self.workspace_config_path = workspace_config_path;
                    self.filter = filter;
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
        let changed = if file_event
            .paths
            .iter()
            .any(|p| self.workspace_config_path.eq(p))
        {
            let new_filter = self
                .manager()
                .get_workspace_globs(&self.repo_root)
                // under some saving strategies a file can be totally empty for a moment
                // during a save. these strategies emit multiple events and so we can
                // a previous or subsequent event in the 'cluster' will still trigger
                .unwrap_or_else(|_| self.filter.clone());

            let changed = self.filter != new_filter;
            self.filter = new_filter;
            changed
        } else {
            false
        };

        if changed {
            // if the glob list has changed, do a recursive walk and replace
            self.rediscover_packages().await;
        } else {
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

                let is_workspace = match self
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
                    if let Ok(true) = package_exists {
                        data.insert(
                            path_workspace,
                            WorkspaceData {
                                package_json,
                                turbo_json: turbo_exists.unwrap_or_default().then_some(turbo_json),
                            },
                        );
                    } else {
                        data.remove(&path_workspace);
                    }
                }
            }
        }
    }

    async fn rediscover_packages(&mut self) {
        tracing::debug!("rediscovering packages");
        if let Ok(data) = self.backup_discovery.discover_packages().await {
            let workspace = data
                .workspaces
                .into_iter()
                .map(|p| (p.package_json.parent().expect("non-root").to_owned(), p))
                .collect();
            let mut data = self.package_data.lock().expect("not poisoned");
            *data = workspace;
        } else {
            tracing::error!("error discovering packages");
        }
    }
}

#[cfg(test)]
mod test {}
