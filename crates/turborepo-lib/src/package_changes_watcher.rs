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
// Re-export the PackageChangeEvent from turborepo-daemon
pub use turborepo_daemon::PackageChangeEvent;
use turborepo_daemon::PackageChangesWatcher as PackageChangesWatcherTrait;
use turborepo_filewatch::{
    hash_watcher::{HashSpec, HashWatcher, InputGlobs},
    NotifyError, OptionalWatch,
};
use turborepo_repository::{
    change_mapper::{
        ChangeMapper, GlobalDepsPackageChangeMapper, LockfileContents, PackageChanges,
    },
    package_graph::{PackageGraph, PackageGraphBuilder, PackageName, WorkspacePackage},
    package_json::PackageJson,
};
use turborepo_scm::GitHashes;

use crate::{
    config::{resolve_turbo_config_path, CONFIG_FILE, CONFIG_FILE_JSONC},
    turbo_json::{TurboJson, TurboJsonReader, UnifiedTurboJsonLoader},
};

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
        custom_turbo_json_path: Option<AbsoluteSystemPathBuf>,
        single_package: bool,
    ) -> Self {
        let (exit_tx, exit_rx) = oneshot::channel();
        let (package_change_events_tx, package_change_events_rx) =
            broadcast::channel(CHANGE_EVENT_CHANNEL_CAPACITY);
        let subscriber = Subscriber::new(
            repo_root,
            file_events_lazy,
            package_change_events_tx,
            hash_watcher,
            custom_turbo_json_path,
            single_package,
        );

        let _handle = tokio::spawn(subscriber.watch(exit_rx));
        Self {
            _exit_tx: exit_tx,
            _handle,
            package_change_events_rx,
        }
    }
}

impl PackageChangesWatcherTrait for PackageChangesWatcher {
    async fn package_changes(&self) -> broadcast::Receiver<PackageChangeEvent> {
        self.package_change_events_rx.resubscribe()
    }
}

enum ChangedFiles {
    All,
    // Trie doesn't support PathBuf as a key on Windows, so we need to use `String`
    Some(Box<Trie<String, ()>>),
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
        ChangedFiles::Some(Box::new(Trie::new()))
    }
}

struct Subscriber {
    file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    changed_files: Mutex<RefCell<ChangedFiles>>,
    repo_root: AbsoluteSystemPathBuf,
    package_change_events_tx: broadcast::Sender<PackageChangeEvent>,
    hash_watcher: Arc<HashWatcher>,
    custom_turbo_json_path: Option<AbsoluteSystemPathBuf>,
    single_package: bool,
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

/// The result of classifying a batch of changed files.
/// Separates the "what happened" decision from the async side effects.
///
/// Note: `.gitignore` changes are handled by the caller *before* calling
/// `classify_changed_files`, so they don't appear as a variant here.
/// This matches the original behavior where gitignore reloads were a
/// side-effect that didn't short-circuit processing of other files in
/// the same batch.
#[derive(Debug)]
enum FileChangeAction {
    /// A turbo config file changed, triggering full rediscovery
    ConfigChanged,
    /// The change mapper failed to classify files. The caller should
    /// trigger rediscovery as a safe fallback. Kept separate from
    /// `ConfigChanged` so callers can log at the appropriate severity.
    MapperFailed,
    /// Files changed that map to specific packages (after gitignore
    /// and .git filtering, before watched-package filtering).
    /// The second field carries the filtered file set for task-level
    /// input matching in watch mode.
    PackagesChanged(PackageChanges, HashSet<AnchoredSystemPathBuf>),
    /// All changed files were filtered out (gitignored, .git, etc.)
    NoRelevantChanges,
}

/// Classify a batch of changed files into an action. This is the synchronous,
/// side-effect-free core of the Subscriber's polling loop. It does not perform
/// any I/O or send any events -- callers act on the returned action.
///
/// `.gitignore` changes are NOT handled here. The caller must check for
/// and reload the gitignore *before* calling this function, so that
/// co-occurring file changes in the same batch are still processed.
fn classify_changed_files(
    trie: &Trie<String, ()>,
    repo_root: &AbsoluteSystemPathBuf,
    root_gitignore: &Gitignore,
    custom_turbo_json_path: Option<&AbsoluteSystemPathBuf>,
    change_mapper: &ChangeMapper<'_, GlobalDepsPackageChangeMapper<'_>>,
) -> FileChangeAction {
    let turbo_json_path = repo_root.join_component(CONFIG_FILE);
    let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);

    let standard_config_changed = trie.get(turbo_json_path.as_str()).is_some()
        || trie.get(turbo_jsonc_path.as_str()).is_some();

    let custom_config_changed = custom_turbo_json_path
        .map(|path| trie.get(path.as_str()).is_some())
        .unwrap_or(false);

    if standard_config_changed || custom_config_changed {
        return FileChangeAction::ConfigChanged;
    }

    let changed_files: HashSet<_> = trie
        .keys()
        .filter_map(|p| {
            let p = match AbsoluteSystemPathBuf::new(p) {
                Ok(p) => p,
                Err(_) => {
                    tracing::warn!(%p, "skipping non-absolute path from file watcher");
                    return None;
                }
            };
            repo_root.anchor(p).ok()
        })
        .filter(|p| !(ancestors_is_ignored(root_gitignore, p) || is_in_git_folder(p)))
        .collect();

    if changed_files.is_empty() {
        return FileChangeAction::NoRelevantChanges;
    }

    match change_mapper.changed_packages(changed_files.clone(), LockfileContents::Unchanged) {
        Ok(changes) => FileChangeAction::PackagesChanged(changes, changed_files),
        Err(err) => {
            tracing::warn!(
                ?err,
                "change mapper failed, triggering rediscovery as fallback"
            );
            FileChangeAction::MapperFailed
        }
    }
}

