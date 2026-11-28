use std::collections::BTreeMap;

use aqua_registry::AquaPackage;
use turborepo_setup::lock::additional::CatalogRequest;
use turborepo_turbo_json::{BUILTIN_REGISTRY, registry_package};

mod plan;
mod registry;
mod resolve;

pub use plan::{Plan, PlannedArtifact, ReleasePlan, Unavailable};
use registry::Registry;
pub use registry::{Position, Problem};
pub use resolve::{CatalogTransport, Resolved, resolve, spelling};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "{tool:?} is not a tool name that turbo setup knows.{}\nEither name its catalog entry \
         with {{ \"version\": {version:?}, \"from\": \"aqua:<package ID>\" }}, or declare its \
         downloads with {{ \"version\": {version:?}, \"artifacts\": {{ ... }} }} under \
         setup.additionalTools.{tool}.",
        suggestions(.candidates)
    )]
    UnknownName {
        tool: String,
        version: String,
        /// Catalog package IDs whose last segment is the tool's name.
        candidates: Vec<String>,
    },
    #[error(
        "the tool catalog has no entry {package:?} for setup.additionalTools.{tool}; check the \
         package ID in `from`, or declare the tool's downloads with `artifacts` instead"
    )]
    UnknownPackage { tool: String, package: String },
    #[error("cannot use the registry {registry:?} ({origin}): {problem}")]
    Registry {
        registry: String,
        /// Where the registry comes from, such as the path of its file.
        origin: String,
        problem: Problem,
    },
    #[error(
        "cannot find {tool} {version}: no download exists under the release names {}. Check \
         that {version:?} is a released version of {package}, written without a `v` or other \
         prefix; or declare the tool's downloads with `artifacts` under \
         setup.additionalTools.{tool}.",
        .tried.join(", ")
    )]
    NoRelease {
        tool: String,
        version: String,
        package: String,
        tried: Vec<String>,
    },
    #[error(
        "{tool} {version} cannot be installed on this machine ({platform}): {reason}. Declare its \
         downloads with `artifacts` under setup.additionalTools.{tool} instead."
    )]
    Host {
        tool: String,
        version: String,
        platform: String,
        reason: Unavailable,
    },
    #[error(
        "{tool} {version} has no download that turbo setup can install:\n{}\nDeclare its \
         downloads with `artifacts` under setup.additionalTools.{tool} instead.",
        reasons(.platforms)
    )]
    NoPlatform {
        tool: String,
        version: String,
        platforms: BTreeMap<String, Unavailable>,
    },
    #[error("cannot download {tool} {version} for {platform}: {source}")]
    Download {
        tool: String,
        version: String,
        platform: String,
        source: turborepo_download::Error,
    },
    #[error("cannot prepare tool catalog downloads: {0}")]
    Transport(#[source] turborepo_download::Error),
}

fn suggestions(candidates: &[String]) -> String {
    match candidates {
        [] => String::new(),
        candidates => format!(
            " The catalog has {}.",
            candidates
                .iter()
                .map(|package| format!("\"{BUILTIN_REGISTRY}:{package}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn reasons(platforms: &BTreeMap<String, Unavailable>) -> String {
    platforms
        .iter()
        .map(|(platform, reason)| format!("  {platform}: {reason}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct Catalog {
    registry: Registry<'static>,
}

impl Catalog {
    pub fn bundled() -> Self {
        Self {
            registry: Registry::aqua(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(names_tsv: &str, registry_yaml: &str) -> Self {
        Self {
            registry: Registry::aqua_for_tests(names_tsv, registry_yaml),
        }
    }

    /// The recipe that answers `request`, and the registry entry that
    /// `turbo.lock` records for it. `from` is recorded as written, because a
    /// recorded entry is reused while it equals `from`. Without `from`, the
    /// entry is the recipe's own package ID, not an alias of it.
    fn entry(&self, request: &CatalogRequest) -> Result<(String, &AquaPackage), Error> {
        let registry = &self.registry;
        let Some(from) = &request.from else {
            let Some((package, recipe)) = registry.named(&request.id)? else {
                return Err(self.unknown_name(request)?);
            };
            return Ok((format!("{}:{package}", registry.name()), recipe));
        };
        let package = registry_package(from).map_or(&**from, |(_, package)| package);
        match registry.recipes()?.recipe(package) {
            Some((_, recipe)) => Ok((from.clone(), recipe)),
            None => Err(Error::UnknownPackage {
                tool: request.id.clone(),
                package: package.to_owned(),
            }),
        }
    }

    /// The error for a tool ID that the registry does not know, with the
    /// packages that `from` could name instead.
    fn unknown_name(&self, request: &CatalogRequest) -> Result<Error, Error> {
        let packages = self.registry.recipes()?.packages_named(&request.id);
        let recordable =
            |package: &&str| registry_package(&format!("{BUILTIN_REGISTRY}:{package}")).is_some();
        Ok(Error::UnknownName {
            tool: request.id.clone(),
            version: request.version.clone(),
            candidates: packages
                .into_iter()
                .filter(recordable)
                .map(str::to_owned)
                .collect(),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
