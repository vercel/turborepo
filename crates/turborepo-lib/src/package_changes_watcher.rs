use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::DerefMut,
    sync::Arc,
};

use ignore::gitignore::Gitignore;
use notify::Event;
use radix_trie::{Trie, TrieCommon};
use tokio::sync::{broadcast, oneshot, Mutex};
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf};
use turborepo_filewatch::{
    hash_watcher::{HashSpec, HashWatcher, InputGlobs},
    NotifyError, OptionalWatch,
};
use turborepo_repository::{
    change_mapper::{ChangeMapper, GlobalDepsPackageChangeMapper, PackageChanges},
    package_graph::{PackageGraph, PackageGraphBuilder, PackageName, WorkspacePackage},
    package_json::PackageJson,
};
use turborepo_scm::package_deps::GitHashes;

use crate::turbo_json::TurboJson;

#[derive(Clone)]
pub enum PackageChangeEvent {
    Package { name: PackageName },
    Rediscover,
}

/// Watches for changes to a package's files and directories.
pub struct PackageChangesWatcher {
    _exit_tx: oneshot::Sender<()>,
    _handle: tokio::task::JoinHandle<()>,
    package_change_events_rx: broadcast::Receiver<PackageChangeEvent>,
}

/// The number of events that can be buffered in the channel.
/// A little arbitrary, so feel free to tune accordingly.
const CHANGE_EVENT_CHANNEL_CAPACITY: usize = 50;

impl PackageChangesWatcher {
    pub fn new(
        repo_root: AbsoluteSystemPathBuf,
        file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
        hash_watcher: Arc<HashWatcher>,
    ) -> Self {
        let (exit_tx, exit_rx) = oneshot::channel();
        let (package_change_events_tx, package_change_events_rx) =
            broadcast::channel(CHANGE_EVENT_CHANNEL_CAPACITY);
        let subscriber = Subscriber::new(
            repo_root,
            file_events_lazy,
            package_change_events_tx,
            hash_watcher,
        );

        let _handle = tokio::spawn(subscriber.watch(exit_rx));
        Self {
            _exit_tx: exit_tx,
            _handle,
            package_change_events_rx,
        }
    }

    pub async fn package_changes(&self) -> broadcast::Receiver<PackageChangeEvent> {
        self.package_change_events_rx.resubscribe()
    }
}

enum ChangedFiles {
    All,
    // Trie doesn't support PathBuf as a key on Windows, so we need to use `String`
    Some(Trie<String, ()>),
}

impl ChangedFiles {
    fn is_empty(&self) -> bool {
        match self {
            ChangedFiles::All => false,
            ChangedFiles::Some(trie) => trie.is_empty(),
        }
    }
}

impl Default for ChangedFiles {
    fn default() -> Self {
        ChangedFiles::Some(Trie::new())
    }
}

struct Subscriber {
    file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    changed_files: Mutex<RefCell<ChangedFiles>>,
    repo_root: AbsoluteSystemPathBuf,
    package_change_events_tx: broadcast::Sender<PackageChangeEvent>,
    hash_watcher: Arc<HashWatcher>,
}

// This is a workaround because `ignore` doesn't match against a path's
// ancestors, i.e. if we have `foo/bar/baz` and the .gitignore has `foo/`, it
// won't match.
fn ancestors_is_ignored(gitignore: &Gitignore, path: &AnchoredSystemPath) -> bool {
    path.ancestors().enumerate().any(|(idx, p)| {
        let is_dir = idx != 0;
        gitignore.matched(p, is_dir).is_ignore()
    })
}

fn is_in_git_folder(path: &AnchoredSystemPath) -> bool {
    path.components().any(|c| c.as_str() == ".git")
}

struct RepoState {
    root_turbo_json: Option<TurboJson>,
    pkg_dep_graph: PackageGraph,
}

impl RepoState {
    fn get_change_mapper(&self) -> Option<ChangeMapper<GlobalDepsPackageChangeMapper>> {
        let Ok(package_change_mapper) = GlobalDepsPackageChangeMapper::new(
            &self.pkg_dep_graph,
            self.root_turbo_json
                .iter()
                .flat_map(|turbo| turbo.global_deps.iter())
                .map(|s| s.as_str()),
        ) else {
            tracing::debug!("package change mapper not available, package watcher not available");
            return None;
        };
        // TODO: Pass in global_deps and ignore_patterns
        Some(ChangeMapper::new(
            &self.pkg_dep_graph,
            vec![],
            package_change_mapper,
        ))
    }
}

