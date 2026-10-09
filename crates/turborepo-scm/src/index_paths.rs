//! The paths a git index records, read without the `git` binary.
//!
//! Callers that must never touch tracked content (e.g. `turbo clean`) need a
//! complete answer or none at all. Unlike the best-effort repository index
//! used for hashing, this never falls back: it works without a `git` binary
//! on `PATH`, and a repository whose index is missing or unreadable is an
//! error rather than "nothing is tracked".

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{Error, worktree::resolve_git_dir};

/// The environment variables that change which repository, working tree and
/// index git uses, plus the directory relative values are resolved against.
#[derive(Debug, Clone, Default)]
pub struct GitEnvironment {
    /// The process working directory: `GIT_DIR` and `GIT_WORK_TREE` are
    /// relative to it.
    pub cwd: PathBuf,
    pub git_dir: Option<OsString>,
    pub work_tree: Option<OsString>,
    pub index_file: Option<OsString>,
    pub common_dir: Option<OsString>,
}

impl GitEnvironment {
    /// The environment of the current process.
    pub fn from_process() -> Result<Self, Error> {
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        Ok(Self {
            cwd: std::env::current_dir()?,
            git_dir: var("GIT_DIR"),
            work_tree: var("GIT_WORK_TREE"),
            index_file: var("GIT_INDEX_FILE"),
            common_dir: var("GIT_COMMON_DIR"),
        })
    }
}

/// Every path recorded in the git index of a repository.
#[derive(Debug, Clone)]
pub struct IndexPaths {
    /// The real path of the working tree root the paths are relative to.
    pub git_root: AbsoluteSystemPathBuf,
    /// The index file that was read.
    pub index_file: PathBuf,
    /// Tracked files and symlinks: `/`-separated, relative to `git_root`,
    /// sorted.
    pub files: Vec<String>,
    /// Directory entries, without a trailing `/`: submodules (gitlinks) and
    /// the collapsed directories of a sparse index. Everything below them
    /// belongs to another tree, or is tracked without being listed.
    pub directories: Vec<String>,
}

fn unsupported(reason: impl std::fmt::Display) -> Error {
    Error::git_error(format!(
        "{reason}, so it is unclear which working tree and index git would use"
    ))
}

/// The settings in the `[core]` section of a repository's own config file
/// that move or remove its working tree.
#[derive(Debug, Default, PartialEq, Eq)]
struct CoreConfig {
    worktree: Option<String>,
    bare: Option<bool>,
}

impl CoreConfig {
    fn read(path: &Path) -> Result<Self, Error> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Self::parse(&contents)
                .ok_or_else(|| unsupported(format!("{} could not be parsed", path.display()))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(Error::git_error(format!(
                "failed to read {}: {error}",
                path.display()
            ))),
        }
    }

    /// Parses just enough of git's config syntax to find `core.worktree`
    /// and `core.bare`. Returns `None` for anything it does not understand
    /// in the `[core]` section, so callers fail closed.
    fn parse(contents: &str) -> Option<Self> {
        let mut config = Self::default();
        let mut in_core = false;
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(header) = line.strip_prefix('[') {
                let (name, rest) = header.split_once(']')?;
                if !rest.trim().is_empty()
                    && !rest.trim_start().starts_with(['#', ';'])
                    && name.trim().eq_ignore_ascii_case("core")
                {
                    // `[core] key = value` on one line.
                    return None;
                }
                in_core = name.trim().eq_ignore_ascii_case("core");
                continue;
            }
            if !in_core {
                continue;
            }
            let (key, value) = match line.split_once('=') {
                Some((key, value)) => (key.trim(), Some(value.trim())),
                None => (line, None),
            };
            let value = match value {
                Some(value) => Some(unquote(value)?),
                None => None,
            };
            if key.eq_ignore_ascii_case("worktree") {
                config.worktree = Some(value?);
            } else if key.eq_ignore_ascii_case("bare") {
                config.bare = Some(match value.as_deref().map(str::to_ascii_lowercase) {
                    None => true,
                    Some(value) => match value.as_str() {
                        "true" | "yes" | "on" | "1" => true,
                        "false" | "no" | "off" | "0" | "" => false,
                        _ => return None,
                    },
                });
            }
        }
        Some(config)
    }
}

