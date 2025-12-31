//! turbo.json configuration
//! Turbo.json module
//!
//! Re-exports from turborepo-turbo-json crate with loader code for
//! turborepo-lib specific functionality (MFE, task_access).

mod loader;

// Re-export the main types from turborepo-turbo-json.
// Some re-exports are used by other crates or in tests, not within this module.
#[allow(unused_imports)]
pub use turborepo_turbo_json::{
    // Functions
    task_outputs_from_processed,
    // FutureFlags
    FutureFlags,
    // Raw types
    HasConfigBeyondExtends,
    Pipeline,
    // Processed types
    ProcessedOutputs,
    ProcessedTaskDefinition,
    RawPackageTurboJson,
    RawRemoteCacheOptions,
    RawRootTurboJson,
    RawTaskDefinition,
    RawTurboJson,
    SpacesJson,
    // Extension traits
    TaskInputsFromProcessed,
    // TurboJson itself
    TurboJson,
    // Validator
    TOPOLOGICAL_PIPELINE_DELIMITER,
};

// Re-export the parser module for types like parser::Error
pub mod parser {
    pub use turborepo_turbo_json::parser::BiomeParseError as Error;
}

// Re-export the validator module
pub mod validator {
    #[allow(unused_imports)]
    pub use turborepo_turbo_json::validator::*;
}

// Loader code stays in turborepo-lib (depends on MFE, task_access)
use std::collections::HashMap;

