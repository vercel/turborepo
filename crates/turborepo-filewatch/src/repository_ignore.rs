#[cfg(unix)]
use std::ffi::OsString;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::{Arc, RwLock},
};

use ignore::{Match, gitignore::Gitignore};
use tracing::warn;

/// A repository-wide, immutable-at-read-time view of Git's ignore rules.
///
/// Refreshes build a new snapshot and swap it under one `RwLock`; event-path
/// checks only take a read lock and never touch the filesystem or spawn Git.
#[derive(Clone)]
pub struct RepositoryIgnore {
    root: Arc<PathBuf>,
    match_root: Arc<PathBuf>,
    snapshot: Arc<RwLock<Snapshot>>,
    control_paths: Arc<RwLock<HashSet<PathBuf>>>,
}

#[derive(Default)]
struct Snapshot {
    worktree_root: PathBuf,
    matchers: HashMap<PathBuf, Arc<Gitignore>>,
    info_exclude: Option<Arc<Gitignore>>,
    global_exclude: Option<Arc<Gitignore>>,
    tracked: HashSet<PathBuf>,
    tracked_ancestors: HashSet<PathBuf>,
}

#[derive(Clone)]
struct GitContext {
    worktree_root: PathBuf,
    index: Option<PathBuf>,
    info_exclude: Option<PathBuf>,
    global_exclude: Option<PathBuf>,
}

impl RepositoryIgnore {
    pub fn new(root: &Path) -> Self {
        let root = Arc::new(normalize_lexically(root));
        let match_root = Arc::new(normalize_path(&root));
        let context = GitContext::discover(&match_root);
        let control_paths = context.control_paths(&match_root);
        let snapshot = Snapshot::load(&match_root, &context);
        Self {
            root,
            match_root,
            snapshot: Arc::new(RwLock::new(snapshot)),
            control_paths: Arc::new(RwLock::new(control_paths)),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Re-read repository ignore rules and tracked index state.
    pub fn refresh(&self) {
        // Re-discovering the context makes an explicit refresh observe changes
        // to core.excludesFile as well as changes to the exclude file itself.
        let context = GitContext::discover(&self.match_root);
        *self
            .control_paths
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            context.control_paths(&self.match_root);
        let replacement = Snapshot::load(&self.match_root, &context);
        *self
            .snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = replacement;
    }

    pub fn is_gitignore(path: &Path) -> bool {
        path.file_name().is_some_and(|name| name == ".gitignore")
    }

    pub fn should_refresh(&self, path: &Path) -> bool {
        Self::is_gitignore(path)
            || self
                .control_paths
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&normalize_event_path(&self.root, &self.match_root, path))
    }

    pub fn is_control_path(&self, path: &Path) -> bool {
        !Self::is_gitignore(path)
            && self
                .control_paths
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&normalize_event_path(&self.root, &self.match_root, path))
    }

    /// Returns whether changing `path` requires conservative invalidation.
    ///
    /// Ignore files inside the Turbo root are ordinary filesystem inputs: once
    /// the snapshot is refreshed, consumers can apply their normal scoped
    /// semantics to the event. An inherited ignore file cannot be represented
    /// by an in-root event (and may change the relevance of the root itself),
    /// so it must invalidate every consumer.
    pub fn invalidates_consumers(&self, path: &Path) -> bool {
        if self.is_control_path(path) {
            return true;
        }
        if !Self::is_gitignore(path) {
            return false;
        }
        let path = normalize_event_path(&self.root, &self.match_root, path);
        !path.starts_with(self.match_root.as_path())
    }

    pub fn control_paths(&self) -> Vec<PathBuf> {
        self.control_paths
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// Returns whether a path should participate in default Git-aware inputs.
    /// Explicit input globs should be checked before this method.
    pub fn is_relevant(&self, path: &Path, is_dir: bool) -> bool {
        let path = normalize_event_path(&self.root, &self.match_root, path);
        let Ok(relative) = path.strip_prefix(self.match_root.as_path()) else {
            return false;
        };
        if relative
            .components()
            .any(|component| component.as_os_str() == ".git")
        {
            return false;
        }
        if Self::is_gitignore(&path) {
            return true;
        }

        let snapshot = self
            .snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if snapshot.tracked.contains(relative)
            || (is_dir && snapshot.tracked_ancestors.contains(relative))
        {
            return true;
        }

        // A parent .gitignore may ignore the Turbo root itself. Git cannot
        // re-include anything below an ignored directory, so check the root and
        // then every descendant ancestor in order.
        if snapshot.is_ignored(self.match_root.as_path(), self.match_root.as_path(), true) {
            return false;
        }
        let mut candidate = self.match_root.as_path().to_path_buf();
        let components = relative.components().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            candidate.push(component.as_os_str());
            let candidate_is_dir = index + 1 != components.len() || is_dir;
            if snapshot.is_ignored(&candidate, self.match_root.as_path(), candidate_is_dir) {
                return false;
            }
        }
        true
    }
}