/// A config value without comments or quotes. `None` for escapes or line
/// continuations, which this reader does not interpret.
fn unquote(value: &str) -> Option<String> {
    let mut out = String::new();
    let mut quoted = false;
    for char in value.chars() {
        match char {
            '"' => quoted = !quoted,
            '#' | ';' if !quoted => break,
            '\\' => return None,
            _ => out.push(char),
        }
    }
    (!quoted).then(|| out.trim().to_owned())
}

/// `path` itself when it is a git directory, or the directory a `.git`-style
/// `gitdir:` file points to.
fn git_dir_at(path: &Path) -> Result<PathBuf, Error> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        Error::git_error(format!("GIT_DIR {} is not usable: {error}", path.display()))
    })?;
    if metadata.is_dir() {
        return Ok(path.to_owned());
    }
    let contents = std::fs::read_to_string(path)?;
    let target = contents
        .strip_prefix("gitdir: ")
        .map(str::trim)
        .ok_or_else(|| unsupported(format!("GIT_DIR {} is not a git directory", path.display())))?;
    Ok(path.parent().unwrap_or(path).join(target))
}

fn real(path: &Path) -> Result<AbsoluteSystemPathBuf, Error> {
    AbsoluteSystemPathBuf::try_from(path)?
        .to_realpath()
        .map_err(|error| Error::git_error(format!("failed to resolve {}: {error}", path.display())))
}

impl IndexPaths {
    /// Reads the index git would use for `path_in_repo`, honoring the
    /// environment of the current process. See [`IndexPaths::read_with`].
    pub fn read(path_in_repo: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        Self::read_with(path_in_repo, &GitEnvironment::from_process()?)
    }

    /// Reads the index git would use for `path_in_repo`.
    ///
    /// The repository is the one `GIT_DIR` names, or else the first ancestor
    /// with a `.git` entry. The working tree is `GIT_WORK_TREE`, else
    /// `core.worktree`, else the directory holding `.git`. The index is
    /// `GIT_INDEX_FILE`, else the repository's own.
    ///
    /// Returns `Ok(None)` when no repository is found. Errors when a
    /// repository is found but its index is missing or cannot be read, when
    /// the working tree does not contain `path_in_repo`, and for setups this
    /// reader cannot resolve unambiguously (`GIT_COMMON_DIR`, `GIT_DIR`
    /// without a working tree, `GIT_WORK_TREE` without `GIT_DIR`, a bare
    /// repository, `core.worktree` in a linked worktree).
    pub fn read_with(
        path_in_repo: &AbsoluteSystemPath,
        env: &GitEnvironment,
    ) -> Result<Option<Self>, Error> {
        let real_path = path_in_repo.to_realpath()?;
        if env.common_dir.is_some() {
            return Err(unsupported("GIT_COMMON_DIR is set"));
        }
        let (git_dir, work_tree) = match (&env.git_dir, &env.work_tree) {
            (Some(git_dir), work_tree) => {
                let git_dir = git_dir_at(&env.cwd.join(git_dir))?;
                let config = Self::core_config(&git_dir)?;
                let work_tree = match (work_tree, config.worktree) {
                    (Some(work_tree), _) => env.cwd.join(work_tree),
                    (None, Some(work_tree)) => git_dir.join(work_tree),
                    (None, None) => {
                        return Err(unsupported(
                            "GIT_DIR is set but neither GIT_WORK_TREE nor core.worktree names a \
                             working tree",
                        ));
                    }
                };
                (git_dir, work_tree)
            }
            (None, Some(_)) => return Err(unsupported("GIT_WORK_TREE is set without GIT_DIR")),
            (None, None) => {
                let Some(top) = real_path
                    .as_std_path()
                    .ancestors()
                    .find(|dir| std::fs::symlink_metadata(dir.join(".git")).is_ok())
                else {
                    return Ok(None);
                };
                let top = AbsoluteSystemPathBuf::try_from(top)?;
                let git_dir = resolve_git_dir(&top)?.as_std_path().to_owned();
                let config = Self::core_config(&git_dir)?;
                let work_tree = match (config.worktree, config.bare) {
                    (Some(work_tree), _) => git_dir.join(work_tree),
                    (None, Some(true)) => {
                        return Err(unsupported(format!(
                            "the repository at {} is bare",
                            git_dir.display()
                        )));
                    }
                    (None, _) => top.as_std_path().to_owned(),
                };
                (git_dir, work_tree)
            }
        };
        let git_root = real(&work_tree)?;
        if !real_path.as_std_path().starts_with(git_root.as_std_path()) {
            return Err(Error::git_error(format!(
                "the git working tree {git_root} does not contain {real_path}"
            )));
        }
        // Like git, a relative `GIT_INDEX_FILE` is relative to the top of the
        // working tree.
        let index_file = match &env.index_file {
            Some(index_file) => git_root.as_std_path().join(index_file),
            None => git_dir.join("index"),
        };
        if !index_file.exists() {
            return Err(Error::git_error(format!(
                "the git index {} does not exist",
                index_file.display()
            )));
        }
        let index = gix_index::File::at(
            &index_file,
            gix_index::hash::Kind::Sha1,
            false,
            gix_index::decode::Options::default(),
        )
        .map_err(|e| Error::git_error(format!("failed to read git index: {e}")))?;

        let mut files = Vec::with_capacity(index.entries().len());
        let mut directories = Vec::new();
        for entry in index.entries() {
            let path = String::from_utf8_lossy(entry.path(&index)).into_owned();
            if entry.mode.is_submodule() || entry.mode.is_sparse() {
                // A sparse directory entry is stored as `dir/`.
                directories.push(path.trim_end_matches('/').to_owned());
            } else {
                files.push(path);
            }
        }
        files.sort();
        files.dedup();
        directories.sort();
        directories.dedup();
        Ok(Some(Self {
            git_root,
            index_file,
            files,
            directories,
        }))
    }

