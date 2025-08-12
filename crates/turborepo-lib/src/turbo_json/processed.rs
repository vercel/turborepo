//! Processed task definition types with DSL token handling

use turborepo_errors::Spanned;
use turborepo_unescape::UnescapedString;

use super::RawTaskDefinition;
use crate::cli::{EnvMode, OutputLogsMode};

const TURBO_ROOT: &str = "$TURBO_ROOT$";
const TURBO_ROOT_SLASH: &str = "$TURBO_ROOT$/";

/// A processed glob with separated components
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedGlob {
    /// The glob pattern without $TURBO_ROOT$ prefix
    pub glob: Spanned<UnescapedString>,
    /// Whether the glob was negated (started with !)
    pub negated: bool,
    /// Whether the glob needs turbo_root prefix (had $TURBO_ROOT$/)
    pub turbo_root: bool,
}

impl ProcessedGlob {
    /// Creates a ProcessedGlob from a raw glob string, stripping prefixes
    fn from_spanned_internal(
        mut value: Spanned<UnescapedString>,
        field: &'static str,
    ) -> Result<Self, crate::config::Error> {
        use camino::Utf8Path;

        use crate::config::Error;

        let original_value = value.clone();
        let mut negated = false;
        let mut turbo_root = false;
        let mut start_idx = 0;

        // Check for negation
        if value.as_str().starts_with("!") {
            negated = true;
            start_idx = 1;
        }

        // Check for TURBO_ROOT at the appropriate position
        if value.as_str()[start_idx..].starts_with(TURBO_ROOT) {
            // Validate it has the required slash
            if !value.as_str()[start_idx..].starts_with(TURBO_ROOT_SLASH) {
                let (span, text) = original_value.span_and_text("turbo.json");
                return Err(Error::InvalidTurboRootNeedsSlash { span, text });
            }
            turbo_root = true;
            // Strip the $TURBO_ROOT$/ prefix (keeping the content after it)
            let new_value = value.as_str()[start_idx + TURBO_ROOT_SLASH.len()..].to_string();
            *value.as_inner_mut() = UnescapedString::from(new_value);
        } else if value.as_str().contains(TURBO_ROOT) {
            // TURBO_ROOT found in the middle is not allowed
            let (span, text) = original_value.span_and_text("turbo.json");
            return Err(Error::InvalidTurboRootUse { span, text });
        } else if negated {
            // If negated but no TURBO_ROOT, strip the negation prefix
            let new_value = value.as_str()[1..].to_string();
            *value.as_inner_mut() = UnescapedString::from(new_value);
        }

        // Check for absolute paths (after stripping prefixes)
        if Utf8Path::new(value.as_str()).is_absolute() {
            let (span, text) = original_value.span_and_text("turbo.json");
            return Err(Error::AbsolutePathInConfig { field, span, text });
        }

        Ok(ProcessedGlob {
            glob: value,
            negated,
            turbo_root,
        })
    }

    /// Creates a ProcessedGlob for outputs (validates as output field)
    pub fn from_spanned_output(
        value: Spanned<UnescapedString>,
    ) -> Result<Self, crate::config::Error> {
        Self::from_spanned_internal(value, "outputs")
    }

    /// Creates a ProcessedGlob for inputs (validates as input field)
    pub fn from_spanned_input(
        value: Spanned<UnescapedString>,
    ) -> Result<Self, crate::config::Error> {
        Self::from_spanned_internal(value, "inputs")
    }

    /// Creates a resolved glob string with the actual path
    pub fn resolve(&self, turbo_root_path: &str) -> String {
        let mut result = String::new();

        if self.negated {
            result.push('!');
        }

        if self.turbo_root {
            result.push_str(turbo_root_path);
            if !turbo_root_path.ends_with('/') && !self.glob.as_str().is_empty() {
                result.push('/');
            }
        }

        result.push_str(self.glob.as_str());
        result
    }
}

/// Processed depends_on field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedDependsOn(pub Spanned<Vec<Spanned<UnescapedString>>>);

/// Processed env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedEnv(pub Vec<Spanned<UnescapedString>>);

/// Processed inputs field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedInputs(pub Vec<ProcessedGlob>);

impl ProcessedInputs {
    /// Resolves all globs with the given turbo_root path
    pub fn resolve(&self, turbo_root_path: &str) -> Vec<String> {
        self.0
            .iter()
            .map(|glob| glob.resolve(turbo_root_path))
            .collect()
    }
}

/// Processed pass_through_env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedPassThroughEnv(pub Vec<Spanned<UnescapedString>>);

/// Processed outputs field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedOutputs(pub Vec<ProcessedGlob>);

impl ProcessedOutputs {
    /// Resolves all globs with the given turbo_root path
    pub fn resolve(&self, turbo_root_path: &str) -> Vec<String> {
        self.0
            .iter()
            .map(|glob| glob.resolve(turbo_root_path))
            .collect()
    }
}

/// Intermediate representation for task definitions with DSL processing
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessedTaskDefinition {
    pub cache: Option<Spanned<bool>>,
    pub depends_on: Option<ProcessedDependsOn>,
    pub env: Option<ProcessedEnv>,
    pub inputs: Option<ProcessedInputs>,
    pub pass_through_env: Option<ProcessedPassThroughEnv>,
    pub persistent: Option<Spanned<bool>>,
    pub interruptible: Option<Spanned<bool>>,
    pub outputs: Option<ProcessedOutputs>,
    pub output_logs: Option<Spanned<OutputLogsMode>>,
    pub interactive: Option<Spanned<bool>>,
    pub env_mode: Option<Spanned<EnvMode>>,
    pub with: Option<Vec<Spanned<UnescapedString>>>,
}

