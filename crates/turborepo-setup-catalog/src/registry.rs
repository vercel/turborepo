//! One registry of the tool catalog: recipes, under a name, that say where a
//! tool's downloads are. The built-in registry also has short names for its
//! recipes.
//!
//! A registry is read from its source the first time a lookup asks it, and at
//! most once. This crate reads no file: whoever declares a registry supplies
//! the source. The bytes become recipes only through [`Recipes::load`], which
//! checks them before the recipe reader sees them, whatever the source.
//!
//! A [`Problem`] says what kind of fault a registry has and where. It never
//! quotes the registry, which can be any file of the repository.

use std::{collections::BTreeMap, io, io::Read, sync::OnceLock};

use aqua_registry::{AquaPackage, RegistryYaml};
use turborepo_turbo_json::BUILTIN_REGISTRY;

use super::Error;

mod limits;

pub use limits::Limit;

const NAMES: &str = include_str!("../catalog/names.tsv");
const BUNDLED: &[u8] = include_bytes!("../catalog/aqua-registry.yaml.gz");
const MAX_BUNDLED_BYTES: usize = 64 * 1024 * 1024;
/// The largest registry that a repository can declare.
#[cfg(test)]
pub const MAX_REGISTRY_BYTES: usize = 16 * 1024 * 1024;

/// Reads a registry's bytes up to the given number. It fails with `NotFound`
/// when there is no registry. When there are more bytes, it fails with
/// `FileTooLarge` or returns more than that number.
type Source<'a> = Box<dyn Fn(usize) -> io::Result<Vec<u8>> + 'a>;

pub struct Registry<'a> {
    name: String,
    /// Where the registry comes from, as an error shows it.
    origin: String,
    /// The tool IDs that the registry answers to, each with its package ID.
    /// Without them, a registry answers to the IDs and aliases of its recipes.
    names: Option<BTreeMap<String, String>>,
    limit: usize,
    source: Source<'a>,
    recipes: OnceLock<Result<Recipes, Problem>>,
}

/// A registry's recipes by package ID.
pub(super) struct Recipes {
    recipes: BTreeMap<String, AquaPackage>,
    /// Other IDs that a recipe answers to, each with the recipe's package ID.
    aliases: BTreeMap<String, String>,
}

/// Why a registry cannot be used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    #[error("it does not exist")]
    Missing,
    #[error("it cannot be read: {0}")]
    Unreadable(String),
    #[error("it is larger than {} MiB", .0 >> 20)]
    TooLarge(usize),
    #[error("it is not UTF-8 text")]
    NotUtf8,
    #[error("it is not a registry of packages in YAML{}", Position::shown(.at))]
    Malformed { at: Option<Position> },
    #[error("it has no packages")]
    NoPackages,
    #[error("{limit}{}", Position::shown(.at))]
    Limit { limit: Limit, at: Option<Position> },
}

/// A place in a registry's text, counted from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub line: u64,
    pub column: u64,
}

impl Position {
    fn of(error: &serde_saphyr::Error) -> Option<Self> {
        let location = error.location()?;
        Some(Self {
            line: location.line(),
            column: location.column(),
        })
    }

    fn shown(position: &Option<Self>) -> String {
        position.map_or_else(String::new, |Self { line, column }| {
            format!(" (line {line}, column {column})")
        })
    }
}

impl<'a> Registry<'a> {
    /// The registry built into `turbo`, named `aqua`.
    pub fn aqua() -> Self {
        Self::built_in(NAMES, Box::new(bundled))
    }

    /// A stand-in for the built-in registry with its own names and recipes.
    #[cfg(any(test, feature = "test-support"))]
    pub fn aqua_for_tests(names_tsv: &str, registry_yaml: &str) -> Self {
        let yaml = registry_yaml.as_bytes().to_vec();
        Self::built_in(names_tsv, Box::new(move |_| Ok(yaml.clone())))
    }

    fn built_in(names_tsv: &str, source: Source<'a>) -> Self {
        Self {
            name: BUILTIN_REGISTRY.to_owned(),
            origin: "built into turbo".to_owned(),
            names: Some(names(names_tsv)),
            limit: MAX_BUNDLED_BYTES,
            source,
            recipes: OnceLock::new(),
        }
    }

