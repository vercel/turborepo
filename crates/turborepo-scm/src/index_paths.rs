//! The paths a git index records.
//!
//! Callers that must never touch tracked content (e.g. `turbo clean`) need a
//! complete answer or none at all. git itself decides which working tree and
//! index apply (`git rev-parse`), so every setting git honors is honored here
//! too; the index file it names is then read directly. Nothing falls back:
//! without a `git` binary, or when the index is missing or unreadable, the
//! answer is an error rather than "nothing is tracked".

use std::{
    collections::HashMap,
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::Error;

/// The environment variables that change which repository, working tree and
/// index git uses, plus the directory relative values are resolved against.
/// Every other variable is inherited by git unchanged.
#[derive(Debug, Clone, Default)]
pub struct GitEnvironment {
    /// The process working directory: `GIT_DIR` and `GIT_WORK_TREE` are
    /// relative to it.
    pub cwd: PathBuf,
    pub git_dir: Option<OsString>,
    pub work_tree: Option<OsString>,
    pub index_file: Option<OsString>,
    pub common_dir: Option<OsString>,
    pub ceiling_directories: Option<OsString>,
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
            ceiling_directories: var("GIT_CEILING_DIRECTORIES"),
        })
    }
}

/// A `git` binary run from one directory with one environment.
#[derive(Debug, Clone)]
struct Git {
    bin: PathBuf,
    dir: PathBuf,
    vars: Vec<(&'static str, Option<OsString>)>,
}

impl Git {
    /// Runs from `dir`. Relative `GIT_DIR` and `GIT_WORK_TREE` values are
    /// made absolute against `env.cwd`, so they mean what they mean to the
    /// process that set them.
    fn new(dir: &Path, env: &GitEnvironment) -> Result<Self, Error> {
        let bin = which::which("git").map_err(|error| {
            Error::git_error(format!(
                "git was not found on PATH ({error}), so which files git tracks is unknown"
            ))
        })?;
        let absolute = |value: &Option<OsString>| value.as_ref().map(|v| env.cwd.join(v).into());
        Ok(Self {
            bin,
            dir: dir.to_owned(),
            vars: vec![
                ("GIT_DIR", absolute(&env.git_dir)),
                ("GIT_WORK_TREE", absolute(&env.work_tree)),
                // Like git, a relative `GIT_INDEX_FILE` is relative to the top
                // of the working tree.
                ("GIT_INDEX_FILE", env.index_file.clone()),
                ("GIT_COMMON_DIR", absolute(&env.common_dir)),
                ("GIT_CEILING_DIRECTORIES", env.ceiling_directories.clone()),
            ],
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.bin);
        command
            .current_dir(&self.dir)
            .env("GIT_OPTIONAL_LOCKS", "0")
            // Messages are matched below; keep them untranslated.
            .env("LC_ALL", "C");
        for (name, value) in &self.vars {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        command
    }
}

/// Every path recorded in the git index of a repository.
#[derive(Debug, Clone)]
pub struct IndexPaths {
    /// The real path of the working tree root the paths are relative to.
    pub git_root: AbsoluteSystemPathBuf,
    /// The index file git uses (`GIT_INDEX_FILE`, else the repository's own).
    pub index_file: PathBuf,
    /// Tracked files and symlinks: `/`-separated, relative to `git_root`,
    /// sorted. When `GIT_INDEX_FILE` names another index than the
    /// repository's own, the files of both.
    pub files: Vec<String>,
    /// Directory entries, without a trailing `/`: submodules (gitlinks) and
    /// the collapsed directories of a sparse index. Everything below them
    /// belongs to another tree, or is tracked without being listed.
    pub directories: Vec<String>,
    /// The real path the index was read for.
    root: AbsoluteSystemPathBuf,
    /// The recorded stat data of the files directly in `root`, by name.
    root_entries: HashMap<String, Vec<gix_index::entry::Stat>>,
    git: Git,
}

/// Asks `git rev-parse` (2.31+) for absolute paths.
const PATH_FORMAT_ABSOLUTE: &str = "--path-format=absolute";

/// The toplevel, git directory and index lines of `git rev-parse
/// --path-format=absolute --show-toplevel --git-dir --git-path index`.
///
/// git before 2.31 does not know `--path-format` and echoes it back as a line
/// of its own (its other answers may then be relative), so that output is
/// refused with the version git needs.
fn parse_rev_parse_output(stdout: &[u8]) -> Result<[&[u8]; 3], Error> {
    let lines: Vec<&[u8]> = stdout
        .strip_suffix(b"\n")
        .unwrap_or(stdout)
        .split(|byte| *byte == b'\n')
        .collect();
    if lines.first() == Some(&PATH_FORMAT_ABSOLUTE.as_bytes()) {
        return Err(Error::git_error(
            "turbo clean requires git >= 2.31: this git does not support `git rev-parse \
             --path-format`"
                .to_owned(),
        ));
    }
    let [top, git_dir, index_file] = lines[..] else {
        return Err(Error::git_error(format!(
            "unexpected git rev-parse output: {}",
            String::from_utf8_lossy(stdout)
        )));
    };
    Ok([top, git_dir, index_file])
}

thread_local! {
    /// Set while this thread decodes an index under `catch_unwind`.
    static DECODING_INDEX: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Serializes the panic hook swaps of [`without_panic_hook`].
static PANIC_HOOK_SWAP: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `decode`, turning a panic into `None` without running the process's
/// panic hook for it (turbo's prints a crash banner and writes a report).
///
/// The hook is process-wide, so it is swapped for one that stays silent only
/// for a panic on this thread while `decode` runs (a thread-local flag), and
/// passes every other panic, on any thread, to the previous hook. Swaps hold
/// a lock, so concurrent reads restore hooks in order. Even if code elsewhere
/// swapped the hook meanwhile, a wrapper left installed behaves exactly like
/// the hook it wraps once the flag is unset, so no interleaving silences
/// another panic.
fn without_panic_hook<T>(decode: impl FnOnce() -> T + std::panic::UnwindSafe) -> Option<T> {
    type Hook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;
    // `decode` cannot unwind past `catch_unwind`, so the lock is never
    // poisoned by it.
    let _swap = PANIC_HOOK_SWAP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous: std::sync::Arc<Hook> = std::sync::Arc::new(std::panic::take_hook());
    let delegate = previous.clone();
    std::panic::set_hook(Box::new(move |info| {
        if !DECODING_INDEX.with(std::cell::Cell::get) {
            delegate(info);
        }
    }));
    DECODING_INDEX.with(|flag| flag.set(true));
    let result = std::panic::catch_unwind(decode);
    DECODING_INDEX.with(|flag| flag.set(false));
    // Dropping the wrapper releases its reference to the previous hook.
    drop(std::panic::take_hook());
    match std::sync::Arc::try_unwrap(previous) {
        Ok(previous) => std::panic::set_hook(previous),
        Err(previous) => std::panic::set_hook(Box::new(move |info| previous(info))),
    }
    result.ok()
}

/// An index git wrote has a 12-byte header and a trailing hash.
const MINIMUM_INDEX_LEN: u64 = 12 + 20;

/// Reads the index at `path`, failing (never panicking) on anything that is
/// not a readable index.
fn read_index(path: &Path) -> Result<gix_index::File, Error> {
    let unreadable =
        |reason: String| Error::git_error(format!("the git index {} {reason}", path.display()));
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(unreadable("does not exist".to_owned()));
        }
        Err(error) => return Err(unreadable(format!("could not be read: {error}"))),
    };
    let len = file.metadata()?.len();
    let mut header = [0u8; 8];
    if len < MINIMUM_INDEX_LEN || file.read_exact(&mut header).is_err() {
        return Err(unreadable("is truncated".to_owned()));
    }
    let version = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    if &header[..4] != b"DIRC" || !(2..=4).contains(&version) {
        return Err(unreadable("is not a git index".to_owned()));
    }
    // gix-index trusts the file's structure in places; a panic while
    // decoding means the index is unreadable, not that nothing is tracked.
    without_panic_hook(|| {
        gix_index::File::at(
            path,
            gix_index::hash::Kind::Sha1,
            false,
            gix_index::decode::Options::default(),
        )
    })
    .ok_or_else(|| unreadable("could not be decoded".to_owned()))?
    .map_err(|error| unreadable(format!("could not be read: {error}")))
}

fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, Error> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(String::from_utf8(bytes.to_vec())?))
    }
}

