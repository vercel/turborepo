//! Fail-closed official-only precursor, not mirror/configuration support.
//! No subprocesses or network. Call before constructing either transport and
//! before mutation. Recheck if discovery/environment changes before traffic.

use std::{
    ffi::OsString,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use turborepo_types::CONFIG_FILES;

use crate::registry_metadata::PUBLIC_NPM_REGISTRY;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigOwner {
    Repository,
    Ancestor,
    User,
    System,
    Prefix,
    RootPolicy,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(
        "official-only setup rejects npm configuration environment overrides; remove them before \
         retrying"
    )]
    Environment,
    #[error("official-only source preflight is not qualified on non-Unix hosts")]
    UnsupportedHost,
    #[error(
        "official-only setup rejects {0:?} configuration, including empty npmrc files; remove it \
         or wait for mirror/policy support"
    )]
    Configured(ConfigOwner),
    #[error(
        "cannot safely inspect {0:?} configuration (unreadable, non-file, link/reparse point or \
         invalid policy); no public traffic is allowed"
    )]
    Inspection(ConfigOwner),
    #[error(
        "cannot establish configuration locations; supply absolute invocation and trusted current \
         Node paths, using a standard bin/node layout"
    )]
    Locations,
}

/// Opaque permission to construct official transports. No public fixture
/// constructor, URL override, CLI flag or environment opt-out exists.
pub struct OfficialSourcePolicy(());
impl OfficialSourcePolicy {
    /// `current_node` is the wrapper's trusted CURRENT_NODE provenance, NOT a
    /// task executable or a PATH probe. None means native standalone bootstrap.
    /// Known system prefixes plus PREFIX and the current Node install prefix
    /// are inspected. Unreported custom npm global prefixes/config locations
    /// are unsupported: do not call this for a wrapper with unknown provenance.
    /// Bundled npmrc is also rejected: it can redirect npm's effective prefix.
    /// Non-Unix hosts remain explicitly unqualified.
    /// Every NPM_CONFIG_* override (including prefix/globalconfig/userconfig),
    /// regardless of spelling/value, is rejected before filesystem inspection.
    pub fn inspect(invocation: &Path, current_node: Option<&Path>) -> Result<Self, Error> {
        if !cfg!(unix) {
            return Err(Error::UnsupportedHost);
        }
        let env: Vec<_> = std::env::vars_os().collect();
        reject_environment(&env)?;
        let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let home = env
            .iter()
            .find(|(key, _)| key == home_key)
            .map(|(_, value)| PathBuf::from(value))
            .ok_or(Error::Locations)?;
        inspect_inputs(
            invocation,
            &home,
            current_node,
            &env,
            system_configs(),
            None,
        )
    }
    pub fn registry(&self) -> &'static str {
        PUBLIC_NPM_REGISTRY
    }
}

fn reject_environment(env: &[(OsString, OsString)]) -> Result<(), Error> {
    if env.iter().any(|(key, _)| {
        let key = key.to_string_lossy().to_ascii_uppercase();
        key.starts_with("NPM_CONFIG_")
            || matches!(
                key.as_str(),
                "COREPACK_NPM_REGISTRY" | "NODEJS_ORG_MIRROR" | "NVM_NODEJS_ORG_MIRROR" | "DESTDIR"
            )
    }) {
        return Err(Error::Environment);
    }
    Ok(())
}

