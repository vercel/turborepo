//! Repository-scoped toolchain installation for `turbo setup`.
//!
//! Turborepo already knows which toolchains a repository uses: the JavaScript
//! package manager declared in `package.json`, the Rust channel pinned in
//! `rust-toolchain.toml`, the `uv` frontend and Python interpreter requested by
//! `pyproject.toml` / `.python-version`, and the Go release named in `go.work`
//! or `go.mod`. Until now every one of those versions was discovered by probing
//! whatever happened to be on `PATH`. This crate turns those declarations into
//! an install plan and materializes the binaries under `.turbo/tools` inside
//! the repository, so a fresh clone can run its tasks without touching the
//! machine's global installs.
//!
//! Layout:
//!
//! ```text
//! .turbo/tools/
//!   manifest.json         what is installed, and the env each tool needs
//!   bin/                  shims / symlinks that `turbo` prepends to PATH
//!   node/<version>/       extracted upstream archives, one dir per version
//!   pnpm/<version>/
//!   go/<version>/
//!   uv/<version>/
//!   rust/{rustup,cargo}/  RUSTUP_HOME and CARGO_HOME for the pinned channel
//!   python/               UV_PYTHON_INSTALL_DIR for uv-managed interpreters
//! ```
//!
//! [`activate`] makes an existing install visible to the current process by
//! prepending `bin/` to `PATH` and exporting the env recorded in the manifest.
//! The shim calls it right after repository inference, so every `which`
//! lookup, hashing probe and task process turbo spawns sees the same tools.

pub mod archive;
pub mod declared;
mod error;
pub mod http;
pub mod install;
pub mod manifest;
pub mod platform;
pub mod shim;
pub mod sources;
pub mod tools;
mod version;

use std::{
    env,
    ffi::{OsStr, OsString},
};

pub use error::Error;
pub use install::{InstallOutcome, InstallStatus, Installer, Reporter};
pub use manifest::{InstalledTool, Manifest};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

/// Path components of the tools directory, relative to the repository root.
pub const TOOLS_DIR_COMPONENTS: [&str; 2] = [".turbo", "tools"];

/// Name of the directory that holds the shims turbo prepends to `PATH`.
pub const BIN_DIR: &str = "bin";

/// Name of the manifest file inside the tools directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// Environment variable [`activate`] sets to the tools directory it applied,
/// so later code (and child processes) can tell which repository's tools are
/// active without repeating the search.
pub const TOOLS_ROOT_ENV: &str = "TURBO_TOOLS_ROOT";

/// The repository-scoped tools directory: `<repo root>/.turbo/tools`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsDir {
    root: AbsoluteSystemPathBuf,
}

impl ToolsDir {
    pub fn new(repo_root: &AbsoluteSystemPath) -> Self {
        Self {
            root: repo_root.join_components(&TOOLS_DIR_COMPONENTS),
        }
    }

    pub fn root(&self) -> &AbsoluteSystemPath {
        &self.root
    }

    pub fn bin_dir(&self) -> AbsoluteSystemPathBuf {
        self.root.join_component(BIN_DIR)
    }

    pub fn manifest_path(&self) -> AbsoluteSystemPathBuf {
        self.root.join_component(MANIFEST_FILE)
    }

    /// Directory a tool version is extracted into, e.g. `node/22.1.0`.
    pub fn tool_dir(&self, tool: &str, version: &str) -> AbsoluteSystemPathBuf {
        self.root.join_components(&[tool, version])
    }

    /// Whether `path` lives inside the tools directory.
    pub fn contains(&self, path: &std::path::Path) -> bool {
        path.starts_with(self.root.as_std_path())
    }

    /// Resolves a `/`-separated path stored in the manifest (relative to the
    /// tools directory) to an absolute path on this system.
    pub fn resolve_relative(&self, relative: &str) -> AbsoluteSystemPathBuf {
        let components: Vec<&str> = relative.split('/').filter(|c| !c.is_empty()).collect();
        self.root.join_components(&components)
    }

    /// Inverse of [`Self::resolve_relative`]: the `/`-separated path of
    /// `path` relative to the tools directory.
    pub fn relative_of(&self, path: &AbsoluteSystemPath) -> Result<String, Error> {
        let anchored = self
            .root
            .anchor(path)
            .map_err(|_| Error::OutsideToolsDir(path.to_string()))?;
        Ok(anchored.to_unix().to_string())
    }

    /// Reads the manifest, returning an empty one when none has been written.
    pub fn read_manifest(&self) -> Result<Manifest, Error> {
        Manifest::read(&self.manifest_path())
    }

    pub fn write_manifest(&self, manifest: &Manifest) -> Result<(), Error> {
        self.root
            .create_dir_all()
            .map_err(|source| Error::io(self.root.as_str(), source))?;
        manifest.write(&self.manifest_path())
    }

    /// Environment the current process (and every child) needs for the
    /// installed tools to be found: `PATH` with `bin/` prepended plus the
    /// per-tool variables recorded in the manifest. Returns `None` when
    /// nothing is installed.
    pub fn activation_env(&self, current_path: Option<&OsStr>) -> Option<Vec<(String, OsString)>> {
        let bin_dir = self.bin_dir();
        if !bin_dir.exists() {
            return None;
        }
        let manifest = self.read_manifest().unwrap_or_default();
        let mut env = Vec::new();
        if let Some(path) = prepend_path(bin_dir.as_std_path(), current_path) {
            env.push(("PATH".to_string(), path));
        }
        for (key, relative) in manifest.env() {
            let value = self.resolve_relative(relative);
            env.push((key.to_string(), OsString::from(value.as_str())));
        }
        env.push((
            TOOLS_ROOT_ENV.to_string(),
            OsString::from(self.root.as_str()),
        ));
        Some(env)
    }

