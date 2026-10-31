//! What each tool exports into an environment's `bin`, and the runtime
//! variables its programs need, as data. The store renders Unix launchers and
//! Windows forwarders from it, and activation resolves the same variables, so
//! a launcher and a managed task always agree.

use std::collections::BTreeMap;
#[cfg(not(windows))]
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::{Error, Tool, artifact_component, component, relative};

/// A `/`-separated path inside one selected tool's tree. An empty path names
/// the tree itself, which only variables use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeRef {
    pub tool: String,
    pub path: String,
}

impl TreeRef {
    pub fn new(tool: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            path: path.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Export {
    /// A file in the owning tool's tree. Unix: a relative symlink. Windows:
    /// an `.exe` hard link or a `.cmd` forwarder.
    File(String),
    /// A generated launcher that runs `program` with leading `args`, which may
    /// name other selected trees. On Unix it also sets the owning tool's
    /// `vars`, so it works without managed activation. Windows renders a
    /// `.cmd` forwarder and leaves variables to activation.
    Launch(Launch),
}

impl From<&str> for Export {
    fn from(path: &str) -> Self {
        Self::File(path.to_owned())
    }
}

impl From<String> for Export {
    fn from(path: String) -> Self {
        Self::File(path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub program: TreeRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<LaunchArg>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LaunchArg {
    Tree(TreeRef),
    Word(String),
}

/// Every variable managed activation may set. The shell hook restores only
/// these, so a tampered hook state cannot touch any other variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ManagedVar {
    CargoHome,
    Goroot,
    Gotoolchain,
    RustupAutoInstall,
    RustupHome,
    RustupToolchain,
    UvPython,
    UvPythonDownloads,
    UvPythonInstallDir,
    UvPythonPreference,
}

impl ManagedVar {
    pub const ALL: [Self; 10] = [
        Self::CargoHome,
        Self::Goroot,
        Self::Gotoolchain,
        Self::RustupAutoInstall,
        Self::RustupHome,
        Self::RustupToolchain,
        Self::UvPython,
        Self::UvPythonDownloads,
        Self::UvPythonInstallDir,
        Self::UvPythonPreference,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::CargoHome => "CARGO_HOME",
            Self::Goroot => "GOROOT",
            Self::Gotoolchain => "GOTOOLCHAIN",
            Self::RustupAutoInstall => "RUSTUP_AUTO_INSTALL",
            Self::RustupHome => "RUSTUP_HOME",
            Self::RustupToolchain => "RUSTUP_TOOLCHAIN",
            Self::UvPython => "UV_PYTHON",
            Self::UvPythonDownloads => "UV_PYTHON_DOWNLOADS",
            Self::UvPythonInstallDir => "UV_PYTHON_INSTALL_DIR",
            Self::UvPythonPreference => "UV_PYTHON_PREFERENCE",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|var| var.name() == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VarValue {
    /// The absolute path of a tree path. The tool need not be selected: uv
    /// without a managed Python still points at the empty `python` location
    /// so it never uses the user's uv-managed interpreters.
    Tree(TreeRef),
    Word(String),
    /// The repository's private Cargo home in the user's tool store. Only
    /// activation sets it: environments are shared across repositories.
    RepoCargoHome,
}

impl Export {
    /// File exports of an artifact's `name -> path` executables.
    pub fn files(paths: &BTreeMap<String, String>) -> BTreeMap<String, Export> {
        paths
            .iter()
            .map(|(name, path)| (name.clone(), Export::File(path.clone())))
            .collect()
    }

    /// The path of a file export.
    pub fn as_file(&self) -> Option<&str> {
        match self {
            Export::File(path) => Some(path),
            Export::Launch(_) => None,
        }
    }

    /// Each tree file the export runs or passes, as (tool, path, executed).
    pub(crate) fn tree_files<'a>(&'a self, owner: &'a str) -> Vec<(&'a str, &'a str, bool)> {
        match self {
            Export::File(path) => vec![(owner, path.as_str(), true)],
            Export::Launch(launch) => std::iter::once((
                launch.program.tool.as_str(),
                launch.program.path.as_str(),
                true,
            ))
            .chain(launch.args.iter().filter_map(|arg| match arg {
                LaunchArg::Tree(tree) => Some((tree.tool.as_str(), tree.path.as_str(), false)),
                LaunchArg::Word(_) => None,
            }))
            .collect(),
        }
    }
}

/// Checks one tool's exports and variables in isolation. References to other
/// tools are checked against the whole selection by the caller.
pub(crate) fn validate(tool: &Tool) -> Result<(), Error> {
    for export in tool.executables.values() {
        match export {
            Export::File(path) => relative(path)?,
            Export::Launch(launch) => {
                for (tool, path, _) in export.tree_files("") {
                    component(tool)?;
                    script_path(path)?;
                }
                for arg in &launch.args {
                    if let LaunchArg::Word(word) = arg {
                        script_word(word)?;
                    }
                }
            }
        }
    }
    for value in tool.vars.values() {
        match value {
            VarValue::Tree(tree) => {
                component(&tree.tool)?;
                if !tree.path.is_empty() {
                    script_path(&tree.path)?;
                }
            }
            VarValue::Word(word) => script_word(word)?,
            VarValue::RepoCargoHome => {}
        }
    }
    Ok(())
}

/// Paths rendered into scripts use a spelling that needs no quoting in sh or
/// cmd, so neither `'` nor `%`/`!` can change what a script runs.
fn script_path(path: &str) -> Result<(), Error> {
    relative(path)?;
    if !path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_./+@".contains(&b))
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

fn script_word(word: &str) -> Result<(), Error> {
    if word.is_empty()
        || word.len() > 256
        || !word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
    {
        return Err(Error::InvalidInventory);
    }
    artifact_component(word)
}

/// A launcher in an environment's `bin`. It resolves the store's physical
/// directory from its own path, so tree paths stay relative to the store and
/// the launcher works when run through the stable per-repository link,
/// without any inherited environment. `tree` names a tool's tree root
/// relative to the store. `CARGO_HOME` is per
/// repository, so a launcher shared by every repository never sets it.
#[cfg(not(windows))]
pub(crate) fn unix_launcher(
    tool: &Tool,
    launch: &Launch,
    tree: impl Fn(&str) -> String,
) -> Result<Vec<u8>, Error> {
    let path = |reference: &TreeRef| match reference.path.as_str() {
        "" => format!("\"$store\"'/{}'", tree(&reference.tool)),
        path => format!("\"$store\"'/{}/{path}'", tree(&reference.tool)),
    };
    let mut script = String::from(
        "#!/bin/sh\ncase \"$0\" in */*) ;; *) exit 126;; esac\nstore=$(CDPATH= cd -P -- \
         \"${0%/*}/../../..\" && pwd) || exit 126\n",
    );
    let mut names = Vec::new();
    for (var, value) in &tool.vars {
        let value = match value {
            VarValue::Tree(reference) => path(reference),
            VarValue::Word(word) => format!("'{word}'"),
            VarValue::RepoCargoHome => continue,
        };
        let _ = writeln!(script, "{}={value}", var.name());
        names.push(var.name());
    }
    if !names.is_empty() {
        let _ = writeln!(script, "export {}", names.join(" "));
    }
    script.push_str("exec ");
    script.push_str(&path(&launch.program));
    for arg in &launch.args {
        script.push(' ');
        match arg {
            LaunchArg::Tree(reference) => script.push_str(&path(reference)),
            LaunchArg::Word(word) => {
                let _ = write!(script, "'{word}'");
            }
        }
    }
    script.push_str(" \"$@\"\n");
    Ok(script.into_bytes())
}

/// Transfers control to `program`, preserving `%*` and its exit status.
/// `tree` names a selected tool's absolute tree root: `%~dp0` would be the
/// stable junction's path, not the environment's, so paths are absolute.
#[cfg(windows)]
pub(crate) fn windows_forwarder(
    launch: &Launch,
    tree: impl Fn(&str) -> Option<String>,
) -> Result<Vec<u8>, Error> {
    let path = |reference: &TreeRef| -> Result<String, Error> {
        let root = tree(&reference.tool).ok_or(Error::InvalidInventory)?;
        Ok(format!("\"{root}\\{}\"", reference.path.replace('/', "\\")))
    };
    let mut line = format!("@{}", path(&launch.program)?);
    for arg in &launch.args {
        line.push(' ');
        match arg {
            LaunchArg::Tree(reference) => line.push_str(&path(reference)?),
            LaunchArg::Word(word) => line.push_str(word),
        }
    }
    line.push_str(" %*\r\n");
    Ok(line.into_bytes())
}