    /// The registry that a repository declares as `name`. `origin` is where it
    /// comes from, such as the path of its file, and `source` reads it.
    #[cfg(test)]
    pub fn declared(
        name: &str,
        origin: &str,
        source: impl Fn(usize) -> io::Result<Vec<u8>> + 'a,
    ) -> Self {
        Self {
            name: name.to_owned(),
            origin: origin.to_owned(),
            names: None,
            limit: MAX_REGISTRY_BYTES,
            source: Box::new(source),
            recipes: OnceLock::new(),
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    /// The recipe that answers to the tool `id`, with its package ID: the one
    /// that the short names give for `id`, or without short names, the one
    /// with that ID or alias. A registry whose short names do not have `id`
    /// is not read.
    pub(super) fn named(&self, id: &str) -> Result<Option<(&str, &AquaPackage)>, Error> {
        let package = match &self.names {
            Some(names) => names.get(id).map(String::as_str),
            None => Some(id),
        };
        match package {
            Some(package) => Ok(self.recipes()?.recipe(package)),
            None => Ok(None),
        }
    }

    /// This registry's recipes, read and checked the first time they are
    /// asked for. A fault is kept, so a registry is read once either way.
    pub(super) fn recipes(&self) -> Result<&Recipes, Error> {
        let recipes = self.recipes.get_or_init(|| {
            let bytes = (self.source)(self.limit).map_err(|error| match error.kind() {
                io::ErrorKind::NotFound => Problem::Missing,
                io::ErrorKind::FileTooLarge => Problem::TooLarge(self.limit),
                _ => Problem::Unreadable(error.to_string()),
            })?;
            Recipes::load(&bytes, self.limit)
        });
        recipes.as_ref().map_err(|problem| Error::Registry {
            registry: self.name.clone(),
            origin: self.origin.clone(),
            problem: problem.clone(),
        })
    }
}

/// The bundled registry's text, read up to one byte past `limit`.
pub(super) fn bundled(limit: usize) -> io::Result<Vec<u8>> {
    let mut yaml = Vec::new();
    flate2::read::GzDecoder::new(BUNDLED)
        .take(limit as u64 + 1)
        .read_to_end(&mut yaml)?;
    Ok(yaml)
}

impl Recipes {
    /// Checks registry bytes, whatever their source, and only then reads
    /// them: their size, that they are UTF-8, and the limits on the document
    /// that keep the recipe reader from crashing or running without bound.
    fn load(bytes: &[u8], limit: usize) -> Result<Self, Problem> {
        if bytes.len() > limit {
            return Err(Problem::TooLarge(limit));
        }
        let yaml = std::str::from_utf8(bytes).map_err(|_| Problem::NotUtf8)?;
        limits::check(yaml)?;
        let rows = RegistryYaml::from_yaml(yaml)
            .map_err(|error| Problem::Malformed {
                at: Position::of(&error),
            })?
            .packages;
        // The recipe reader's index neither lists its packages nor says
        // which one answered an alias, so this one follows its rules: a
        // recipe's ID is its name, else its repository, else its path, a
        // later recipe replaces an earlier one with the same ID, and an alias
        // never hides a recipe's ID.
        let mut recipes = BTreeMap::new();
        let mut aliases = BTreeMap::new();
        for row in rows {
            let recipe = row.package;
            let repository = (!recipe.repo_owner.is_empty() && !recipe.repo_name.is_empty())
                .then(|| format!("{}/{}", recipe.repo_owner, recipe.repo_name));
            let id = recipe.name.clone().or(repository);
            let Some(id) = id.or_else(|| recipe.path.clone()) else {
                continue;
            };
            aliases.extend(row.aliases.into_iter().map(|alias| (alias, id.clone())));
            recipes.insert(id, recipe);
        }
        aliases.retain(|alias, _| !recipes.contains_key(alias));
        if recipes.is_empty() {
            return Err(Problem::NoPackages);
        }
        Ok(Self { recipes, aliases })
    }

    /// The recipe with this package ID or alias, and its package ID.
    pub(super) fn recipe(&self, package: &str) -> Option<(&str, &AquaPackage)> {
        let id = self.aliases.get(package).map_or(package, String::as_str);
        let (id, recipe) = self.recipes.get_key_value(id)?;
        Some((id, recipe))
    }

    /// Package IDs whose last segment is `name`, sorted.
    pub(super) fn packages_named(&self, name: &str) -> Vec<&str> {
        let ids = self.recipes.keys().map(String::as_str);
        ids.filter(|id| id.rsplit('/').next() == Some(name))
            .collect()
    }

    #[cfg(test)]
    pub(super) fn all(&self) -> impl Iterator<Item = (&str, &AquaPackage)> {
        let recipes = self.recipes.iter();
        recipes.map(|(id, recipe)| (id.as_str(), recipe))
    }
}

fn names(tsv: &str) -> BTreeMap<String, String> {
    tsv.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('\t'))
        .map(|(name, package)| (name.to_owned(), package.trim_end().to_owned()))
        .collect()
}

#[cfg(test)]
impl Registry<'_> {
    pub(super) fn names(&self) -> &BTreeMap<String, String> {
        self.names.as_ref().expect("the built-in registry")
    }
}
