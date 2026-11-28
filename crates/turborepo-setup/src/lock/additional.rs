//! Repository-declared additional tools. A declaration with explicit artifacts
//! is already an exact selection, so its lock entry is a pure projection of
//! the captured root configuration: every platform the repository declares is
//! locked, and only those.
//!
//! A catalog declaration names only a version. Its lock entry is the one a
//! previous `turbo setup` resolved and recorded, reused unchanged while the
//! declaration still asks for it and `setup.registries` still declares the
//! registry that the entry came from. The lock, not the current registries or
//! their order, decides what an existing repository installs. Without such an
//! entry the declaration is unresolved: projection reports it and never
//! resolves, reads the network, or discovers platforms.
//!
//! A recorded entry may lack a platform in [`TARGETS`]: the catalog had no
//! installable download for it when the entry was resolved. Such an entry is
//! still reused. Projection also reports it as incomplete, so that
//! `turbo setup --update-lock` can resolve it again.

use std::collections::BTreeMap;

use turborepo_turbo_json::{
    AdditionalToolSource, ArtifactFormat, ArtifactPlatform, RawRootTurboJson, RawTurboJson,
    SetupPolicy, SetupRegistry, command_name, derived_artifact_violation, registry_package,
};

use super::{Artifact, Declaration, Format, Installation, Lock, Platform, Tool};

/// Lock adapter for verified artifacts declared in `setup.additionalTools`.
pub const ADAPTER: &str = "artifact";
/// Each platform installs exactly one artifact part.
pub const PART: &str = "artifact";
/// The option that records which registry entry gave a tool's downloads, as
/// one `<registry>:<package ID>` value. It selects no bytes. An entry is
/// reused only while its registry is still declared.
pub const SOURCE: &str = "source";
const FIELD: &str = "/setup/additionalTools/";
/// The platforms that a host can install an additional tool on. Not musl
/// Linux or Windows arm64: no host installs managed tools there. The tool
/// catalog resolves exactly these, and a catalog entry that records all of
/// them is complete.
pub const TARGETS: [ArtifactPlatform; 5] = [
    ArtifactPlatform::MacosX64,
    ArtifactPlatform::MacosArm64,
    ArtifactPlatform::LinuxX64Gnu,
    ArtifactPlatform::LinuxArm64Gnu,
    ArtifactPlatform::WindowsX64,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRequest {
    pub id: String,
    /// The tool's own exact version, without a release-name prefix.
    pub version: String,
    /// The registry entry as declared, `<registry>:<package ID>`. Without it,
    /// the declared registries are asked for the tool ID in order.
    pub from: Option<String>,
    /// The root configuration file that declares the tool.
    pub file: String,
}

impl CatalogRequest {
    fn declaration(&self) -> Declaration {
        declaration(&self.file, &self.id, &self.version)
    }

    /// The lock entry for this declaration once the registry entry `source`
    /// (`<registry>:<package ID>`) resolved it to one artifact per platform.
    /// Executable paths are relative to each archive's top level.
    pub fn resolved(&self, source: &str, artifacts: BTreeMap<Platform, Artifact>) -> Tool {
        let options = BTreeMap::from([(SOURCE.into(), vec![source.into()])]);
        let artifacts = artifacts
            .into_iter()
            .map(|(platform, artifact)| (platform, BTreeMap::from([(PART.into(), artifact)])));
        entry(self.declaration(), options, artifacts.collect())
    }

    /// Whether `tool` is the entry this declaration asks for, from one of
    /// the declared `registries`.
    pub(super) fn records(&self, tool: &Tool, registries: &[SetupRegistry]) -> bool {
        let source = match tool.options.get(SOURCE).map(Vec::as_slice) {
            Some([source]) if tool.options.len() == 1 => source,
            _ => return false,
        };
        tool.adapter == ADAPTER
            && covered(&self.id, tool)
            && tool.version == self.version
            && tool.declarations == [self.declaration()]
            && registry_package(source).is_some_and(|(registry, _)| {
                registries
                    .iter()
                    .any(|declared| declared.name() == registry)
            })
            && self.from.as_ref().is_none_or(|from| from == source)
            && resolved_shape(tool)
    }
}

fn resolved_shape(tool: &Tool) -> bool {
    let Installation::Managed { artifacts } = &tool.installation else {
        return false;
    };
    artifacts.iter().all(|(platform, parts)| {
        let (Some(platform), Some(artifact)) = (
            declared_platform(*platform).filter(|platform| TARGETS.contains(platform)),
            parts.get(PART).filter(|_| parts.len() == 1),
        ) else {
            return false;
        };
        artifact.sha256.is_some()
            && artifact.sha512.is_none()
            && artifact.root_prefix.is_none()
            && artifact.destination.is_none()
            && artifact.alternates.is_empty()
            && derived_artifact_violation(
                platform,
                &artifact.url,
                declared_format(artifact.format),
                &artifact.executables,
            )
            .is_none()
    })
}

fn complete(tool: &Tool) -> bool {
    matches!(&tool.installation, Installation::Managed { artifacts }
        if TARGETS.iter().all(|target| artifacts.contains_key(&lock_platform(*target))))
}

pub(super) struct Collision<'a> {
    pub tool: &'a str,
    pub other: &'a str,
    pub executable: &'a str,
}

