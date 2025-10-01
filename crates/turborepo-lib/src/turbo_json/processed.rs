//! Processed task definition types with DSL token handling

use camino::Utf8Path;
use turbopath::RelativeUnixPath;
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;
use turborepo_unescape::UnescapedString;

use super::{FutureFlags, RawTaskDefinition};
use crate::{
    cli::{EnvMode, OutputLogsMode},
    config::Error,
};

const TURBO_DEFAULT: &str = "$TURBO_DEFAULT$";
const TURBO_ROOT: &str = "$TURBO_ROOT$";
const TURBO_ROOT_SLASH: &str = "$TURBO_ROOT$/";
const TURBO_EXTENDS: &str = "$TURBO_EXTENDS$";
const ENV_PIPELINE_DELIMITER: &str = "$";
const TOPOLOGICAL_PIPELINE_DELIMITER: &str = "^";

/// Helper function to check for and remove $TURBO_EXTENDS$ from an array
/// Returns (processed_array, extends_found)
fn extract_turbo_extends(
    mut items: Vec<Spanned<UnescapedString>>,
    future_flags: &FutureFlags,
) -> (Vec<Spanned<UnescapedString>>, bool) {
    if !future_flags.turbo_extends_keyword {
        return (items, false);
    }

    if let Some(pos) = items.iter().position(|item| item.as_str() == TURBO_EXTENDS) {
        items.remove(pos);
        (items, true)
    } else {
        (items, false)
    }
}

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
pub struct ProcessedDependsOn {
    pub deps: Vec<Spanned<UnescapedString>>,
    pub extends: bool,
}

impl ProcessedDependsOn {
    /// Creates a ProcessedDependsOn, validating that dependencies don't use env
    /// prefix and handling TURBO_EXTENDS if enabled
    pub fn new(
        raw_deps: Spanned<Vec<Spanned<UnescapedString>>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_deps, extends) = extract_turbo_extends(raw_deps.into_inner(), future_flags);

        // Validate that no dependency starts with ENV_PIPELINE_DELIMITER ($)
        for dep in processed_deps.iter() {
            if dep.starts_with(ENV_PIPELINE_DELIMITER) {
                let (span, text) = dep.span_and_text("turbo.json");
                return Err(Error::InvalidDependsOnValue {
                    field: "dependsOn",
                    span,
                    text,
                });
            }
        }
        Ok(ProcessedDependsOn {
            deps: processed_deps,
            extends,
        })
    }
}

/// Processed env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedEnv {
    pub vars: Vec<String>,
    pub extends: bool,
}

impl ProcessedEnv {
    /// Creates a ProcessedEnv, validating that env vars don't use invalid
    /// prefixes and handling TURBO_EXTENDS if enabled
    pub fn new(
        raw_env: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_env, extends) = extract_turbo_extends(raw_env, future_flags);

        Ok(ProcessedEnv {
            vars: extract_env_vars(processed_env, "env")?,
            extends,
        })
    }
}

/// Processed inputs field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedInputs {
    pub globs: Vec<ProcessedGlob>,
    pub default: bool,
    pub extends: bool,
}

impl ProcessedInputs {
    pub fn new(
        raw_globs: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_globs, extends) = extract_turbo_extends(raw_globs, future_flags);

        let mut globs = Vec::with_capacity(processed_globs.len());
        let mut default = false;
        for raw_glob in processed_globs {
            if raw_glob.as_str() == TURBO_DEFAULT {
                default = true;
            }
            globs.push(ProcessedGlob::from_spanned_input(raw_glob)?);
        }

        Ok(ProcessedInputs {
            globs,
            default,
            extends,
        })
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
pub struct ProcessedPassThroughEnv {
    pub vars: Vec<String>,
    pub extends: bool,
}

impl ProcessedPassThroughEnv {
    /// Creates a ProcessedPassThroughEnv, validating that env vars don't use
    /// invalid prefixes and handling TURBO_EXTENDS if enabled
    pub fn new(
        raw_env: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_env, extends) = extract_turbo_extends(raw_env, future_flags);

        Ok(ProcessedPassThroughEnv {
            vars: extract_env_vars(processed_env, "passThroughEnv")?,
            extends,
        })
    }
}

