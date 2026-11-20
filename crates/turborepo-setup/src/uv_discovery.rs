//! Root-only uv version declarations. Precedence follows uv: a root `uv.toml`
//! replaces the `[tool.uv]` table of `pyproject.toml`, even when it does not
//! set `required-version`. A bare version means `==version`, as in uv. User
//! and system uv configuration are never repository policy. No binaries,
//! network or lock writes.

use std::io;

use crate::{
    lock::Declaration,
    pep440::{self, Release, Specifiers},
};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub const SOURCES: [&str; 2] = ["uv.toml", "pyproject.toml"];

#[derive(Debug, thiserror::Error)]
pub enum UvDiscoveryError {
    #[error("cannot read {0}: {1}")]
    Read(&'static str, #[source] io::Error),
    #[error("{0} exceeds the {MAX_CONFIG_BYTES}-byte discovery limit")]
    TooLarge(&'static str),
    #[error("invalid {file}: {reason}")]
    Invalid { file: &'static str, reason: String },
    #[error("unsupported uv request in {file} {field}: {source}")]
    Specifier {
        file: &'static str,
        field: &'static str,
        #[source]
        source: pep440::Error,
    },
}

/// Lock-ready provenance for the single effective uv declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvSource {
    pub file: &'static str,
    pub field: &'static str,
    pub request: String,
}

#[derive(Clone, Debug)]
pub struct UvRequirement {
    source: UvSource,
    specifiers: Specifiers,
}

impl UvRequirement {
    /// `read` returns raw root file bytes (or None when absent). The caller may
    /// supply captured snapshot bytes; limits are enforced here either way.
    pub fn read_with(
        mut read: impl FnMut(&'static str, usize) -> io::Result<Option<Vec<u8>>>,
    ) -> Result<Option<Self>, UvDiscoveryError> {
        for file in SOURCES {
            let Some(bytes) = read(file, MAX_CONFIG_BYTES)
                .map_err(|error| UvDiscoveryError::Read(file, error))?
            else {
                continue;
            };
            if bytes.len() > MAX_CONFIG_BYTES {
                return Err(UvDiscoveryError::TooLarge(file));
            }
            let invalid = |reason: &str| UvDiscoveryError::Invalid {
                file,
                reason: reason.into(),
            };
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid("expected UTF-8"))?;
            // Duplicate keys are TOML syntax errors. Do not echo parser
            // diagnostics: they can quote unrelated configuration values.
            let table: toml::Table = text.parse().map_err(|_| invalid("expected valid TOML"))?;
            let (settings, field) = if file == "uv.toml" {
                (Some(&table), "required-version")
            } else {
                let tool = match table.get("tool") {
                    None => None,
                    Some(tool) => Some(tool.as_table().ok_or_else(|| invalid("`tool` table"))?),
                };
                match tool.and_then(|tool| tool.get("uv")) {
                    // Without [tool.uv], uv itself has no project setting here.
                    None => return Ok(None),
                    Some(uv) => (
                        Some(uv.as_table().ok_or_else(|| invalid("`tool.uv` table"))?),
                        "tool.uv.required-version",
                    ),
                }
            };
            let Some(value) = settings.and_then(|table| table.get("required-version")) else {
                // uv.toml takes precedence even without a version request.
                return Ok(None);
            };
            let request = value
                .as_str()
                .ok_or_else(|| invalid("`required-version` must be a string"))?
                .trim();
            let specifier = |source| UvDiscoveryError::Specifier {
                file,
                field,
                source,
            };
            if request.len() > pep440::MAX_SPECIFIER_BYTES {
                return Err(specifier(pep440::Error::Limit));
            }
            let specifiers = match Release::parse(request) {
                Some(release) => Specifiers::exact(release),
                None => Specifiers::parse(request).map_err(specifier)?,
            };
            return Ok(Some(Self {
                source: UvSource {
                    file,
                    field,
                    request: request.to_owned(),
                },
                specifiers,
            }));
        }
        Ok(None)
    }

    pub fn source(&self) -> &UvSource {
        &self.source
    }

    pub fn declaration(&self) -> Declaration {
        Declaration {
            file: self.source.file.into(),
            field: Some(self.source.field.into()),
            request: Some(self.source.request.clone()),
        }
    }

    /// Whether an exact release satisfies the authored request.
    pub fn matches(&self, version: &semver::Version) -> bool {
        version.pre.is_empty()
            && version.build.is_empty()
            && self.specifiers.contains(&Release::from(version))
    }
}

#[cfg(test)]
mod tests;
