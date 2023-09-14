use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::package_manager::{Error, PackageManager};

const GLOBAL_BUNFIG: &'static str = ".bunfig.toml";
const LOCAL_BUNFIG: &'static str = "bunfig.toml";
pub const LOCKFILE: &'static str = "bun.lockb";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Bunfig {
    pub install: Option<Install>,
}

impl Default for Bunfig {
    fn default() -> Self {
        Self {
            install: Some(Default::default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Install {
    pub lockfile: Option<Lockfile>,
}

impl Default for Install {
    fn default() -> Self {
        Self {
            lockfile: Some(Default::default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Lockfile {
    pub path: Option<String>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self {
            path: Some(String::from("bun.lockb")),
        }
    }
}

fn global_bunfig() -> Result<Option<Bunfig>, std::io::Error> {
    // Ordering: https://github.com/oven-sh/bun/blob/75b5c715405d49b5026c13143efecd580d27be1b/src/cli.zig#L303
    if let Some(data_dir) = std::env::var_os("XDG_CONFIG_HOME").or_else(|| std::env::var_os("HOME"))
    {
        process_bunfig_read(fs::read_to_string(
            PathBuf::from(data_dir).join(GLOBAL_BUNFIG),
        ))
    } else {
        Ok(None)
    }
}

fn local_bunfig(repo_root: &AbsoluteSystemPath) -> Result<Option<Bunfig>, std::io::Error> {
    process_bunfig_read(repo_root.join_component(LOCAL_BUNFIG).read_to_string())
}

fn process_bunfig_read(
    read_result: Result<String, std::io::Error>,
) -> Result<Option<Bunfig>, std::io::Error> {
    match read_result {
        Ok(bunfig_contents) => {
            let deserialize_result: Bunfig = toml::from_str(&bunfig_contents).unwrap();
            Ok(Some(deserialize_result))
        }
        Err(error) => {
            if matches!(error.kind(), std::io::ErrorKind::NotFound) {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}

pub fn get_lockfile_path(
    repo_root: &AbsoluteSystemPath,
) -> Result<AbsoluteSystemPathBuf, std::io::Error> {
    let global = global_bunfig()?;
    let local = local_bunfig(repo_root)?;

    let path = match (global, local) {
        (None, None) => None,
        (None, Some(local_bunfig)) => local_bunfig
            .install
            .and_then(|install| install.lockfile)
            .and_then(|lockfile| lockfile.path)
            .and_then(|path| if path.is_empty() { None } else { Some(path) }),
        (Some(global_bunfig), None) => global_bunfig
            .install
            .and_then(|install| install.lockfile)
            .and_then(|lockfile| lockfile.path)
            .and_then(|path| if path.is_empty() { None } else { Some(path) }),
        (Some(global_bunfig), Some(local_bunfig)) => {
            // Bun shallow merges.
            local_bunfig
                .install
                .or(global_bunfig.install)
                .and_then(|install| install.lockfile)
                .and_then(|lockfile| lockfile.path)
                .and_then(|path| if path.is_empty() { None } else { Some(path) })
        }
    };

    match path {
        Some(path) => {
            // This is possibly garbage-in.
            // But if it is garbage-in, we can be garbage out.
            Ok(AbsoluteSystemPathBuf::from_unknown(repo_root, path))
        }
        None => Ok(repo_root.join_component(LOCKFILE)),
    }
}

pub struct BunDetector<'a> {
    repo_root: &'a AbsoluteSystemPath,
    found: bool,
}

impl<'a> BunDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self {
            repo_root,
            found: false,
        }
    }
}

impl<'a> Iterator for BunDetector<'a> {
    type Item = Result<PackageManager, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }

        self.found = true;
        let bunfig_lockfile_path = get_lockfile_path(self.repo_root)
            .unwrap_or_else(|_| self.repo_root.join_component(LOCKFILE));

        if bunfig_lockfile_path.exists() {
            Some(Ok(PackageManager::Bun))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_lockfile_path() {
        let a = AbsoluteSystemPathBuf::new("/Users/nathanhammond/repos/triage/test-bun").unwrap();
        let out = get_lockfile_path(&a);
    }
}
