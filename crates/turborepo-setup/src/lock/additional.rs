//! Repository-declared additional tools. A declaration is already an exact
//! selection, so its lock entry is a pure projection of the captured root
//! configuration: no resolver, network, or platform discovery. Every
//! platform the repository declares is locked, and only those.

use std::collections::BTreeMap;

use turborepo_turbo_json::{
    ArtifactFormat, ArtifactPlatform, RawRootTurboJson, RawTurboJson, SetupPolicy,
};

use super::{Artifact, Declaration, Format, Installation, Platform, Tool};

/// Lock adapter for verified artifacts declared in `setup.additionalTools`.
pub const ADAPTER: &str = "artifact";
/// Each platform installs exactly one artifact part.
pub const PART: &str = "artifact";
const FIELD: &str = "/setup/additionalTools/";

/// Projects `setup.additionalTools` from the root configuration `file`.
pub(super) fn project(file: &str, bytes: &[u8]) -> Option<BTreeMap<String, Tool>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let raw: RawTurboJson = RawRootTurboJson::parse(text, file).ok()?.try_into().ok()?;
    let flags = raw.future_flags.as_ref().map(|flags| *flags.as_inner());
    let policy = SetupPolicy::from_raw(raw.setup.as_ref(), &flags.unwrap_or_default()).ok()?;
    let tools = policy.additional_tools().iter().map(|(id, tool)| {
        let artifacts = tool.artifacts.iter().map(|(platform, artifact)| {
            let artifact = Artifact {
                url: artifact.url.as_inner().clone(),
                sha256: artifact.sha256.as_inner().clone(),
                format: match *artifact.format.as_inner() {
                    ArtifactFormat::Binary => Format::Binary,
                    ArtifactFormat::Tar => Format::Tar,
                    ArtifactFormat::TarGz => Format::TarGz,
                    ArtifactFormat::Zip => Format::Zip,
                },
                root_prefix: artifact.root_prefix.as_ref().map(|p| p.as_inner().clone()),
                alternates: BTreeMap::new(),
                destination: None,
                executables: artifact
                    .executables
                    .iter()
                    .map(|(name, path)| (name.clone(), path.as_inner().clone()))
                    .collect(),
            };
            (
                lock_platform(*platform),
                BTreeMap::from([(PART.into(), artifact)]),
            )
        });
        let version = tool.version.as_inner().clone();
        let tool = Tool {
            adapter: ADAPTER.into(),
            declarations: vec![Declaration {
                file: file.into(),
                field: Some(format!("{FIELD}{id}")),
                request: Some(version.clone()),
            }],
            version,
            options: BTreeMap::new(),
            installation: Installation::Managed {
                artifacts: artifacts.collect(),
            },
        };
        (id.clone(), tool)
    });
    Some(tools.collect())
}

/// Whether a locked entry claims a declaration that the snapshot's root
/// configuration covers. Its contents must still match the projection.
pub(super) fn covered(id: &str, tool: &Tool) -> bool {
    tool.adapter == ADAPTER
        && matches!(tool.declarations.as_slice(), [Declaration { file, field: Some(field), .. }]
            if turborepo_types::CONFIG_FILES.contains(&file.as_str())
                && field.strip_prefix(FIELD) == Some(id))
}

fn lock_platform(platform: ArtifactPlatform) -> Platform {
    match platform {
        ArtifactPlatform::MacosX64 => Platform::MacosX64,
        ArtifactPlatform::MacosArm64 => Platform::MacosArm64,
        ArtifactPlatform::LinuxX64Gnu => Platform::LinuxX64Gnu,
        ArtifactPlatform::LinuxArm64Gnu => Platform::LinuxArm64Gnu,
        ArtifactPlatform::LinuxX64Musl => Platform::LinuxX64Musl,
        ArtifactPlatform::LinuxArm64Musl => Platform::LinuxArm64Musl,
        ArtifactPlatform::WindowsX64 => Platform::WindowsX64,
        ArtifactPlatform::WindowsArm64 => Platform::WindowsArm64,
    }
}

#[cfg(test)]
mod tests;
