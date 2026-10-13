//! Root package.json discovery for setup/turbo.lock, without graph or I/O.
//! `packageManager` is authoritative; devEngines OR alternatives use npm semver
//! and onFail. Mixed manager names require a pin to select an alternative.

use semver::Version;
use serde_json::Value;
use thiserror::Error;
pub use turborepo_package_manager::Family as Manager;
use turborepo_package_manager::{EntryError, SpecError, parse_entry, parse_spec};

use crate::version_request::{MAX_REQUEST_BYTES, VersionRequest, is_valid_release};

const TOP: &str = "package.json#/packageManager";
const DEV: &str = "package.json#/devEngines/packageManager";
const MAX_ALTERNATIVES: usize = 32;
const MAX_VERSION_FIELD_BYTES: usize = MAX_REQUEST_BYTES + "+sha512.".len() + 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precedence {
    Manager,
    DevEngines,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFail {
    Error,
    Warn,
    Ignore,
}

/// Authored Corepack hex digest; algorithm trust policy is deferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorepackIntegrity {
    pub algorithm: &'static str,
    pub digest: String,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub manager: Manager,
    /// JSON pointer in the root package.json, including array index if present.
    pub source: String,
    /// Canonical version/range without integrity; `None` is unconstrained.
    pub version: Option<String>,
    pub request: Option<VersionRequest>,
    pub integrity: Option<CorepackIntegrity>,
    pub on_fail: OnFail,
}

impl Request {
    fn compatible(&self, other: &Self) -> bool {
        let versions = match (&self.request, &other.request) {
            (Some(a), Some(b)) => a
                .exact_version()
                .map_or_else(|| a.intersects(b), |v| b.matches(v)),
            _ => true,
        };
        self.manager == other.manager
            && versions
            && !matches!((&self.integrity, &other.integrity), (Some(a), Some(b))
                if a.algorithm == b.algorithm && a.digest != b.digest)
    }
}

/// Normalized resolver inputs with all constraints and authored sources.
#[derive(Debug, Clone)]
pub struct Declaration {
    pub manager: Manager,
    pub precedence: Precedence,
    pub package_manager: Option<Request>,
    pub dev_engines: Vec<Request>,
    /// Advisory mismatches known at discovery; ignore emits no diagnostics.
    pub warnings: Vec<Error>,
}

/// npm's array policy belongs to its last entry, not its strictest entry.
fn group_on_fail(alternatives: &[Request]) -> OnFail {
    alternatives.last().map_or(OnFail::Ignore, |r| r.on_fail)
}

impl Declaration {
    /// Build-sensitive pins; artifact checks and range advisories are deferred.
    pub fn matches(&self, release: &Version) -> bool {
        if !is_valid_release(release) {
            return false;
        }
        let matched = self.dev_engines.iter().any(|r| {
            r.manager == self.manager && r.request.as_ref().is_none_or(|v| v.matches(release))
        });
        let Some(pin) = &self.package_manager else {
            return matched;
        };
        pin.request
            .as_ref()
            .is_some_and(|v| v.matches_with_exact(release, |a, b| a == b))
            && (group_on_fail(&self.dev_engines) != OnFail::Error || matched)
    }
}

/// Source-bearing diagnostic: returned as an error or retained as a warning.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("{}: {message}", sources.join(" and "))]
pub struct Error {
    pub sources: Vec<String>,
    pub message: String,
}

fn invalid(source: &str, message: impl Into<String>) -> Error {
    Error {
        sources: vec![source.to_owned()],
        message: message.into(),
    }
}

fn manager(family: Manager, source: &str) -> Result<Manager, Error> {
    match family {
        Manager::Npm | Manager::Pnpm => Ok(family),
        Manager::Aube | Manager::Bun | Manager::Nub | Manager::Yarn => Err(invalid(
            source,
            "unsupported manager; setup supports npm and pnpm",
        )),
    }
}

// Reserve checksum-like prefixes; retain genuine metadata like shared.42.
fn checksum_like(suffix: &str) -> bool {
    let algorithm = suffix.split_once('.').map_or(suffix, |(name, _)| name);
    ["sha", "shake", "md", "blake", "ripemd"].iter().any(|p| {
        algorithm
            .get(..p.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(p))
            && algorithm
                .as_bytes()
                .get(p.len())
                .is_none_or(|b| b.is_ascii_digit() || *b == b'-')
    })
}

fn request(
    manager: Manager,
    version: Option<&str>,
    source: &str,
    on_fail: OnFail,
) -> Result<Request, Error> {
    let mut result = Request {
        manager,
        source: source.to_owned(),
        version: None,
        request: None,
        integrity: None,
        on_fail,
    };
    let Some(version) = version else {
        return Ok(result);
    };
    let pointer = if source == TOP {
        source.to_owned()
    } else {
        format!("{source}/version")
    };
    let bad = |message: String| invalid(&pointer, message);
    if version.len() > MAX_VERSION_FIELD_BYTES {
        return Err(bad("version request exceeds setup's byte limit".into()));
    }
    // VersionRequest bounds the body; integrity has its own exact hex limit.
    if version.contains([':', '/', '\\']) {
        return Err(bad("URLs, paths and aliases are unsupported".into()));
    }
    let mut body = version;
    let mut integrity = None;
    if let Some((prefix, suffix)) = version.rsplit_once('+') {
        if prefix.split('+').skip(1).any(checksum_like) {
            return Err(bad("Corepack integrity requires one trailing checksum on \
                            an exact version"
                .into()));
        }
        let (algorithm, digest) = suffix.split_once('.').unwrap_or((suffix, ""));
        if checksum_like(suffix) {
            let (algorithm, bytes) = match algorithm {
                "sha1" => ("sha1", 40),
                "sha224" => ("sha224", 56),
                "sha256" => ("sha256", 64),
                "sha384" => ("sha384", 96),
                "sha512" => ("sha512", 128),
                _ => return Err(bad("unsupported Corepack checksum algorithm".into())),
            };
            if digest.len() != bytes || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(bad(format!(
                    "{algorithm} integrity requires exactly {bytes} hexadecimal characters"
                )));
            }
            body = prefix;
            integrity = Some(CorepackIntegrity {
                algorithm,
                digest: digest.to_ascii_lowercase(),
            });
        }
    }
    let request = VersionRequest::parse(body).map_err(|e| bad(e.to_string()))?;
    if integrity.is_some() && !request.is_exact() {
        return Err(bad(
            "Corepack integrity requires a single exact version".into()
        ));
    }
    if !request.intersects(&request) {
        return Err(bad("version request admits no versions".into()));
    }
    let version = request
        .exact_version()
        .map_or_else(|| body.trim().to_owned(), ToString::to_string);
    result.version = Some(version);
    result.request = Some(request);
    result.integrity = integrity;
    Ok(result)
}