pub use loader::{TurboJsonLoader, TurboJsonReader};
// Re-export TaskDefinitionFromProcessed from turborepo-engine (used by dependent crates)
#[allow(unused_imports)]
pub use turborepo_engine::TaskDefinitionFromProcessed;
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
    use serde_json::json;
    use test_case::test_case;
    use turbopath::RelativeUnixPath;
    use turborepo_task_id::TaskName;
    use turborepo_types::{OutputLogsMode, TaskInputs, UIMode};
    use turborepo_unescape::UnescapedString;

    use super::*;
    use crate::{
        boundaries::BoundariesConfig,
        task_graph::{TaskDefinition, TaskOutputs},
        turbo_json::RawTaskDefinition,
    };

    #[test_case("{}", "empty boundaries")]
    #[test_case(r#"{"tags": {} }"#, "empty tags")]
    #[test_case(
        r#"{"tags": { "my-tag": { "dependencies": { "allow": ["my-package"] } } }"#,
        "tags and dependencies"
    )]
    #[test_case(
        r#"{
        "tags": {
            "my-tag": {
                "dependencies": {
                    "allow": ["my-package"],
                    "deny": ["my-other-package"]
                }
            }
        }
    }"#,
        "tags and dependencies 2"
    )]
    #[test_case(
        r#"{
        "tags": {
            "my-tag": {
                "dependents": {
                    "allow": ["my-package"],
                    "deny": ["my-other-package"]
                }
            }
        }
    }"#,
        "tags and dependents"
    )]
    #[test_case(
        r#"{
            "implicitDependencies": ["my-package"],
        }"#,
        "implicit dependencies"
    )]
    #[test_case(
        r#"{
            "implicitDependencies": ["my-package"],
            "tags": {
                "my-tag": {
                    "dependents": {
                        "allow": ["my-package"],
                        "deny": ["my-other-package"]
                    }
                }
            },
        }"#,
        "implicit dependencies and tags"
    )]
    #[test_case(
        r#"{
          "dependencies": {
              "allow": ["my-package"]
          }
      }"#,
        "package rule"
    )]
    fn test_deserialize_boundaries(json: &str, name: &str) {
        let deserialized_result = deserialize_from_json_str(
            json,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_boundaries_config: BoundariesConfig =
            deserialized_result.into_deserialized().unwrap();
        insta::assert_json_snapshot!(name.replace(' ', "_"), raw_boundaries_config);
    }

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
            depends_on: Some(Spanned::new(vec![Spanned::<UnescapedString>::new("cli#build".into()).with_range(26..37)]).with_range(25..38)),
            env: Some(vec![Spanned::<UnescapedString>::new("OS".into()).with_range(58..62)]),
            pass_through_env: Some(vec![Spanned::<UnescapedString>::new("AWS_SECRET_KEY".into()).with_range(94..110)]),
            outputs: Some(vec![Spanned::<UnescapedString>::new("package/a/dist".into()).with_range(135..151)]),
            cache: Some(Spanned::new(false).with_range(173..178)),
            inputs: Some(vec![Spanned::<UnescapedString>::new("package/a/src/**".into()).with_range(201..219)]),
            output_logs: Some(Spanned::new(OutputLogsMode::Full).with_range(246..252)),
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
          output_logs: OutputLogsMode::Full,
          pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
          task_dependencies: vec![Spanned::<TaskName<'_>>::new("cli#build".into()).with_range(26..37)],
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
            depends_on: Some(Spanned::new(vec![Spanned::<UnescapedString>::new("cli#build".into()).with_range(30..41)]).with_range(29..42)),
            env: Some(vec![Spanned::<UnescapedString>::new("OS".into()).with_range(66..70)]),
            pass_through_env: Some(vec![Spanned::<UnescapedString>::new("AWS_SECRET_KEY".into()).with_range(106..122)]),
            outputs: Some(vec![Spanned::<UnescapedString>::new("package\\a\\dist".into()).with_range(151..169)]),
            cache: Some(Spanned::new(false).with_range(195..200)),
            inputs: Some(vec![Spanned::<UnescapedString>::new("package\\a\\src\\**".into()).with_range(227..248)]),
            output_logs: Some(Spanned::new(OutputLogsMode::Full).with_range(279..285)),
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
            output_logs: OutputLogsMode::Full,
            pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
            task_dependencies: vec![Spanned::<TaskName<'_>>::new("cli#build".into()).with_range(30..41)],
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
            inputs: Some(vec![Spanned::new(UnescapedString::from("$TURBO_ROOT$/config.txt")).with_range(25..50)]),
            outputs: Some(vec![
                Spanned::new(UnescapedString::from("$TURBO_ROOT$/coverage/**")).with_range(77..103),
                Spanned::new(UnescapedString::from("!$TURBO_ROOT$/coverage/index.html")).with_range(105..140),
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
                Spanned::new(UnescapedString::from("proxy")).with_range(23..30),
            ]),
            ..RawTaskDefinition::default()
        },
        TaskDefinition {
            with: Some(vec![Spanned::new(TaskName::from("proxy")).with_range(23..30)]),
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

    #[test_case("[]", TaskOutputs::default() ; "empty")]
    #[test_case(r#"["target/**"]"#, TaskOutputs { inclusions: vec!["target/**".to_string()], exclusions: vec![] })]
    #[test_case(
        r#"[".next/**", "!.next/cache/**"]"#,
        TaskOutputs {
             inclusions: vec![".next/**".to_string()],
             exclusions: vec![".next/cache/**".to_string()]
        }
        ; "with .next"
    )]
    #[test_case(
        r#"[".next\\**", "!.next\\cache\\**"]"#,
        TaskOutputs {
            inclusions: vec![".next\\**".to_string()],
            exclusions: vec![".next\\cache\\**".to_string()]
        }
        ; "with .next (windows)"
    )]
    fn test_deserialize_task_outputs(
        task_outputs_str: &str,
        expected_task_outputs: TaskOutputs,
    ) -> Result<()> {
        let raw_task_outputs: Vec<UnescapedString> = serde_json::from_str(task_outputs_str)?;
        let turbo_root = RelativeUnixPath::new("../..")?;
        let processed_outputs = ProcessedOutputs::new(
            raw_task_outputs.into_iter().map(Spanned::new).collect(),
            &FutureFlags::default(),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;
        let task_outputs = task_outputs_from_processed(processed_outputs, turbo_root)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        assert_eq!(task_outputs, expected_task_outputs);

        Ok(())
    }

    #[test]
    fn test_turbo_task_pruning() {
        let json = RawTurboJson::parse_from_serde(json!({
            "tasks": {
                "//#top": {},
                "build": {},
                "a#build": {},
                "b#build": {},
            }
        }))
        .unwrap();
        let pruned_json = json.prune_tasks(&["a"]);
        let expected: RawTurboJson = RawTurboJson::parse_from_serde(json!({
            "tasks": {
                "//#top": {},
                "build": {},
                "a#build": {},
            }
        }))
        .unwrap();
        // We do this comparison manually so we don't compare the `task_name_range`
        // fields, which are expected to be different
        let pruned_pipeline = pruned_json.tasks.unwrap();
        let expected_pipeline = expected.tasks.unwrap();
        for (
            (pruned_task_name, pruned_pipeline_entry),
            (expected_task_name, expected_pipeline_entry),
        ) in pruned_pipeline
            .into_iter()
            .zip(expected_pipeline.into_iter())
        {
            assert_eq!(pruned_task_name, expected_task_name);
            assert_eq!(pruned_pipeline_entry.value, expected_pipeline_entry.value);
        }
    }

    #[test_case("full", Some(OutputLogsMode::Full) ; "full")]
    #[test_case("hash-only", Some(OutputLogsMode::HashOnly) ; "hash-only")]
    #[test_case("new-only", Some(OutputLogsMode::NewOnly) ; "new-only")]
    #[test_case("errors-only", Some(OutputLogsMode::ErrorsOnly) ; "errors-only")]
    #[test_case("none", Some(OutputLogsMode::None) ; "none")]
    #[test_case("junk", None ; "invalid value")]
    fn test_parsing_output_logs_mode(output_logs: &str, expected: Option<OutputLogsMode>) {
        let json: Result<RawTurboJson, _> = RawTurboJson::parse_from_serde(json!({
            "tasks": {
                "build": {
                    "outputLogs": output_logs,
                }
            }
        }));

        let actual = json
            .as_ref()
            .ok()
            .and_then(|j| j.tasks.as_ref())
            .and_then(|pipeline| pipeline.0.get(&TaskName::from("build")))
            .and_then(|build| build.value.output_logs.clone())
            .map(|mode| mode.into_inner());
        assert_eq!(actual, expected);
    }

    #[test_case(r#"{ "tags": [] }"#, "empty tags in package")]
    #[test_case(r#"{ "tags": ["my-tag"] }"#, "one tag")]
    #[test_case(r#"{ "tags": ["my-tag", "my-other-tag"] }"#, "two tags")]
    fn test_tags(json: &str, name: &str) {
        let json = RawRootTurboJson::parse(json, "").unwrap();
        insta::assert_json_snapshot!(name.replace(' ', "_"), json.tags);
    }

    #[test_case(r#"{ "ui": "tui" }"#, Some(UIMode::Tui) ; "tui")]
    #[test_case(r#"{ "ui": "stream" }"#, Some(UIMode::Stream) ; "stream")]
    #[test_case(r#"{}"#, None ; "missing")]
    fn test_ui(json: &str, expected: Option<UIMode>) {
        let json = RawRootTurboJson::parse(json, "").unwrap();
        assert_eq!(json.ui.as_ref().map(|ui| *ui.as_inner()), expected);
    }

    #[test_case(r#"{ "experimentalSpaces": { "id": "hello-world" } }"#, Some(SpacesJson { id: Some("hello-world".to_string().into()) }))]
    #[test_case(r#"{ "experimentalSpaces": {} }"#, Some(SpacesJson { id: None }))]
    #[test_case(r#"{}"#, None)]
    fn test_spaces(json: &str, expected: Option<SpacesJson>) {
        let json = RawRootTurboJson::parse(json, "").unwrap();
        assert_eq!(json.experimental_spaces, expected);
    }

    #[test_case(r#"{ "daemon": true }"#, r#"{"daemon":true}"# ; "daemon_on")]
    #[test_case(r#"{ "daemon": false }"#, r#"{"daemon":false}"# ; "daemon_off")]
    fn test_daemon(json: &str, expected: &str) {
        let parsed: RawTurboJson = RawRootTurboJson::parse(json, "").unwrap().into();
        let actual = serde_json::to_string(&parsed).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case(r#"{ "ui": "tui" }"#, r#"{"ui":"tui"}"# ; "tui")]
    #[test_case(r#"{ "ui": "stream" }"#, r#"{"ui":"stream"}"# ; "stream")]
    fn test_ui_serialization(input: &str, expected: &str) {
        let parsed: RawTurboJson = RawRootTurboJson::parse(input, "").unwrap().into();
        let actual = serde_json::to_string(&parsed).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case(r#"{"dangerouslyDisablePackageManagerCheck":true}"#, Some(true) ; "t")]
    #[test_case(r#"{"dangerouslyDisablePackageManagerCheck":false}"#, Some(false) ; "f")]
    #[test_case(r#"{}"#, None ; "missing")]
    fn test_allow_no_package_manager_serde(json_str: &str, expected: Option<bool>) {
        let json: RawTurboJson = RawRootTurboJson::parse(json_str, "").unwrap().into();
        assert_eq!(
            json.allow_no_package_manager
                .as_ref()
                .map(|allow| *allow.as_inner()),
            expected
        );
        let serialized = serde_json::to_string(&json).unwrap();
        assert_eq!(serialized, json_str);
    }

    #[test]
    fn test_with_proxy_empty() {
        let mut json = TurboJson::default();
        json.with_proxy(None);
        assert_eq!(json.extends.as_inner().as_slice(), &["//".to_string()]);
        assert!(json.tasks.contains_key(&TaskName::from("proxy")));
    }

    #[test]
    fn test_with_proxy_existing() {
        let mut json = TurboJson::default();
        json.tasks.insert(
            TaskName::from("build"),
            Spanned::new(RawTaskDefinition::default()),
        );
        json.with_proxy(None);
        assert_eq!(json.extends.as_inner().as_slice(), &["//".to_string()]);
        assert!(json.tasks.contains_key(&TaskName::from("proxy")));
        assert!(json.tasks.contains_key(&TaskName::from("build")));
    }

    #[test]
    fn test_with_proxy_with_proxy_build() {
        let mut json = TurboJson::default();
        json.with_proxy(Some("my-proxy"));
        assert_eq!(json.extends.as_inner().as_slice(), &["//".to_string()]);
        let proxy_task = json.tasks.get(&TaskName::from("proxy"));
        assert!(proxy_task.is_some());
        let proxy_task = proxy_task.unwrap().as_inner();
        assert_eq!(
            proxy_task
                .depends_on
                .as_ref()
                .unwrap()
                .as_inner()
                .as_slice(),
            &[Spanned::new(UnescapedString::from("my-proxy#build"))]
        );
    }

    #[test]
    fn test_with_sibling_empty() {
        let mut json = TurboJson::default();
        json.with_task(TaskName::from("dev"), &TaskName::from("api#server"));
        let dev_task = json.tasks.get(&TaskName::from("dev"));
        assert!(dev_task.is_some());
        let dev_task = dev_task.unwrap().as_inner();
        assert_eq!(
            dev_task.with.as_ref().unwrap().as_slice(),
            &[Spanned::new(UnescapedString::from("api#server"))]
        );
    }

    #[test]
    fn test_with_sibling_existing() {
        let mut json = TurboJson::default();
        json.tasks.insert(
            TaskName::from("dev"),
            Spanned::new(RawTaskDefinition {
                persistent: Some(Spanned::new(true)),
                ..Default::default()
            }),
        );
        json.with_task(TaskName::from("dev"), &TaskName::from("api#server"));
        let dev_task = json.tasks.get(&TaskName::from("dev"));
        assert!(dev_task.is_some());
        let dev_task = dev_task.unwrap().as_inner();
        assert_eq!(dev_task.persistent, Some(Spanned::new(true)));
        assert_eq!(
            dev_task.with.as_ref().unwrap().as_slice(),
            &[Spanned::new(UnescapedString::from("api#server"))]
        );
    }

    #[test]
    fn test_future_flags_not_allowed_in_workspace() {
        let json = r#"{
            "extends": ["//"],
            "tasks": {
                "build": {}
            },
            "futureFlags": {
                "newFeature": true
            }
        }"#;

        let deserialized_result = deserialize_from_json_str(
            json,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_turbo_json: RawTurboJson = deserialized_result.into_deserialized().unwrap();

        // Try to convert to TurboJson - this should fail
        let turbo_json_result = TurboJson::try_from(raw_turbo_json);
        assert!(turbo_json_result.is_err());

        let error = turbo_json_result.unwrap_err();
        let error_str = error.to_string();
        assert!(
            error_str.contains("The \"futureFlags\" key can only be used in the root turbo.json")
        );
    }

    #[test]
    fn test_deserialize_future_flags() {
        let json = r#"{
            "tasks": {
                "build": {}
            },
            "futureFlags": {
            }
        }"#;

        let deserialized_result = deserialize_from_json_str(
            json,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_turbo_json: RawTurboJson = deserialized_result.into_deserialized().unwrap();

        // Verify that futureFlags is parsed correctly (empty now that flags are
        // removed)
        assert!(raw_turbo_json.future_flags.is_some());
        let future_flags = raw_turbo_json.future_flags.as_ref().unwrap();
        assert_eq!(future_flags.as_inner(), &FutureFlags {});

        // Verify that the futureFlags field doesn't cause errors during conversion to
        // TurboJson
        let turbo_json = TurboJson::try_from(raw_turbo_json);
        assert!(turbo_json.is_ok());
    }

    #[test_case(
        r#"{"extends": ["//"], "tasks": {"build": {}}}"#,
        false ; "root config with extends should fail"
    )]
    #[test_case(
        r#"{"globalEnv": ["NODE_ENV"], "globalDependencies": ["package.json"], "tasks": {"build": {}}}"#,
        true ; "root config with global fields should succeed"
    )]
    #[test_case(
        r#"{"futureFlags": {}, "tasks": {"build": {}}}"#,
        true ; "root config with futureFlags should succeed"
    )]
    #[test_case(
        r#"{"remoteCache": {"enabled": true}, "tasks": {"build": {}}}"#,
        true ; "root config with remoteCache should succeed"
    )]
    fn test_root_config_validation(json: &str, should_succeed: bool) {
        let result = RawRootTurboJson::parse(json, "turbo.json");
        assert_eq!(result.is_ok(), should_succeed);

        if should_succeed {
            let raw_config = RawTurboJson::from(result.unwrap());
            assert!(raw_config.extends.is_none());
        }
    }

    #[test_case(
        r#"{"extends": ["//"], "tasks": {"build": {}}, "tags": ["frontend"]}"#,
        true ; "package config with extends and tags should succeed"
    )]
    #[test_case(
        r#"{"extends": ["//"], "boundaries": {}, "tasks": {"test": {}}}"#,
        true ; "package config with extends and boundaries should succeed"
    )]
    #[test_case(
        r#"{"globalEnv": ["NODE_ENV"], "tasks": {"test": {}}}"#,
        false ; "package config with globalEnv should fail"
    )]
    #[test_case(
        r#"{"extends": ["//"], "globalDependencies": ["package.json"], "tasks": {"test": {}}}"#,
        false ; "package config with globalDependencies should fail"
    )]
    #[test_case(
        r#"{"extends": ["//"], "futureFlags": {}, "tasks": {"test": {}}}"#,
        false ; "package config with futureFlags should fail"
    )]
    #[test_case(
        r#"{"extends": ["//"], "remoteCache": {"enabled": true}, "tasks": {"test": {}}}"#,
        false ; "package config with remoteCache should fail"
    )]
    #[test_case(
        r#"{"extends": ["//"], "ui": "tui", "tasks": {"test": {}}}"#,
        false ; "package config with ui should fail"
    )]
    fn test_package_config_validation(json: &str, should_succeed: bool) {
        let result = RawPackageTurboJson::parse(json, "packages/foo/turbo.json");
        assert_eq!(result.is_ok(), should_succeed);

        if should_succeed {
            let package_config = result.unwrap();
            let raw_config = RawTurboJson::from(package_config);
            assert!(raw_config.extends.is_some());
            // Verify root-only fields are None
            assert!(raw_config.global_env.is_none());
            assert!(raw_config.global_dependencies.is_none());
            assert!(raw_config.future_flags.is_none());
        }
    }

    #[test]
    fn test_boundaries_permissions_serialization_skip_none() {
        let json_with_partial_permissions = r#"{
            "boundaries": {
                "dependencies": {
                    "allow": ["package-a"]
                }
            }
        }"#;

        let parsed: RawTurboJson =
            RawRootTurboJson::parse(json_with_partial_permissions, "turbo.json")
                .unwrap()
                .into();

        let serialized = serde_json::to_string(&parsed).unwrap();

        // The serialized JSON should not contain "deny":null
        let reparsed: RawTurboJson = RawRootTurboJson::parse(&serialized, "turbo.json")
            .unwrap()
            .into();

        // Verify the structure is preserved
        assert!(reparsed.boundaries.is_some());
        let boundaries = reparsed.boundaries.as_ref().unwrap();
        assert!(boundaries.dependencies.is_some());
        let deps = boundaries.dependencies.as_ref().unwrap();
        assert!(deps.allow.is_some());
        assert!(deps.deny.is_none()); // This should be None, not null
    }

    #[test]
    fn test_prune_tasks_preserves_boundaries_structure() {
        let json_with_boundaries = r#"{
            "tasks": {
                "build": {},
                "app-a#build": {}
            },
            "boundaries": {
                "dependencies": {
                    "allow": []
                }
            }
        }"#;

        let parsed: RawTurboJson = RawRootTurboJson::parse(json_with_boundaries, "turbo.json")
            .unwrap()
            .into();

        // Simulate the prune operation
        let pruned = parsed.prune_tasks(&["app-a"]);

        // Serialize the pruned config
        let serialized = serde_json::to_string_pretty(&pruned).unwrap();

        // Parse the serialized config to ensure it's valid
        let reparsed_result = RawRootTurboJson::parse(&serialized, "turbo.json");
        assert!(
            reparsed_result.is_ok(),
            "Failed to parse pruned config: {:?}",
            reparsed_result.err()
        );

        let reparsed: RawTurboJson = reparsed_result.unwrap().into();

        // Verify boundaries structure is preserved
        assert!(reparsed.boundaries.is_some());
        let boundaries = reparsed.boundaries.as_ref().unwrap();
        assert!(boundaries.dependencies.is_some());
        let deps = boundaries.dependencies.as_ref().unwrap();
        assert!(deps.allow.is_some());
        assert!(deps.deny.is_none()); // This should be None, not serialized as
                                      // null
    }

    #[test]
    fn test_is_root_config_with_root_path() {
        let turbo_json = TurboJson::default().with_path("turbo.json");
        assert!(
            turbo_json.is_root_config(),
            "turbo.json should be detected as root config"
        );
    }

    #[test]
    fn test_is_root_config_with_jsonc_extension() {
        let turbo_json = TurboJson::default().with_path("turbo.jsonc");
        assert!(
            turbo_json.is_root_config(),
            "turbo.jsonc should be detected as root config"
        );
    }

    #[test]
    fn test_is_root_config_with_package_path() {
        let turbo_json = TurboJson::default().with_path("packages/my-app/turbo.json");
        assert!(
            !turbo_json.is_root_config(),
            "packages/my-app/turbo.json should NOT be detected as root config"
        );
    }
}
