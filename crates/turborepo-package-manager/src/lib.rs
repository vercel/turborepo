//! Shared package-manager families and borrowed declaration structure.
//!
//! No version resolution, integrity interpretation, lockfile/workspace flavor,
//! installation capabilities, filesystem access, or runtime invocation lives
//! here. Consumers retain their own grammar, bounds, policy, and diagnostics.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Aube,
    Bun,
    Npm,
    Nub,
    Pnpm,
    Yarn,
}

impl Family {
    /// Canonical names, in the legacy declaration-pattern order.
    pub const ALL: [Self; 6] = [
        Self::Aube,
        Self::Bun,
        Self::Npm,
        Self::Nub,
        Self::Pnpm,
        Self::Yarn,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Aube => "aube",
            Self::Bun => "bun",
            Self::Npm => "npm",
            Self::Nub => "nub",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|family| family.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecError {
    ExpectedNameAtVersion,
    UnknownName,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ExpectedNameAtVersion => "expected a name@version declaration",
            Self::UnknownName => "unknown package-manager name",
        })
    }
}

impl std::error::Error for SpecError {}

/// Borrowed `packageManager` structure; version syntax belongs to the consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec<'a> {
    pub family: Family,
    pub name: &'a str,
    pub version: &'a str,
}

pub fn parse_spec(input: &str) -> Result<Spec<'_>, SpecError> {
    let (name, version) = input
        .split_once('@')
        .ok_or(SpecError::ExpectedNameAtVersion)?;
    let family = Family::parse(name).ok_or(SpecError::UnknownName)?;
    Ok(Spec {
        family,
        name,
        version,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryError {
    ExpectedObject,
    MissingName,
    NameNotString,
    EmptyName,
    NameWhitespace,
    UnknownName,
    VersionNotString,
}

impl std::fmt::Display for EntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ExpectedObject => "expected a package-manager object",
            Self::MissingName => "package-manager name is required",
            Self::NameNotString => "package-manager name must be a string",
            Self::EmptyName => "package-manager name must not be empty",
            Self::NameWhitespace => "package-manager name must not contain surrounding whitespace",
            Self::UnknownName => "unknown package-manager name",
            Self::VersionNotString => "package-manager version must be a string",
        })
    }
}

impl std::error::Error for EntryError {}

impl EntryError {
    pub fn field(self) -> Option<&'static str> {
        match self {
            Self::ExpectedObject => None,
            Self::VersionNotString => Some("version"),
            _ => Some("name"),
        }
    }
}

/// One `devEngines.packageManager` object. Raw fields let consumers check
/// capabilities before version types and apply their own failure policy.
/// Absence of a version is not replaced with `*`.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub family: Family,
    pub version: Option<&'a Value>,
    pub on_fail: Option<&'a Value>,
}

impl<'a> Entry<'a> {
    pub fn version(&self) -> Result<Option<&'a str>, EntryError> {
        self.version
            .map(|value| value.as_str().ok_or(EntryError::VersionNotString))
            .transpose()
    }
}

pub fn parse_entry(value: &Value) -> Result<Entry<'_>, EntryError> {
    let object = value.as_object().ok_or(EntryError::ExpectedObject)?;
    let name = object.get("name").ok_or(EntryError::MissingName)?;
    let name = name.as_str().ok_or(EntryError::NameNotString)?;
    if name.is_empty() {
        return Err(EntryError::EmptyName);
    }
    if name.trim() != name {
        return Err(EntryError::NameWhitespace);
    }
    let family = Family::parse(name).ok_or(EntryError::UnknownName)?;
    Ok(Entry {
        family,
        version: object.get("version"),
        on_fail: object.get("onFail"),
    })
}

#[cfg(test)]
mod tests;
