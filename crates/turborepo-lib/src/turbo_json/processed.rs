//! Processed task definition types with DSL token handling

use camino::Utf8Path;
use turbopath::RelativeUnixPath;
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;
use turborepo_unescape::UnescapedString;

use super::RawTaskDefinition;
use crate::{
    cli::{EnvMode, OutputLogsMode},
    config::Error,
};

const TURBO_DEFAULT: &str = "$TURBO_DEFAULT$";
const TURBO_ROOT: &str = "$TURBO_ROOT$";
const TURBO_ROOT_SLASH: &str = "$TURBO_ROOT$/";
const ENV_PIPELINE_DELIMITER: &str = "$";
const TOPOLOGICAL_PIPELINE_DELIMITER: &str = "^";

/// A processed glob with separated components
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedGlob {
    /// The glob pattern without $TURBO_ROOT$ prefix
    glob: String,
    /// Whether the glob was negated (started with !)
    negated: bool,
    /// Whether the glob needs turbo_root prefix (had $TURBO_ROOT$/)
    turbo_root: bool,
}

impl ProcessedGlob {
    /// Creates a ProcessedGlob from a raw glob string, stripping prefixes
    fn from_spanned_internal(
        value: Spanned<UnescapedString>,
        field: &'static str,
    ) -> Result<Self, crate::config::Error> {
        let mut negated = false;
        let mut turbo_root = false;

        let without_negation = if let Some(value) = value.strip_prefix('!') {
            negated = true;
            value
        } else {
            value.as_str()
        };

        let glob = if let Some(stripped) = without_negation.strip_prefix(TURBO_ROOT_SLASH) {
            turbo_root = true;
            stripped
        } else if without_negation.starts_with(TURBO_ROOT) {
            // Leading $TURBO_ROOT$ without slash
            let (span, text) = value.span_and_text("turbo.json");
            return Err(Error::InvalidTurboRootNeedsSlash { span, text });
        } else if without_negation.contains(TURBO_ROOT) {
            // non leading $TURBO_ROOT$
            let (span, text) = value.span_and_text("turbo.json");
            return Err(Error::InvalidTurboRootUse { span, text });
        } else {
            without_negation
        };

        // Check for absolute paths (after stripping prefixes)
        if Utf8Path::new(glob).is_absolute() {
            let (span, text) = value.span_and_text("turbo.json");
            return Err(Error::AbsolutePathInConfig { field, span, text });
        }

        Ok(ProcessedGlob {
            glob: glob.to_owned(),
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
    pub fn resolve(&self, turbo_root_path: &RelativeUnixPath) -> String {
        let prefix = if self.negated { "!" } else { "" };

        let glob = &self.glob;
        if self.turbo_root {
            format!("{prefix}{turbo_root_path}/{glob}")
        } else {
            format!("{prefix}{glob}")
        }
    }
}

/// Processed depends_on field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedDependsOn(pub Spanned<Vec<Spanned<UnescapedString>>>);

impl ProcessedDependsOn {
    /// Creates a ProcessedDependsOn, validating that dependencies don't use env
    /// prefix
    pub fn new(raw_deps: Spanned<Vec<Spanned<UnescapedString>>>) -> Result<Self, Error> {
        // Validate that no dependency starts with ENV_PIPELINE_DELIMITER ($)
        for dep in raw_deps.value.iter() {
            if dep.starts_with(ENV_PIPELINE_DELIMITER) {
                let (span, text) = dep.span_and_text("turbo.json");
                return Err(Error::InvalidDependsOnValue {
                    field: "dependsOn",
                    span,
                    text,
                });
            }
        }
        Ok(ProcessedDependsOn(raw_deps))
    }
}

/// Processed env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedEnv(pub Vec<String>);

impl ProcessedEnv {
    /// Creates a ProcessedEnv, validating that env vars don't use invalid
    /// prefixes
    pub fn new(raw_env: Vec<Spanned<UnescapedString>>) -> Result<Self, Error> {
        Ok(ProcessedEnv(extract_env_vars(raw_env)?))
    }
}

/// Processed inputs field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedInputs {
    globs: Vec<ProcessedGlob>,
    pub default: bool,
}

impl ProcessedInputs {
    pub fn new(raw_globs: Vec<Spanned<UnescapedString>>) -> Result<Self, Error> {
        let mut globs = Vec::with_capacity(raw_globs.len());
        let mut default = false;
        for raw_glob in raw_globs {
            if raw_glob.as_str() == TURBO_DEFAULT {
                default = true;
            }
            globs.push(ProcessedGlob::from_spanned_input(raw_glob)?);
        }

        Ok(ProcessedInputs { globs, default })
    }

    /// Resolves all globs with the given turbo_root path
    pub fn resolve(&self, turbo_root_path: &RelativeUnixPath) -> Vec<String> {
        self.globs
            .iter()
            .map(|glob| glob.resolve(turbo_root_path))
            .collect()
    }
}

/// Processed pass_through_env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedPassThroughEnv(pub Vec<String>);

impl ProcessedPassThroughEnv {
    /// Creates a ProcessedPassThroughEnv, validating that env vars don't use
    /// invalid prefixes
    pub fn new(raw_env: Vec<Spanned<UnescapedString>>) -> Result<Self, Error> {
        Ok(ProcessedPassThroughEnv(extract_env_vars(raw_env)?))
    }
}

fn extract_env_vars(raw_env: Vec<Spanned<UnescapedString>>) -> Result<Vec<String>, Error> {
    use crate::config::InvalidEnvPrefixError;

    let mut env_vars = Vec::with_capacity(raw_env.len());
    // Validate that no env var starts with ENV_PIPELINE_DELIMITER ($)
    for var in raw_env {
        if var.starts_with(ENV_PIPELINE_DELIMITER) {
            let (span, text) = var.span_and_text("turbo.json");
            return Err(Error::InvalidEnvPrefix(Box::new(InvalidEnvPrefixError {
                key: "passThroughEnv".to_string(),
                value: var.as_str().to_string(),
                span,
                text,
                env_pipeline_delimiter: ENV_PIPELINE_DELIMITER,
            })));
        }

        env_vars.push(String::from(var.into_inner()));
    }
    env_vars.sort();
    Ok(env_vars)
}

/// Processed outputs field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedOutputs {
    globs: Vec<ProcessedGlob>,
}

impl ProcessedOutputs {
    pub fn new(raw_globs: Vec<Spanned<UnescapedString>>) -> Result<Self, Error> {
        let globs = raw_globs
            .into_iter()
            .map(ProcessedGlob::from_spanned_input)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ProcessedOutputs { globs })
    }

