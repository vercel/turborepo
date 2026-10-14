//! Download mirrors under `setup.mirrors`. A mirror changes only where
//! `turbo setup` downloads from: `turbo.lock` keeps the official URLs, and
//! every download is verified against its locked digest. The matching
//! `TURBO_SETUP_*` environment variables override these values for one
//! invocation.

use std::sync::Arc;

use biome_deserialize_macros::Deserializable;
use miette::{NamedSource, SourceSpan};
use schemars::JsonSchema;
use serde::Serialize;
use turborepo_errors::{Spanned, WithMetadata};

use crate::error::Error;

/// Credential-free HTTPS mirrors for `turbo setup` downloads. Each value is
/// a base URL that serves the same paths as the official source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserializable, JsonSchema)]
#[schemars(rename = "SetupMirrors")]
#[deserializable(unknown_fields = "deny")]
pub struct RawSetupMirrors {
    /// Replaces `https://nodejs.org/dist` for Node.js release metadata and
    /// archives.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<Spanned<String>>,
    /// Replaces `https://registry.npmjs.org` for npm, pnpm, and Yarn package
    /// metadata and tarballs.
    #[serde(rename = "npmRegistry", skip_serializing_if = "Option::is_none")]
    pub npm_registry: Option<Spanned<String>>,
    /// Replaces the `https://github.com` origin for Bun release checksums and
    /// archives, keeping their official paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bun: Option<Spanned<String>>,
    /// Replaces the `https://releases.astral.sh` origin for uv's version index
    /// and release archives, keeping their official paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uv: Option<Spanned<String>>,
    /// Replaces the `https://releases.astral.sh` origin for CPython build
    /// checksums and archives, keeping their official paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python: Option<Spanned<String>>,
}

/// Validated `setup.mirrors` bases. `None` keeps the official source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupMirrors {
    pub node: Option<String>,
    pub npm_registry: Option<String>,
    pub bun: Option<String>,
    pub uv: Option<String>,
    pub python: Option<String>,
}

impl RawSetupMirrors {
    fn values(&self) -> [(&'static str, &Option<Spanned<String>>); 5] {
        [
            ("node", &self.node),
            ("npmRegistry", &self.npm_registry),
            ("bun", &self.bun),
            ("uv", &self.uv),
            ("python", &self.python),
        ]
    }
}

/// The mirror rule shared with the `TURBO_SETUP_*` environment variables: an
/// absolute URL with a host and no userinfo, query, or fragment. Callers
/// approve the scheme. The raw authority is inspected too, because URL
/// normalization erases empty userinfo such as `https://@host`.
pub fn mirror_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    let authority = value
        .split_once("://")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())?;
    let valid = !authority.is_empty()
        && !authority.contains('@')
        && !value.contains(['?', '#'])
        && url.host().is_some();
    valid.then_some(url)
}

pub(super) fn validate(raw: Option<&Spanned<RawSetupMirrors>>) -> Result<SetupMirrors, Error> {
    let Some(raw) = raw else {
        return Ok(SetupMirrors::default());
    };
    let values = raw.values();
    for (key, value) in values {
        let Some(value) = value else { continue };
        if !mirror_url(value).is_some_and(|url| url.scheme() == "https") {
            let (span, text) = masked(value, values.iter().filter_map(|(_, value)| value.as_ref()));
            return Err(Error::SetupInvalidMirror { key, span, text });
        }
    }
    let base = |value: &Option<Spanned<String>>| value.as_ref().map(|v| v.as_inner().clone());
    Ok(SetupMirrors {
        node: base(&raw.node),
        npm_registry: base(&raw.npm_registry),
        bun: base(&raw.bun),
        uv: base(&raw.uv),
        python: base(&raw.python),
    })
}

/// Points at `value` while masking every mirror value in the rendered source,
/// because a committed value can still carry a secret. Without exact spans,
/// no source is shown at all.
fn masked<'a>(
    value: &Spanned<String>,
    values: impl Iterator<Item = &'a Spanned<String>>,
) -> (Option<SourceSpan>, NamedSource<String>) {
    let path = value.path.as_deref().unwrap_or("turbo.json");
    let unmasked = || (None, NamedSource::new(path, String::new()));
    let (Some(range), Some(text)) = (value.range.clone(), value.text.as_deref()) else {
        return unmasked();
    };
    let mut text = text.to_owned();
    for range in values.map(|value| value.range.clone()) {
        match range {
            Some(range) if text.get(range.clone()).is_some() => {
                text.replace_range(range.clone(), &"*".repeat(range.len()));
            }
            _ => return unmasked(),
        }
    }
    (Some(range.into()), NamedSource::new(path, text))
}

impl RawSetupMirrors {
    fn values_mut(&mut self) -> [&mut Option<Spanned<String>>; 5] {
        [
            &mut self.node,
            &mut self.npm_registry,
            &mut self.bun,
            &mut self.uv,
            &mut self.python,
        ]
    }
}

impl WithMetadata for RawSetupMirrors {
    fn add_text(&mut self, text: Arc<str>) {
        self.values_mut()
            .into_iter()
            .for_each(|value| value.add_text(text.clone()));
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.values_mut()
            .into_iter()
            .for_each(|value| value.add_path(path.clone()));
    }
}
