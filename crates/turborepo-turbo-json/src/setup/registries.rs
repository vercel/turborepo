//! Registries under `setup.registries`: the ordered sources that `turbo setup`
//! asks where an additional tool comes from. A registry is consulted only to
//! resolve a declaration that `turbo.lock` does not record.
//!
//! An absent list is `["aqua"]`, the registry built into `turbo`. Names are
//! unique, and every `from` in `setup.additionalTools` names a declared
//! registry, so leaving `"aqua"` out blocks it.

use std::sync::Arc;

use biome_deserialize::{
    Deserializable, DeserializableValue, DeserializationDiagnostic, VisitableType,
};
use biome_deserialize_macros::Deserializable;
use schemars::JsonSchema;
use serde::Serialize;
use turborepo_errors::{Spanned, WithMetadata};

use super::additional_tools::{identifier, portable_path};
use crate::error::Error;

/// The name of the registry built into `turbo`.
pub const BUILTIN_REGISTRY: &str = "aqua";

/// The registry built into `turbo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserializable, JsonSchema)]
pub enum BuiltinRegistry {
    #[serde(rename = "aqua")]
    Aqua,
}

// The derive's required-field check cannot name generic paths.
type SpannedString = Spanned<String>;

/// A registry that `turbo setup` asks where an additional tool comes from.
/// The string `"aqua"` is the registry built into `turbo`. An object is a
/// registry file in the repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
#[schemars(rename = "SetupRegistry")]
pub enum RawSetupRegistry {
    Builtin(BuiltinRegistry),
    Declaration(RawSetupRegistryDeclaration),
}

/// The object form of a registry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserializable, JsonSchema)]
#[schemars(rename = "SetupRegistryDeclaration")]
#[deserializable(unknown_fields = "deny")]
pub struct RawSetupRegistryDeclaration {
    /// The name that `from` and `turbo.lock` use for this registry: a
    /// lowercase letter, then lowercase letters, digits, `-`, or `_`. It
    /// cannot be `aqua`.
    #[deserializable(required)]
    pub name: SpannedString,
    /// The registry file, as a path relative to the repository root that
    /// ends in `.yaml` or `.yml`.
    #[deserializable(required)]
    pub path: SpannedString,
}

impl Deserializable for RawSetupRegistry {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let actual = value.visitable_type()?;
        if actual == VisitableType::STR {
            Deserializable::deserialize(value, name, diagnostics).map(Self::Builtin)
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

/// A validated `setup.registries` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupRegistry {
    /// The registry built into `turbo`, named `aqua`.
    Aqua,
    /// A registry file at `path`, relative to the repository root.
    File { name: String, path: String },
}

impl SetupRegistry {
    pub fn name(&self) -> &str {
        match self {
            Self::Aqua => BUILTIN_REGISTRY,
            Self::File { name, .. } => name,
        }
    }
}

/// Splits a tool's source, written as `<registry>:<package ID>`, into its
/// registry name and package ID. The registry follows the registry-name rule.
/// The package ID is one or more `/`-separated segments of ASCII letters,
/// digits, `.`, `_`, and `-`, at most 256 bytes. `from` and `turbo.lock` share
/// this spelling.
pub fn registry_package(source: &str) -> Option<(&str, &str)> {
    let (registry, package) = source.split_once(':')?;
    let segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    (identifier(registry) && package.len() <= 256 && package.split('/').all(segment))
        .then_some((registry, package))
}

pub(super) fn validate(raw: &[Spanned<RawSetupRegistry>]) -> Result<Vec<SetupRegistry>, Error> {
    let mut registries: Vec<SetupRegistry> = Vec::new();
    for entry in raw {
        let (registry, declared_at) = match entry.as_inner() {
            RawSetupRegistry::Builtin(BuiltinRegistry::Aqua) => (SetupRegistry::Aqua, entry.to(())),
            RawSetupRegistry::Declaration(RawSetupRegistryDeclaration { name, path }) => {
                if name.as_str() == BUILTIN_REGISTRY {
                    let reason = "`aqua` names the built-in registry, which is declared as the \
                                  string \"aqua\"; give this registry another name";
                    return Err(invalid(name, reason));
                }
                if !identifier(name) {
                    let reason = "registry names start with a lowercase letter and use at most 64 \
                                  lowercase letters, digits, `-`, or `_`";
                    return Err(invalid(name, reason));
                }
                if !portable_path(path) {
                    let reason = "`path` must be a portable path relative to the repository root, \
                                  without `..`";
                    return Err(invalid(path, reason));
                }
                // Without this, `path` could make any file of the repository
                // a registry, such as one that holds secrets.
                if !(path.ends_with(".yaml") || path.ends_with(".yml")) {
                    let reason = "`path` must name a YAML file, ending in `.yaml` or `.yml`";
                    return Err(invalid(path, reason));
                }
                let registry = SetupRegistry::File {
                    name: name.as_inner().clone(),
                    path: path.as_inner().clone(),
                };
                (registry, name.to(()))
            }
        };
        let name = registry.name();
        if registries.iter().any(|other| other.name() == name) {
            let reason = format!("the registry {name:?} is declared more than once");
            return Err(invalid(&declared_at, reason));
        }
        registries.push(registry);
    }
    Ok(registries)
}

fn invalid<T>(at: &Spanned<T>, reason: impl Into<String>) -> Error {
    let (span, text) = at.span_and_text("turbo.json");
    Error::SetupInvalidRegistry {
        reason: reason.into(),
        span,
        text,
    }
}

impl WithMetadata for RawSetupRegistry {
    fn add_text(&mut self, text: Arc<str>) {
        if let Self::Declaration(declaration) = self {
            declaration.name.add_text(text.clone());
            declaration.path.add_text(text);
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        if let Self::Declaration(declaration) = self {
            declaration.name.add_path(path.clone());
            declaration.path.add_path(path);
        }
    }
}

#[cfg(test)]
mod tests;