    /// Resolves all globs with the given turbo_root path
    pub fn resolve(&self, turbo_root_path: &RelativeUnixPath) -> Vec<String> {
        self.globs
            .iter()
            .map(|glob| glob.resolve(turbo_root_path))
            .collect()
    }
}

/// Processed with field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedWith(pub Vec<Spanned<TaskName<'static>>>);

impl ProcessedWith {
    /// Creates a ProcessedWith, validating that siblings don't use topological
    /// prefix
    pub fn new(raw_with: Vec<Spanned<UnescapedString>>) -> Result<Self, Error> {
        // Validate that no sibling starts with TOPOLOGICAL_PIPELINE_DELIMITER (^)
        let mut with = Vec::with_capacity(raw_with.len());
        for sibling in raw_with {
            if sibling.starts_with(TOPOLOGICAL_PIPELINE_DELIMITER) {
                let (span, text) = sibling.span_and_text("turbo.json");
                return Err(Error::InvalidTaskWith { span, text });
            }
            let (sibling, span) = sibling.split();
            with.push(span.to(TaskName::from(String::from(sibling))));
        }
        Ok(ProcessedWith(with))
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
    pub with: Option<ProcessedWith>,
}

impl ProcessedTaskDefinition {
    /// Creates a processed task definition from raw task
    pub fn from_raw(raw_task: RawTaskDefinition) -> Result<Self, crate::config::Error> {
        Ok(ProcessedTaskDefinition {
            cache: raw_task.cache,
            depends_on: raw_task
                .depends_on
                .map(ProcessedDependsOn::new)
                .transpose()?,
            env: raw_task.env.map(ProcessedEnv::new).transpose()?,
            inputs: raw_task.inputs.map(ProcessedInputs::new).transpose()?,
            pass_through_env: raw_task
                .pass_through_env
                .map(ProcessedPassThroughEnv::new)
                .transpose()?,
            persistent: raw_task.persistent,
            interruptible: raw_task.interruptible,
            outputs: raw_task.outputs.map(ProcessedOutputs::new).transpose()?,
            output_logs: raw_task.output_logs,
            interactive: raw_task.interactive,
            env_mode: raw_task.env_mode,
            with: raw_task.with.map(ProcessedWith::new).transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, sync::Arc};

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
        let replacement = RelativeUnixPath::new(replacement).unwrap();
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
        let raw_task = RawTaskDefinition {
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
        let turbo_root = RelativeUnixPath::new("../..").unwrap();

        // Verify TURBO_ROOT detection
        let inputs = processed.inputs.as_ref().unwrap();
        assert!(inputs.globs[0].turbo_root);
        assert!(!inputs.globs[0].negated);
        assert!(!inputs.globs[1].turbo_root);

        let outputs = processed.outputs.as_ref().unwrap();
        assert!(outputs.globs[0].turbo_root);
        assert!(outputs.globs[0].negated);
        assert!(!outputs.globs[1].turbo_root);

        // Resolve with turbo_root path
        let resolved_inputs = inputs.resolve(turbo_root);
        assert_eq!(resolved_inputs[0], "../../config.txt");
        assert_eq!(resolved_inputs[1], "src/**/*.ts");

        let resolved_outputs = outputs.resolve(turbo_root);
        assert_eq!(resolved_outputs[0], "!../../README.md");
        assert_eq!(resolved_outputs[1], "dist/**");
    }

    #[test]
    fn test_detects_turbo_default() {
        let raw_globs = vec![Spanned::new(UnescapedString::from(TURBO_DEFAULT))];

        let inputs = ProcessedInputs::new(raw_globs).unwrap();
        assert!(inputs.default);
        assert_eq!(
            inputs.globs,
            vec![ProcessedGlob {
                glob: TURBO_DEFAULT.to_string(),
                negated: false,
                turbo_root: false
            }]
        );
    }

    #[test]
    fn test_absolute_paths_error_in_inputs() {
        let absolute_path = if cfg!(windows) {
            "C:\\win32"
        } else {
            "/dev/null"
        };

        // The error should be caught when creating the ProcessedGlob
        let result =
            ProcessedGlob::from_spanned_input(Spanned::new(UnescapedString::from(absolute_path)));

        assert_matches!(result, Err(Error::AbsolutePathInConfig { .. }));
    }
}
