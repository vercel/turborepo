//! Repository-declared tools under `setup.additionalTools`: an exact version
//! plus explicit per-platform artifacts. Field and platform names match the
//! managed artifacts in `turbo.lock`. Tools that a built-in setup adapter
//! manages stay declared in their native files.

use std::{
    borrow::Cow,
    collections::{BTreeMap, btree_map::Entry},
    sync::Arc,
};

use biome_deserialize_macros::Deserializable;
use miette::{NamedSource, SourceSpan};
use schemars::JsonSchema;
use serde::Serialize;
use ts_rs::TS;
use turborepo_errors::{Spanned, WithMetadata};

use crate::error::Error;

/// Identities of built-in setup adapters and toolchains.
const BUILTIN_TOOLS: &str = "bun go javascript node npm pnpm python rust rustup turbo uv yarn";
/// Executables of built-in setup adapters, plus `turbo` itself.
const BUILTIN_EXECUTABLES: &str = "bun bunx cargo cargo-clippy cargo-fmt clippy-driver corepack \
                                   go gofmt node npm npx pip pip3 pnpm pnpx python python3 \
                                   rust-analyzer rust-gdb rust-gdbgui rust-lldb rustc rustdoc \
                                   rustfmt rustup turbo uv uvx yarn yarnpkg";
const MAX_EXECUTABLES: usize = 64; // The lock's per-artifact bound.

/// The OS, architecture, and Linux C library that an artifact runs on.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserializable, JsonSchema, TS,
)]
pub enum ArtifactPlatform {
    #[serde(rename = "macos-x64")]
    MacosX64,
    #[serde(rename = "macos-arm64")]
    MacosArm64,
    #[serde(rename = "linux-x64-gnu")]
    LinuxX64Gnu,
    #[serde(rename = "linux-arm64-gnu")]
    LinuxArm64Gnu,
    #[serde(rename = "linux-x64-musl")]
    LinuxX64Musl,
    #[serde(rename = "linux-arm64-musl")]
    LinuxArm64Musl,
    #[serde(rename = "windows-x64")]
    WindowsX64,
    #[serde(rename = "windows-arm64")]
    WindowsArm64,
}

/// How an artifact is packaged. `binary` is a single executable file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserializable, JsonSchema, TS)]
pub enum ArtifactFormat {
    #[default]
    #[serde(rename = "binary")]
    Binary,
    #[serde(rename = "tar")]
    Tar,
    #[serde(rename = "tar-gz")]
    TarGz,
    #[serde(rename = "zip")]
    Zip,
}

// The derive's required-field check cannot name generic paths.
type SpannedString = Spanned<String>;
type Format = Spanned<ArtifactFormat>;
type Executables = BTreeMap<String, Spanned<String>>;
type Artifacts = Spanned<BTreeMap<ArtifactPlatform, Spanned<RawAdditionalToolArtifact>>>;

/// One platform's artifact for an additional tool.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserializable, JsonSchema)]
#[schemars(rename = "AdditionalToolArtifact")]
#[deserializable(unknown_fields = "deny")]
pub struct RawAdditionalToolArtifact {
    /// HTTPS download URL without credentials, a query, or a fragment.
    #[deserializable(required)]
    pub url: SpannedString,
    /// Lowercase hex SHA-256 digest of the downloaded bytes. A digest pins the
    /// expected bytes; it does not establish that the publisher is
    /// trustworthy.
    #[deserializable(required)]
    pub sha256: SpannedString,
    /// `binary` for a single executable file, or the archive format.
    #[deserializable(required)]
    pub format: Format,
    /// The single top-level directory that contains every archive entry. Omit
    /// it for an archive whose entries sit at its top level. Not allowed for
    /// binaries.
    #[serde(rename = "rootPrefix", skip_serializing_if = "Option::is_none")]
    pub root_prefix: Option<Spanned<String>>,
    /// Executable names mapped to paths relative to `rootPrefix`, or to the
    /// archive's top level without one. A binary has exactly one entry, which
    /// names where the file is installed.
    #[deserializable(required)]
    pub executables: Executables,
}

/// A tool that `turbo setup` downloads, verifies, and installs from explicit
/// artifacts. The full installation tree is kept; no tool-provided scripts
/// run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserializable, JsonSchema)]
#[schemars(rename = "AdditionalTool")]
#[deserializable(unknown_fields = "deny")]
pub struct RawAdditionalTool {
    /// The exact version of the tool. Ranges and channels are not allowed.
    #[deserializable(required)]
    pub version: SpannedString,
    /// Artifacts keyed by platform, such as `linux-x64-gnu` or `macos-arm64`.
    /// Platforms without an artifact are unsupported for this tool.
    #[deserializable(required)]
    pub artifacts: Artifacts,
}

