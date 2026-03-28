//! turbo.json configuration
//!
//! Re-exports from turborepo-turbo-json crate with loader code for
//! turborepo-lib specific functionality (MFE, task_access).

mod loader;

// Re-export types from turborepo-turbo-json that are used within turborepo-lib.
// Note: This module is private to turborepo-lib, so we only re-export what's
// needed internally.
pub use turborepo_turbo_json::{
    FutureFlags, Pipeline, RawRootTurboJson, RawTaskDefinition, RawTurboJson, TurboJson,
};

// Re-export the parser module for types like parser::Error
pub mod parser {
    pub use turborepo_turbo_json::parser::BiomeParseError as Error;
}

// Loader code stays in turborepo-lib (depends on MFE, task_access)
use std::collections::HashMap;

pub use loader::{TurboJsonReader, UnifiedTurboJsonLoader};
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;
use turborepo_unescape::UnescapedString;

use crate::run::task_access::TaskAccessTraceFile;

/// Extension trait for RawTurboJson with turborepo-lib specific functionality
pub trait RawTurboJsonExt {
    /// Create a RawTurboJson from a task access trace
    fn from_task_access_trace(trace: &HashMap<String, TaskAccessTraceFile>)
        -> Option<RawTurboJson>;
}

impl RawTurboJsonExt for RawTurboJson {
    fn from_task_access_trace(
        trace: &HashMap<String, TaskAccessTraceFile>,
    ) -> Option<RawTurboJson> {
        if trace.is_empty() {
            return None;
        }

        let mut pipeline = Pipeline::default();

        for (task_name, trace_file) in trace {
            let spanned_outputs: Vec<Spanned<UnescapedString>> = trace_file
                .outputs
                .iter()
                .map(|output| Spanned::new(output.clone()))
                .collect();
            let task_definition = RawTaskDefinition {
                outputs: Some(spanned_outputs),
                env: Some(
                    trace_file
                        .accessed
                        .env_var_keys
                        .iter()
                        .map(|unescaped_string| Spanned::new(unescaped_string.clone()))
                        .collect(),
                ),
                ..Default::default()
            };

            let name = TaskName::from(task_name.as_str());
            let root_task = name.into_root_task();
            pipeline.insert(root_task, Spanned::new(task_definition.clone()));
        }

        Some(RawTurboJson {
            tasks: Some(pipeline),
            ..RawTurboJson::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use biome_deserialize::json::deserialize_from_json_str;
    use biome_json_parser::JsonParserOptions;
    use pretty_assertions::assert_eq;
    use test_case::test_case;
    use turbopath::RelativeUnixPath;
    use turborepo_engine::TaskDefinitionFromProcessed;
    use turborepo_errors::Spanned;
    use turborepo_types::{TaskDefinition, TaskInputs, TaskOutputs};

    use super::RawTaskDefinition;

    // This test must stay in turborepo-lib because it uses TaskDefinition::from_raw
    // which requires turborepo-engine (and turborepo-turbo-json cannot depend on
    // turborepo-engine due to the reverse dependency).
    #[test_case(
        "{}",
        RawTaskDefinition::default(),
        TaskDefinition::default()
    ; "empty task definition")]
    #[test_case(
        r#"{ "persistent": false }"#,
        RawTaskDefinition {
            persistent: Some(Spanned::new(false).with_range(16..21)),
            ..RawTaskDefinition::default()
        },
        TaskDefinition::default()
    ; "just persistent"
    )]
    #[test_case(
        r#"{
          "dependsOn": ["cli#build"],
          "env": ["OS"],
          "passThroughEnv": ["AWS_SECRET_KEY"],
          "outputs": ["package/a/dist"],
          "cache": false,
          "inputs": ["package/a/src/**"],
          "outputLogs": "full",
          "persistent": true,
          "interactive": true,
          "interruptible": true
        }"#,
        RawTaskDefinition {
            extends: None,
            description: None,
            depends_on: Some(Spanned::new(vec![Spanned::<turborepo_unescape::UnescapedString>::new("cli#build".into()).with_range(26..37)]).with_range(25..38)),
            env: Some(vec![Spanned::<turborepo_unescape::UnescapedString>::new("OS".into()).with_range(58..62)]),
            pass_through_env: Some(vec![Spanned::<turborepo_unescape::UnescapedString>::new("AWS_SECRET_KEY".into()).with_range(94..110)]),
            outputs: Some(vec![Spanned::<turborepo_unescape::UnescapedString>::new("package/a/dist".into()).with_range(135..151)]),
            cache: Some(Spanned::new(false).with_range(173..178)),
            inputs: Some(vec![Spanned::<turborepo_unescape::UnescapedString>::new("package/a/src/**".into()).with_range(201..219)]),
            output_logs: Some(Spanned::new(turborepo_types::OutputLogsMode::Full).with_range(246..252)),
            persistent: Some(Spanned::new(true).with_range(278..282)),
            interactive: Some(Spanned::new(true).with_range(309..313)),
            interruptible: Some(Spanned::new(true).with_range(342..346)),
            env_mode: None,
            with: None,
        },
        TaskDefinition {
          env: vec!["OS".to_string()],
          outputs: TaskOutputs {
              inclusions: vec!["package/a/dist".to_string()],
              exclusions: vec![],
          },
          cache: false,
          inputs: TaskInputs::new(vec!["package/a/src/**".to_string()]),
          output_logs: turborepo_types::OutputLogsMode::Full,
          pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
          task_dependencies: vec![Spanned::<turborepo_task_id::TaskName<'_>>::new("cli#build".into()).with_range(26..37)],
          topological_dependencies: vec![],
          persistent: true,
          interactive: true,
          interruptible: true,
          env_mode: None,
          with: None,
        }
      ; "full"
    )]
    #[test_case(
        r#"{
              "dependsOn": ["cli#build"],
              "env": ["OS"],
              "passThroughEnv": ["AWS_SECRET_KEY"],
              "outputs": ["package\\a\\dist"],
              "cache": false,
              "inputs": ["package\\a\\src\\**"],
              "outputLogs": "full",
              "persistent": true,
              "interruptible": true
            }"#,
        RawTaskDefinition {
            extends: None,
            description: None,
            depends_on: Some(Spanned::new(vec![Spanned::<turborepo_unescape::UnescapedString>::new("cli#build".into()).with_range(30..41)]).with_range(29..42)),
            env: Some(vec![Spanned::<turborepo_unescape::UnescapedString>::new("OS".into()).with_range(66..70)]),
            pass_through_env: Some(vec![Spanned::<turborepo_unescape::UnescapedString>::new("AWS_SECRET_KEY".into()).with_range(106..122)]),
            outputs: Some(vec![Spanned::<turborepo_unescape::UnescapedString>::new("package\\a\\dist".into()).with_range(151..169)]),
            cache: Some(Spanned::new(false).with_range(195..200)),
            inputs: Some(vec![Spanned::<turborepo_unescape::UnescapedString>::new("package\\a\\src\\**".into()).with_range(227..248)]),
            output_logs: Some(Spanned::new(turborepo_types::OutputLogsMode::Full).with_range(279..285)),
            persistent: Some(Spanned::new(true).with_range(315..319)),
            interruptible: Some(Spanned::new(true).with_range(352..356)),
            interactive: None,
            env_mode: None,
            with: None,
        },
        TaskDefinition {
            env: vec!["OS".to_string()],
            outputs: TaskOutputs {
                inclusions: vec!["package\\a\\dist".to_string()],
                exclusions: vec![],
            },
            cache: false,
            inputs: TaskInputs::new(vec!["package\\a\\src\\**".to_string()]),
            output_logs: turborepo_types::OutputLogsMode::Full,
            pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
            task_dependencies: vec![Spanned::<turborepo_task_id::TaskName<'_>>::new("cli#build".into()).with_range(30..41)],
            topological_dependencies: vec![],
            persistent: true,
            interruptible: true,
            interactive: false,
            env_mode: None,
            with: None,
        }
      ; "full (windows)"
    )]
    #[test_case(
        r#"{
            "inputs": ["$TURBO_ROOT$/config.txt"],
            "outputs": ["$TURBO_ROOT$/coverage/**", "!$TURBO_ROOT$/coverage/index.html"]
        }"#,
        RawTaskDefinition {
            inputs: Some(vec![Spanned::new(turborepo_unescape::UnescapedString::from("$TURBO_ROOT$/config.txt")).with_range(25..50)]),
            outputs: Some(vec![
                Spanned::new(turborepo_unescape::UnescapedString::from("$TURBO_ROOT$/coverage/**")).with_range(77..103),
                Spanned::new(turborepo_unescape::UnescapedString::from("!$TURBO_ROOT$/coverage/index.html")).with_range(105..140),
            ]),
            ..RawTaskDefinition::default()
        },
        TaskDefinition {
            inputs: TaskInputs::new(vec!["../../config.txt".to_owned()]),
            outputs: TaskOutputs {
                inclusions: vec!["../../coverage/**".to_owned()],
                exclusions: vec!["../../coverage/index.html".to_owned()],
            },
            ..TaskDefinition::default()
        }
    ; "turbo root"
    )]
    #[test_case(
        r#"{
            "with": ["proxy"]
        }"#,
        RawTaskDefinition {
            with: Some(vec![
                Spanned::new(turborepo_unescape::UnescapedString::from("proxy")).with_range(23..30),
            ]),
            ..RawTaskDefinition::default()
        },
        TaskDefinition {
            with: Some(vec![Spanned::new(turborepo_task_id::TaskName::from("proxy")).with_range(23..30)]),
            ..TaskDefinition::default()
        }
    ; "with task"
    )]
    fn test_deserialize_task_definition(
        task_definition_content: &str,
        expected_raw_task_definition: RawTaskDefinition,
        expected_task_definition: TaskDefinition,
    ) -> Result<()> {
        let deserialized_result = deserialize_from_json_str(
            task_definition_content,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_task_definition: RawTaskDefinition =
            deserialized_result.into_deserialized().unwrap();
        assert_eq!(raw_task_definition, expected_raw_task_definition);

        let task_definition =
            TaskDefinition::from_raw(raw_task_definition, RelativeUnixPath::new("../..").unwrap())?;
        assert_eq!(task_definition, expected_task_definition);

        Ok(())
    }
}