impl GitContext {
    fn discover(root: &Path) -> Self {
        let worktree_root = command_path(root, &["rev-parse", "--show-toplevel"], false, root)
            .unwrap_or_else(|| root.to_path_buf());
        let index = git_path(root, "index");
        let info_exclude = git_path(root, "info/exclude");
        let discovered_global_exclude = command_path(
            root,
            &["config", "--path", "--null", "--get", "core.excludesFile"],
            true,
            &worktree_root,
        );
        #[cfg(target_os = "macos")]
        let global_exclude = discovered_global_exclude.and_then(|exclude| {
            if paths_share_device(root, &exclude) {
                Some(exclude)
            } else {
                warn!(
                    path = %exclude.display(),
                    "ignoring cross-device global excludes because changes cannot be watched"
                );
                None
            }
        });
        #[cfg(not(target_os = "macos"))]
        let global_exclude = discovered_global_exclude;
        Self {
            worktree_root,
            index,
            info_exclude,
            global_exclude,
        }
    }

    fn control_paths(&self, root: &Path) -> HashSet<PathBuf> {
        let mut paths = HashSet::new();
        paths.extend(self.index.iter().cloned());
        paths.extend(self.info_exclude.iter().cloned());
        paths.extend(self.global_exclude.iter().cloned());
        // These files can change core.excludesFile. They are also useful
        // controls in linked worktrees, where the administrative directory is
        // outside the worktree.
        paths.extend(git_path(root, "config"));
        paths.extend(git_path(root, "config.worktree"));
        paths.extend(
            directories_between(&self.worktree_root, root)
                .into_iter()
                .map(|directory| directory.join(".gitignore")),
        );
        paths
    }
}

#[cfg(target_os = "macos")]
fn paths_share_device(left: &Path, right: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    matches!(
        (fs::metadata(left), fs::metadata(right)),
        (Ok(left), Ok(right)) if left.dev() == right.dev()
    )
}

impl Snapshot {
    fn load(root: &Path, context: &GitContext) -> Self {
        let mut snapshot = Self {
            worktree_root: context.worktree_root.clone(),
            ..Self::default()
        };
        snapshot.load_matchers(root, context);
        snapshot.load_tracked(root);
        snapshot
    }

    fn load_matchers(&mut self, root: &Path, context: &GitContext) {
        let mut loaded = HashSet::new();

        // Git consults every .gitignore from the worktree root down to the
        // directory containing the path. This includes ancestors of a nested
        // Turbo root.
        for directory in directories_between(&context.worktree_root, root) {
            let path = directory.join(".gitignore");
            self.add_directory_matcher(&path, &mut loaded);
        }

        // Discover all descendants without applying ignore/ripgrep pruning.
        // Whether a discovered file is reachable is handled by ancestor checks
        // in is_relevant, matching Git's ignored-directory rule.
        let mut walk = ignore::WalkBuilder::new(root);
        walk.hidden(false)
            .parents(false)
            .require_git(false)
            .ignore(false)
            .git_ignore(false)
            .git_exclude(false)
            .git_global(false)
            .filter_entry(|entry| entry.file_name() != ".git");
        for entry in walk.build().filter_map(Result::ok).filter(|entry| {
            entry.file_type().is_some_and(|kind| kind.is_file())
                && entry.file_name() == ".gitignore"
        }) {
            self.add_directory_matcher(entry.path(), &mut loaded);
        }

        self.info_exclude = context
            .info_exclude
            .as_deref()
            .and_then(|path| build_matcher(&context.worktree_root, path));
        self.global_exclude = context
            .global_exclude
            .as_deref()
            .and_then(|path| build_matcher(&context.worktree_root, path));
    }

    fn add_directory_matcher(&mut self, path: &Path, loaded: &mut HashSet<PathBuf>) {
        let path = normalize_path(path);
        if !loaded.insert(path.clone()) || !path.is_file() {
            return;
        }
        let Some(parent) = path.parent() else {
            return;
        };
        if let Some(matcher) = build_matcher(parent, &path) {
            self.matchers.insert(parent.to_path_buf(), matcher);
        }
    }

