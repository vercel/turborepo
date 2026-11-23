use std::collections::BTreeMap;

use aqua_registry::AquaPackage;
use turborepo_setup::lock::additional::CatalogRequest;
use turborepo_turbo_json::registry_package;

mod plan;
mod registry;
mod resolve;

pub use plan::{Plan, PlannedArtifact, ReleasePlan, Unavailable};
#[cfg(test)]
use registry::MAX_REGISTRY_BYTES;
pub use registry::{Limit, Position, Problem, Registry};
pub use resolve::{CatalogTransport, Resolved, resolve, spelling};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}", unknown_name(.tool, .version, .registries, .candidates))]
    UnknownName {
        tool: String,
        version: String,
        /// The registries that were asked, in order.
        registries: Vec<String>,
        /// Entries whose package ID ends in the tool's name, as
        /// `<registry>:<package ID>`.
        candidates: Vec<String>,
    },
    #[error(
        "the registry {registry:?} has no entry {package:?} for setup.additionalTools.{tool}; \
         check the package ID in `from`, or declare the tool's downloads with `artifacts` instead"
    )]
    UnknownPackage {
        tool: String,
        registry: String,
        package: String,
    },
    #[error(
        "the registry {registry:?} knows {tool:?} under a package name that turbo.lock cannot \
         record. A package ID is `/`-separated segments of ASCII letters, digits, `.`, `_`, and \
         `-`, at most 256 bytes. Rename the package in the registry, or declare the tool's \
         downloads with `artifacts` under setup.additionalTools.{tool}."
    )]
    UnrecordablePackage { tool: String, registry: String },
    #[error(
        "setup.additionalTools.{tool} names {from:?} in `from`, which is not an entry of a \
         registry that the tool catalog asks"
    )]
    UnknownRegistry { tool: String, from: String },
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

fn unknown_name(tool: &str, version: &str, registries: &[String], candidates: &[String]) -> String {
    let artifacts = format!(
        "{{ \"version\": {version:?}, \"artifacts\": {{ ... }} }} under \
         setup.additionalTools.{tool}"
    );
    let (asked, registry) = match registries {
        [registry] => (format!("the registry {registry:?} knows"), &**registry),
        registries => {
            let asked = format!(
                "any registry knows. These were asked, in order: {}",
                registries.join(", ")
            );
            (asked, "<registry>")
        }
    };
    let entries = match candidates {
        [] => String::new(),
        candidates => {
            let quoted: Vec<_> = candidates
                .iter()
                .map(|entry| format!("{entry:?}"))
                .collect();
            format!(" Entries with that name: {}.", quoted.join(", "))
        }
    };
    format!(
        "{tool:?} is not a tool name that {asked}.{entries}\nEither name its registry entry with \
         {{ \"version\": {version:?}, \"from\": \"{registry}:<package ID>\" }}, or declare its \
         downloads with {artifacts}."
    )
}

fn reasons(platforms: &BTreeMap<String, Unavailable>) -> String {
    platforms
        .iter()
        .map(|(platform, reason)| format!("  {platform}: {reason}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct Catalog<'a> {
    registries: Vec<Registry<'a>>,
}

impl<'a> Catalog<'a> {
    pub fn new(registries: Vec<Registry<'a>>) -> Self {
        Self { registries }
    }

    /// The recipe that answers `request`, and the registry entry that
    /// `turbo.lock` records for it. `from` reaches only the registry it names
    /// and is recorded as written, because a recorded entry is reused while
    /// it equals `from`. Without `from`, the first registry that knows the
    /// tool ID answers, and the entry is its recipe's own package ID, not an
    /// alias of it. A registry that cannot be read before that fails the
    /// lookup.
    fn entry(&self, request: &CatalogRequest) -> Result<(String, &AquaPackage), Error> {
        let Some(from) = &request.from else {
            for registry in &self.registries {
                let Some((package, recipe)) = registry.named(&request.id)? else {
                    continue;
                };
                let source = format!("{}:{package}", registry.name());
                if registry_package(&source).is_none() {
                    return Err(Error::UnrecordablePackage {
                        tool: request.id.clone(),
                        registry: registry.name().to_owned(),
                    });
                }
                return Ok((source, recipe));
            }
            return Err(self.unknown_name(request)?);
        };
        let unknown = || Error::UnknownRegistry {
            tool: request.id.clone(),
            from: from.clone(),
        };
        let (name, package) = registry_package(from).ok_or_else(unknown)?;
        let registry = self.registries.iter().find(|r| r.name() == name);
        match registry.ok_or_else(unknown)?.recipes()?.recipe(package) {
            Some((_, recipe)) => Ok((from.clone(), recipe)),
            None => Err(Error::UnknownPackage {
                tool: request.id.clone(),
                registry: name.to_owned(),
                package: package.to_owned(),
            }),
        }
    }

    /// The error for a tool ID that no registry knows, with the entries that
    /// `from` could name instead.
    fn unknown_name(&self, request: &CatalogRequest) -> Result<Error, Error> {
        let mut candidates = Vec::new();
        for registry in &self.registries {
            let packages = registry.recipes()?.packages_named(&request.id);
            let entries = packages
                .iter()
                .map(|package| format!("{}:{package}", registry.name()));
            candidates.extend(entries.filter(|entry| registry_package(entry).is_some()));
        }
        Ok(Error::UnknownName {
            tool: request.id.clone(),
            version: request.version.clone(),
            registries: self
                .registries
                .iter()
                .map(|r| r.name().to_owned())
                .collect(),
            candidates,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