pub(super) fn collision(tools: &BTreeMap<String, Tool>) -> Option<Collision<'_>> {
    let mut owners = BTreeMap::new();
    for (id, tool) in tools {
        let Installation::Managed { artifacts } = &tool.installation else {
            continue;
        };
        for (platform, parts) in artifacts {
            let names = parts.values().flat_map(|part| part.executables.keys());
            for name in names {
                let command = command_name(name, platform.windows());
                match owners.insert((*platform, command), id.as_str()) {
                    Some(owner) if owner != id => {
                        return Some(Collision {
                            tool: owner,
                            other: id,
                            executable: name,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

#[derive(Default)]
pub(super) struct Projection {
    /// `setup.registries` in declared order. Empty without a root
    /// configuration, which declares no tool to resolve either.
    pub registries: Vec<SetupRegistry>,
    pub tools: BTreeMap<String, Tool>,
    pub unresolved: BTreeMap<String, CatalogRequest>,
    pub incomplete: BTreeMap<String, CatalogRequest>,
}

pub(super) fn project(file: &str, bytes: &[u8], previous: Option<&Lock>) -> Option<Projection> {
    let text = std::str::from_utf8(bytes).ok()?;
    let raw: RawTurboJson = RawRootTurboJson::parse(text, file).ok()?.try_into().ok()?;
    let flags = raw.future_flags.as_ref().map(|flags| *flags.as_inner());
    let policy = SetupPolicy::from_raw(raw.setup.as_ref(), &flags.unwrap_or_default()).ok()?;
    let mut projection = Projection {
        registries: policy.registries().to_vec(),
        ..Projection::default()
    };
    for (id, tool) in policy.additional_tools() {
        let artifacts = match &tool.source {
            AdditionalToolSource::Artifacts(artifacts) => artifacts,
            AdditionalToolSource::Catalog { from } => {
                let request = CatalogRequest {
                    id: id.clone(),
                    version: tool.version.clone(),
                    from: from.clone(),
                    file: file.into(),
                };
                let locked = previous.and_then(|lock| lock.tools().get(id));
                match locked.filter(|tool| request.records(tool, &projection.registries)) {
                    Some(tool) => {
                        projection.tools.insert(id.clone(), tool.clone());
                        if !complete(tool) {
                            projection.incomplete.insert(id.clone(), request);
                        }
                    }
                    None => {
                        projection.unresolved.insert(id.clone(), request);
                    }
                }
                continue;
            }
        };
        let artifacts = artifacts.iter().map(|(platform, artifact)| {
            let artifact = Artifact {
                url: artifact.url.as_inner().clone(),
                sha256: Some(artifact.sha256.as_inner().clone()),
                sha512: None,
                format: lock_format(*artifact.format.as_inner()),
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
        let declaration = declaration(file, id, &tool.version);
        let tool = entry(declaration, BTreeMap::new(), artifacts.collect());
        projection.tools.insert(id.clone(), tool);
    }
    Some(projection)
}

fn declaration(file: &str, id: &str, version: &str) -> Declaration {
    Declaration {
        file: file.into(),
        field: Some(format!("{FIELD}{id}")),
        request: Some(version.into()),
    }
}

fn entry(
    declaration: Declaration,
    options: BTreeMap<String, Vec<String>>,
    artifacts: BTreeMap<Platform, BTreeMap<String, Artifact>>,
) -> Tool {
    Tool {
        adapter: ADAPTER.into(),
        version: declaration.request.clone().unwrap_or_default(),
        declarations: vec![declaration],
        options,
        installation: Installation::Managed { artifacts },
    }
}

/// Whether a locked entry claims a declaration that the snapshot's root
/// configuration covers. Its contents must still match the projection.
pub(super) fn covered(id: &str, tool: &Tool) -> bool {
    tool.adapter == ADAPTER
        && matches!(tool.declarations.as_slice(), [Declaration { file, field: Some(field), .. }]
            if turborepo_types::CONFIG_FILES.contains(&file.as_str())
                && field.strip_prefix(FIELD) == Some(id))
}

pub fn lock_format(format: ArtifactFormat) -> Format {
    match format {
        ArtifactFormat::Binary => Format::Binary,
        ArtifactFormat::Tar => Format::Tar,
        ArtifactFormat::TarGz => Format::TarGz,
        ArtifactFormat::Zip => Format::Zip,
    }
}

fn declared_format(format: Format) -> ArtifactFormat {
    match format {
        Format::Binary => ArtifactFormat::Binary,
        Format::Tar => ArtifactFormat::Tar,
        Format::TarGz => ArtifactFormat::TarGz,
        Format::Zip => ArtifactFormat::Zip,
    }
}

fn declared_platform(platform: Platform) -> Option<ArtifactPlatform> {
    Some(match platform {
        Platform::Any => return None,
        Platform::MacosX64 => ArtifactPlatform::MacosX64,
        Platform::MacosArm64 => ArtifactPlatform::MacosArm64,
        Platform::LinuxX64Gnu => ArtifactPlatform::LinuxX64Gnu,
        Platform::LinuxArm64Gnu => ArtifactPlatform::LinuxArm64Gnu,
        Platform::LinuxX64Musl => ArtifactPlatform::LinuxX64Musl,
        Platform::LinuxArm64Musl => ArtifactPlatform::LinuxArm64Musl,
        Platform::WindowsX64 => ArtifactPlatform::WindowsX64,
        Platform::WindowsArm64 => ArtifactPlatform::WindowsArm64,
    })
}

pub fn lock_platform(platform: ArtifactPlatform) -> Platform {
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
