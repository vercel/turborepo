//! Bounded read-only effective marker identities. Handles stay open so
//! replacing a referenced directory cannot pass via inode reuse. Never print
//! input bytes.
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

use turbopath::AbsoluteSystemPath;
use turborepo_turbo_json::FutureFlags;

use super::Error;

#[derive(PartialEq, Eq)]
enum Entry {
    Missing,
    File(Arc<same_file::Handle>, Vec<u8>),
    Directory(Arc<same_file::Handle>),
    Link(PathBuf),
}
#[derive(Default, PartialEq, Eq)]
pub(super) struct Inputs(BTreeMap<PathBuf, Entry>, BTreeMap<PathBuf, PathBuf>);
impl Inputs {
    fn record(&mut self, path: &Path, directory: bool, links: bool) -> io::Result<Option<Vec<u8>>> {
        if self.0.len() >= 256 && !self.0.contains_key(path) {
            return Err(io::Error::other("too many discovery indirections"));
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(value) => Some(value),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let entry = match metadata {
            None => Entry::Missing,
            Some(m) if m.file_type().is_symlink() => {
                if !links {
                    return Err(io::Error::other(
                        "workspace/indirection marker symlinks are unsupported",
                    ));
                }
                Entry::Link(fs::read_link(path)?)
            }
            Some(m) if m.is_dir() && directory => {
                #[cfg(unix)]
                let handle = {
                    use std::os::unix::fs::OpenOptionsExt;
                    let file = fs::OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_DIRECTORY)
                        .open(path)?;
                    same_file::Handle::from_file(file)?
                };
                #[cfg(not(unix))]
                let handle = same_file::Handle::from_path(path)?;
                Entry::Directory(Arc::new(handle))
            }
            Some(m) if m.is_file() => {
                let mut options = fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                }
                let file = options.open(path)?;
                if !file.metadata()?.is_file() {
                    return Err(io::Error::other("unsafe discovery marker"));
                }
                let handle = Arc::new(same_file::Handle::from_file(file.try_clone()?)?);
                let mut bytes = Vec::new();
                file.take(1_048_577).read_to_end(&mut bytes)?;
                if bytes.len() > 1_048_576 {
                    return Err(io::Error::other("discovery marker exceeds byte limit"));
                }
                let total: usize = self
                    .0
                    .iter()
                    .filter(|(key, _)| key.as_path() != path)
                    .filter_map(|(_, entry)| match entry {
                        Entry::File(_, bytes) => Some(bytes.len()),
                        _ => None,
                    })
                    .sum();
                if total + bytes.len() > 4_194_304 {
                    return Err(io::Error::other("discovery inputs exceed byte limit"));
                }
                Entry::File(handle, bytes)
            }
            _ => return Err(io::Error::other("unsafe discovery marker")),
        };
        let data = match &entry {
            Entry::File(_, bytes) => Some(bytes.clone()),
            _ => None,
        };
        let link = matches!(entry, Entry::Link(_));
        if let Some(old) = self.0.insert(path.to_owned(), entry)
            && old != self.0[path]
        {
            return Err(io::Error::other("marker changed during capture"));
        }
        if link {
            return self.record(&path.canonicalize()?, directory, false);
        }
        Ok(data)
    }
    fn directory(&mut self, path: &Path) -> io::Result<()> {
        if self.record(path, true, true)?.is_some() {
            return Err(io::Error::other("Git target is not a directory"));
        }
        Ok(())
    }
    fn git(&mut self, marker: &Path) -> io::Result<()> {
        if [
            "GIT_COMMON_DIR",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
        {
            return Err(io::Error::other(
                "Git environment indirections are unsupported",
            ));
        }
        let bytes = self.record(marker, true, true)?;
        let dir = match bytes {
            Some(bytes) => pointer(marker, &bytes, "gitdir: ")?,
            None => marker.to_owned(),
        };
        self.directory(&dir)?;
        let common = match self.record(&dir.join("commondir"), false, false)? {
            Some(bytes) => pointer(&dir.join("commondir"), &bytes, "")?,
            None => dir.clone(),
        };
        self.directory(&common)?;
        // Git resolves core.worktree, includes and worktree config; do not parse
        // its grammar here. Empty read-only fixture boundaries have no HEAD.
        if self.record(&dir.join("HEAD"), false, true)?.is_some() {
            let output = std::process::Command::new("git")
                .current_dir(
                    marker
                        .parent()
                        .ok_or_else(|| io::Error::other("invalid Git boundary"))?,
                )
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .args(["rev-parse", "--show-toplevel"])
                .output()?;
            if !output.status.success() {
                return Err(io::Error::other("cannot guard effective Git worktree"));
            }
            let worktree = pointer(marker, &output.stdout, "")?.canonicalize()?;
            self.directory(&worktree)?;
            self.1.insert(marker.to_owned(), worktree);
        }
        // config/core.worktree and config.worktree can redirect Git's effective
        // worktree. Pin their bytes as well as common/worktree pointer targets.
        for base in [&dir, &common] {
            for name in ["config", "config.worktree"] {
                self.record(&base.join(name), false, false)?;
            }
        }
        if let Some(bytes) = self.record(&dir.join("gitdir"), false, false)? {
            let backlink = pointer(&dir.join("gitdir"), &bytes, "")?;
            self.record(&backlink, true, true)?;
            self.directory(
                backlink
                    .parent()
                    .ok_or_else(|| io::Error::other("invalid Git backlink"))?,
            )?;
        }
        Ok(())
    }
}
fn pointer(file: &Path, bytes: &[u8], prefix: &str) -> io::Result<PathBuf> {
    let value = std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.strip_prefix(prefix))
        .map(str::trim);
    let value = value
        .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.contains(['\n', '\r', '\0']))
        .ok_or_else(|| io::Error::other("invalid bounded Git indirection"))?;
    Ok(file
        .parent()
        .ok_or_else(|| io::Error::other("invalid Git marker"))?
        .join(value))
}
pub(super) fn capture(cwd: &AbsoluteSystemPath, flags: FutureFlags) -> Result<Inputs, Error> {
    let cwd = cwd.to_realpath()?;
    let mut inputs = Inputs::default();
    for dir in cwd.ancestors() {
        inputs
            .directory(dir.as_std_path())
            .map_err(|e| super::super::marker_error(dir, e))?;
        let names = [
            "turbo.json",
            "turbo.jsonc",
            "package.json",
            "pnpm-workspace.yaml",
            ".git",
        ]
        .into_iter()
        .chain(flags.experimental_cargo_workspaces.then_some("Cargo.toml"))
        .chain(
            flags
                .experimental_python_workspaces
                .then_some("pyproject.toml"),
        )
        .chain(flags.experimental_go_workspaces.then_some("go.work"));
        for name in names {
            let path = dir.join_component(name);
            let result = if name == ".git" && path.exists() {
                inputs.git(path.as_std_path())
            } else {
                inputs
                    .record(path.as_std_path(), false, name.starts_with("turbo.json"))
                    .map(|_| ())
            };
            result.map_err(|e| super::super::marker_error(&path, e))?;
        }
        if dir.join_component(".git").exists() {
            break;
        }
    }
    Ok(inputs)
}