fn system_configs() -> Vec<PathBuf> {
    if cfg!(unix) {
        [
            "/etc/npmrc",
            "/usr/etc/npmrc",
            "/usr/local/etc/npmrc",
            "/opt/homebrew/etc/npmrc",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    } else {
        Vec::new()
    }
}

// Owned inputs are the fixture seam. Only this module's tests can substitute
// host environment/home/system policy; production always captures real inputs.
fn inspect_inputs(
    invocation: &Path,
    home: &Path,
    current_node: Option<&Path>,
    env: &[(OsString, OsString)],
    system: Vec<PathBuf>,
    boundary: Option<&Path>,
) -> Result<OfficialSourcePolicy, Error> {
    reject_environment(env)?;
    if !invocation.is_absolute() || !home.is_absolute() {
        return Err(Error::Locations);
    }
    let mut paths = vec![(home.join(".npmrc"), ConfigOwner::User)];
    paths.extend(system.into_iter().flat_map(|path| {
        let bundled = path.parent().and_then(Path::parent).map(|prefix| {
            (
                prefix.join("lib/node_modules/npm/npmrc"),
                ConfigOwner::System,
            )
        });
        std::iter::once((path, ConfigOwner::System)).chain(bundled)
    }));
    let actual = fs::canonicalize(invocation).map_err(|_| Error::Locations)?;
    let actual_boundary = boundary
        .map(fs::canonicalize)
        .transpose()
        .map_err(|_| Error::Locations)?;
    for base in [invocation, actual.as_path()] {
        for (index, ancestor) in base.ancestors().enumerate() {
            paths.push((
                ancestor.join(".npmrc"),
                if index == 0 {
                    ConfigOwner::Repository
                } else {
                    ConfigOwner::Ancestor
                },
            ));
            paths.extend(CONFIG_FILES.map(|name| (ancestor.join(name), ConfigOwner::RootPolicy)));
            if boundary == Some(ancestor) || actual_boundary.as_deref() == Some(ancestor) {
                break;
            }
        }
    }
    for (_, prefix) in env.iter().filter(|(key, _)| key == "PREFIX") {
        let prefix = PathBuf::from(prefix);
        if !prefix.is_absolute() {
            return Err(Error::Locations);
        }
        paths.push((prefix.join("etc/npmrc"), ConfigOwner::Prefix));
    }
    if let Some(node) = current_node {
        // Cover both a symlink's selected location and its actual install prefix.
        for node in [
            node.to_path_buf(),
            fs::canonicalize(node).map_err(|_| Error::Locations)?,
        ] {
            if !node.is_absolute() {
                return Err(Error::Locations);
            }
            let bin = node.parent().ok_or(Error::Locations)?;
            let prefix = if cfg!(windows) && node.file_name().is_some_and(|name| name == "node.exe")
            {
                bin
            } else if node.file_name().is_some_and(|name| name == "node")
                && bin.file_name().is_some_and(|name| name == "bin")
            {
                bin.parent().ok_or(Error::Locations)?
            } else {
                return Err(Error::Locations);
            };
            paths.push((prefix.join("etc/npmrc"), ConfigOwner::Prefix));
            paths.push((
                prefix.join("lib/node_modules/npm/npmrc"),
                ConfigOwner::Prefix,
            ));
        }
    }
    for (path, owner) in paths {
        validate(owner, observe(&path, owner))?;
    }
    Ok(OfficialSourcePolicy(()))
}

#[derive(Clone, Copy)]
enum Observation {
    Missing,
    Readable,
    PolicyAbsent,
    Unsafe,
}

fn observe(path: &Path, owner: ConfigOwner) -> Observation {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Observation::Missing,
        Err(_) => return Observation::Unsafe,
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Observation::Unsafe;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Observation::Unsafe;
        }
    }
    let Ok(mut file) = fs::File::open(path) else {
        return Observation::Unsafe;
    };
    if owner != ConfigOwner::RootPolicy {
        return if file.read(&mut [0u8; 1]).is_ok() {
            Observation::Readable
        } else {
            Observation::Unsafe
        };
    }
    let mut text = String::new();
    if file.take(1_048_577).read_to_string(&mut text).is_err() || text.len() > 1_048_576 {
        return Observation::Unsafe;
    }
    let Ok(parsed) = jsonc_parser::parse_to_ast(&text, &Default::default(), &Default::default())
    else {
        return Observation::Unsafe;
    };
    let Some(jsonc_parser::ast::Value::Object(object)) = parsed.value else {
        return Observation::Unsafe;
    };
    if object
        .properties
        .iter()
        .any(|property| property.name.as_str() == "setup")
    {
        Observation::Readable
    } else {
        Observation::PolicyAbsent
    }
}

// Pure policy decision: no reads, probes, URLs, environment mutation or
// secrets.
fn validate(owner: ConfigOwner, observation: Observation) -> Result<(), Error> {
    match observation {
        Observation::Missing | Observation::PolicyAbsent => Ok(()),
        Observation::Readable => Err(Error::Configured(owner)),
        Observation::Unsafe => Err(Error::Inspection(owner)),
    }
}

#[cfg(test)]
mod tests;