    /// The tools directory of the nearest repository at or above `start`
    /// that has one installed. Used when repository inference has nothing to
    /// anchor on (no root `package.json`), so the invocation directory may be
    /// deep inside a Cargo, uv, or Go workspace.
    pub fn find_nearest(start: &AbsoluteSystemPath) -> Option<Self> {
        start
            .ancestors()
            .map(Self::new)
            .find(|tools| tools.bin_dir().exists())
    }
}

/// Builds a `PATH` value with `bin_dir` in front, or `None` when it already
/// leads the current value so repeated activation stays idempotent.
fn prepend_path(bin_dir: &std::path::Path, current: Option<&OsStr>) -> Option<OsString> {
    let mut entries: Vec<std::path::PathBuf> = current
        .map(|value| env::split_paths(value).collect())
        .unwrap_or_default();
    if entries.first().is_some_and(|first| first == bin_dir) {
        return None;
    }
    entries.retain(|entry| entry != bin_dir);
    entries.insert(0, bin_dir.to_path_buf());
    env::join_paths(entries).ok()
}

/// Makes a repository's installed tools visible to this process by mutating
/// its environment. Returns whether anything was applied.
///
/// This must run early in startup, before any threads that read the
/// environment exist; the shim calls it immediately after repository
/// inference for exactly that reason.
pub fn activate(repo_root: &AbsoluteSystemPath) -> bool {
    apply(&ToolsDir::new(repo_root))
}

/// Like [`activate`], but for the nearest repository at or above `start`
/// with installed tools. Returns the repository root that was activated.
pub fn activate_nearest(start: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
    let tools = ToolsDir::find_nearest(start)?;
    let repo_root = tools.root().parent()?.parent()?.to_owned();
    apply(&tools).then_some(repo_root)
}

fn apply(tools: &ToolsDir) -> bool {
    let Some(env) = tools.activation_env(env::var_os("PATH").as_deref()) else {
        return false;
    };
    for (key, value) in &env {
        tracing::debug!(%key, value = %value.to_string_lossy(), "activating repository tool env");
        // SAFETY: called from the shim during single-threaded startup, matching
        // the existing `TURBO_INVOCATION_DIR` handling there.
        unsafe { env::set_var(key, value) };
    }
    !env.is_empty()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn prepend_path_is_idempotent() {
        let bin = Path::new("/repo/.turbo/tools/bin");
        let joined = env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
        let first = prepend_path(bin, Some(&joined)).unwrap();
        let entries: Vec<_> = env::split_paths(&first).collect();
        assert_eq!(entries[0], bin);
        assert_eq!(entries.len(), 3);
        assert!(prepend_path(bin, Some(&first)).is_none());
    }

    #[test]
    fn prepend_path_moves_existing_entry_to_front() {
        let bin = Path::new("/repo/.turbo/tools/bin");
        let joined = env::join_paths([Path::new("/usr/bin"), bin]).unwrap();
        let result = prepend_path(bin, Some(&joined)).unwrap();
        let entries: Vec<_> = env::split_paths(&result).collect();
        assert_eq!(
            entries,
            vec![bin.to_path_buf(), Path::new("/usr/bin").to_path_buf()]
        );
    }

    #[test]
    fn activation_env_requires_bin_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let tools = ToolsDir::new(&root);
        assert!(tools.activation_env(None).is_none());

        tools.bin_dir().create_dir_all().unwrap();
        let mut manifest = Manifest::default();
        manifest.tools.insert(
            "rust".into(),
            InstalledTool {
                version: "stable".into(),
                source: "rust-toolchain.toml".into(),
                path: "rust".into(),
                bins: vec!["cargo".into()],
                env: [("RUSTUP_HOME".to_string(), "rust/rustup".to_string())].into(),
            },
        );
        tools.write_manifest(&manifest).unwrap();

        let env = tools.activation_env(None).unwrap();
        assert_eq!(env[0].0, "PATH");
        assert_eq!(env[0].1, OsString::from(tools.bin_dir().as_str()));
        assert_eq!(env[1].0, "RUSTUP_HOME");
        assert_eq!(
            env[1].1,
            OsString::from(tools.root().join_components(&["rust", "rustup"]).as_str())
        );
        assert_eq!(env[2].0, TOOLS_ROOT_ENV);
        assert_eq!(env[2].1, OsString::from(tools.root().as_str()));
    }

    #[test]
    fn find_nearest_walks_up_to_an_installed_tools_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let nested = root.join_components(&["crates", "a", "src"]);
        nested.create_dir_all().unwrap();
        assert!(ToolsDir::find_nearest(&nested).is_none());

        ToolsDir::new(&root).bin_dir().create_dir_all().unwrap();
        let found = ToolsDir::find_nearest(&nested).unwrap();
        assert_eq!(found, ToolsDir::new(&root));
        assert_eq!(ToolsDir::find_nearest(&root).unwrap(), ToolsDir::new(&root));
    }
}