    fn load_tracked(&mut self, root: &Path) {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.worktree_root)
            .args(["ls-files", "-z", "--cached", "--full-name"])
            .output();
        let Ok(output) = output else {
            return;
        };
        if !output.status.success() {
            return;
        }
        for raw in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|raw| !raw.is_empty())
        {
            let absolute = normalize_lexically(&self.worktree_root.join(bytes_to_path(raw)));
            let Ok(path) = absolute.strip_prefix(root) else {
                continue;
            };
            let path = path.to_path_buf();
            self.tracked.insert(path.clone());
            let mut ancestor = path.parent();
            while let Some(path) = ancestor.filter(|path| !path.as_os_str().is_empty()) {
                self.tracked_ancestors.insert(path.to_path_buf());
                ancestor = path.parent();
            }
        }
    }

    fn is_ignored(&self, path: &Path, turbo_root: &Path, is_dir: bool) -> bool {
        let mut directory = path.parent().unwrap_or(turbo_root);
        loop {
            if let Some(matcher) = self.matchers.get(directory) {
                match matcher.matched(path, is_dir) {
                    Match::None => {}
                    Match::Ignore(_) => return true,
                    Match::Whitelist(_) => return false,
                }
            }
            if directory == self.worktree_root {
                break;
            }
            let Some(parent) = directory.parent() else {
                break;
            };
            directory = parent;
        }

        for matcher in [&self.info_exclude, &self.global_exclude]
            .into_iter()
            .flatten()
        {
            match matcher.matched(path, is_dir) {
                Match::None => {}
                Match::Ignore(_) => return true,
                Match::Whitelist(_) => return false,
            }
        }
        false
    }
}

fn build_matcher(base: &Path, path: &Path) -> Option<Arc<Gitignore>> {
    if !path.is_file() {
        return None;
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(base);
    if let Some(error) = builder.add(path) {
        warn!(%error, path = %path.display(), "invalid Git ignore file");
    }
    match builder.build() {
        Ok(matcher) if !matcher.is_empty() => Some(Arc::new(matcher)),
        Ok(_) => None,
        Err(error) => {
            warn!(%error, path = %path.display(), "invalid Git ignore file");
            None
        }
    }
}

fn directories_between(base: &Path, descendant: &Path) -> Vec<PathBuf> {
    let Ok(relative) = descendant.strip_prefix(base) else {
        return vec![descendant.to_path_buf()];
    };
    let mut directories = vec![base.to_path_buf()];
    let mut current = base.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        directories.push(current.clone());
    }
    directories
}

fn git_path(root: &Path, name: &str) -> Option<PathBuf> {
    command_path(root, &["rev-parse", "--git-path", name], false, root)
}

fn command_path(
    root: &Path,
    args: &[&str],
    nul_terminated: bool,
    relative_base: &Path,
) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let terminator = if nul_terminated { 0 } else { b'\n' };
    let raw = output.stdout.split(|byte| *byte == terminator).next()?;
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    if raw.is_empty() {
        return None;
    }
    let path = bytes_to_path(raw);
    let path = if path.is_absolute() {
        path
    } else {
        relative_base.join(path)
    };
    Some(normalize_path(&path))
}

fn bytes_to_path(raw: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(OsString::from_vec(raw.to_vec()))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(raw).into_owned())
    }
}