impl Subscriber {
    fn new(
        repo_root: AbsoluteSystemPathBuf,
        file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
        package_change_events_tx: broadcast::Sender<PackageChangeEvent>,
        hash_watcher: Arc<HashWatcher>,
    ) -> Self {
        Subscriber {
            repo_root,
            file_events_lazy,
            changed_files: Default::default(),
            package_change_events_tx,
            hash_watcher,
        }
    }

    async fn initialize_repo_state(&self) -> Option<(RepoState, Gitignore)> {
        let Ok(root_package_json) =
            PackageJson::load(&self.repo_root.join_component("package.json"))
        else {
            tracing::debug!("no package.json found, package watcher not available");
            return None;
        };

        let root_turbo_json = TurboJson::load(
            &self.repo_root,
            &AnchoredSystemPathBuf::default(),
            &root_package_json,
            false,
        )
        .ok();

        let gitignore_path = self.repo_root.join_component(".gitignore");
        let (root_gitignore, _) = Gitignore::new(&gitignore_path);

        let Ok(pkg_dep_graph) = PackageGraphBuilder::new(&self.repo_root, root_package_json)
            .build()
            .await
        else {
            tracing::debug!("package graph not available, package watcher not available");
            return None;
        };

        Some((
            RepoState {
                root_turbo_json,
                pkg_dep_graph,
            },
            root_gitignore,
        ))
    }

    async fn is_same_hash(
        &self,
        pkg: &WorkspacePackage,
        package_file_hashes: &mut HashMap<AnchoredSystemPathBuf, GitHashes>,
    ) -> bool {
        let Ok(hash) = self
            .hash_watcher
            .get_file_hashes(HashSpec {
                package_path: pkg.path.clone(),
                // TODO: Support inputs
                inputs: InputGlobs::Default,
            })
            .await
        else {
            return false;
        };

        let old_hash = package_file_hashes.get(&pkg.path).cloned();

        if Some(&hash) != old_hash.as_ref() {
            package_file_hashes.insert(pkg.path.clone(), hash);
            return false;
        }

        tracing::warn!("hashes are the same, no need to rerun");

        true
    }

    async fn watch(mut self, exit_rx: oneshot::Receiver<()>) {
        let Ok(mut file_events) = self.file_events_lazy.get().await.map(|r| r.resubscribe()) else {
            // if we get here, it means that file watching has not started, so we should
            // just report that the package watcher is not available
            tracing::debug!("file watching shut down, package watcher not available");
            return;
        };

        // This just processes the events and puts the changed files into a `Trie`.
        // Must be fast to avoid lagging the file events channel.
        let event_fut = async {
            loop {
                match file_events.recv().await {
                    Ok(Ok(Event { paths, .. })) => {
                        if let ChangedFiles::Some(trie) =
                            self.changed_files.lock().await.borrow_mut().deref_mut()
                        {
                            for path in paths {
                                if let Some(path) = path.to_str() {
                                    trie.insert(path.to_string(), ());
                                }
                            }
                        }
                    }
                    Ok(Err(err)) => {
                        tracing::error!("file event error: {:?}", err);
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        tracing::warn!("file event lagged");
                        // Lagged essentially means we're not keeping up with
                        // the file events, so
                        // we can catch up by declaring all files changed
                        *self.changed_files.lock().await.borrow_mut() = ChangedFiles::All;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!("file event channel closed");
                        break;
                    }
                }
            }
        };

        let changes_fut = async {
            let root_pkg = WorkspacePackage::root();

            let Some((mut repo_state, mut root_gitignore)) = self.initialize_repo_state().await
            else {
                return;
            };
            // We store the hash of the package's files. If the hash is already
            // in here, we don't need to recompute it
            let mut package_file_hashes = HashMap::new();

            let mut change_mapper = match repo_state.get_change_mapper() {
                Some(change_mapper) => change_mapper,
                None => {
                    return;
                }
            };

            self.package_change_events_tx
                .send(PackageChangeEvent::Rediscover)
                .ok();
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));

