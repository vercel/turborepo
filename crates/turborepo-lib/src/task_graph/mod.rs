mod visitor;

// Re-export TaskDefinition from turborepo-types for backward compatibility.
// New code should import directly from `turborepo_types::TaskDefinition`.
#[deprecated(
    since = "2.4.0",
    note = "Import `TaskDefinition` directly from `turborepo_types` instead"
)]
#[allow(unused_imports)]
pub use turborepo_types::TaskDefinition;
// Re-export extension traits from turborepo-types for backward compatibility.
// New code should import directly from `turborepo_types`.
#[deprecated(
    since = "2.4.0",
    note = "Import `TaskDefinitionExt` directly from `turborepo_types` instead"
)]
#[allow(unused_imports)]
pub use turborepo_types::TaskDefinitionExt;
// Re-export TaskInputs from turborepo-types for backward compatibility.
// New code should import directly from `turborepo_types::TaskInputs`.
#[deprecated(
    since = "2.4.0",
    note = "Import `TaskInputs` directly from `turborepo_types` instead"
)]
#[allow(unused_imports)]
pub use turborepo_types::TaskInputs;
// Re-export TaskOutputs from turborepo-types for backward compatibility.
// New code should import directly from `turborepo_types::TaskOutputs`.
#[deprecated(
    since = "2.4.0",
    note = "Import `TaskOutputs` directly from `turborepo_types` instead"
)]
#[allow(unused_imports)]
pub use turborepo_types::TaskOutputs;
#[deprecated(
    since = "2.4.0",
    note = "Import `TaskOutputsExt` directly from `turborepo_types` instead"
)]
#[allow(unused_imports)]
pub use turborepo_types::TaskOutputsExt;
// Re-export log file utilities from turborepo-types for backward compatibility
#[allow(unused_imports)]
pub use turborepo_types::{task_log_filename, LOG_DIR};
pub use visitor::{Error as VisitorError, Visitor};

#[cfg(test)]
mod test {
    use std::path::MAIN_SEPARATOR_STR;

    use pretty_assertions::assert_eq;
    use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf};
    use turborepo_task_id::TaskId;

    #[allow(deprecated)]
    use super::*;

    #[test]
    fn test_relative_output_globs() {
        let task_defn = TaskDefinition {
            outputs: TaskOutputs {
                inclusions: vec![".next/**/*".to_string()],
                exclusions: vec![".next/bad-file".to_string()],
            },
            ..Default::default()
        };

        let task_id = TaskId::new("foo", "build");
        let workspace_dir = AnchoredSystemPath::new(match cfg!(windows) {
            true => "apps\\foo",
            false => "apps/foo",
        })
        .unwrap();

        let relative_outputs = task_defn.repo_relative_hashable_outputs(&task_id, workspace_dir);
        let relative_prefix = match cfg!(windows) {
            true => "apps\\foo\\",
            false => "apps/foo/",
        };
        assert_eq!(
            relative_outputs,
            TaskOutputs {
                inclusions: vec![
                    format!("{relative_prefix}.next/**/*"),
                    format!("{relative_prefix}.turbo/turbo-build.log"),
                ],
                exclusions: vec![format!("{relative_prefix}.next/bad-file")],
            }
        );
    }

    #[test]
    fn test_escape_log_file() {
        let build_log = <TaskDefinition as TaskDefinitionExt>::workspace_relative_log_file("build");
        let build_expected =
            AnchoredSystemPathBuf::from_raw([".turbo", "turbo-build.log"].join(MAIN_SEPARATOR_STR))
                .unwrap();
        assert_eq!(build_log, build_expected);

        let build_log =
            <TaskDefinition as TaskDefinitionExt>::workspace_relative_log_file("build:prod");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            [".turbo", "turbo-build$colon$prod.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);

        let build_log =
            <TaskDefinition as TaskDefinitionExt>::workspace_relative_log_file("build:prod:extra");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            [".turbo", "turbo-build$colon$prod$colon$extra.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);
    }
}