fn real(path: &Path) -> Result<AbsoluteSystemPathBuf, Error> {
    AbsoluteSystemPathBuf::try_from(path)?
        .to_realpath()
        .map_err(|error| Error::git_error(format!("failed to resolve {}: {error}", path.display())))
}

/// Whether the stat data git recorded for a file still describes `metadata`.
/// git keeps the low 32 bits of sizes, times and inode numbers.
fn stat_matches(stat: &gix_index::entry::Stat, metadata: &std::fs::Metadata) -> bool {
    if stat.size != metadata.len() as u32 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        stat.mtime.secs == metadata.mtime() as u32
            && (stat.mtime.nsecs == 0 || stat.mtime.nsecs == metadata.mtime_nsec() as u32)
            && (stat.ino == 0 || stat.ino == metadata.ino() as u32)
    }
    #[cfg(not(unix))]
    {
        metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|mtime| stat.mtime.secs == mtime.as_secs() as u32)
    }
}

impl IndexPaths {
    /// Reads the index git would use for `path_in_repo`, honoring the
    /// environment of the current process. See [`IndexPaths::read_with`].
    pub fn read(path_in_repo: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        Self::read_with(path_in_repo, &GitEnvironment::from_process()?)
    }

    /// Reads the index git would use for `path_in_repo`.
    ///
    /// git decides: `git rev-parse --show-toplevel --git-dir --git-path
    /// index`, run from `path_in_repo` with `env`, names the working tree and
    /// the index. When that index is not the repository's own (a
    /// `GIT_INDEX_FILE` such as the temporary index of a partial commit), the
    /// paths of both are returned.
    ///
    /// Returns `Ok(None)` when git finds no repository. Errors when git is
    /// missing or fails, when the working tree does not contain
    /// `path_in_repo` (or there is none), when an index is missing or cannot
    /// be read, and for environments that would make git answer differently
    /// here than in the calling process (`GIT_COMMON_DIR`, `GIT_WORK_TREE`
    /// without `GIT_DIR`, `GIT_DIR` without `GIT_WORK_TREE`). Requires git
    /// 2.31 or later (`rev-parse --path-format`).
    pub fn read_with(
        path_in_repo: &AbsoluteSystemPath,
        env: &GitEnvironment,
    ) -> Result<Option<Self>, Error> {
        let real_path = path_in_repo.to_realpath()?;
        let unsupported = |reason: &str| {
            Error::git_error(format!(
                "{reason}, so it is unclear which working tree and index git would use"
            ))
        };
        if env.common_dir.is_some() {
            return Err(unsupported("GIT_COMMON_DIR is set"));
        }
        match (&env.git_dir, &env.work_tree) {
            (None, Some(_)) => return Err(unsupported("GIT_WORK_TREE is set without GIT_DIR")),
            // Without `GIT_WORK_TREE`, git takes the directory it runs from as
            // the top of the working tree. git hooks of a linked worktree run
            // like this, and a hook that runs turbo from a subdirectory would
            // make git report that subdirectory as the top, hiding every
            // tracked path in it. So this is refused wherever turbo runs.
            (Some(_), None) => {
                return Err(unsupported("GIT_DIR is set without GIT_WORK_TREE"));
            }
            _ => {}
        }

        let git = Git::new(real_path.as_std_path(), env)?;
        let output = git
            .command()
            .args([
                "rev-parse",
                PATH_FORMAT_ABSOLUTE,
                "--show-toplevel",
                "--git-dir",
                "--git-path",
                "index",
            ])
            .output()
            .map_err(|error| Error::git_error(format!("failed to run git rev-parse: {error}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not a git repository") {
                return Ok(None);
            }
            return Err(Error::git_error(format!(
                "git rev-parse failed: {}",
                stderr.trim()
            )));
        }
        let [top, git_dir, index_file] = parse_rev_parse_output(&output.stdout)?;
        let git_root = real(&path_from_bytes(top)?)?;
        if !real_path.as_std_path().starts_with(git_root.as_std_path()) {
            return Err(Error::git_error(format!(
                "the git working tree {git_root} does not contain {real_path}"
            )));
        }
        let index_file = path_from_bytes(index_file)?;
        let git_dir = path_from_bytes(git_dir)?;
        let default_index = git_dir.join("index");

        let mut indexes = vec![read_index(&index_file)?];
        let same_file = match (
            std::fs::canonicalize(&index_file),
            std::fs::canonicalize(&default_index),
        ) {
            (Ok(effective), Ok(default)) => effective == default,
            _ => false,
        };
        if !same_file {
            indexes.push(read_index(&default_index)?);
        }

        let prefix = git_root.anchor(&real_path)?.to_unix().as_str().to_owned();
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut root_entries: HashMap<String, Vec<gix_index::entry::Stat>> = HashMap::new();
        for index in &indexes {
            for entry in index.entries() {
                let path = String::from_utf8_lossy(entry.path(index)).into_owned();
                if entry.mode.is_submodule() || entry.mode.is_sparse() {
                    // A sparse directory entry is stored as `dir/`.
                    directories.push(path.trim_end_matches('/').to_owned());
                    continue;
                }
                let name = if prefix.is_empty() {
                    Some(path.as_str())
                } else {
                    path.strip_prefix(prefix.as_str())
                        .and_then(|rest| rest.strip_prefix('/'))
                };
                if let Some(name) = name.filter(|name| !name.contains('/')) {
                    root_entries
                        .entry(name.to_owned())
                        .or_default()
                        .push(entry.stat);
                }
                files.push(path);
            }
        }
        files.sort();
        files.dedup();
        directories.sort();
        directories.dedup();
        Ok(Some(Self {
            // git confirms a root manifest against exactly the repository,
            // working tree and index validated above, whatever the
            // environment says.
            git: Git {
                vars: vec![
                    ("GIT_DIR", Some(git_dir.into())),
                    ("GIT_WORK_TREE", Some(git_root.as_std_path().into())),
                    ("GIT_INDEX_FILE", Some(index_file.clone().into())),
                    ("GIT_COMMON_DIR", None),
                    ("GIT_CEILING_DIRECTORIES", None),
                ],
                ..git
            },
            git_root,
            index_file,
            files,
            directories,
            root: real_path,
            root_entries,
        }))
    }

    /// Whether `path` (relative to `git_root`, `/`-separated) is a tracked
    /// file or symlink, compared byte for byte.
    pub fn contains_file(&self, path: &str) -> bool {
        self.files
            .binary_search_by(|entry| entry.as_str().cmp(path))
            .is_ok()
    }

    /// Whether the index demonstrably describes the file `name` directly in
    /// the directory it was read for: the stat data recorded for it (size,
    /// modification time, and inode where recorded) matches the file on disk,
    /// or `git ls-files --error-unmatch` reports it as tracked. git runs with
    /// the repository, working tree and index validated by
    /// [`IndexPaths::read_with`], never those of the environment.
    pub fn describes_file_on_disk(&self, name: &str) -> bool {
        let on_disk = self.root.as_std_path().join(name);
        let Ok(metadata) = std::fs::symlink_metadata(&on_disk) else {
            return false;
        };
        let stat_matches = self
            .root_entries
            .get(name)
            .is_some_and(|stats| stats.iter().any(|stat| stat_matches(stat, &metadata)));
        stat_matches
            || self
                .git
                .command()
                .env("GIT_LITERAL_PATHSPECS", "1")
                .args(["ls-files", "--error-unmatch", "--"])
                .arg(name)
                .output()
                .is_ok_and(|output| output.status.success())
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
        // Not even for a repository with a working tree, run from its top.
        let env = GitEnvironment {
            git_dir: Some(".git".into()),
            ..clean_env(&home)
        };
        assert!(IndexPaths::read_with(&home, &env).is_err());
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

    /// A committed repository whose root holds `package.json`.
    fn committed(root: &AbsoluteSystemPath) {
        git(root, &["init", "--quiet"]);
        write(root, &["package.json"], "{}");
        git(root, &["add", "package.json"]);
        git(root, &["commit", "--quiet", "-m", "init"]);
    }

    /// Security round 3 F1: git also reads `core.worktree` from
    /// `config.worktree` in the main worktree (with
    /// `extensions.worktreeConfig`). The working tree is then a package
    /// directory, which does not contain the root.
    #[test]
    fn core_worktree_from_config_worktree_is_honored() {
        let (_tmp, root) = repo();
        write(&root, &["packages", "a", "package.json"], "{}");
        committed(&root);
        git(&root, &["config", "extensions.worktreeConfig", "true"]);
        git(
            &root,
            &["config", "--worktree", "core.worktree", "../packages/a"],
        );
        assert!(read(&root).is_err());
        assert_eq!(
            read(&root.join_components(&["packages", "a"]))
                .unwrap()
                .unwrap()
                .git_root,
            root.join_components(&["packages", "a"])
        );
    }

    /// Security round 3 F1: git skips a UTF-8 byte order mark before the
    /// first section of a config file.
    #[test]
    fn config_with_a_byte_order_mark_is_honored() {
        let (_tmp, root) = repo();
        write(&root, &["packages", "a", "package.json"], "{}");
        committed(&root);
        let config = root.join_components(&[".git", "config"]);
        let original = std::fs::read_to_string(config.as_std_path()).unwrap();
        std::fs::write(
            config.as_std_path(),
            format!("\u{feff}[core]\n\tworktree = ../packages/a\n{original}"),
        )
        .unwrap();
        assert!(read(&root).is_err());
    }

    /// Security round 3 F2: `GIT_CEILING_DIRECTORIES` hides an ancestor
    /// repository from git, so it is not used here either.
    #[test]
    fn honors_git_ceiling_directories() {
        let (_tmp, home) = repo();
        let project = home.join_component("project");
        write(&project, &["package.json"], "{}");
        git(&home, &["init", "--quiet"]);
        git(&home, &["add", "project/package.json"]);
        assert_eq!(read(&project).unwrap().unwrap().git_root, home);

        let env = GitEnvironment {
            ceiling_directories: Some(home.as_std_path().into()),
            ..clean_env(&project)
        };
        assert!(IndexPaths::read_with(&project, &env).unwrap().is_none());
    }

    /// Security round 3 F3: a truncated or corrupt index is an error, never a
    /// panic.
    #[test]
    fn a_corrupt_index_is_an_error_not_a_panic() {
        let (_tmp, root) = repo();
        committed(&root);
        let index = root.join_components(&[".git", "index"]);
        for contents in [
            &b"DIRC\0\0\0\x02garbage"[..],
            &b""[..],
            &b"DIRC\0\0\0\x09"[..],
            &[b"DIRC\0\0\0\x02\xff\xff\xff\xff".as_slice(), &[0; 40]].concat()[..],
        ] {
            std::fs::write(index.as_std_path(), contents).unwrap();
            assert!(read(&root).is_err(), "{contents:?}");
        }
    }

    /// Correctness round 3 R3-1: a `GIT_INDEX_FILE` other than the
    /// repository's own (the temporary index of a partial commit) adds to the
    /// repository's index rather than replacing it.
    #[test]
    fn another_index_file_is_read_with_the_repository_index() {
        let (_tmp, root) = repo();
        committed(&root);
        let temporary = root.join_components(&[".git", "next-index.lock"]);
        std::fs::copy(
            root.join_components(&[".git", "index"]).as_std_path(),
            temporary.as_std_path(),
        )
        .unwrap();
        write(&root, &["dist", "staged.js"], "staged");
        git(&root, &["add", "dist/staged.js"]);

        let env = GitEnvironment {
            index_file: Some(temporary.as_std_path().into()),
            ..clean_env(&root)
        };
        let paths = IndexPaths::read_with(&root, &env).unwrap().unwrap();
        assert_eq!(paths.index_file, temporary.as_std_path());
        assert!(paths.contains_file("dist/staged.js"));
        assert!(paths.contains_file("package.json"));
    }

    /// Security round 3 F1 (defense in depth): the index describes a file
    /// when its stat data matches the file on disk, or when git confirms it
    /// is tracked.
    #[test]
    fn describes_files_by_stat_data_or_by_git() {
        let (_tmp, root) = repo();
        committed(&root);
        write(&root, &["untracked.json"], "{}");
        let paths = read(&root).unwrap().unwrap();
        assert!(paths.describes_file_on_disk("package.json"));
        assert!(!paths.describes_file_on_disk("untracked.json"));
        assert!(!paths.describes_file_on_disk("missing.json"));

        // Edited since it was staged: the stat data no longer matches, but
        // git still reports the file as tracked.
        write(&root, &["package.json"], "{\"name\": \"edited\"}");
        let paths = read(&root).unwrap().unwrap();
        assert!(paths.describes_file_on_disk("package.json"));

        // Only the repository's own index records it, with stale stat data,
        // and the index git uses does not track it.
        let other = root.join_components(&[".git", "other-index"]);
        git_in(
            root.as_std_path(),
            &[("GIT_INDEX_FILE", other.as_std_path())],
            &["add", "untracked.json"],
        );
        let env = GitEnvironment {
            index_file: Some(other.as_std_path().into()),
            ..clean_env(&root)
        };
        let paths = IndexPaths::read_with(&root, &env).unwrap().unwrap();
        assert!(paths.contains_file("package.json"));
        assert!(!paths.describes_file_on_disk("package.json"));
    }

    /// Round 4 G1: git hooks of a linked worktree run with `GIT_DIR` set and
    /// no `GIT_WORK_TREE`. From a subdirectory, git then takes that
    /// subdirectory as the top of the working tree, so it is refused there
    /// as everywhere. With both set, `git ls-files` confirms against the
    /// validated working tree.
    #[test]
    fn git_dir_without_a_work_tree_is_refused_in_a_linked_worktree() {
        let (_tmp, base) = repo();
        let main = base.join_component("main");
        write(
            &main,
            &["package.json"],
            "{\"devDependencies\": {\"husky\": \"9\"}}",
        );
        write(&main, &["web", "package.json"], "{}");
        committed_all(&main);
        git(&main, &["worktree", "add", "--quiet", "../wt"]);
        let worktree = base.join_component("wt");
        let web = worktree.join_component("web");
        let output = Command::new("git")
            .args(["rev-parse", "--absolute-git-dir"])
            .env_remove("GIT_DIR")
            .current_dir(&worktree)
            .output()
            .unwrap();
        let git_dir = String::from_utf8(output.stdout).unwrap().trim().to_owned();

        for dir in [&web, &worktree] {
            let env = GitEnvironment {
                git_dir: Some(git_dir.clone().into()),
                ..clean_env(dir)
            };
            assert!(IndexPaths::read_with(dir, &env).is_err(), "{dir}");
        }

        let env = GitEnvironment {
            git_dir: Some(git_dir.into()),
            work_tree: Some("..".into()),
            ..clean_env(&web)
        };
        let paths = IndexPaths::read_with(&web, &env).unwrap().unwrap();
        assert_eq!(paths.git_root, worktree);
        write(&web, &["package.json"], "{\"name\": \"edited\"}");
        assert!(paths.describes_file_on_disk("package.json"));
    }

    fn committed_all(root: &AbsoluteSystemPath) {
        git(root, &["init", "--quiet"]);
        git(root, &["add", "."]);
        git(root, &["commit", "--quiet", "-m", "init"]);
    }

    /// Round 4 G2: git before 2.31 echoes `--path-format=absolute` back.
    #[test]
    fn rev_parse_output_without_path_format_support_needs_a_newer_git() {
        let old = b"--path-format=absolute\n/repo\n.git\n.git/index\n";
        let error = parse_rev_parse_output(old).unwrap_err().to_string();
        assert!(
            error.contains("turbo clean requires git >= 2.31"),
            "{error}"
        );

        let [top, git_dir, index] =
            parse_rev_parse_output(b"/repo\n/repo/.git\n/repo/.git/index\n").unwrap();
        assert_eq!(top, b"/repo");
        assert_eq!(git_dir, b"/repo/.git");
        assert_eq!(index, b"/repo/.git/index");

        let error = parse_rev_parse_output(b"/repo\n/repo/.git\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("unexpected git rev-parse output"), "{error}");
    }

    thread_local! {
        static HOOK_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Round 4 I1: a panic caught while decoding the index does not reach
    /// the process's panic hook (turbo's prints "Oops! Turbo has crashed"),
    /// and the hook is back in place afterwards.
    #[test]
    fn a_panic_while_decoding_the_index_skips_the_panic_hook() {
        let (_tmp, root) = repo();
        committed(&root);
        // One entry whose path runs to the end of the file: gix-index slices
        // past the end looking for the padding after it.
        let name = b"package.json";
        let index = [
            &b"DIRC\0\0\0\x02\0\0\0\x01"[..],
            &[0; 60],
            &((name.len() + 20) as u16).to_be_bytes(),
            name,
            &[0; 20],
        ]
        .concat();
        std::fs::write(
            root.join_components(&[".git", "index"]).as_std_path(),
            index,
        )
        .unwrap();

        let swap = || {
            PANIC_HOOK_SWAP
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        let previous = {
            let _swap = swap();
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {
                HOOK_CALLS.with(|calls| calls.set(calls.get() + 1))
            }));
            previous
        };
        let error = read(&root).map(|_| ()).unwrap_err().to_string();
        let calls_while_decoding = HOOK_CALLS.with(std::cell::Cell::get);
        let caught = std::panic::catch_unwind(|| panic!("outside the index read"));
        let calls_after = HOOK_CALLS.with(std::cell::Cell::get);
        {
            let _swap = swap();
            std::panic::set_hook(previous);
        }

        assert!(error.contains("could not be decoded"), "{error}");
        assert_eq!(calls_while_decoding, 0);
        assert!(caught.is_err());
        assert_eq!(calls_after, 1);
    }
}