impl ProcessedTaskDefinition {
    /// Creates a processed task definition from raw task
    pub fn from_raw(raw_task: RawTaskDefinition) -> Result<Self, crate::config::Error> {
        let inputs = raw_task
            .inputs
            .map(|inputs| -> Result<ProcessedInputs, crate::config::Error> {
                let globs = inputs
                    .into_iter()
                    .map(ProcessedGlob::from_spanned_input)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ProcessedInputs(globs))
            })
            .transpose()?;

        let outputs = raw_task
            .outputs
            .map(
                |outputs| -> Result<ProcessedOutputs, crate::config::Error> {
                    let globs = outputs
                        .into_iter()
                        .map(ProcessedGlob::from_spanned_output)
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(ProcessedOutputs(globs))
                },
            )
            .transpose()?;

        Ok(ProcessedTaskDefinition {
            cache: raw_task.cache,
            depends_on: raw_task.depends_on.map(ProcessedDependsOn),
            env: raw_task.env.map(ProcessedEnv),
            inputs,
            pass_through_env: raw_task.pass_through_env.map(ProcessedPassThroughEnv),
            persistent: raw_task.persistent,
            interruptible: raw_task.interruptible,
            outputs,
            output_logs: raw_task.output_logs,
            interactive: raw_task.interactive,
            env_mode: raw_task.env_mode,
            with: raw_task.with,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use test_case::test_case;
    use turborepo_errors::Spanned;
    use turborepo_unescape::UnescapedString;

    use super::*;

    #[test_case("$TURBO_ROOT$/config.txt", Ok((true, false)) ; "detects turbo root")]
    #[test_case("!$TURBO_ROOT$/README.md", Ok((true, true)) ; "detects negated turbo root")]
    #[test_case("src/**/*.ts", Ok((false, false)) ; "no turbo root")]
    fn test_processed_glob_detection(input: &str, expected: Result<(bool, bool), &str>) {
        // Test with input variant
        let result = ProcessedGlob::from_spanned_input(Spanned::new(UnescapedString::from(
            input.to_string(),
        )));

        match expected {
            Ok((turbo_root, negated)) => {
                let glob = result.unwrap();
                assert_eq!(glob.turbo_root, turbo_root);
                assert_eq!(glob.negated, negated);
            }
            Err(_) => {
                assert!(result.is_err());
            }
        }
    }

    #[test_case("$TURBO_ROOT$config.txt", "must be followed by a '/'" ; "missing slash")]
    #[test_case("../$TURBO_ROOT$/config.txt", "must be used at the start of glob" ; "middle turbo root")]
    fn test_processed_glob_validation_errors(input: &str, expected_error: &str) {
        // Test with input variant
        let result = ProcessedGlob::from_spanned_input(
            Spanned::new(UnescapedString::from(input.to_string()))
                .with_path(Arc::from("turbo.json"))
                .with_text(format!("\"{}\"", input))
                .with_range(1..input.len() + 1),
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains(expected_error));
    }

    #[test_case("$TURBO_ROOT$/config.txt", "../..", "../../config.txt" ; "replace turbo root")]
    #[test_case("!$TURBO_ROOT$/README.md", "../..", "!../../README.md" ; "replace negated turbo root")]
    #[test_case("src/**/*.ts", "../..", "src/**/*.ts" ; "no replacement needed")]
    fn test_processed_glob_resolution(input: &str, replacement: &str, expected: &str) {
        // Test with output variant
        let glob = ProcessedGlob::from_spanned_output(Spanned::new(UnescapedString::from(
            input.to_string(),
        )))
        .unwrap();

        let resolved = glob.resolve(replacement);
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_processed_task_definition_resolve() {
        // Create a raw task definition with TURBO_ROOT tokens
        let raw_task = super::RawTaskDefinition {
            inputs: Some(vec![
                Spanned::new(UnescapedString::from("$TURBO_ROOT$/config.txt")),
                Spanned::new(UnescapedString::from("src/**/*.ts")),
            ]),
            outputs: Some(vec![
                Spanned::new(UnescapedString::from("!$TURBO_ROOT$/README.md")),
                Spanned::new(UnescapedString::from("dist/**")),
            ]),
            ..Default::default()
        };

        // Convert to processed task definition
        let processed = ProcessedTaskDefinition::from_raw(raw_task).unwrap();

        // Verify TURBO_ROOT detection
        let inputs = processed.inputs.as_ref().unwrap();
        assert!(inputs.0[0].turbo_root);
        assert!(!inputs.0[0].negated);
        assert!(!inputs.0[1].turbo_root);

        let outputs = processed.outputs.as_ref().unwrap();
        assert!(outputs.0[0].turbo_root);
        assert!(outputs.0[0].negated);
        assert!(!outputs.0[1].turbo_root);

        // Resolve with turbo_root path
        let resolved_inputs = inputs.resolve("../..");
        assert_eq!(resolved_inputs[0], "../../config.txt");
        assert_eq!(resolved_inputs[1], "src/**/*.ts");

        let resolved_outputs = outputs.resolve("../..");
        assert_eq!(resolved_outputs[0], "!../../README.md");
        assert_eq!(resolved_outputs[1], "dist/**");
    }
}
