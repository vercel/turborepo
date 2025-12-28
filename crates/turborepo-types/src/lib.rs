//! Shared types for Turborepo
//!
//! This crate contains types that are used across multiple crates in the
//! turborepo ecosystem. It serves as a foundation layer to avoid circular
//! dependencies between higher-level crates.

use std::fmt;

use biome_deserialize_macros::Deserializable;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Environment mode for task execution.
///
/// Controls how environment variables are handled during task execution:
/// - `Loose`: Allows all environment variables to be passed through
/// - `Strict`: Only explicitly configured environment variables are passed
///   through
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Serialize, ValueEnum, Deserialize, Eq, Deserializable,
)]
#[serde(rename_all = "lowercase")]
pub enum EnvMode {
    Loose,
    #[default]
    Strict,
}

impl fmt::Display for EnvMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EnvMode::Loose => "loose",
            EnvMode::Strict => "strict",
        })
    }
}

/// TaskOutputs represents the patterns for including and excluding files from
/// outputs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskOutputs {
    pub inclusions: Vec<String>,
    pub exclusions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_mode_display() {
        assert_eq!(format!("{}", EnvMode::Loose), "loose");
        assert_eq!(format!("{}", EnvMode::Strict), "strict");
    }

    #[test]
    fn env_mode_default() {
        assert_eq!(EnvMode::default(), EnvMode::Strict);
    }

    #[test]
    fn task_outputs_default() {
        let outputs = TaskOutputs::default();
        assert!(outputs.inclusions.is_empty());
        assert!(outputs.exclusions.is_empty());
    }
}
