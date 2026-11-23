use std::{collections::BTreeMap, io::Read, sync::OnceLock};

use aqua_registry::ParsedRegistry;
use turborepo_setup::lock::additional::CatalogRequest;
use turborepo_turbo_json::{BUILTIN_REGISTRY, registry_package};

mod plan;
mod resolve;

pub use plan::{Plan, PlannedArtifact, ReleasePlan, Unavailable};
pub use resolve::{CatalogTransport, Resolved, resolve, spelling};

const NAMES: &str = include_str!("../catalog/names.tsv");
const REGISTRY: &[u8] = include_bytes!("../catalog/aqua-registry.yaml.gz");
const MAX_REGISTRY_BYTES: u64 = 64 * 1024 * 1024;

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
    #[error("cannot read the bundled tool catalog: {0}")]
    Registry(String),
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
    names: BTreeMap<String, String>,
    recipes: Recipes,
    registry: OnceLock<Result<ParsedRegistry, String>>,
}

enum Recipes {
    Bundled,
    #[cfg(any(test, feature = "test-support"))]
    Yaml(String),
}

impl Catalog {
    pub fn bundled() -> Self {
        Self {
            names: names(NAMES),
            recipes: Recipes::Bundled,
            registry: OnceLock::new(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(names_tsv: &str, registry_yaml: &str) -> Self {
        Self {
            names: names(names_tsv),
            recipes: Recipes::Yaml(registry_yaml.into()),
            registry: OnceLock::new(),
        }
    }

    fn yaml(&self) -> Result<String, Error> {
        match &self.recipes {
            Recipes::Bundled => {
                let mut yaml = String::new();
                flate2::read::GzDecoder::new(REGISTRY)
                    .take(MAX_REGISTRY_BYTES)
                    .read_to_string(&mut yaml)
                    .map_err(|error| Error::Registry(error.to_string()))?;
                Ok(yaml)
            }
            #[cfg(any(test, feature = "test-support"))]
            Recipes::Yaml(yaml) => Ok(yaml.clone()),
        }
    }

    fn registry(&self) -> Result<&ParsedRegistry, Error> {
        let parse = || {
            let yaml = self.yaml().map_err(|error| error.to_string())?;
            ParsedRegistry::parse_yaml(&yaml).map_err(|error| error.to_string())
        };
        self.registry
            .get_or_init(parse)
            .as_ref()
            .map_err(|error| Error::Registry(error.clone()))
    }

    fn package(&self, request: &CatalogRequest) -> Result<String, Error> {
        if let Some((_, package)) = request.from.as_deref().and_then(registry_package) {
            return Ok(package.into());
        }
        if let Some(package) = self.names.get(&request.id) {
            return Ok(package.clone());
        }
        Err(Error::UnknownName {
            tool: request.id.clone(),
            version: request.version.clone(),
            candidates: self.packages_named(&request.id)?,
        })
    }

    /// Package IDs whose last segment is `name`, sorted. The registry lookup
    /// has no listing, so this error path reads the recipes a second time.
    fn packages_named(&self, name: &str) -> Result<Vec<String>, Error> {
        let registry = aqua_registry::RegistryYaml::from_yaml(&self.yaml()?)
            .map_err(|error| Error::Registry(error.to_string()))?;
        let mut packages: Vec<_> = registry
            .packages
            .into_iter()
            .filter_map(|row| {
                let package = row.package;
                package.name.or_else(|| {
                    (!package.repo_owner.is_empty() && !package.repo_name.is_empty())
                        .then(|| format!("{}/{}", package.repo_owner, package.repo_name))
                })
            })
            .filter(|package| {
                package.rsplit('/').next() == Some(name) && catalog_package_id(package)
            })
            .collect();
        packages.sort();
        packages.dedup();
        Ok(packages)
    }
}

fn catalog_package_id(package: &str) -> bool {
    registry_package(&format!("{BUILTIN_REGISTRY}:{package}")).is_some()
}

fn names(tsv: &str) -> BTreeMap<String, String> {
    tsv.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('\t'))
        .map(|(name, package)| (name.to_owned(), package.trim_end().to_owned()))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests;
