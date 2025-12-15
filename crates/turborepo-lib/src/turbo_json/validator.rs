use miette::{NamedSource, SourceSpan};
use turborepo_repository::package_graph::{PackageName, ROOT_PKG_NAME};

use super::{Error, FutureFlags, TurboJson, TOPOLOGICAL_PIPELINE_DELIMITER};
use crate::config::UnnecessaryPackageTaskSyntaxError;

pub type TurboJSONValidation = fn(&Validator, &TurboJson) -> Vec<Error>;

/// Validator for TurboJson structures with context-aware validation
pub struct Validator {
    non_root_extends: bool,
}

const ROOT_VALIDATIONS: &[TurboJSONValidation] =
    &[validate_with_has_no_topo, validate_no_task_extends_in_root];
const PACKAGE_VALIDATIONS: &[TurboJSONValidation] = &[
    validate_with_has_no_topo,
    validate_no_package_task_syntax,
    validate_extends,
];

impl Validator {
    /// Creates a new validator instance
    pub fn new() -> Self {
        Self {
            non_root_extends: false,
        }
    }

    pub fn with_future_flags(mut self, future_flags: FutureFlags) -> Self {
        self.non_root_extends = future_flags.non_root_extends;
        self
    }

    /// Validates a TurboJson based on its package context
    ///
    /// Root turbo.json files have different validation rules than workspace
    /// turbo.json files
    pub fn validate_turbo_json(
        &self,
        package_name: &PackageName,
        turbo_json: &TurboJson,
    ) -> Vec<Error> {
        let validations = match package_name {
            PackageName::Root => ROOT_VALIDATIONS,
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

pub fn validate_extends(validator: &Validator, turbo_json: &TurboJson) -> Vec<Error> {
    if turbo_json.extends.is_empty() {
        let path = turbo_json
            .path
            .as_ref()
            .map_or("turbo.json", |p| p.as_ref());

        let (span, text) = match turbo_json.text {
            Some(ref text) => {
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
    if let Some(package_name) = turbo_json.extends.first() {
        if package_name != ROOT_PKG_NAME && validator.non_root_extends {
            let path = turbo_json
                .path
                .as_ref()
                .map_or("turbo.json", |p| p.as_ref());

            let (span, text) = match turbo_json.text {
                Some(ref text) => {
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
    // If we allow for non-root extends we don't need to perform this check
    (!validator.non_root_extends
        && turbo_json
            .extends
            .iter()
            .any(|package_name| package_name != ROOT_PKG_NAME))
    .then(|| {
        let (span, text) = turbo_json.extends.span_and_text("turbo.json");
        Error::ExtendFromNonRoot { span, text }
    })
    .into_iter()
    .collect()
}

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
mod test {
    use std::assert_matches::assert_matches;

    use test_case::test_case;
    use turborepo_errors::Spanned;
    use turborepo_task_id::TaskName;
    use turborepo_unescape::UnescapedString;

    use super::*;
    use crate::turbo_json::{Pipeline, RawTaskDefinition};

    #[test]
    fn test_validate_with_has_no_topo() {
        let turbo_json = TurboJson {
            tasks: Pipeline(
                vec![(
                    TaskName::from("dev"),
                    Spanned::new(RawTaskDefinition {
                        with: Some(vec![Spanned::new(UnescapedString::from("^proxy"))]),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let validator = Validator::new();
        let errs = validate_with_has_no_topo(&validator, &turbo_json);
        let error_messages: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        insta::assert_debug_snapshot!("validate_with_has_no_topo", error_messages);
    }

    #[test_case(
        vec!["my-package#build"],
        "single_package_task"
    )]
    #[test_case(
        vec!["build"],
        "non_package_task"
    )]
    #[test_case(
        vec!["pkg-a#test", "pkg-b#lint", "build"],
        "multiple_mixed_tasks"
    )]
    fn test_validate_no_package_task_syntax(tasks: Vec<&str>, name: &str) {
        let turbo_json = TurboJson {
            tasks: Pipeline(
                tasks
                    .into_iter()
                    .map(|task_name| {
                        (
                            TaskName::from(task_name.to_string()),
                            Spanned::new(RawTaskDefinition::default()),
                        )
                    })
                    .collect(),
            ),
            ..Default::default()
        };

        let validator = Validator::new();
        let errs = validate_no_package_task_syntax(&validator, &turbo_json);
        let error_messages: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        let snapshot_name = format!("validate_no_package_task_syntax_{}", name);
        insta::assert_debug_snapshot!(snapshot_name, error_messages);
    }

    #[test_case(
        vec![],
        "no_extends"
    )]
    #[test_case(
        vec!["//"],
        "valid_extends_from_root"
    )]
    #[test_case(
        vec!["some-package"],
        "extends_from_non_root_package"
    )]
    #[test_case(
        vec!["//", "other-package"],
        "multiple_extends_including_root"
    )]
    #[test_case(
        vec!["package-a", "package-b"],
        "multiple_extends_not_including_root"
    )]
    #[test_case(
        vec!["some-package", "//"],
        "extends_from_non_root_package_then_root"
    )]
    fn test_validate_extends(extends: Vec<&str>, name: &str) {
        let turbo_json = TurboJson {
            extends: Spanned::new(extends.into_iter().map(String::from).collect()),
            ..Default::default()
        };

        for non_root_extends in [false, true] {
            let validator = Validator { non_root_extends };
            let errs = validate_extends(&validator, &turbo_json);
            let error_messages: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
            let mut snapshot_name = format!("validate_extends_{}", name);
            if non_root_extends {
                snapshot_name.push_str("_true");
            }
            insta::assert_debug_snapshot!(snapshot_name, error_messages);
        }
    }

    #[test]
    fn test_validator_with_root_package() {
        let validator = Validator::new();

        // Root turbo.json can have package task syntax
        let turbo_json = TurboJson {
            tasks: Pipeline(
                vec![(TaskName::from("app#build"), Spanned::default())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::Root, &turbo_json);
        assert!(
            errs.is_empty(),
            "Root turbo.json should allow package task syntax"
        );
    }

    #[test]
    fn test_validator_with_missing_extends() {
        let validator = Validator::new();

        // Workspace turbo.json without extends should error
        let turbo_json = TurboJson {
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);
        assert_eq!(errs.len(), 1, "Workspace turbo.json should have extends");
        assert_matches!(errs[0], Error::NoExtends { .. });
    }

    #[test]
    fn test_task_extends_in_root_turbo_json_errors() {
        let validator = Validator::new();

        // Root turbo.json with task-level extends should error
        let turbo_json = TurboJson {
            tasks: Pipeline(
                vec![(
                    TaskName::from("build"),
                    Spanned::new(RawTaskDefinition {
                        extends: Some(Spanned::new(false)),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::Root, &turbo_json);
        assert_eq!(
            errs.len(),
            1,
            "Root turbo.json should not allow task-level extends"
        );
        assert_matches!(errs[0], Error::TaskExtendsInRoot { .. });
    }

    #[test]
    fn test_task_extends_in_package_turbo_json_allowed() {
        let validator = Validator::new();

        // Package turbo.json with task-level extends should be allowed
        let turbo_json = TurboJson {
            extends: Spanned::new(vec!["//".to_string()]),
            tasks: Pipeline(
                vec![(
                    TaskName::from("lint"),
                    Spanned::new(RawTaskDefinition {
                        extends: Some(Spanned::new(false)),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);
        // Should only have no errors related to task-level extends
        // (there might be other validation errors but not TaskExtendsInRoot)
        let extends_errors: Vec<_> = errs
            .iter()
            .filter(|e| matches!(e, Error::TaskExtendsInRoot { .. }))
            .collect();
        assert!(
            extends_errors.is_empty(),
            "Package turbo.json should allow task-level extends"
        );
    }

    // ==================== Additional Validator Test Coverage ====================

    // Test that multiple task-level extends in root all produce errors
    #[test]
    fn test_multiple_task_extends_in_root_turbo_json_errors() {
        let validator = Validator::new();

        // Root turbo.json with multiple task-level extends should produce multiple
        // errors
        let turbo_json = TurboJson {
            tasks: Pipeline(
                vec![
                    (
                        TaskName::from("build"),
                        Spanned::new(RawTaskDefinition {
                            extends: Some(Spanned::new(false)),
                            ..Default::default()
                        }),
                    ),
                    (
                        TaskName::from("lint"),
                        Spanned::new(RawTaskDefinition {
                            extends: Some(Spanned::new(true)),
                            ..Default::default()
                        }),
                    ),
                    (
                        TaskName::from("test"),
                        Spanned::new(RawTaskDefinition {
                            // No extends - should not produce error
                            ..Default::default()
                        }),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::Root, &turbo_json);
        let extends_errors: Vec<_> = errs
            .iter()
            .filter(|e| matches!(e, Error::TaskExtendsInRoot { .. }))
            .collect();

        // Should have 2 errors (build and lint have extends, test does not)
        assert_eq!(
            extends_errors.len(),
            2,
            "Should have 2 TaskExtendsInRoot errors"
        );
    }

    // Test task-level extends: true in root also errors
    #[test]
    fn test_task_extends_true_in_root_also_errors() {
        let validator = Validator::new();

        let turbo_json = TurboJson {
            tasks: Pipeline(
                vec![(
                    TaskName::from("build"),
                    Spanned::new(RawTaskDefinition {
                        extends: Some(Spanned::new(true)), // extends: true
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::Root, &turbo_json);
        assert_matches!(errs.first(), Some(Error::TaskExtendsInRoot { .. }));
    }

    // Test extends validation with non_root_extends disabled
    #[test]
    fn test_extends_non_root_packages_error_without_future_flag() {
        let validator = Validator::new(); // non_root_extends = false by default

        let turbo_json = TurboJson {
            extends: Spanned::new(vec!["//".to_string(), "other-package".to_string()]),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);

        // Should error because non_root_extends is false
        let extend_errors: Vec<_> = errs
            .iter()
            .filter(|e| matches!(e, Error::ExtendFromNonRoot { .. }))
            .collect();
        assert!(
            !extend_errors.is_empty(),
            "Should have ExtendFromNonRoot error when non_root_extends is false"
        );
    }

    // Test extends validation with non_root_extends enabled but root not first
    #[test]
    fn test_extends_root_must_be_first_with_future_flag() {
        let future_flags = FutureFlags {
            non_root_extends: true,
            ..Default::default()
        };
        let validator = Validator::new().with_future_flags(future_flags);

        // Root is NOT first
        let turbo_json = TurboJson {
            extends: Spanned::new(vec!["other-package".to_string(), "//".to_string()]),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);

        let root_first_errors: Vec<_> = errs
            .iter()
            .filter(|e| matches!(e, Error::ExtendsRootFirst { .. }))
            .collect();
        assert!(
            !root_first_errors.is_empty(),
            "Should have ExtendsRootFirst error when root is not first"
        );
    }

    // Test that extends with only root is valid
    #[test]
    fn test_extends_only_root_valid() {
        let validator = Validator::new();

        let turbo_json = TurboJson {
            extends: Spanned::new(vec!["//".to_string()]),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);

        // Should have no extends-related errors
        let extends_errors: Vec<_> = errs
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Error::ExtendFromNonRoot { .. }
                        | Error::ExtendsRootFirst { .. }
                        | Error::NoExtends { .. }
                )
            })
            .collect();
        assert!(
            extends_errors.is_empty(),
            "Extending only from root should be valid"
        );
    }

    // Test empty extends produces error
    #[test]
    fn test_empty_extends_errors() {
        let validator = Validator::new();

        let turbo_json = TurboJson {
            extends: Spanned::new(vec![]),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);

        assert_matches!(errs.first(), Some(Error::NoExtends { .. }));
    }

    // Test multiple non-root extends with future flag enabled
    #[test]
    fn test_multiple_non_root_extends_with_future_flag() {
        let future_flags = FutureFlags {
            non_root_extends: true,
            ..Default::default()
        };
        let validator = Validator::new().with_future_flags(future_flags);

        // Root first, then multiple other packages
        let turbo_json = TurboJson {
            extends: Spanned::new(vec![
                "//".to_string(),
                "config-a".to_string(),
                "config-b".to_string(),
            ]),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);

        // Should have no extends-related errors
        let extends_errors: Vec<_> = errs
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Error::ExtendFromNonRoot { .. }
                        | Error::ExtendsRootFirst { .. }
                        | Error::NoExtends { .. }
                )
            })
            .collect();
        assert!(
            extends_errors.is_empty(),
            "Multiple non-root extends should be valid with future flag enabled"
        );
    }

    // Test task-level extends: false allowed in package turbo.json
    #[test]
    fn test_task_extends_false_allowed_in_package() {
        let validator = Validator::new();

        let turbo_json = TurboJson {
            extends: Spanned::new(vec!["//".to_string()]),
            tasks: Pipeline(
                vec![(
                    TaskName::from("lint"),
                    Spanned::new(RawTaskDefinition {
                        extends: Some(Spanned::new(false)),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);

        // Should have no TaskExtendsInRoot errors
        let task_extends_errors: Vec<_> = errs
            .iter()
            .filter(|e| matches!(e, Error::TaskExtendsInRoot { .. }))
            .collect();
        assert!(
            task_extends_errors.is_empty(),
            "Task-level extends: false should be allowed in package turbo.json"
        );
    }

    // Test task-level extends: true allowed in package turbo.json
    #[test]
    fn test_task_extends_true_allowed_in_package() {
        let validator = Validator::new();

        let turbo_json = TurboJson {
            extends: Spanned::new(vec!["//".to_string()]),
            tasks: Pipeline(
                vec![(
                    TaskName::from("build"),
                    Spanned::new(RawTaskDefinition {
                        extends: Some(Spanned::new(true)),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let errs = validator.validate_turbo_json(&PackageName::from("app"), &turbo_json);

        let task_extends_errors: Vec<_> = errs
            .iter()
            .filter(|e| matches!(e, Error::TaskExtendsInRoot { .. }))
            .collect();
        assert!(
            task_extends_errors.is_empty(),
            "Task-level extends: true should be allowed in package turbo.json"
        );
    }

    // Test with_has_no_topo validation for task-level extends combinations
    #[test]
    fn test_with_has_no_topo_with_task_extends() {
        let turbo_json = TurboJson {
            extends: Spanned::new(vec!["//".to_string()]),
            tasks: Pipeline(
                vec![(
                    TaskName::from("dev"),
                    Spanned::new(RawTaskDefinition {
                        extends: Some(Spanned::new(false)),
                        with: Some(vec![Spanned::new(UnescapedString::from("^proxy"))]),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let validator = Validator::new();
        let errs = validate_with_has_no_topo(&validator, &turbo_json);

        // Should have InvalidTaskWith error for ^proxy
        assert!(
            !errs.is_empty(),
            "Should have error for topo dependency in with"
        );
        assert_matches!(errs.first(), Some(Error::InvalidTaskWith { .. }));
    }
}
