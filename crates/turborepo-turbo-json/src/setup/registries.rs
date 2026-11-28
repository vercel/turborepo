//! Registries under `setup.registries`: the ordered sources that `turbo setup`
//! asks where an additional tool comes from. A registry is consulted only to
//! resolve a declaration that `turbo.lock` does not record.
//!
//! An absent list is `["aqua"]`, the registry built into `turbo`. Names are
//! unique, and every `from` in `setup.additionalTools` names a declared
//! registry, so leaving `"aqua"` out blocks it.

use biome_deserialize::{
    Deserializable, DeserializableValue, DeserializationDiagnostic, VisitableType,
};
use biome_deserialize_macros::Deserializable;
use schemars::JsonSchema;
use serde::Serialize;
use turborepo_errors::Spanned;

use super::additional_tools::identifier;
use crate::error::Error;

/// The name of the registry built into `turbo`.
pub const BUILTIN_REGISTRY: &str = "aqua";

/// The registry built into `turbo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserializable, JsonSchema)]
pub enum BuiltinRegistry {
    #[serde(rename = "aqua")]
    Aqua,
}

/// A registry that `turbo setup` asks where an additional tool comes from.
/// The string `"aqua"` is the registry built into `turbo`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
#[schemars(rename = "SetupRegistry")]
pub enum RawSetupRegistry {
    Builtin(BuiltinRegistry),
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
        } else {
            diagnostics.push(DeserializationDiagnostic::new_incorrect_type(
                actual,
                VisitableType::STR,
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
}

impl SetupRegistry {
    pub fn name(&self) -> &str {
        match self {
            Self::Aqua => BUILTIN_REGISTRY,
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

#[cfg(test)]
mod tests;