impl RepoState {
    fn get_change_mapper(&self) -> Option<ChangeMapper<'_, GlobalDepsPackageChangeMapper<'_>>> {
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
        custom_turbo_json_path: Option<AbsoluteSystemPathBuf>,
        single_package: bool,
    ) -> Self {
        // Try to canonicalize the custom path to match what the file watcher reports
        let normalized_custom_path = custom_turbo_json_path.map(|path| {
            // Check if the custom turbo.json path is outside the repository
            if repo_root.anchor(&path).is_err() {
                tracing::warn!(
                    "turbo.json is located outside of repository at {}. Changes to this file will \
                     not be watched.",
                    path
                );
            }

            match path.to_realpath() {
                Ok(real_path) => {
                    tracing::info!(
                        "PackageChangesWatcher: monitoring custom turbo.json at: {} \
                         (canonicalized: {})",
                        path,
                        real_path
                    );
                    real_path
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to canonicalize custom turbo.json path {}: {}, using original path",
                        path,
                        e
                    );
                    path
                }
            }
        });

        Subscriber {
            repo_root,
            file_events_lazy,
            changed_files: Default::default(),
            package_change_events_tx,
            hash_watcher,
            custom_turbo_json_path: normalized_custom_path,
            single_package,
        }
    }

    async fn initialize_repo_state(&self) -> Option<(RepoState, Gitignore)> {
        let Ok(root_package_json) =
            PackageJson::load(&self.repo_root.join_component("package.json"))
        else {
            tracing::debug!("no package.json found, package watcher not available");
            return None;
        };
        let Ok(pkg_dep_graph) =
            PackageGraphBuilder::new(&self.repo_root, root_package_json.clone())
                .with_single_package_mode(self.single_package)
                .build()
                .await
        else {
            tracing::debug!("package graph not available, package watcher not available");
            return None;
        };

        // Use custom turbo.json path if provided, otherwise use standard paths
        let config_path = if let Some(custom_path) = &self.custom_turbo_json_path {
            custom_path.clone()
        } else {
            match resolve_turbo_config_path(&self.repo_root) {
                Ok(path) => path,
                Err(_) => {
                    // TODO: If both turbo.json and turbo.jsonc exist, log warning and default to
                    // turbo.json to preserve existing behavior for file
                    // watching prior to refactoring.
                    tracing::warn!(
                        "Found both turbo.json and turbo.jsonc in {}. Using turbo.json for \
                         watching.",
                        self.repo_root
                    );
                    self.repo_root.join_component(CONFIG_FILE)
                }
            }
        };

        let reader = TurboJsonReader::new(self.repo_root.clone());
        let root_turbo_json = if self.single_package {
            UnifiedTurboJsonLoader::single_package(reader, config_path, root_package_json)
        } else {
            UnifiedTurboJsonLoader::workspace(reader, config_path, pkg_dep_graph.packages())
        }
        .load(&PackageName::Root)
        .ok()
        .cloned();

        let gitignore_path = self.repo_root.join_component(".gitignore");
        let (root_gitignore, _) = Gitignore::new(&gitignore_path);

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

        tracing::debug!("hashes are the same, no need to rerun");

        true
    }

    /// Send a Rediscover event and reinitialize repo state. Returns the new
    /// state tuple on success, or `None` if the caller should break.
    async fn rediscover_and_reinit(&self) -> Option<(RepoState, Gitignore)> {
        let _ = self
            .package_change_events_tx
            .send(PackageChangeEvent::Rediscover);
        self.initialize_repo_state().await
    }

    async fn watch(mut self, exit_rx: oneshot::Receiver<()>) {
        let file_events_result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.file_events_lazy.get(),
        )
        .await;
        let Ok(mut file_events) = file_events_result
            .map_err(|_elapsed| {
                tracing::warn!(
                    "timed out waiting for file watching to become ready after 5s. This usually \
                     means the daemon's file watcher failed to initialize. Try running `turbo \
                     daemon clean` and retrying."
                );
            })
            .and_then(|r| {
                r.map(|r| r.resubscribe()).map_err(|_| {
                    tracing::debug!("file watching shut down, package watcher not available");
                })
            })
        else {
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
                                } else {
                                    tracing::debug!(
                                        ?path,
                                        "skipping non-UTF-8 path from file watcher"
                                    );
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
            // Pre-populate hash baselines for all known packages. Without
            // this, the first file change for each package would always be
            // treated as "new" (no old hash to compare against), causing
            // spurious rebuilds from build output writes on the initial run.
            let mut package_file_hashes = HashMap::new();
            for (name, info) in repo_state.pkg_dep_graph.packages() {
                let pkg = WorkspacePackage {
                    name: name.clone(),
                    path: info.package_path().to_owned(),
                };
                if let Ok(hash) = self
                    .hash_watcher
                    .get_file_hashes(HashSpec {
                        package_path: pkg.path.clone(),
                        inputs: InputGlobs::Default,
                    })
                    .await
                {
                    package_file_hashes.insert(pkg.path, hash);
                }
            }

            let mut change_mapper = match repo_state.get_change_mapper() {
                Some(change_mapper) => change_mapper,
                None => {
                    return;
                }
            };

            // Macro to avoid repeating the rediscover+reinit+rebuild-mapper
            // pattern. The borrow checker prevents extracting this into a method
            // because `change_mapper` borrows from `repo_state`.
            macro_rules! rediscover {
                ($self:expr, $repo_state:ident, $root_gitignore:ident, $change_mapper:ident) => {{
                    let Some((new_state, new_gitignore)) = $self.rediscover_and_reinit().await
                    else {
                        break;
                    };
                    $repo_state = new_state;
                    $root_gitignore = new_gitignore;
                    $change_mapper = match $repo_state.get_change_mapper() {
                        Some(m) => m,
                        None => break,
                    };
                }};
            }

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
                    rediscover!(self, repo_state, root_gitignore, change_mapper);
                    continue;
                };

                // Handle .gitignore changes before classification so that
                // co-occurring file changes in the same batch are still processed
                // with the updated gitignore rules.
                let gitignore_path = self.repo_root.join_component(".gitignore");
                if trie.get(gitignore_path.as_str()).is_some() {
                    let (new_root_gitignore, _) = Gitignore::new(&gitignore_path);
                    root_gitignore = new_root_gitignore;
                }

                let action = classify_changed_files(
                    &trie,
                    &self.repo_root,
                    &root_gitignore,
                    self.custom_turbo_json_path.as_ref(),
                    &change_mapper,
                );
                tracing::debug!(?action, "classified file changes");

                match action {
                    FileChangeAction::ConfigChanged => {
                        tracing::info!(
                            "Detected change to turbo configuration file. Triggering rediscovery."
                        );
                        rediscover!(self, repo_state, root_gitignore, change_mapper);
                        continue;
                    }
                    FileChangeAction::MapperFailed => {
                        tracing::info!("Change mapper failed. Triggering rediscovery.");
                        rediscover!(self, repo_state, root_gitignore, change_mapper);
                        continue;
                    }
                    FileChangeAction::NoRelevantChanges => {
                        continue;
                    }
                    FileChangeAction::PackagesChanged(PackageChanges::All(_), _) => {
                        rediscover!(self, repo_state, root_gitignore, change_mapper);
                    }
                    FileChangeAction::PackagesChanged(
                        PackageChanges::Some(filtered_pkgs),
                        changed_files,
                    ) => {
                        let mut filtered_pkgs: HashSet<WorkspacePackage> =
                            filtered_pkgs.into_keys().collect();
                        // Only propagate root package changes when the config defines
                        // root tasks; otherwise the event just creates output noise.
                        // In single-package mode the root IS the only package, so
                        // all changes must propagate regardless.
                        if !self.single_package && filtered_pkgs.contains(&root_pkg) {
                            let has_root_tasks = repo_state
                                .root_turbo_json
                                .as_ref()
                                .is_some_and(|turbo| turbo.has_root_tasks());
                            if !has_root_tasks {
                                filtered_pkgs.remove(&root_pkg);
                            }
                        }

                        let changed_files = Arc::new(changed_files);
                        for pkg in filtered_pkgs {
                            if !self.is_same_hash(&pkg, &mut package_file_hashes).await {
                                let _ = self.package_change_events_tx.send(
                                    PackageChangeEvent::Package {
                                        name: pkg.name.clone(),
                                        changed_files: changed_files.clone(),
                                    },
                                );
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

#[cfg(test)]
mod test {
    use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Duration};

    use ignore::gitignore::GitignoreBuilder;
    use notify::event::{CreateKind, EventKind};
    use radix_trie::Trie;
    use tokio::sync::{broadcast, watch};
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    use turborepo_filewatch::{hash_watcher::HashWatcher, NotifyError, OptionalWatch};
    use turborepo_repository::{
        change_mapper::{ChangeMapper, GlobalDepsPackageChangeMapper, PackageChanges},
        package_graph::{PackageGraph, PackageGraphBuilder},
        package_json::PackageJson,
    };
    use turborepo_scm::SCM;

    use super::{
        ancestors_is_ignored, classify_changed_files, is_in_git_folder, ChangedFiles,
        FileChangeAction, PackageChangeEvent, PackageChangesWatcher, CONFIG_FILE,
    };

    fn anchored(s: &str) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf::try_from(s).unwrap()
    }

    fn gitignore_from_lines(root: &str, lines: &[&str]) -> ignore::gitignore::Gitignore {
        let mut builder = GitignoreBuilder::new(root);
        for line in lines {
            builder.add_line(None, line).unwrap();
        }
        builder.build().unwrap()
    }

    /// Build a minimal PackageGraph for testing classify_changed_files.
    /// The graph has a root package and one workspace package at `packages/a`.
    async fn build_test_graph(repo_root: &AbsoluteSystemPathBuf) -> PackageGraph {
        // Write root package.json with npm as package manager
        let root_pkg_json = repo_root.join_component("package.json");
        root_pkg_json
            .create_with_contents(
                r#"{"name":"root","packageManager":"npm@10.0.0","workspaces":["packages/*"]}"#
                    .as_bytes(),
            )
            .unwrap();

        // Write a package-lock.json so npm is detected
        let lockfile = repo_root.join_component("package-lock.json");
        lockfile
            .create_with_contents(r#"{"lockfileVersion":3}"#.as_bytes())
            .unwrap();

        // Write package a
        let pkg_a_dir = repo_root.join_components(&["packages", "a"]);
        pkg_a_dir.create_dir_all().unwrap();
        let pkg_a_json = pkg_a_dir.join_component("package.json");
        pkg_a_json
            .create_with_contents(r#"{"name":"a","scripts":{"build":"echo built"}}"#.as_bytes())
            .unwrap();

        let root_package_json = PackageJson::load(&root_pkg_json).unwrap();
        PackageGraphBuilder::new(repo_root, root_package_json)
            .build()
            .await
            .unwrap()
    }

    /// Reusable fixture for `classify_changed_files` tests. Holds a tempdir,
    /// repo root, and package graph so each test only needs to build a trie
    /// and call `classify`.
    struct ClassifyFixture {
        _tmp: tempfile::TempDir,
        repo_root: AbsoluteSystemPathBuf,
        pkg_graph: PackageGraph,
    }

    impl ClassifyFixture {
        async fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let repo_root =
                AbsoluteSystemPathBuf::new(tmp.path().to_str().unwrap().to_string()).unwrap();
            let repo_root = repo_root.to_realpath().unwrap();
            let pkg_graph = build_test_graph(&repo_root).await;
            Self {
                _tmp: tmp,
                repo_root,
                pkg_graph,
            }
        }

        fn classify(
            &self,
            trie: &Trie<String, ()>,
            gitignore_lines: &[&str],
            custom_turbo_json: Option<&AbsoluteSystemPathBuf>,
        ) -> FileChangeAction {
            let mapper =
                GlobalDepsPackageChangeMapper::new(&self.pkg_graph, std::iter::empty::<&str>())
                    .unwrap();
            let change_mapper = ChangeMapper::new(&self.pkg_graph, vec![], mapper);
            let gitignore = gitignore_from_lines(self.repo_root.as_str(), gitignore_lines);
            classify_changed_files(
                trie,
                &self.repo_root,
                &gitignore,
                custom_turbo_json,
                &change_mapper,
            )
        }
    }

    #[test]
    fn changed_files_default_is_empty() {
        let cf = ChangedFiles::default();
        assert!(cf.is_empty());
    }

    #[test]
    fn changed_files_all_is_not_empty() {
        assert!(!ChangedFiles::All.is_empty());
    }

    #[test]
    fn changed_files_trie_with_entry_is_not_empty() {
        let mut trie = Trie::new();
        trie.insert("/repo/packages/a/src/index.ts".to_string(), ());
        let cf = ChangedFiles::Some(Box::new(trie));
        assert!(!cf.is_empty());
    }

    #[test]
    fn is_in_git_folder_detects_git_paths() {
        assert!(is_in_git_folder(&anchored(".git/objects/abc")));
        assert!(is_in_git_folder(&anchored(".git/HEAD")));
        assert!(is_in_git_folder(&anchored(".git/refs/heads/main")));
        assert!(is_in_git_folder(&anchored("foo/.git/config")));
    }

    #[test]
    fn is_in_git_folder_ignores_non_git_paths() {
        assert!(!is_in_git_folder(&anchored("src/main.rs")));
        assert!(!is_in_git_folder(&anchored("packages/a/index.ts")));
        assert!(!is_in_git_folder(&anchored(".github/workflows/ci.yml")));
        assert!(!is_in_git_folder(&anchored(".gitignore")));
    }

    #[test]
    fn ancestors_is_ignored_matches_directory_pattern() {
        let gitignore = gitignore_from_lines("/repo", &["node_modules/"]);

        assert!(ancestors_is_ignored(
            &gitignore,
            &anchored("node_modules/foo/bar.js")
        ));
        assert!(ancestors_is_ignored(
            &gitignore,
            &anchored("node_modules/package/index.js")
        ));
    }

    #[test]
    fn ancestors_is_ignored_skips_non_matching() {
        let gitignore = gitignore_from_lines("/repo", &["node_modules/"]);

        assert!(!ancestors_is_ignored(&gitignore, &anchored("src/main.rs")));
        assert!(!ancestors_is_ignored(
            &gitignore,
            &anchored("packages/a/index.ts")
        ));
    }

    #[test]
    fn ancestors_is_ignored_nested_directory() {
        let gitignore = gitignore_from_lines("/repo", &["dist/"]);

        assert!(ancestors_is_ignored(
            &gitignore,
            &anchored("packages/a/dist/bundle.js")
        ));
        assert!(!ancestors_is_ignored(
            &gitignore,
            &anchored("packages/a/src/index.ts")
        ));
    }

    #[test]
    fn ancestors_is_ignored_file_pattern() {
        let gitignore = gitignore_from_lines("/repo", &["*.log"]);

        assert!(ancestors_is_ignored(&gitignore, &anchored("debug.log")));
        assert!(!ancestors_is_ignored(&gitignore, &anchored("src/main.rs")));
    }

    #[test]
    fn ancestors_is_ignored_turbo_dir() {
        let gitignore = gitignore_from_lines("/repo", &[".turbo/"]);

        assert!(ancestors_is_ignored(
            &gitignore,
            &anchored(".turbo/cache/abc123")
        ));
        assert!(ancestors_is_ignored(
            &gitignore,
            &anchored("packages/a/.turbo/turbo-build.log")
        ));
    }

    // Tests for classify_changed_files

    #[tokio::test]
    async fn classify_turbo_json_change_triggers_config_changed() {
        let f = ClassifyFixture::new().await;
        let turbo_json_path = f.repo_root.join_component("turbo.json");
        let mut trie = Trie::new();
        trie.insert(turbo_json_path.to_string(), ());

        let action = f.classify(&trie, &["node_modules/"], None);
        assert!(matches!(action, FileChangeAction::ConfigChanged));
    }

    #[tokio::test]
    async fn classify_turbo_jsonc_change_triggers_config_changed() {
        let f = ClassifyFixture::new().await;
        let turbo_jsonc_path = f.repo_root.join_component("turbo.jsonc");
        let mut trie = Trie::new();
        trie.insert(turbo_jsonc_path.to_string(), ());

        let action = f.classify(&trie, &[], None);
        assert!(matches!(action, FileChangeAction::ConfigChanged));
    }

    #[tokio::test]
    async fn classify_custom_turbo_json_triggers_config_changed() {
        let f = ClassifyFixture::new().await;
        let custom_path = f.repo_root.join_components(&["config", "turbo.json"]);
        let mut trie = Trie::new();
        trie.insert(custom_path.to_string(), ());

        let action = f.classify(&trie, &[], Some(&custom_path));
        assert!(matches!(action, FileChangeAction::ConfigChanged));
    }

    #[tokio::test]
    async fn classify_gitignore_only_change_maps_to_root_package() {
        // .gitignore changes are handled by the caller before classify_changed_files.
        // If the trie still contains the .gitignore path, classify treats it as a
        // normal root-package file change (since .gitignore lives in the repo root).
        let f = ClassifyFixture::new().await;
        let gitignore_path = f.repo_root.join_component(".gitignore");
        let mut trie = Trie::new();
        trie.insert(gitignore_path.to_string(), ());

        let action = f.classify(&trie, &[], None);
        assert!(
            matches!(
                action,
                FileChangeAction::PackagesChanged(PackageChanges::Some(_), _)
            ),
            "expected PackagesChanged(Some) for .gitignore in trie, got {action:?}"
        );
    }

    #[tokio::test]
    async fn classify_gitignore_plus_package_file_detects_package_change() {
        // When .gitignore and a package source file change in the same batch,
        // the caller reloads the gitignore first, then classify_changed_files
        // processes the remaining files including the package source.
        let f = ClassifyFixture::new().await;
        let gitignore_path = f.repo_root.join_component(".gitignore");
        let src_path = f
            .repo_root
            .join_components(&["packages", "a", "src", "index.ts"]);
        let mut trie = Trie::new();
        trie.insert(gitignore_path.to_string(), ());
        trie.insert(src_path.to_string(), ());

        let action = f.classify(&trie, &["node_modules/"], None);
        match action {
            FileChangeAction::PackagesChanged(PackageChanges::Some(pkgs), _) => {
                let pkg_names: HashSet<_> = pkgs.keys().map(|p| p.name.to_string()).collect();
                assert!(
                    pkg_names.contains("a"),
                    "expected package 'a' in {pkg_names:?}"
                );
            }
            other => panic!(
                "expected PackagesChanged(Some) for .gitignore + package change, got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn classify_gitignore_plus_config_change_detects_config() {
        // When .gitignore and turbo.json both change in the same batch,
        // classify_changed_files should detect the config change.
        let f = ClassifyFixture::new().await;
        let gitignore_path = f.repo_root.join_component(".gitignore");
        let config_path = f.repo_root.join_component(CONFIG_FILE);
        let mut trie = Trie::new();
        trie.insert(gitignore_path.to_string(), ());
        trie.insert(config_path.to_string(), ());

        let action = f.classify(&trie, &[], None);
        assert!(
            matches!(action, FileChangeAction::ConfigChanged),
            "expected ConfigChanged for .gitignore + turbo.json, got {action:?}"
        );
    }

    #[tokio::test]
    async fn classify_gitignored_files_returns_no_relevant_changes() {
        let f = ClassifyFixture::new().await;
        let ignored_path = f
            .repo_root
            .join_components(&["node_modules", "foo", "index.js"]);
        let mut trie = Trie::new();
        trie.insert(ignored_path.to_string(), ());

        let action = f.classify(&trie, &["node_modules/"], None);
        assert!(matches!(action, FileChangeAction::NoRelevantChanges));
    }

    #[tokio::test]
    async fn classify_git_dir_files_returns_no_relevant_changes() {
        let f = ClassifyFixture::new().await;
        let git_path = f.repo_root.join_components(&[".git", "objects", "abc123"]);
        let mut trie = Trie::new();
        trie.insert(git_path.to_string(), ());

        let action = f.classify(&trie, &[], None);
        assert!(matches!(action, FileChangeAction::NoRelevantChanges));
    }

    #[tokio::test]
    async fn classify_empty_trie_returns_no_relevant_changes() {
        let f = ClassifyFixture::new().await;
        let trie = Trie::new();

        let action = f.classify(&trie, &[], None);
        assert!(matches!(action, FileChangeAction::NoRelevantChanges));
    }

    #[tokio::test]
    async fn classify_package_file_change_returns_packages_changed() {
        let f = ClassifyFixture::new().await;
        let src_path = f
            .repo_root
            .join_components(&["packages", "a", "src", "index.ts"]);
        let mut trie = Trie::new();
        trie.insert(src_path.to_string(), ());

        let action = f.classify(&trie, &["node_modules/"], None);
        match action {
            FileChangeAction::PackagesChanged(PackageChanges::Some(pkgs), _) => {
                let pkg_names: HashSet<_> = pkgs.keys().map(|p| p.name.to_string()).collect();
                assert!(
                    pkg_names.contains("a"),
                    "expected package 'a' in {:?}",
                    pkg_names
                );
            }
            other => panic!("expected PackagesChanged(Some), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn classify_mixed_git_and_real_changes_filters_git() {
        let f = ClassifyFixture::new().await;
        // Mix of .git paths and real package files
        let git_path = f.repo_root.join_components(&[".git", "HEAD"]);
        let src_path = f
            .repo_root
            .join_components(&["packages", "a", "src", "lib.ts"]);
        let mut trie = Trie::new();
        trie.insert(git_path.to_string(), ());
        trie.insert(src_path.to_string(), ());

        let action = f.classify(&trie, &[], None);
        assert!(matches!(
            action,
            FileChangeAction::PackagesChanged(PackageChanges::Some(_), _)
        ));
    }

    #[tokio::test]
    async fn classify_lockfile_change_returns_packages_changed() {
        // When the lockfile changes, the change mapper conservatively marks
        // affected packages. Verify classify propagates the result.
        let f = ClassifyFixture::new().await;

        let lockfile_path = f.repo_root.join_component("package-lock.json");
        let src_path = f
            .repo_root
            .join_components(&["packages", "a", "src", "index.ts"]);
        let mut trie = Trie::new();
        trie.insert(lockfile_path.to_string(), ());
        trie.insert(src_path.to_string(), ());

        let action = f.classify(&trie, &[], None);
        assert!(
            matches!(action, FileChangeAction::PackagesChanged(_, _)),
            "expected PackagesChanged for lockfile change, got {action:?}"
        );
    }

    // Integration tests for the full PackageChangesWatcher pipeline.
    // These feed synthetic notify::Events through a broadcast channel and
    // assert the correct PackageChangeEvents emerge.

    /// Create a notify::Event from AbsoluteSystemPathBufs. Using turbopath
    /// ensures paths are consistently resolved (e.g. /private/var on macOS).
    fn make_notify_event_from(paths: &[&AbsoluteSystemPathBuf]) -> notify::Event {
        notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: paths
                .iter()
                .map(|p| p.as_std_path().to_path_buf())
                .collect(),
            attrs: Default::default(),
        }
    }

    /// Set up a git-initialized temp dir with an npm monorepo and return
    /// everything needed to create a PackageChangesWatcher.
    fn setup_git_repo() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root =
            AbsoluteSystemPathBuf::new(tmp.path().to_str().unwrap().to_string()).unwrap();
        let repo_root = repo_root.to_realpath().unwrap();

        // git init
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();

        // Write root package.json
        let root_pkg = repo_root.join_component("package.json");
        root_pkg
            .create_with_contents(
                r#"{"name":"root","packageManager":"npm@10.0.0","workspaces":["packages/*"]}"#
                    .as_bytes(),
            )
            .unwrap();

        let lockfile = repo_root.join_component("package-lock.json");
        lockfile
            .create_with_contents(r#"{"lockfileVersion":3}"#.as_bytes())
            .unwrap();

        // turbo.json
        let turbo_json = repo_root.join_component("turbo.json");
        turbo_json
            .create_with_contents(r#"{"tasks":{"build":{}}}"#.as_bytes())
            .unwrap();

        // .gitignore
        let gitignore = repo_root.join_component(".gitignore");
        gitignore
            .create_with_contents("node_modules/\n.turbo/\n".as_bytes())
            .unwrap();

        // Package a
        let pkg_a_dir = repo_root.join_components(&["packages", "a"]);
        pkg_a_dir.create_dir_all().unwrap();
        pkg_a_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name":"a","scripts":{"build":"echo built"}}"#.as_bytes())
            .unwrap();
        pkg_a_dir
            .join_component("index.ts")
            .create_with_contents(b"export const a = 1;")
            .unwrap();

        // Package b
        let pkg_b_dir = repo_root.join_components(&["packages", "b"]);
        pkg_b_dir.create_dir_all().unwrap();
        pkg_b_dir
            .join_component("package.json")
            .create_with_contents(
                r#"{"name":"b","dependencies":{"a":"*"},"scripts":{"build":"echo built"}}"#
                    .as_bytes(),
            )
            .unwrap();
        pkg_b_dir
            .join_component("index.ts")
            .create_with_contents(b"export const b = 2;")
            .unwrap();

        // Initial commit
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init", "--quiet"])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();

        (tmp, repo_root)
    }

    use turborepo_repository::discovery::DiscoveryResponse;

    /// Holds onto channels that must stay alive for the test watcher to work.
    struct TestWatcherHandle {
        watcher: PackageChangesWatcher,
        file_events_tx: broadcast::Sender<Result<notify::Event, NotifyError>>,
        // These must be kept alive to prevent the HashWatcher from busy-looping
        // on closed channels.
        _pkg_discovery_tx: watch::Sender<Option<Result<DiscoveryResponse, String>>>,
        _hash_events_tx: broadcast::Sender<Result<notify::Event, NotifyError>>,
    }

    /// Create a PackageChangesWatcher backed by a synthetic file event channel.
    fn create_test_watcher(repo_root: &AbsoluteSystemPathBuf) -> TestWatcherHandle {
        create_test_watcher_with_opts(repo_root, false)
    }

    fn create_test_watcher_with_opts(
        repo_root: &AbsoluteSystemPathBuf,
        single_package: bool,
    ) -> TestWatcherHandle {
        let (file_events_tx, file_events_rx) = broadcast::channel(128);
        let (opt_tx, opt_watch) = OptionalWatch::new();
        opt_tx.send(Some(file_events_rx)).unwrap();

        // Keep the discovery sender alive so the HashWatcher doesn't busy-loop
        // on a closed watch channel. The hash watcher subscriber won't find any
        // packages, so get_file_hashes will return Err and is_same_hash
        // returns false (meaning: hash changed, send event).
        let (pkg_discovery_tx, pkg_discovery_rx) = watch::channel(None);

        let scm = SCM::new(repo_root);

        // Keep hash events sender alive too
        let (hash_events_tx, hash_events_rx) = broadcast::channel(128);
        let (hash_opt_tx, hash_opt_watch) = OptionalWatch::new();
        hash_opt_tx.send(Some(hash_events_rx)).unwrap();

        let hash_watcher = Arc::new(HashWatcher::new(
            repo_root.clone(),
            pkg_discovery_rx,
            hash_opt_watch,
            scm,
        ));

        let watcher = PackageChangesWatcher::new(
            repo_root.clone(),
            opt_watch,
            hash_watcher,
            None,
            single_package,
        );

        TestWatcherHandle {
            watcher,
            file_events_tx,
            _pkg_discovery_tx: pkg_discovery_tx,
            _hash_events_tx: hash_events_tx,
        }
    }

    /// Helper to receive a PackageChangeEvent with timeout.
    async fn recv_event(
        rx: &mut broadcast::Receiver<PackageChangeEvent>,
        timeout: Duration,
    ) -> Option<PackageChangeEvent> {
        tokio::time::timeout(timeout, rx.recv())
            .await
            .ok()
            .and_then(|r| r.ok())
    }

    #[tokio::test]
    async fn watcher_sends_initial_rediscover_on_startup() {
        let (_tmp, repo_root) = setup_git_repo();
        let handle = create_test_watcher(&repo_root);
        let mut rx = handle.watcher.package_change_events_rx.resubscribe();

        let event = recv_event(&mut rx, Duration::from_secs(2)).await;
        assert!(
            matches!(event, Some(PackageChangeEvent::Rediscover)),
            "expected Rediscover, got {:?}",
            event
        );
    }

    #[tokio::test]
    async fn subscriber_can_initialize_repo_state() {
        // Verify that our test repo setup is valid for building a PackageGraph
        let (_tmp, repo_root) = setup_git_repo();

        let root_pkg_json = PackageJson::load(&repo_root.join_component("package.json")).unwrap();
        let pkg_graph = PackageGraphBuilder::new(&repo_root, root_pkg_json)
            .build()
            .await
            .unwrap();

        let pkg_names: Vec<_> = pkg_graph
            .packages()
            .map(|(name, _)| name.to_string())
            .collect();
        assert!(
            pkg_names.iter().any(|n| n == "a"),
            "package 'a' not found in graph. packages: {:?}",
            pkg_names
        );
    }

    #[tokio::test]
    async fn broadcast_resubscribe_through_optional_watch_works() {
        // Verify our test setup pattern: events sent through broadcast::Sender
        // are received by subscribers created via OptionalWatch
        let (file_tx, file_rx) = broadcast::channel::<Result<notify::Event, NotifyError>>(128);
        let (opt_tx, mut opt_watch) = OptionalWatch::new();
        opt_tx.send(Some(file_rx)).unwrap();

        // Simulate what Subscriber does: get the value and resubscribe
        let mut events = opt_watch.get().await.unwrap().resubscribe();

        // Send an event
        let path = PathBuf::from("/tmp/test/file.ts");
        file_tx
            .send(Ok(notify::Event {
                kind: EventKind::Create(CreateKind::File),
                paths: vec![path.clone()],
                attrs: Default::default(),
            }))
            .unwrap();

        // Should receive it
        let received = events.recv().await.unwrap().unwrap();
        assert_eq!(received.paths, vec![path]);
    }

    #[tokio::test]
    async fn classify_works_with_git_repo_fixture() {
        // Direct test: classify_changed_files works with our git repo fixture
        let (_tmp, repo_root) = setup_git_repo();
        let pkg_graph = build_test_graph(&repo_root).await;

        let mapper =
            GlobalDepsPackageChangeMapper::new(&pkg_graph, std::iter::empty::<&str>()).unwrap();
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], mapper);

        let gitignore_path = repo_root.join_component(".gitignore");
        let (gitignore, _) = ignore::gitignore::Gitignore::new(&gitignore_path);

        let changed_file = repo_root.join_components(&["packages", "a", "index.ts"]);
        let mut trie = Trie::new();
        trie.insert(changed_file.to_string(), ());

        let action = classify_changed_files(&trie, &repo_root, &gitignore, None, &change_mapper);

        match &action {
            FileChangeAction::PackagesChanged(PackageChanges::Some(pkgs), _) => {
                let names: Vec<_> = pkgs.keys().map(|p| p.name.to_string()).collect();
                assert!(names.contains(&"a".to_string()), "got {:?}", names);
            }
            _ => panic!("expected PackagesChanged(Some), got variant"),
        }
    }

    #[tokio::test]
    async fn watcher_emits_package_event_for_file_change() {
        let (_tmp, repo_root) = setup_git_repo();
        let handle = create_test_watcher(&repo_root);
        let file_tx = &handle.file_events_tx;
        let mut rx = handle.watcher.package_change_events_rx.resubscribe();

        // Consume the initial Rediscover
        let initial = recv_event(&mut rx, Duration::from_secs(2)).await;
        assert!(
            matches!(initial, Some(PackageChangeEvent::Rediscover)),
            "expected initial Rediscover, got {:?}",
            initial
        );

        // Give the subscriber time to enter its polling loop
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Check subscriber is alive
        let receivers = file_tx.receiver_count();
        assert!(receivers > 0, "subscriber died: 0 receivers on file_tx");

        // Send multiple events to be safe (the subscriber may have
        // lagged the first one)
        let changed_file = repo_root.join_components(&["packages", "a", "index.ts"]);
        for _ in 0..3 {
            let _ = file_tx.send(Ok(make_notify_event_from(&[&changed_file])));
            tokio::time::sleep(Duration::from_millis(150)).await;
        }

        // Collect all events within a window
        let mut events = vec![];
        while let Some(evt) = recv_event(&mut rx, Duration::from_secs(2)).await {
            events.push(evt);
        }

        // We should have gotten at least one package change or rediscover event
        assert!(
            !events.is_empty(),
            "expected at least one event, got none. Receiver count at send time: {receivers}. \
             Repo root: {repo_root}"
        );

        let has_package_event = events.iter().any(|e| {
            matches!(
                e,
                PackageChangeEvent::Package { .. } | PackageChangeEvent::Rediscover
            )
        });
        assert!(
            has_package_event,
            "expected Package or Rediscover event, got: {:?}",
            events
        );
    }

    #[tokio::test]
    async fn watcher_emits_rediscover_for_turbo_json_change() {
        let (_tmp, repo_root) = setup_git_repo();
        let handle = create_test_watcher(&repo_root);
        let file_tx = &handle.file_events_tx;
        let mut rx = handle.watcher.package_change_events_rx.resubscribe();

        // Consume the initial Rediscover
        let _ = recv_event(&mut rx, Duration::from_secs(2)).await;

        // Send a turbo.json change
        let turbo_path = repo_root.join_component("turbo.json");
        file_tx
            .send(Ok(make_notify_event_from(&[&turbo_path])))
            .unwrap();

        let event = recv_event(&mut rx, Duration::from_secs(3)).await;
        assert!(
            matches!(event, Some(PackageChangeEvent::Rediscover)),
            "expected Rediscover for turbo.json change, got {:?}",
            event
        );
    }

    #[tokio::test]
    async fn watcher_ignores_git_dir_changes() {
        let (_tmp, repo_root) = setup_git_repo();
        let handle = create_test_watcher(&repo_root);
        let file_tx = &handle.file_events_tx;
        let mut rx = handle.watcher.package_change_events_rx.resubscribe();

        // Consume the initial Rediscover
        let _ = recv_event(&mut rx, Duration::from_secs(2)).await;

        // Send a .git directory change (should be ignored)
        let git_path = repo_root.join_components(&[".git", "objects", "abc123"]);
        file_tx
            .send(Ok(make_notify_event_from(&[&git_path])))
            .unwrap();

        // Then send a real config change as a sentinel — when we see its
        // Rediscover event, we know the watcher processed both events
        // and chose to skip the .git one.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let turbo_path = repo_root.join_component("turbo.json");
        file_tx
            .send(Ok(make_notify_event_from(&[&turbo_path])))
            .unwrap();

        let event = recv_event(&mut rx, Duration::from_secs(3)).await;
        assert!(
            matches!(event, Some(PackageChangeEvent::Rediscover)),
            "expected Rediscover from turbo.json sentinel, got {:?}",
            event
        );
    }

    /// Set up a git-initialized temp dir with a single-package (non-monorepo)
    /// layout and return everything needed to create a PackageChangesWatcher.
    fn setup_single_package_git_repo() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root =
            AbsoluteSystemPathBuf::new(tmp.path().to_str().unwrap().to_string()).unwrap();
        let repo_root = repo_root.to_realpath().unwrap();

        // git init
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();

        // Root package.json — no "workspaces" field
        let root_pkg = repo_root.join_component("package.json");
        root_pkg
            .create_with_contents(
                r#"{"name":"my-app","packageManager":"npm@10.0.0","scripts":{"build":"echo built","lint":"echo linted"}}"#
                    .as_bytes(),
            )
            .unwrap();

        let lockfile = repo_root.join_component("package-lock.json");
        lockfile
            .create_with_contents(r#"{"lockfileVersion":3}"#.as_bytes())
            .unwrap();

        // turbo.json with root tasks (no "//" prefix needed in single-package)
        let turbo_json = repo_root.join_component("turbo.json");
        turbo_json
            .create_with_contents(r#"{"tasks":{"build":{},"lint":{}}}"#.as_bytes())
            .unwrap();

        // .gitignore
        let gitignore = repo_root.join_component(".gitignore");
        gitignore
            .create_with_contents("node_modules/\n.turbo/\n".as_bytes())
            .unwrap();

        // Source file
        let src_dir = repo_root.join_component("src");
        src_dir.create_dir_all().unwrap();
        src_dir
            .join_component("index.ts")
            .create_with_contents(b"console.log('hello');")
            .unwrap();

        // Initial commit
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init", "--quiet"])
            .current_dir(repo_root.as_std_path())
            .status()
            .unwrap();

        (tmp, repo_root)
    }

    #[tokio::test]
    async fn single_package_watcher_sends_initial_rediscover() {
        let (_tmp, repo_root) = setup_single_package_git_repo();
        let handle = create_test_watcher_with_opts(&repo_root, true);
        let mut rx = handle.watcher.package_change_events_rx.resubscribe();

        let event = recv_event(&mut rx, Duration::from_secs(2)).await;
        assert!(
            matches!(event, Some(PackageChangeEvent::Rediscover)),
            "expected Rediscover from single-package watcher, got {:?}",
            event
        );
    }

    #[tokio::test]
    async fn single_package_subscriber_can_initialize_repo_state() {
        let (_tmp, repo_root) = setup_single_package_git_repo();

        let root_pkg_json = PackageJson::load(&repo_root.join_component("package.json")).unwrap();
        let pkg_graph = PackageGraphBuilder::new(&repo_root, root_pkg_json)
            .with_single_package_mode(true)
            .build()
            .await
            .unwrap();

        let pkg_names: Vec<_> = pkg_graph
            .packages()
            .map(|(name, _)| name.to_string())
            .collect();
        assert!(
            pkg_names.iter().any(|n| n == "//"),
            "root package '//' not found in graph. packages: {:?}",
            pkg_names
        );
    }

    #[tokio::test]
    async fn watcher_ignores_gitignored_paths() {
        let (_tmp, repo_root) = setup_git_repo();
        let handle = create_test_watcher(&repo_root);
        let file_tx = &handle.file_events_tx;
        let mut rx = handle.watcher.package_change_events_rx.resubscribe();

        // Consume the initial Rediscover
        let _ = recv_event(&mut rx, Duration::from_secs(2)).await;

        // Send a node_modules change (gitignored, should be dropped)
        let ignored_path =
            repo_root.join_components(&["packages", "a", "node_modules", "foo", "index.js"]);
        file_tx
            .send(Ok(make_notify_event_from(&[&ignored_path])))
            .unwrap();

        // Then send a real config change as a sentinel — when we see its
        // Rediscover event, we know the watcher processed the ignored event
        // and correctly dropped it.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let turbo_path = repo_root.join_component("turbo.json");
        file_tx
            .send(Ok(make_notify_event_from(&[&turbo_path])))
            .unwrap();

        let event = recv_event(&mut rx, Duration::from_secs(3)).await;
        assert!(
            matches!(event, Some(PackageChangeEvent::Rediscover)),
            "expected Rediscover from turbo.json sentinel, got {:?}",
            event
        );
    }
}
