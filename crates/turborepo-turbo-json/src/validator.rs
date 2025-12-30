//! Validation logic for turbo.json configurations
//!
//! This module provides the `Validator` struct and validation functions for
//! turbo.json files. Validation rules differ between root turbo.json and
//! package configuration files.

use miette::{NamedSource, SourceSpan};
use turborepo_repository::package_graph::{PackageName, ROOT_PKG_NAME};

use crate::{
    TurboJson,
    error::{Error, UnnecessaryPackageTaskSyntaxError},
};

/// Delimiter used for topological dependencies in task definitions (e.g.,
/// "^build")
pub const TOPOLOGICAL_PIPELINE_DELIMITER: &str = "^";

/// Type alias for validation functions
///
/// Each validation function takes a reference to a Validator and a TurboJson,
/// and returns a vector of errors found during validation.
pub type TurboJSONValidation = fn(&Validator, &TurboJson) -> Vec<Error>;

const ROOT_VALIDATIONS: &[TurboJSONValidation] =
    &[validate_with_has_no_topo, validate_no_task_extends_in_root];
const PACKAGE_VALIDATIONS: &[TurboJSONValidation] = &[
    validate_with_has_no_topo,
    validate_no_package_task_syntax,
    validate_extends,
];

/// Validator for TurboJson structures with context-aware validation
///
/// The validator applies different validation rules based on whether the
/// turbo.json is a root configuration or a package configuration:
///
/// - Root turbo.json: Cannot have task-level `extends`, can have `globalEnv`,
///   etc.
/// - Package turbo.json: Must have `extends`, cannot use package task syntax
pub struct Validator {}

impl Validator {
    /// Creates a new validator instance
    pub fn new() -> Self {
        Self {}
    }

    /// Builder method to configure the validator with future flags
    ///
    /// Future flags can enable or disable certain validation rules based on
    /// feature flags defined in the root turbo.json.
    pub fn with_future_flags(self, _future_flags: crate::FutureFlags) -> Self {
        // Currently a no-op, but allows for future extension
        self
    }

    /// Validates a TurboJson based on its package context
    ///
    /// Root turbo.json files have different validation rules than Package
    /// Configuration files. This method automatically selects the appropriate
    /// validation rules based on:
    ///
    /// 1. The package name (Root vs Other)
    /// 2. Whether the config file is actually the root turbo.json (detected by
    ///    path)
    ///
    /// # Arguments
    ///
    /// * `package_name` - The name of the package this turbo.json belongs to
    /// * `turbo_json` - The parsed TurboJson to validate
    ///
    /// # Returns
    ///
    /// A vector of validation errors. Empty if validation passes.
    pub fn validate_turbo_json(
        &self,
        package_name: &PackageName,
        turbo_json: &TurboJson,
    ) -> Vec<Error> {
        // Check if this is the root turbo.json based on its path.
        // This can happen when a workspace includes "." as a package. In that
        // case, the root turbo.json would be loaded for that package but should
        // still be validated with root schema.
        let is_root_turbo_json = turbo_json.is_root_config();
        let validations = match package_name {
            PackageName::Root => ROOT_VALIDATIONS,
            PackageName::Other(_) if is_root_turbo_json => ROOT_VALIDATIONS,
            PackageName::Other(_) => PACKAGE_VALIDATIONS,
        };
        validations
            .iter()
            .flat_map(|validation| validation(self, turbo_json))
            .collect()
    }
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
    }
}

/// Validates that package task syntax is not used in workspace turbo.json
///
/// In workspace turbo.json files, tasks should be defined without the package
/// prefix (e.g., "build" instead of "my-package#build").
pub fn validate_no_package_task_syntax(
    _validator: &Validator,
    turbo_json: &TurboJson,
) -> Vec<Error> {
    turbo_json
        .tasks
        .iter()
        .filter(|(task_name, _)| task_name.is_package_task())
        .map(|(task_name, entry)| {
            let (span, text) = entry.span_and_text("turbo.json");
            Error::UnnecessaryPackageTaskSyntax(Box::new(UnnecessaryPackageTaskSyntaxError {
                actual: task_name.to_string(),
                wanted: task_name.task().to_string(),
                span,
                text,
            }))
        })
        .collect()
}

