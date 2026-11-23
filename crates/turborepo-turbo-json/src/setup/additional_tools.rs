//! Repository-declared tools under `setup.additionalTools`: an exact version,
//! plus either explicit per-platform artifacts or nothing more, in which case
//! `turbo setup` finds the downloads in its bundled catalog. Field and
//! platform names match the managed artifacts in `turbo.lock`. Tools that a
//! built-in setup adapter manages stay declared in their native files.

use std::{
    borrow::Cow,
    collections::{BTreeMap, btree_map::Entry},
    sync::Arc,
};

use biome_deserialize::{
    Deserializable, DeserializableValue, DeserializationDiagnostic, VisitableType,
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
pub const CATALOG_PREFIX: &str = "aqua:";

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
    #[serde(rename = "tar-xz")]
    TarXz,
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

/// A tool that `turbo setup` downloads, verifies, and installs. The full
/// installation tree is kept; no tool-provided scripts run.
///
/// A string is the exact version of a tool in the bundled catalog, looked up
/// by its tool ID. An object can name the catalog entry with `from`, or
/// declare explicit `artifacts`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
#[schemars(rename = "AdditionalTool")]
pub enum RawAdditionalTool {
    Version(SpannedString),
    Declaration(RawAdditionalToolDeclaration),
}

/// The object form of an additional tool.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserializable, JsonSchema)]
#[schemars(rename = "AdditionalToolDeclaration")]
#[deserializable(unknown_fields = "deny")]
pub struct RawAdditionalToolDeclaration {
    /// The exact version of the tool, as the tool itself reports it, such as
    /// `1.8.2`. Ranges and channels are not allowed.
    #[deserializable(required)]
    pub version: SpannedString,
    /// The bundled catalog entry that provides the downloads, as `aqua:`
    /// followed by a package ID, such as `aqua:jj-vcs/jj`. Without `from` and
    /// `artifacts`, the tool ID is looked up in the catalog's name list. Not
    /// allowed with `artifacts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<SpannedString>,
    /// Artifacts keyed by platform, such as `linux-x64-gnu` or `macos-arm64`.
    /// Platforms without an artifact are unsupported for this tool. Not
    /// allowed with `from`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Artifacts>,
}

impl Deserializable for RawAdditionalTool {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let actual = value.visitable_type()?;
        if actual == VisitableType::STR {
            Spanned::deserialize(value, name, diagnostics).map(Self::Version)
        } else if actual == VisitableType::MAP {
            Deserializable::deserialize(value, name, diagnostics).map(Self::Declaration)
        } else {
            diagnostics.push(DeserializationDiagnostic::new_incorrect_type(
                actual,
                VisitableType::STR.union(VisitableType::MAP),
                value.range(),
            ));
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdditionalTool {
    pub version: String,
    pub source: AdditionalToolSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdditionalToolSource {
    /// `turbo setup` finds the downloads in its bundled catalog and pins them
    /// in `turbo.lock`. `from` is the entry as written, such as
    /// `aqua:jj-vcs/jj`; without it, the tool ID is looked up in the catalog's
    /// name list.
    Catalog { from: Option<String> },
    /// The downloads the repository declared, by platform.
    Artifacts(BTreeMap<ArtifactPlatform, Spanned<RawAdditionalToolArtifact>>),
}

/// The package ID of a catalog entry written as `aqua:<package ID>`: two or
/// more `/`-separated segments of ASCII letters, digits, `.`, `_`, and `-`,
/// at most 256 bytes. `turbo.lock` records the same spelling.
pub fn catalog_package(from: &str) -> Option<&str> {
    let package = from.strip_prefix(CATALOG_PREFIX)?;
    let segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    (package.len() <= 256 && package.contains('/') && package.split('/').all(segment))
        .then_some(package)
}

/// Why an artifact that setup derived, rather than read from `turbo.json`,
/// breaks the rules a hand-declared artifact for `platform` obeys. `None`
/// means it could have been declared by hand.
pub fn derived_artifact_violation(
    platform: ArtifactPlatform,
    url: &str,
    format: ArtifactFormat,
    executables: &BTreeMap<String, String>,
) -> Option<String> {
    let url = Spanned::new(url.to_owned());
    let format = Spanned::new(format);
    let executables: Executables = executables
        .iter()
        .map(|(name, path)| (name.clone(), Spanned::new(path.clone())))
        .collect();
    let mut commands = std::collections::BTreeSet::new();
    let checked = validate_layout(&url, &format, None, &executables).and_then(|()| {
        executables.iter().try_for_each(|(name, path)| {
            let command = validate_executable(platform, name, path)?;
            if commands.insert(command) {
                return Ok(());
            }
            let reason = format!("executable {name:?} is the same command as another one");
            Err((path as &dyn SpanSource, reason.into()))
        })
    });
    checked.err().map(|(_, reason)| reason.into_owned())
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
) -> Result<BTreeMap<String, AdditionalTool>, Error> {
    let mut tools = BTreeMap::new();
    // Executables of every additional tool are active together on a platform.
    let mut owners: BTreeMap<(ArtifactPlatform, String), &str> = BTreeMap::new();
    for (id, raw_tool) in raw.into_iter().flatten() {
        let tool = validate_tool(id, raw_tool, &mut owners).map_err(|(source, reason)| {
            let (span, text) = source.span_and_text();
            Error::SetupInvalidAdditionalTool {
                tool: id.clone(),
                reason: reason.into_owned(),
                span,
                text,
            }
        })?;
        tools.insert(id.clone(), tool);
    }
    Ok(tools)
}

fn validate_tool<'a>(
    id: &'a str,
    raw: &'a Spanned<RawAdditionalTool>,
    owners: &mut BTreeMap<(ArtifactPlatform, String), &'a str>,
) -> Result<AdditionalTool, Invalid<'a>> {
    if builtin(BUILTIN_TOOLS, id) {
        return Err((raw, format!("{id:?} is {BUILTIN}").into()));
    }
    ensure(
        identifier(id),
        raw,
        "tool IDs start with a lowercase letter and use at most 64 lowercase letters, digits, \
         `-`, or `_`",
    )?;
    let (version, from, artifacts) = match raw.as_inner() {
        RawAdditionalTool::Version(version) => (version, None, None),
        RawAdditionalTool::Declaration(tool) => {
            (&tool.version, tool.from.as_ref(), tool.artifacts.as_ref())
        }
    };
    ensure(
        exact_version(version),
        version,
        "`version` must be an exact version, not a range or channel",
    )?;
    let source = match (from, artifacts) {
        (Some(from), Some(_)) => {
            let reason =
                "`from` cannot be combined with `artifacts`, which already name the downloads";
            return Err((from, reason.into()));
        }
        (Some(from), None) => {
            ensure(
                catalog_package(from).is_some(),
                from,
                "`from` must be `aqua:` followed by a catalog package ID, such as `aqua:jj-vcs/jj`",
            )?;
            AdditionalToolSource::Catalog {
                from: Some(from.as_inner().clone()),
            }
        }
        (None, None) => AdditionalToolSource::Catalog { from: None },
        (None, Some(artifacts)) => {
            validate_artifacts(id, artifacts, owners)?;
            AdditionalToolSource::Artifacts(artifacts.as_inner().clone())
        }
    };
    Ok(AdditionalTool {
        version: version.as_inner().clone(),
        source,
    })
}

fn validate_artifacts<'a>(
    id: &'a str,
    artifacts: &'a Artifacts,
    owners: &mut BTreeMap<(ArtifactPlatform, String), &'a str>,
) -> Result<(), Invalid<'a>> {
    ensure(
        !artifacts.is_empty(),
        artifacts,
        "`artifacts` must declare at least one platform",
    )?;
    for (platform, artifact) in artifacts.iter() {
        validate_artifact(artifact)?;
        for (name, path) in &artifact.executables {
            match owners.entry((*platform, validate_executable(*platform, name, path)?)) {
                Entry::Vacant(entry) => {
                    entry.insert(id);
                }
                Entry::Occupied(owner) => {
                    let owner = owner.get();
                    let reason = format!("executable {name:?} is also declared by {owner:?}");
                    return Err((path, reason.into()));
                }
            }
        }
    }
    Ok(())
}

