//! Fail-closed source preflight: npm configuration never redirects setup
//! downloads. Only root `setup.mirrors` and the `TURBO_SETUP_*` mirror
//! variables do, so any npmrc, including a committed one whose registry equals
//! a configured mirror, is still rejected rather than parsed.
//! No subprocesses or network. Call before constructing either transport and
//! before mutation. Recheck if discovery/environment changes before traffic.

use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use turborepo_types::CONFIG_FILES;

use crate::registry_metadata::PUBLIC_NPM_REGISTRY;

/// npm's builtin config layer, relative to an install prefix.
// Windows global prefixes have no lib/ level.
const BUNDLED_NPMRC: &str = if cfg!(windows) {
    "node_modules/npm/npmrc"
} else {
    "lib/node_modules/npm/npmrc"
};
/// Homebrew's file is one short line; anything this large is not that shape.
const BUNDLED_NPMRC_LIMIT: u64 = 4096;

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
    #[error("official-only source preflight is not qualified on this host")]
    UnsupportedHost,
    #[error(
        "official-only setup rejects {0:?} configuration, including empty npmrc files; remove it, \
         and configure download mirrors with setup.mirrors or TURBO_SETUP_NODE_MIRROR and \
         TURBO_SETUP_NPM_REGISTRY instead"
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

/// Opaque permission to construct official transports. Owned fixtures are
/// feature-gated for dev-dependencies; production has no URL/root override,
/// CLI flag or environment opt-out.
pub struct OfficialSourcePolicy(());
impl OfficialSourcePolicy {
    /// `current_node` is the wrapper's trusted CURRENT_NODE provenance, NOT a
    /// task executable or a PATH probe. None means native standalone bootstrap.
    /// Known system prefixes plus PREFIX and the current Node install prefix
    /// are inspected. Unreported custom npm global prefixes/config locations
    /// are unsupported: do not call this for a wrapper with unknown provenance.
    /// Bundled npmrc is also rejected because it can redirect npm's effective
    /// prefix, unless its only setting is an absolute `prefix` whose
    /// `etc/npmrc` is inspected here too (Homebrew's Node formulas write
    /// exactly `prefix = $HOMEBREW_PREFIX`).
    /// Windows inspects npm's default global prefix, `%APPDATA%\npm`.
    /// NPM_CONFIG_* variables outside the wrapper allowlist, regardless of
    /// spelling/value, are rejected before filesystem inspection. Registry
    /// keys pass only at their official defaults. Reported
    /// prefix/globalconfig/userconfig locations are inspected, never trusted.
    pub fn inspect(invocation: &Path, current_node: Option<&Path>) -> Result<Self, Error> {
        if !cfg!(any(unix, windows)) {
            return Err(Error::UnsupportedHost);
        }
        let env: Vec<_> = std::env::vars_os().collect();
        environment_configs(&env)?;
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
            system_configs(&env)?,
            None,
        )
    }
    pub fn registry(&self) -> &'static str {
        PUBLIC_NPM_REGISTRY
    }
}

/// `npm_config_*` keys that `npx`, `npm exec`, `npm run`, and pnpm export by
/// default or for common invocation flags. None of them selects a registry,
/// proxy, TLS setting, or credential.
const WRAPPER_NPM_CONFIG: &[&str] = &[
    "cache",
    "call",
    "frozen_lockfile",
    "init_module",
    "loglevel",
    "node_gyp",
    "npm_version",
    "package",
    "script_shell",
    "user_agent",
    "verify_deps_before_run",
    "yes",
];

/// Registry keys that `pnpm run` (pnpm 10 and earlier) exports from its
/// defaults, accepted only at exactly those official values. `_jsr_registry`
/// is pnpm's `@jsr:registry`, which setup never uses.
const WRAPPER_DEFAULT_REGISTRIES: &[(&str, &str)] = &[
    ("registry", PUBLIC_NPM_REGISTRY),
    ("_jsr_registry", "https://npm.jsr.io"),
];