/// Validates that the `extends` field is properly configured
///
/// Package turbo.json files must:
/// 1. Have an `extends` field (cannot be empty)
/// 2. Have "//" (root) as the first entry when extending from multiple packages
pub fn validate_extends(_validator: &Validator, turbo_json: &TurboJson) -> Vec<Error> {
    if turbo_json.extends.is_empty() {
        let path = turbo_json.path().map_or("turbo.json", |p| p.as_ref());

        let (span, text) = match turbo_json.text() {
            Some(text) => {
                let len = text.len();
                let span: SourceSpan = (0, len - 1).into();
                (Some(span), text.to_string())
            }
            None => (None, String::new()),
        };

        return vec![Error::NoExtends {
            span,
            text: NamedSource::new(path, text),
        }];
    }
    // Root must always be first when extending from multiple packages
    if let Some(package_name) = turbo_json.extends.first() {
        if package_name != ROOT_PKG_NAME {
            let path = turbo_json.path().map_or("turbo.json", |p| p.as_ref());

            let (span, text) = match turbo_json.text() {
                Some(text) => {
                    let len = text.len();
                    let span: SourceSpan = (0, len - 1).into();
                    (Some(span), text.to_string())
                }
                None => (None, String::new()),
            };
            // Root needs to be first
            return vec![Error::ExtendsRootFirst {
                span,
                text: NamedSource::new(path, text),
            }];
        }
    }
    vec![]
}

/// Validates that the `with` field does not contain topological dependencies
///
/// The `with` field is used to specify sibling tasks that should run alongside
/// the current task. It cannot use the "^" prefix which denotes topological
/// (dependency) relationships.
pub fn validate_with_has_no_topo(_validator: &Validator, turbo_json: &TurboJson) -> Vec<Error> {
    turbo_json
        .tasks
        .iter()
        .flat_map(|(_, definition)| {
            definition.with.iter().flatten().filter_map(|with_task| {
                if with_task.starts_with(TOPOLOGICAL_PIPELINE_DELIMITER) {
                    let (span, text) = with_task.span_and_text("turbo.json");
                    Some(Error::InvalidTaskWith { span, text })
                } else {
                    None
                }
            })
        })
        .collect()
}

/// Validates that task-level `extends` is not used in root turbo.json
///
/// The task-level `extends` field (which controls whether a task inherits
/// from root configuration) can only be used in package turbo.json files,
/// not in the root turbo.json.
pub fn validate_no_task_extends_in_root(
    _validator: &Validator,
    turbo_json: &TurboJson,
) -> Vec<Error> {
    turbo_json
        .tasks
        .iter()
        .filter_map(|(task_name, definition)| {
            if definition.extends.is_some() {
                let (span, text) = definition
                    .extends
                    .as_ref()
                    .unwrap()
                    .span_and_text("turbo.json");
                Some(Error::TaskExtendsInRoot {
                    task_name: task_name.to_string(),
                    span,
                    text,
                })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_new() {
        let validator = Validator::new();
        // Just verify it can be created
        drop(validator);
    }

    #[test]
    fn test_validator_default() {
        let validator = Validator::default();
        // Just verify default works
        drop(validator);
    }

    #[test]
    fn test_validator_with_future_flags() {
        let validator = Validator::new().with_future_flags(crate::FutureFlags::default());
        // Just verify the builder pattern works
        drop(validator);
    }

    #[test]
    fn test_topological_delimiter_constant() {
        assert_eq!(TOPOLOGICAL_PIPELINE_DELIMITER, "^");
    }
}