fn validate_executable<'a>(
    platform: ArtifactPlatform,
    name: &str,
    path: &'a Spanned<String>,
) -> Result<String, Invalid<'a>> {
    let windows = matches!(
        platform,
        ArtifactPlatform::WindowsX64 | ArtifactPlatform::WindowsArm64
    );
    if builtin(BUILTIN_EXECUTABLES, &command_name(name, true)) {
        return Err((path, format!("executable {name:?} is {BUILTIN}").into()));
    }
    if windows {
        windows_executable(name, path)?;
    }
    Ok(command_name(name, windows))
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
    if command_name(name, true) != name.to_ascii_lowercase() {
        let reason = format!("Windows executable {name:?} must omit its `.exe` or `.cmd` suffix");
        return Err((path, reason.into()));
    }
    Ok(())
}

fn validate_artifact(raw: &RawAdditionalToolArtifact) -> Result<(), Invalid<'_>> {
    ensure(
        raw.sha256.len() == 64
            && raw
                .sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        &raw.sha256,
        "`sha256` must be a lowercase hex SHA-256 digest",
    )?;
    validate_layout(
        &raw.url,
        &raw.format,
        raw.root_prefix.as_ref(),
        &raw.executables,
    )
}

fn validate_layout<'a>(
    url: &'a Spanned<String>,
    format: &'a Format,
    root_prefix: Option<&'a Spanned<String>>,
    executables: &'a Executables,
) -> Result<(), Invalid<'a>> {
    ensure(
        artifact_url(url),
        url,
        "`url` must be an HTTPS URL without credentials, a query, or a fragment",
    )?;
    match (*format.as_inner(), root_prefix) {
        (ArtifactFormat::Binary, Some(prefix)) => {
            return Err((prefix, "binaries do not have a `rootPrefix`".into()));
        }
        (ArtifactFormat::Binary, None) => ensure(
            executables.len() == 1,
            format,
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
        executables.len() <= MAX_EXECUTABLES,
        format,
        "artifacts map at most 64 executables",
    )?;
    for (name, path) in executables {
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

/// The command an executable name claims, which no two additional tools may
/// share on a platform: case-folded, and on Windows without its suffix,
/// because Windows resolves `name.exe` and `name` alike.
pub fn command_name(name: &str, windows: bool) -> String {
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

impl RawAdditionalTool {
    fn each_span(&mut self, mut add: impl FnMut(&mut dyn WithMetadata)) {
        match self {
            Self::Version(version) => add(version),
            Self::Declaration(tool) => {
                add(&mut tool.version);
                add(&mut tool.from);
                add(&mut tool.artifacts);
                let artifacts = tool.artifacts.iter_mut();
                for artifact in artifacts.flat_map(|artifacts| artifacts.value.values_mut()) {
                    add(artifact);
                    add(&mut artifact.value);
                }
            }
        }
    }
}

impl WithMetadata for RawAdditionalTool {
    fn add_text(&mut self, text: Arc<str>) {
        self.each_span(|span| span.add_text(text.clone()));
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.each_span(|span| span.add_path(path.clone()));
    }
}

#[cfg(test)]
mod tests;