            loop {
                interval.tick().await;
                let changed_files = {
                    let changed_files = self.changed_files.lock().await;
                    if changed_files.borrow().is_empty() {
                        continue;
                    }

                    changed_files.take()
                };

                let ChangedFiles::Some(trie) = changed_files else {
                    let _ = self
                        .package_change_events_tx
                        .send(PackageChangeEvent::Rediscover);

                    match self.initialize_repo_state().await {
                        Some((new_repo_state, new_gitignore)) => {
                            repo_state = new_repo_state;
                            root_gitignore = new_gitignore;
                            change_mapper = match repo_state.get_change_mapper() {
                                Some(change_mapper) => change_mapper,
                                None => {
                                    break;
                                }
                            };
                        }
                        None => {
                            break;
                        }
                    }
                    continue;
                };

                let gitignore_path = self.repo_root.join_component(".gitignore");
                if trie.get(gitignore_path.as_str()).is_some() {
                    let (new_root_gitignore, _) = Gitignore::new(&gitignore_path);
                    root_gitignore = new_root_gitignore;
                }

                let changed_files: HashSet<_> = trie
                    .keys()
                    .filter_map(|p| {
                        let p = AbsoluteSystemPathBuf::new(p)
                            .expect("file watching should return absolute paths");
                        self.repo_root.anchor(p).ok()
                    })
                    .filter(|p| {
                        // If in .gitignore or in .git, filter out
                        !(ancestors_is_ignored(&root_gitignore, p) || is_in_git_folder(p))
                    })
                    .collect();

                if changed_files.is_empty() {
                    continue;
                }

                let changed_packages = change_mapper.changed_packages(changed_files.clone(), None);

                tracing::warn!("changed_files: {:?}", changed_files);
                tracing::warn!("changed_packages: {:?}", changed_packages);

                match changed_packages {
                    Ok(PackageChanges::All(_)) => {
                        // We tell the client that we need to rediscover the packages, i.e.
                        // all bets are off, just re-run everything
                        let _ = self
                            .package_change_events_tx
                            .send(PackageChangeEvent::Rediscover);
                        match self.initialize_repo_state().await {
                            Some((new_repo_state, new_gitignore)) => {
                                repo_state = new_repo_state;
                                root_gitignore = new_gitignore;
                                change_mapper = match repo_state.get_change_mapper() {
                                    Some(change_mapper) => change_mapper,
                                    None => {
                                        break;
                                    }
                                };
                            }
                            None => {
                                break;
                            }
                        }
                    }
                    Ok(PackageChanges::Some(mut filtered_pkgs)) => {
                        // If the root package has changed, we only send it if we have root
                        // tasks. Otherwise it's not worth sending as it will only
                        // pollute up the output logs
                        if filtered_pkgs.contains(&root_pkg) {
                            let has_root_tasks = repo_state
                                .root_turbo_json
                                .as_ref()
                                .map_or(false, |turbo| turbo.has_root_tasks());
                            if !has_root_tasks {
                                filtered_pkgs.remove(&root_pkg);
                            }
                        }

                        for pkg in filtered_pkgs {
                            if !self.is_same_hash(&pkg, &mut package_file_hashes).await {
                                let _ = self.package_change_events_tx.send(
                                    PackageChangeEvent::Package {
                                        name: pkg.name.clone(),
                                    },
                                );
                            }
                        }
                    }
                    Err(err) => {
                        // Log the error, rediscover the packages and try again
                        tracing::error!("error: {:?}", err);

                        let _ = self
                            .package_change_events_tx
                            .send(PackageChangeEvent::Rediscover);
                        match self.initialize_repo_state().await {
                            Some((new_repo_state, new_gitignore)) => {
                                repo_state = new_repo_state;
                                root_gitignore = new_gitignore;
                                change_mapper = match repo_state.get_change_mapper() {
                                    Some(change_mapper) => change_mapper,
                                    None => {
                                        break;
                                    }
                                }
                            }
                            None => {
                                break;
                            }
                        }
                    }
                }
            }
        };

        tokio::select! {
            biased;
            _ = exit_rx => {
                tracing::debug!("exiting package changes watcher due to signal");
            },
            _ = event_fut => {
                tracing::debug!("exiting package changes watcher due to file event end");
            },
            _ = changes_fut => {
                tracing::debug!("exiting package changes watcher due to process end");
            }
        }
    }
}