/// Only scheme/host case and one trailing slash are normalized; any port,
/// credential, path, query, or whitespace makes the value non-default.
fn is_official_registry(value: &OsStr, official: &str) -> bool {
    value.to_str().is_some_and(|value| {
        value
            .strip_suffix('/')
            .unwrap_or(value)
            .eq_ignore_ascii_case(official)
    })
}

/// Rejects every source-changing environment variable and returns the npm
/// configuration files that wrapper-reported locations point to. Callers must
/// inspect those files like any other configuration.
fn environment_configs(env: &[(OsString, OsString)]) -> Result<Vec<(PathBuf, ConfigOwner)>, Error> {
    let mut paths = Vec::new();
    for (key, value) in env {
        let key = key.to_string_lossy().to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "corepack_npm_registry" | "nodejs_org_mirror" | "nvm_nodejs_org_mirror" | "destdir"
        ) {
            return Err(Error::Environment);
        }
        // npm treats `-` and non-leading `_` in environment keys alike.
        let Some(name) = key.strip_prefix("npm_config_").map(|n| n.replace('-', "_")) else {
            continue;
        };
        // npm exports an empty `noproxy` by default; any value is a proxy setting.
        if WRAPPER_NPM_CONFIG.contains(&name.as_str())
            || (name == "noproxy" && value.is_empty())
            || WRAPPER_DEFAULT_REGISTRIES
                .iter()
                .any(|(key, official)| name == *key && is_official_registry(value, official))
        {
            continue;
        }
        let location = Path::new(value);
        if !location.is_absolute() {
            return Err(Error::Environment);
        }
        paths.push(match name.as_str() {
            "userconfig" => (location.to_owned(), ConfigOwner::User),
            "globalconfig" | "npm_globalconfig" => (location.to_owned(), ConfigOwner::System),
            "prefix" | "global_prefix" => (location.join("etc/npmrc"), ConfigOwner::Prefix),
            "local_prefix" => (location.join(".npmrc"), ConfigOwner::Repository),
            _ => return Err(Error::Environment),
        });
    }
    Ok(paths)
}

fn system_configs(env: &[(OsString, OsString)]) -> Result<Vec<PathBuf>, Error> {
    if cfg!(windows) {
        // Node's Windows installers set npm's global prefix to %APPDATA%\npm.
        let app_data = env
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("APPDATA"))
            .map(|(_, value)| PathBuf::from(value))
            .filter(|path| path.is_absolute())
            .ok_or(Error::Locations)?;
        return Ok(vec![app_data.join("npm").join("etc").join("npmrc")]);
    }
    Ok([
        "/etc/npmrc",
        "/usr/etc/npmrc",
        "/usr/local/etc/npmrc",
        "/opt/homebrew/etc/npmrc",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect())
}

// Owned inputs are the fixture seam. Only tests/test-support can substitute
// host environment/home/system policy; production always captures real inputs.
fn inspect_inputs(
    invocation: &Path,
    home: &Path,
    current_node: Option<&Path>,
    env: &[(OsString, OsString)],
    system: Vec<PathBuf>,
    boundary: Option<&Path>,
) -> Result<OfficialSourcePolicy, Error> {
    let reported = environment_configs(env)?;
    if !invocation.is_absolute() || !home.is_absolute() {
        return Err(Error::Locations);
    }
    let mut paths = vec![(home.join(".npmrc"), ConfigOwner::User)];
    paths.extend(system.into_iter().flat_map(|path| {
        let bundled = path
            .parent()
            .and_then(Path::parent)
            .map(|prefix| (prefix.join(BUNDLED_NPMRC), ConfigOwner::System));
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
            paths.push((prefix.join(BUNDLED_NPMRC), ConfigOwner::Prefix));
        }
    }
    paths.extend(reported);
    let inspected: Vec<PathBuf> = paths.iter().map(|(path, _)| path.clone()).collect();
    for (path, owner) in paths {
        // Only owned fixtures have a boundary. Check the closest existing
        // parent before observation so an intermediate directory symlink cannot
        // cause even a missing config lookup to inspect the contributor's host.
        if let Some(boundary) = actual_boundary.as_deref() {
            let mut parent = path.parent();
            loop {
                let candidate = parent.ok_or(Error::Inspection(owner))?;
                match fs::canonicalize(candidate) {
                    Ok(actual) if actual.starts_with(boundary) => break,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        parent = candidate.parent();
                    }
                    _ => return Err(Error::Inspection(owner)),
                }
            }
        }
        let observation = match observe(&path, owner) {
            // npm reads its global config from `<prefix>/etc/npmrc`, so this
            // prefix only matters through a file this loop also validates.
            Observation::BundledPrefix(prefix) if inspected.contains(&prefix.join("etc/npmrc")) => {
                Observation::Missing
            }
            observation => observation,
        };
        validate(owner, observation)?;
    }
    Ok(OfficialSourcePolicy(()))
}