    /// The `[core]` settings git applies to `git_dir`. For a linked worktree
    /// the shared config applies too; `core.worktree` there is refused as
    /// ambiguous.
    fn core_config(git_dir: &Path) -> Result<CoreConfig, Error> {
        let own = CoreConfig::read(&git_dir.join("config"))?;
        let Ok(common_dir) = std::fs::read_to_string(git_dir.join("commondir")) else {
            return Ok(own);
        };
        let common_dir = git_dir.join(common_dir.trim());
        let shared = CoreConfig::read(&common_dir.join("config"))?;
        let worktree = CoreConfig::read(&git_dir.join("config.worktree"))?;
        if own.worktree.is_some() || shared.worktree.is_some() || worktree.worktree.is_some() {
            return Err(unsupported(format!(
                "core.worktree is set for the linked worktree at {}",
                git_dir.display()
            )));
        }
        // A linked worktree always has a working tree, even when the shared
        // repository is bare.
        Ok(CoreConfig::default())
    }

    /// Whether `path` (relative to `git_root`, `/`-separated) is a tracked
    /// file or symlink, compared byte for byte.
    pub fn contains_file(&self, path: &str) -> bool {
        self.files
            .binary_search_by(|entry| entry.as_str().cmp(path))
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn git_in(dir: &Path, envs: &[(&str, &Path)], args: &[&str]) {
        let output = Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .envs(envs.iter().copied())
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    fn git(root: &AbsoluteSystemPath, args: &[&str]) {
        git_in(root.as_std_path(), &[], args);
    }

    fn repo() -> (TempDir, AbsoluteSystemPathBuf) {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp, root)
    }

    /// No git variables set, run from `cwd`.
    fn clean_env(cwd: &AbsoluteSystemPath) -> GitEnvironment {
        GitEnvironment {
            cwd: cwd.as_std_path().to_owned(),
            ..Default::default()
        }
    }

    fn read(path: &AbsoluteSystemPath) -> Result<Option<IndexPaths>, Error> {
        IndexPaths::read_with(path, &clean_env(path))
    }

    fn write(root: &AbsoluteSystemPath, components: &[&str], contents: &str) {
        let file = root.join_components(components);
        file.ensure_dir().unwrap();
        file.create_with_contents(contents).unwrap();
    }

    #[test]
    fn outside_git_is_none() {
        let (_tmp, root) = repo();
        assert!(read(&root).unwrap().is_none());
    }

    #[test]
    fn a_repository_without_an_index_is_an_error() {
        let (_tmp, root) = repo();
        git(&root, &["init", "--quiet"]);
        assert!(read(&root).is_err());
    }

    #[test]
    fn records_files_symlinks_and_gitlinks() {
        let (_tmp, root) = repo();
        git(&root, &["init", "--quiet"]);
        let nested = root.join_components(&["pkg", "src"]);
        nested.create_dir_all().unwrap();
        nested
            .join_component("index.ts")
            .create_with_contents("source")
            .unwrap();
        #[cfg(unix)]
        root.join_components(&["pkg", "link.js"])
            .symlink_to_file("src/index.ts")
            .unwrap();
        git(&root, &["add", "."]);
        // A submodule entry, added without cloning anything.
        git(
            &root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000,5b5dbe4fa71b9ba5e09e1e0b5d2bd8ea2b0d4a4d,pkg/vendor",
            ],
        );

        // Read from a nested directory: the repository is found above it.
        let paths = read(&nested).unwrap().unwrap();
        assert_eq!(paths.git_root, root);
        assert!(paths.contains_file("pkg/src/index.ts"));
        #[cfg(unix)]
        assert!(paths.contains_file("pkg/link.js"));
        assert!(!paths.contains_file("pkg/vendor"));
        assert_eq!(paths.directories, ["pkg/vendor"]);
    }