/// A failed check: the value to point at and why it is invalid.
type Invalid<'a> = (&'a dyn SpanSource, Cow<'static, str>);

fn ensure<'a>(
    ok: bool,
    source: &'a dyn SpanSource,
    reason: &'static str,
) -> Result<(), Invalid<'a>> {
    ok.then_some(()).ok_or((source, reason.into()))
}

pub(super) fn validate(
    raw: Option<&BTreeMap<String, Spanned<RawAdditionalTool>>>,
) -> Result<BTreeMap<String, RawAdditionalTool>, Error> {
    let mut tools = BTreeMap::new();
    // Executables of every additional tool are active together on a platform.
    let mut owners: BTreeMap<(ArtifactPlatform, String), &str> = BTreeMap::new();
    for (id, raw_tool) in raw.into_iter().flatten() {
        validate_tool(id, raw_tool, &mut owners).map_err(|(source, reason)| {
            let (span, text) = source.span_and_text();
            Error::SetupInvalidAdditionalTool {
                tool: id.clone(),
                reason: reason.into_owned(),
                span,
                text,
            }
        })?;
        tools.insert(id.clone(), raw_tool.as_inner().clone());
    }
    Ok(tools)
}

fn validate_tool<'a>(
    id: &'a str,
    raw: &'a Spanned<RawAdditionalTool>,
    owners: &mut BTreeMap<(ArtifactPlatform, String), &'a str>,
) -> Result<(), Invalid<'a>> {
    if builtin(BUILTIN_TOOLS, id) {
        return Err((raw, format!("{id:?} is {BUILTIN}").into()));
    }
    ensure(
        identifier(id),
        raw,
        "tool IDs start with a lowercase letter and use at most 64 lowercase letters, digits, \
         `-`, or `_`",
    )?;
    let version = &raw.version;
    ensure(
        exact_version(version),
        version,
        "`version` must be an exact version, not a range or channel",
    )?;
    ensure(
        !raw.artifacts.is_empty(),
        &raw.artifacts,
        "`artifacts` must declare at least one platform",
    )?;
    for (platform, artifact) in raw.artifacts.iter() {
        validate_artifact(artifact)?;
        let windows = matches!(
            platform,
            ArtifactPlatform::WindowsX64 | ArtifactPlatform::WindowsArm64
        );
        for (name, path) in &artifact.executables {
            if builtin(BUILTIN_EXECUTABLES, &fold(name, true)) {
                return Err((path, format!("executable {name:?} is {BUILTIN}").into()));
            }
            match owners.entry((*platform, fold(name, windows))) {
                Entry::Vacant(entry) => {
                    entry.insert(id);
                }
                Entry::Occupied(owner) => {
                    let owner = owner.get();
                    let reason = format!("executable {name:?} is also declared by {owner:?}");
                    return Err((path, reason.into()));
                }
            }
            if windows {
                windows_executable(name, path)?;
            }
        }
    }
    Ok(())
}

/// The managed bin directory exports a Windows executable as `name.exe` (a
/// hard link) or `name.cmd` (a forwarder that names the target in a batch
/// line), adding that suffix itself.
fn windows_executable<'a>(name: &str, path: &'a Spanned<String>) -> Result<(), Invalid<'a>> {
    let lower = path.to_ascii_lowercase();
    let launcher = lower.ends_with(".cmd")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b));
    if !lower.ends_with(".exe") && !launcher {
        let reason = format!(
            "Windows executable {name:?} must be a `.exe` file or a `.cmd` launcher whose path \
             uses only letters, digits, `-`, `_`, `.`, and `/`"
        );
        return Err((path, reason.into()));
    }
    if fold(name, true) != name.to_ascii_lowercase() {
        let reason = format!("Windows executable {name:?} must omit its `.exe` or `.cmd` suffix");
        return Err((path, reason.into()));
    }
    Ok(())
}

fn validate_artifact(raw: &RawAdditionalToolArtifact) -> Result<(), Invalid<'_>> {
    ensure(
        artifact_url(&raw.url),
        &raw.url,
        "`url` must be an HTTPS URL without credentials, a query, or a fragment",
    )?;
    ensure(
        raw.sha256.len() == 64
            && raw
                .sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        &raw.sha256,
        "`sha256` must be a lowercase hex SHA-256 digest",
    )?;
    let format = *raw.format.as_inner();
    match (format, &raw.root_prefix) {
        (ArtifactFormat::Binary, Some(prefix)) => {
            return Err((prefix, "binaries do not have a `rootPrefix`".into()));
        }
        (ArtifactFormat::Binary, None) => ensure(
            raw.executables.len() == 1,
            &raw.format,
            "a binary maps exactly one executable",
        )?,
        // Entries at the archive's top level, such as protoc's release zips.
        (_, None) => {}
        (_, Some(prefix)) => ensure(
            portable_path(prefix) && !prefix.contains('/'),
            prefix,
            "`rootPrefix` must be one portable directory name",
        )?,
    }
    ensure(
        raw.executables.len() <= MAX_EXECUTABLES,
        &raw.format,
        "artifacts map at most 64 executables",
    )?;
    for (name, path) in &raw.executables {
        ensure(
            portable_path(name) && !name.contains('/'),
            path,
            "executable names must be portable file names",
        )?;
        ensure(
            portable_path(path),
            path,
            "executable paths must be portable relative paths inside the artifact",
        )?;
    }
    Ok(())
}