#[derive(Clone)]
enum Observation {
    Missing,
    Readable,
    /// Bundled npmrc whose only setting is this absolute `prefix`.
    BundledPrefix(PathBuf),
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
    if owner != ConfigOwner::RootPolicy && path.ends_with(BUNDLED_NPMRC) {
        let mut bytes = Vec::new();
        return match file.take(BUNDLED_NPMRC_LIMIT + 1).read_to_end(&mut bytes) {
            Ok(_) => {
                bundled_prefix(&bytes).map_or(Observation::Readable, Observation::BundledPrefix)
            }
            Err(_) => Observation::Unsafe,
        };
    }
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
        .any(|property| property.name.as_str() == "setup" && !applied_policy(&property.value))
    {
        Observation::Readable
    } else {
        Observation::PolicyAbsent
    }
}

/// Recognizes only the single `prefix = /absolute/path` line Homebrew writes.
/// Anything npm's ini parser could read differently is not recognized and
/// stays rejected: non-ASCII bytes, other keys, repeated or array keys,
/// sections, quotes, escapes, inline comments, `${}` or `~` expansion, and
/// relative or non-normalized paths.
fn bundled_prefix(bytes: &[u8]) -> Option<PathBuf> {
    if bytes.len() as u64 > BUNDLED_NPMRC_LIMIT || !bytes.is_ascii() {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let mut settings = text
        .split(['\r', '\n'])
        .map(|line| line.trim_matches(|c: char| c.is_ascii_whitespace()))
        .filter(|line| !line.is_empty() && !line.starts_with([';', '#']));
    let (key, value) = settings.next()?.split_once('=')?;
    let value = value.trim_matches(|c: char| c.is_ascii_whitespace());
    if settings.next().is_some()
        || key.trim_matches(|c: char| c.is_ascii_whitespace()) != "prefix"
        || value.contains([';', '#', '\\', '"', '\'', '$'])
    {
        return None;
    }
    let prefix = Path::new(value);
    let mut components = prefix.components();
    (components.next() == Some(Component::RootDir)
        && components.all(|part| matches!(part, Component::Normal(_))))
    .then(|| prefix.to_path_buf())
}

/// Setup applies these root policy keys. Only `mirrors` configures download
/// sources, and setup routes those through validated transports. Any other
/// shape, including an empty policy, is not applied yet.
fn applied_policy(setup: &jsonc_parser::ast::Value) -> bool {
    use jsonc_parser::ast::Value;
    const VALUES: [&str; 3] = ["tools", "dependencies", "lifecycleScripts"];
    const TOOLCHAINS: [&str; 4] = ["javascript", "rust", "python", "go"];
    const MIRRORS: [&str; 2] = ["node", "npmRegistry"];
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
        // `additionalTools` declares tools no built-in adapter manages.
        value(name, &p.value)
            || toolchain(name, &p.value)
            || name == "additionalTools"
            || (name == "mirrors" && strings(&MIRRORS, &p.value))
    }))
}

// Pure policy decision: no reads, probes, URLs, environment mutation or
// secrets.
fn validate(owner: ConfigOwner, observation: Observation) -> Result<(), Error> {
    match observation {
        Observation::Missing | Observation::PolicyAbsent => Ok(()),
        Observation::Readable | Observation::BundledPrefix(_) => Err(Error::Configured(owner)),
        Observation::Unsafe => Err(Error::Inspection(owner)),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;