    /// Security round 2 #1: git stores collapsed sparse directories as
    /// `dir/`; they are recorded without the slash so prefix checks work.
    #[test]
    fn sparse_index_directories_lose_their_trailing_slash() {
        let (_tmp, root) = repo();
        git(&root, &["init", "--quiet"]);
        write(&root, &["gen", "deep", "t1.ts"], "1");
        write(&root, &["src", "x.ts"], "x");
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "init"]);
        git(
            &root,
            &["sparse-checkout", "init", "--cone", "--sparse-index"],
        );
        git(&root, &["sparse-checkout", "set", "src"]);

        let paths = read(&root).unwrap().unwrap();
        assert_eq!(paths.directories, ["gen"]);
        assert!(paths.contains_file("src/x.ts"));
    }

    /// Security round 2 #2: a `GIT_DIR` store with `GIT_WORK_TREE` is the
    /// repository git uses, not an unrelated `.git` further up.
    #[test]
    fn honors_git_dir_and_git_work_tree() {
        let (_tmp, home) = repo();
        git(&home, &["init", "--quiet"]);
        write(&home, &[".bashrc"], "dotfiles");
        git(&home, &["add", ".bashrc"]);
        git(&home, &["commit", "--quiet", "-m", "dotfiles"]);
        let project = home.join_component("project");
        write(&project, &["gen", "t.ts"], "handwritten");
        let store = home.join_component("store.git");
        git(&home, &["init", "--quiet", "--bare", "store.git"]);
        let envs = [
            ("GIT_DIR", store.as_std_path()),
            ("GIT_WORK_TREE", project.as_std_path()),
        ];
        git_in(project.as_std_path(), &envs, &["add", "gen/t.ts"]);

        // Without the variables, the ancestor repository is found.
        let ancestor = read(&project).unwrap().unwrap();
        assert_eq!(ancestor.git_root, home);
        assert!(!ancestor.contains_file("project/gen/t.ts"));

        // With them, the store's index, relative to the work tree.
        let env = GitEnvironment {
            git_dir: Some("../store.git".into()),
            work_tree: Some(".".into()),
            ..clean_env(&project)
        };
        let paths = IndexPaths::read_with(&project, &env).unwrap().unwrap();
        assert_eq!(paths.git_root, project);
        assert!(paths.contains_file("gen/t.ts"));

        // `GIT_DIR` alone does not say where the working tree is.
        let env = GitEnvironment {
            work_tree: None,
            ..env
        };
        assert!(IndexPaths::read_with(&project, &env).is_err());
        // Nor does `GIT_WORK_TREE` alone say which repository.
        let env = GitEnvironment {
            work_tree: Some(".".into()),
            ..clean_env(&project)
        };
        assert!(IndexPaths::read_with(&project, &env).is_err());
        // `GIT_COMMON_DIR` is refused.
        let env = GitEnvironment {
            common_dir: Some("x".into()),
            ..clean_env(&project)
        };
        assert!(IndexPaths::read_with(&project, &env).is_err());
    }

    #[test]
    fn honors_git_index_file() {
        let (_tmp, root) = repo();
        git(&root, &["init", "--quiet"]);
        write(&root, &["README.md"], "readme");
        write(&root, &["gen", "t.ts"], "staged elsewhere");
        git(&root, &["add", "README.md"]);
        let alternate = root.join_components(&[".git", "alt-index"]);
        std::fs::copy(
            root.join_components(&[".git", "index"]).as_std_path(),
            alternate.as_std_path(),
        )
        .unwrap();
        git_in(
            root.as_std_path(),
            &[("GIT_INDEX_FILE", alternate.as_std_path())],
            &["add", "gen/t.ts"],
        );

        assert!(!read(&root).unwrap().unwrap().contains_file("gen/t.ts"));
        // Relative to the top of the working tree, like git, even when read
        // from a subdirectory.
        let subdirectory = root.join_component("gen");
        let env = GitEnvironment {
            index_file: Some(".git/alt-index".into()),
            ..clean_env(&subdirectory)
        };
        let paths = IndexPaths::read_with(&subdirectory, &env).unwrap().unwrap();
        assert!(paths.contains_file("gen/t.ts"));
        // A missing index is an error, never "nothing is tracked".
        let env = GitEnvironment {
            index_file: Some("missing-index".into()),
            ..clean_env(&root)
        };
        assert!(IndexPaths::read_with(&root, &env).is_err());
    }

    #[test]
    fn honors_core_worktree_and_requires_it_to_contain_the_path() {
        let (_tmp, root) = repo();
        let project = root.join_component("project");
        let elsewhere = root.join_component("elsewhere");
        project.create_dir_all().unwrap();
        elsewhere.create_dir_all().unwrap();
        git(&root, &["init", "--quiet"]);
        write(&project, &["src", "a.ts"], "a");
        git(&root, &["add", "project/src/a.ts"]);

        // The working tree moved to a directory that does not hold the path.
        git(&root, &["config", "core.worktree", "../elsewhere"]);
        assert!(read(&project).is_err());
        // Moved to `project`: paths are relative to it.
        git(&root, &["config", "core.worktree", "../project"]);
        let paths = read(&project).unwrap().unwrap();
        assert_eq!(paths.git_root, project);
    }

    #[test]
    fn bare_repositories_are_refused() {
        let (_tmp, root) = repo();
        git(&root, &["init", "--quiet"]);
        write(&root, &["a.ts"], "a");
        git(&root, &["add", "a.ts"]);
        git(&root, &["config", "core.bare", "true"]);
        assert!(read(&root).is_err());
    }

    #[test]
    fn parses_core_settings_or_refuses() {
        let parse = CoreConfig::parse;
        assert_eq!(
            parse("[core]\n\tbare = false\n\tworktree = \"../a b\" # moved\n[user]\nworktree = x"),
            Some(CoreConfig {
                worktree: Some("../a b".to_owned()),
                bare: Some(false),
            })
        );
        assert_eq!(parse("[CORE]\nBare\n").unwrap().bare, Some(true));
        assert_eq!(parse("[core]\nworktree = a\\\\b\n"), None);
        assert_eq!(parse("[core] worktree = a\n"), None);
        assert_eq!(parse("[core]\nbare = maybe\n"), None);
    }
}