/// Spanned values of any type can locate a diagnostic.
trait SpanSource {
    fn span_and_text(&self) -> (Option<SourceSpan>, NamedSource<String>);
}
impl<T> SpanSource for Spanned<T> {
    fn span_and_text(&self) -> (Option<SourceSpan>, NamedSource<String>) {
        Spanned::span_and_text(self, "turbo.json")
    }
}

const BUILTIN: &str = "managed by a built-in setup adapter; declare it in its native file";

fn builtin(names: &str, name: &str) -> bool {
    names.split_whitespace().any(|builtin| builtin == name)
}

/// Case-folded command name; Windows resolves `name.exe` and `name` alike.
fn fold(name: &str, windows: bool) -> String {
    let folded = name.to_ascii_lowercase();
    let stem = [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find_map(|suffix| folded.strip_suffix(suffix));
    match stem {
        Some(stem) if windows => stem.to_owned(),
        _ => folded.clone(),
    }
}

fn identifier(value: &str) -> bool {
    value.len() <= 64
        && value.starts_with(|c: char| c.is_ascii_lowercase())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
}

fn exact_version(value: &str) -> bool {
    value.len() <= 128
        && value.starts_with(|c: char| c.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        && !value
            .split(['.', '-', '+'])
            .any(|part| part.eq_ignore_ascii_case("x"))
        && !["latest", "stable", "lts", "nightly", "current"]
            .iter()
            .any(|channel| value.eq_ignore_ascii_case(channel))
}

fn artifact_url(value: &str) -> bool {
    let authority = value
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    value.len() <= 4096
        && !authority.is_empty()
        && !authority.contains('@')
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"\\?#".contains(&b))
}

/// The conservative root-relative ASCII path rule that `turbo.lock` applies,
/// checked here so invalid declarations point at their source.
fn portable_path(value: &str) -> bool {
    if value.is_empty() || value.len() > 1024 || value.split('/').count() > 32 {
        return false;
    }
    value.split('/').all(|part| {
        // `.` and `..` end with a dot. Device names are reserved on Windows.
        let base = part.split('.').next().unwrap_or_default().as_bytes();
        let device = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
            .iter()
            .any(|name| base.eq_ignore_ascii_case(name.as_bytes()))
            || (base.len() == 4
                && [b"COM", b"LPT"]
                    .iter()
                    .any(|prefix| base[..3].eq_ignore_ascii_case(*prefix))
                && matches!(base[3], b'1'..=b'9'));
        !part.is_empty()
            && part.len() <= 255
            && !part.ends_with('.')
            && !device
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-+@".contains(&b))
    })
}

impl RawAdditionalToolArtifact {
    fn spans(&mut self) -> impl Iterator<Item = &mut dyn WithMetadata> {
        let fields: [&mut dyn WithMetadata; 4] = [
            &mut self.url,
            &mut self.sha256,
            &mut self.format,
            &mut self.root_prefix,
        ];
        let paths = self.executables.values_mut();
        fields
            .into_iter()
            .chain(paths.map(|path| path as &mut dyn WithMetadata))
    }
}

impl WithMetadata for RawAdditionalToolArtifact {
    fn add_text(&mut self, text: Arc<str>) {
        self.spans().for_each(|span| span.add_text(text.clone()));
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.spans().for_each(|span| span.add_path(path.clone()));
    }
}

impl WithMetadata for RawAdditionalTool {
    fn add_text(&mut self, text: Arc<str>) {
        self.version.add_text(text.clone());
        self.artifacts.add_text(text.clone());
        for artifact in self.artifacts.values_mut() {
            artifact.add_text(text.clone());
            artifact.value.add_text(text.clone());
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.version.add_path(path.clone());
        self.artifacts.add_path(path.clone());
        for artifact in self.artifacts.values_mut() {
            artifact.add_path(path.clone());
            artifact.value.add_path(path.clone());
        }
    }
}

#[cfg(test)]
mod tests;