fn normalize_event_path(root: &Path, match_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        let path = normalize_lexically(path);
        path.strip_prefix(root)
            .map_or(path.clone(), |relative| match_root.join(relative))
    } else {
        normalize_lexically(&match_root.join(path))
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| normalize_lexically(path))
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = normalized
                    .components()
                    .next_back()
                    .is_some_and(|last| matches!(last, Component::Normal(_)));
                if can_pop {
                    normalized.pop();
                } else if !path.is_absolute() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use super::{RepositoryIgnore, normalize_lexically};

    fn git(root: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn nested_root_uses_all_git_ignore_sources_with_correct_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let root = worktree.join("projects/turbo");
        fs::create_dir_all(root.join("pkg/kept")).unwrap();
        git(&worktree, &["init", "-q"]);

        let global = temp.path().join("global-ignore");
        fs::write(&global, "*.global\nprecedence.txt\n").unwrap();
        git(
            &worktree,
            &["config", "core.excludesFile", "../global-ignore"],
        );
        fs::write(worktree.join(".gitignore"), "*.parent\nprecedence.txt\n").unwrap();
        fs::write(
            worktree.join("projects/.gitignore"),
            "!turbo/precedence.txt\n",
        )
        .unwrap();
        fs::write(root.join(".gitignore"), "root.tmp\n!precedence.txt\n").unwrap();
        fs::write(root.join("pkg/.gitignore"), "*.tmp\n!kept/\n").unwrap();
        fs::write(root.join(".ignore"), "visible.txt\n").unwrap();
        fs::write(root.join("tracked.global"), "tracked").unwrap();
        fs::write(worktree.join(".git/info/exclude"), "*.info\n").unwrap();
        git(&worktree, &["add", "-f", "projects/turbo/tracked.global"]);

        let model = RepositoryIgnore::new(&root);
        assert!(!model.is_relevant(&root.join("value.parent"), false));
        assert!(!model.is_relevant(&root.join("value.global"), false));
        assert!(!model.is_relevant(&root.join("value.info"), false));
        assert!(!model.is_relevant(&root.join("root.tmp"), false));
        assert!(!model.is_relevant(&root.join("pkg/nope.tmp"), false));
        assert!(model.is_relevant(&root.join("pkg/kept/value.txt"), false));
        assert!(model.is_relevant(&root.join("precedence.txt"), false));
        assert!(model.is_relevant(&root.join("tracked.global"), false));
        assert!(model.is_relevant(&root.join("visible.txt"), false));

        let normalized_global = fs::canonicalize(&global).unwrap();
        assert!(
            model
                .control_paths()
                .iter()
                .any(|path| path == &normalized_global)
        );
        assert!(
            model
                .control_paths()
                .iter()
                .any(|path| path == &fs::canonicalize(worktree.join(".gitignore")).unwrap())
        );
        assert!(model.should_refresh(&normalized_global));
    }

    #[test]
    fn refreshes_global_excludes_and_nested_rules() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let root = worktree.join("nested/turbo");
        fs::create_dir_all(root.join("pkg")).unwrap();
        git(&worktree, &["init", "-q"]);
        let global = temp.path().join("global-ignore");
        fs::write(&global, "before.txt\n").unwrap();
        git(
            &worktree,
            &["config", "core.excludesFile", global.to_str().unwrap()],
        );
        fs::write(root.join("pkg/.gitignore"), "before.tmp\n").unwrap();

        let model = RepositoryIgnore::new(&root);
        assert!(!model.is_relevant(&root.join("before.txt"), false));
        assert!(!model.is_relevant(&root.join("pkg/before.tmp"), false));

        fs::write(&global, "after.txt\n").unwrap();
        fs::write(root.join("pkg/.gitignore"), "after.tmp\n").unwrap();
        model.refresh();
        assert!(model.is_relevant(&root.join("before.txt"), false));
        assert!(model.is_relevant(&root.join("pkg/before.tmp"), false));
        assert!(!model.is_relevant(&root.join("after.txt"), false));
        assert!(!model.is_relevant(&root.join("pkg/after.tmp"), false));

        let replacement_global = temp.path().join("replacement-global-ignore");
        fs::write(&replacement_global, "replacement.txt\n").unwrap();
        git(
            &worktree,
            &[
                "config",
                "core.excludesFile",
                replacement_global.to_str().unwrap(),
            ],
        );
        model.refresh();
        let controls = model.control_paths();
        assert!(!controls.contains(&fs::canonicalize(&global).unwrap()));
        assert!(controls.contains(&fs::canonicalize(&replacement_global).unwrap()));
    }

    #[test]
    fn only_inherited_gitignore_changes_conservatively_invalidate() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let root = worktree.join("nested/turbo");
        fs::create_dir_all(root.join("pkg")).unwrap();
        git(&worktree, &["init", "-q"]);
        let inherited = worktree.join("nested/.gitignore");
        let root_ignore = root.join(".gitignore");
        let nested = root.join("pkg/.gitignore");
        fs::write(&inherited, "ignored\n").unwrap();
        fs::write(&root_ignore, "ignored\n").unwrap();
        fs::write(&nested, "ignored\n").unwrap();

        let model = RepositoryIgnore::new(&root);
        assert!(model.should_refresh(&inherited));
        assert!(model.invalidates_consumers(&inherited));
        assert!(model.should_refresh(&root_ignore));
        assert!(!model.invalidates_consumers(&root_ignore));
        assert!(model.should_refresh(&nested));
        assert!(!model.invalidates_consumers(&nested));
    }

    #[test]
    fn normalizes_parent_components_without_io() {
        assert_eq!(
            normalize_lexically(Path::new("/repo/nested/../.git/info/exclude")),
            Path::new("/repo/.git/info/exclude")
        );
        assert_eq!(
            normalize_lexically(Path::new("../../repo/./file")),
            Path::new("../../repo/file")
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn tracked_non_utf8_paths_remain_relevant() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "-q"]);
        fs::write(root.join(".gitignore"), "*\n").unwrap();
        let name = OsString::from_vec(b"tracked-\xff".to_vec());
        let path = root.join(name);
        fs::write(&path, "tracked").unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["add", "-f", "--"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );

        let model = RepositoryIgnore::new(root);
        assert!(model.is_relevant(&path, false));
    }
}