fn extract_env_vars(
    raw_env: Vec<Spanned<UnescapedString>>,
    field_name: &str,
) -> Result<Vec<String>, Error> {
    use crate::config::InvalidEnvPrefixError;

    let mut env_vars = Vec::with_capacity(raw_env.len());
    // Validate that no env var starts with ENV_PIPELINE_DELIMITER ($)
    for var in raw_env {
        if var.starts_with(ENV_PIPELINE_DELIMITER) {
            let (span, text) = var.span_and_text("turbo.json");
            return Err(Error::InvalidEnvPrefix(Box::new(InvalidEnvPrefixError {
                key: field_name.to_string(),
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
    pub globs: Vec<ProcessedGlob>,
    pub extends: bool,
}

impl ProcessedOutputs {
    pub fn new(
        raw_globs: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_globs, extends) = extract_turbo_extends(raw_globs, future_flags);

        let globs = processed_globs
            .into_iter()
            .map(ProcessedGlob::from_spanned_output)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ProcessedOutputs { globs, extends })
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
pub struct ProcessedWith {
    pub tasks: Vec<Spanned<TaskName<'static>>>,
    pub extends: bool,
}

impl ProcessedWith {
    /// Creates a ProcessedWith, validating that siblings don't use topological
    /// prefix and handling TURBO_EXTENDS if enabled
    pub fn new(
        raw_with: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_with, extends) = extract_turbo_extends(raw_with, future_flags);

        // Validate that no sibling starts with TOPOLOGICAL_PIPELINE_DELIMITER (^)
        let mut tasks = Vec::with_capacity(processed_with.len());
        for sibling in processed_with {
            if sibling.starts_with(TOPOLOGICAL_PIPELINE_DELIMITER) {
                let (span, text) = sibling.span_and_text("turbo.json");
                return Err(Error::InvalidTaskWith { span, text });
            }
            let (sibling, span) = sibling.split();
            tasks.push(span.to(TaskName::from(String::from(sibling))));
        }
        Ok(ProcessedWith { tasks, extends })
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
    pub fn from_raw(
        raw_task: RawTaskDefinition,
        future_flags: &FutureFlags,
    ) -> Result<Self, crate::config::Error> {
        Ok(ProcessedTaskDefinition {
            cache: raw_task.cache,
            depends_on: raw_task
                .depends_on
                .map(|deps| ProcessedDependsOn::new(deps, future_flags))
                .transpose()?,
            env: raw_task
                .env
                .map(|env| ProcessedEnv::new(env, future_flags))
                .transpose()?,
            inputs: raw_task
                .inputs
                .map(|inputs| ProcessedInputs::new(inputs, future_flags))
                .transpose()?,
            pass_through_env: raw_task
                .pass_through_env
                .map(|env| ProcessedPassThroughEnv::new(env, future_flags))
                .transpose()?,
            persistent: raw_task.persistent,
            interruptible: raw_task.interruptible,
            outputs: raw_task
                .outputs
                .map(|outputs| ProcessedOutputs::new(outputs, future_flags))
                .transpose()?,
            output_logs: raw_task.output_logs,
            interactive: raw_task.interactive,
            env_mode: raw_task.env_mode,
            with: raw_task
                .with
                .map(|with| ProcessedWith::new(with, future_flags))
                .transpose()?,
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
    use crate::turbo_json::FutureFlags;

    #[test]
    fn test_extract_turbo_extends_with_flag_enabled() {
        let items = vec![
            Spanned::new(UnescapedString::from("item1")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("item2")),
        ];

        let (processed, extends) = extract_turbo_extends(
            items,
            &FutureFlags {
                turbo_extends_keyword: true,
                non_root_extends: false,
            },
        );

        assert!(extends);
        assert_eq!(processed.len(), 2);
        assert_eq!(processed[0].as_str(), "item1");
        assert_eq!(processed[1].as_str(), "item2");
    }

    #[test]
    fn test_extract_turbo_extends_with_flag_disabled() {
        let items = vec![
            Spanned::new(UnescapedString::from("item1")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("item2")),
        ];

        let (processed, extends) = extract_turbo_extends(
            items,
            &FutureFlags {
                turbo_extends_keyword: false,
                non_root_extends: false,
            },
        );

        assert!(!extends);
        assert_eq!(processed.len(), 3);
        assert_eq!(processed[1].as_str(), "$TURBO_EXTENDS$");
    }

    #[test]
    fn test_extract_turbo_extends_no_marker() {
        let items = vec![
            Spanned::new(UnescapedString::from("item1")),
            Spanned::new(UnescapedString::from("item2")),
        ];

        let (processed, extends) = extract_turbo_extends(
            items,
            &FutureFlags {
                turbo_extends_keyword: true,
                non_root_extends: false,
            },
        );

        assert!(!extends);
        assert_eq!(processed.len(), 2);
    }

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
        let processed =
            ProcessedTaskDefinition::from_raw(raw_task, &FutureFlags::default()).unwrap();
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

        let inputs = ProcessedInputs::new(raw_globs, &FutureFlags::default()).unwrap();
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

    // Test that demonstrates the extends field is properly set when the helper is
    // used
    #[test]
    fn test_processed_inputs_with_turbo_extends() {
        let raw_globs: Vec<Spanned<UnescapedString>> = vec![
            Spanned::new(UnescapedString::from("src/**")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("lib/**")),
        ];

        let inputs = ProcessedInputs::new(
            raw_globs,
            &FutureFlags {
                turbo_extends_keyword: true,
                non_root_extends: false,
            },
        )
        .unwrap();

        assert!(inputs.extends);
        assert_eq!(inputs.globs.len(), 2);
        assert_eq!(inputs.globs[0].glob, "src/**");
        assert_eq!(inputs.globs[1].glob, "lib/**");
    }

    #[test]
    fn test_processed_env_turbo_extends_disabled_errors() {
        // When turbo_extends is disabled, $TURBO_EXTENDS$ triggers validation error
        let raw_env: Vec<Spanned<UnescapedString>> = vec![
            Spanned::new(UnescapedString::from("NODE_ENV")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("API_KEY")),
        ];

        let result = ProcessedEnv::new(
            raw_env,
            &FutureFlags {
                turbo_extends_keyword: false,
                non_root_extends: false,
            },
        );
        assert!(result.is_err());
        assert_matches!(result, Err(Error::InvalidEnvPrefix(_)));
    }

    #[test]
    fn test_processed_depends_on_turbo_extends_disabled_errors() {
        // When turbo_extends is disabled, $TURBO_EXTENDS$ triggers validation error
        let raw_deps: Vec<Spanned<UnescapedString>> = vec![
            Spanned::new(UnescapedString::from("build")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("test")),
        ];

        let result = ProcessedDependsOn::new(
            Spanned::new(raw_deps),
            &FutureFlags {
                turbo_extends_keyword: false,
                non_root_extends: false,
            },
        );
        assert!(result.is_err());
        assert_matches!(result, Err(Error::InvalidDependsOnValue { .. }));
    }
}