fn dev_request(value: &Value, source: &str) -> Result<Request, Error> {
    let structural_error = |error: EntryError| {
        let pointer = error
            .field()
            .map_or_else(|| source.to_owned(), |field| format!("{source}/{field}"));
        let message = match error {
            EntryError::ExpectedObject => "expected a package-manager object",
            EntryError::MissingName | EntryError::NameNotString | EntryError::VersionNotString => {
                "required string"
            }
            EntryError::EmptyName | EntryError::NameWhitespace | EntryError::UnknownName => {
                "unsupported manager; setup supports npm and pnpm"
            }
        };
        invalid(&pointer, message)
    };
    let entry = parse_entry(value).map_err(structural_error)?;
    // Capabilities precede version types, which precede onFail and version grammar.
    let manager = manager(entry.family, &format!("{source}/name"))?;
    let version = entry.version().map_err(structural_error)?;
    let on_fail = match entry.on_fail.map(Value::as_str) {
        None | Some(Some("error")) => OnFail::Error,
        Some(Some("warn")) => OnFail::Warn,
        Some(Some("ignore")) => OnFail::Ignore,
        _ => {
            return Err(invalid(
                &format!("{source}/onFail"),
                "expected error, warn, or ignore",
            ));
        }
    };
    request(manager, version, source, on_fail)
}

/// Root declarations only. Missing fields return `None`; malformed data never
/// falls back.
pub fn discover_package_manager(root: &Value) -> Result<Option<Declaration>, Error> {
    let root = root
        .as_object()
        .ok_or_else(|| invalid("package.json#", "expected an object"))?;
    let top = root
        .get("packageManager")
        .map(|value| {
            let text = value
                .as_str()
                .ok_or_else(|| invalid(TOP, "expected name@version string"))?;
            if text.len() > MAX_VERSION_FIELD_BYTES + "pnpm@".len() {
                return Err(invalid(
                    TOP,
                    "package-manager declaration exceeds setup's byte limit",
                ));
            }
            let spec = parse_spec(text).map_err(|error| {
                invalid(
                    TOP,
                    match error {
                        SpecError::ExpectedNameAtVersion => "expected name@version string",
                        SpecError::UnknownName => {
                            "unsupported manager; setup supports npm and pnpm"
                        }
                    },
                )
            })?;
            request(
                manager(spec.family, TOP)?,
                Some(spec.version),
                TOP,
                OnFail::Error,
            )
        })
        .transpose()?;
    let mut dev = Vec::new();
    if let Some(engines) = root.get("devEngines") {
        let engines = engines
            .as_object()
            .ok_or_else(|| invalid("package.json#/devEngines", "expected an object"))?;
        if let Some(value) = engines.get("packageManager") {
            if let Some(array) = value.as_array() {
                if array.is_empty() || array.len() > MAX_ALTERNATIVES {
                    return Err(invalid(DEV, "expected 1..=32 package-manager alternatives"));
                }
                for (index, value) in array.iter().enumerate() {
                    dev.push(dev_request(value, &format!("{DEV}/{index}"))?);
                }
            } else {
                dev.push(dev_request(value, DEV)?);
            }
        }
    }
    let Some(selected) = top.as_ref().or_else(|| dev.first()) else {
        return Ok(None);
    };
    let manager = selected.manager;
    let conflict = |sources| Error {
        sources,
        message: "packageManager does not satisfy devEngines alternatives".to_owned(),
    };
    let mut warnings = Vec::new();
    if let Some(pin) = &top {
        if !dev.is_empty() && !dev.iter().any(|r| pin.compatible(r)) {
            let diagnostic = conflict(top.iter().chain(&dev).map(|r| r.source.clone()).collect());
            match group_on_fail(&dev) {
                OnFail::Error => return Err(diagnostic),
                OnFail::Warn => warnings.push(diagnostic),
                OnFail::Ignore => {}
            }
        }
    } else if dev.iter().any(|r| r.manager != manager) {
        return Err(Error {
            sources: dev.iter().map(|r| r.source.clone()).collect(),
            message: "ambiguous manager alternatives; set packageManager to select one".to_owned(),
        });
    }
    Ok(Some(Declaration {
        manager,
        precedence: if top.is_some() {
            Precedence::Manager
        } else {
            Precedence::DevEngines
        },
        package_manager: top,
        dev_engines: dev,
        warnings,
    }))
}

mod locked_integrity;

#[cfg(test)]
mod tests;
