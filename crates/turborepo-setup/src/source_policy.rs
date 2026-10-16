//! Tool-download preflight. Only root `setup.mirrors` and `TURBO_SETUP_*`
//! variables select mirrors; ecosystem configuration is left to native package
//! managers and is never inspected here. Transports independently validate
//! their approved sources and verify artifacts before installation.
//! No subprocesses or network. Call before constructing transports and before
//! mutation; recheck if discovery changes before traffic.

#[cfg(any(test, feature = "test-support"))]
use std::path::PathBuf;
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

use turborepo_types::CONFIG_FILES;

use crate::registry_metadata::PUBLIC_NPM_REGISTRY;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigOwner {
    RootPolicy,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("tool-download source preflight is not qualified on this host")]
    UnsupportedHost,
    #[error("tool-download setup rejects unsupported {0:?} configuration")]
    Configured(ConfigOwner),
    #[error(
        "cannot safely inspect {0:?} configuration (unreadable, non-file, link/reparse point or \
         invalid policy); no tool-download traffic is allowed"
    )]
    Inspection(ConfigOwner),
    #[error("cannot establish setup configuration locations; supply an absolute invocation path")]
    Locations,
}

/// Permission to construct tool transports after checking setup policy. Each
/// transport approves its official origin or an explicitly configured mirror;
/// this token does not authorize sources from native package-manager config.
pub struct ToolDownloadPolicy(());
impl ToolDownloadPolicy {
    pub fn inspect(invocation: &Path) -> Result<Self, Error> {
        if !cfg!(any(unix, windows)) {
            return Err(Error::UnsupportedHost);
        }
        inspect_inputs(invocation, None)
    }

    pub fn registry(&self) -> &'static str {
        PUBLIC_NPM_REGISTRY
    }
}

// Only owned fixtures supply a boundary, so their ancestor checks cannot read
// configuration outside the fixture. Production checks lexical and canonical
// invocation ancestors without consulting home, Node, or ecosystem settings.
fn inspect_inputs(invocation: &Path, boundary: Option<&Path>) -> Result<ToolDownloadPolicy, Error> {
    if !invocation.is_absolute() {
        return Err(Error::Locations);
    }
    if let Some(boundary) = boundary
        && (!boundary.is_absolute()
            || invocation
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir)))
    {
        return Err(Error::Locations);
    }
    let actual = fs::canonicalize(invocation).map_err(|_| Error::Locations)?;
    let actual_boundary = boundary
        .map(fs::canonicalize)
        .transpose()
        .map_err(|_| Error::Locations)?;
    if let Some(boundary) = actual_boundary.as_deref()
        && (!invocation.starts_with(boundary) || !actual.starts_with(boundary))
    {
        return Err(Error::Locations);
    }
    for base in [invocation, actual.as_path()] {
        for ancestor in base.ancestors() {
            if let Some(boundary) = actual_boundary.as_deref()
                && !fs::canonicalize(ancestor)
                    .map_err(|_| Error::Locations)?
                    .starts_with(boundary)
            {
                return Err(Error::Locations);
            }
            for name in CONFIG_FILES {
                validate(observe(&ancestor.join(name)))?;
            }
            if boundary == Some(ancestor) || actual_boundary.as_deref() == Some(ancestor) {
                break;
            }
        }
    }
    Ok(ToolDownloadPolicy(()))
}

#[derive(Clone)]
enum Observation {
    Missing,
    Readable,
    PolicyAbsent,
    Unsafe,
}

fn observe(path: &Path) -> Observation {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Observation::Missing,
        Err(_) => return Observation::Unsafe,
    };
    if !metadata.is_file() {
        return Observation::Unsafe;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Observation::Unsafe;
        }
    }
    let Ok(file) = fs::File::open(path) else {
        return Observation::Unsafe;
    };
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
        .any(|property| property.name.as_str() == "setup" && !applied_policy(&property.value))
    {
        Observation::Readable
    } else {
        Observation::PolicyAbsent
    }
}

/// Setup applies these root policy keys. Only `mirrors` configures download
/// sources, and setup routes those through validated transports. Any other
/// shape, including an empty policy, is not applied yet.
fn applied_policy(setup: &jsonc_parser::ast::Value) -> bool {
    use jsonc_parser::ast::Value;
    const VALUES: [&str; 3] = ["tools", "dependencies", "lifecycleScripts"];
    const TOOLCHAINS: [&str; 4] = ["javascript", "rust", "python", "go"];
    const MIRRORS: [&str; 7] = ["node", "npmRegistry", "bun", "uv", "python", "go", "rust"];
    let strings = |keys: &[&str], value: &Value| {
        matches!(value, Value::Object(object) if object.properties.iter().all(|p| {
            keys.contains(&p.name.as_str()) && matches!(p.value, Value::StringLit(_))
        }))
    };
    let value =
        |name: &str, value: &Value| VALUES.contains(&name) && matches!(value, Value::StringLit(_));
    let toolchain =
        |name: &str, toolchain: &Value| TOOLCHAINS.contains(&name) && strings(&VALUES, toolchain);
    matches!(setup, Value::Object(object) if !object.properties.is_empty()
    && object.properties.iter().all(|p| {
        let name = p.name.as_str();
        value(name, &p.value)
            || toolchain(name, &p.value)
            || name == "additionalTools"
            || (name == "mirrors" && strings(&MIRRORS, &p.value))
    }))
}

fn validate(observation: Observation) -> Result<(), Error> {
    match observation {
        Observation::Missing | Observation::PolicyAbsent => Ok(()),
        Observation::Readable => Err(Error::Configured(ConfigOwner::RootPolicy)),
        Observation::Unsafe => Err(Error::Inspection(ConfigOwner::RootPolicy)),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;
