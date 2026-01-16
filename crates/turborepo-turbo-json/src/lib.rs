//! Turborepo turbo.json parsing and validation
//! Turbo.json configuration crate
//!
//! This crate provides functionality for parsing, validating, and processing
//! turbo.json configuration files.

#![feature(assert_matches)]
#![feature(error_generic_member_access)]
// Allow unused_assignments for error/diagnostic struct fields that are read by
// miette's derive macros, not directly by code. The derive macros generate code
// that reads these fields for error formatting and display.
#![allow(unused_assignments)]
// The Error type is large due to miette diagnostic fields (NamedSource, SourceSpan).
// This is intentional for rich error reporting. Boxing would add indirection overhead
// for error paths that are not performance-critical.
#![allow(clippy::result_large_err)]

use std::{collections::HashSet, sync::Arc};

use turbopath::{AbsoluteSystemPath, RelativeUnixPath};
use turborepo_boundaries::BoundariesConfig;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::ROOT_PKG_NAME;
use turborepo_task_id::{TaskId, TaskName};
use turborepo_types::EnvMode;
use turborepo_unescape::UnescapedString;

pub mod error;
mod extend;
pub mod future_flags;
pub mod loader;
pub mod parser;
pub mod processed;
pub mod raw;
pub mod validator;

pub use error::{Error, InvalidEnvPrefixError, ParseError, UnnecessaryPackageTaskSyntaxError};
pub use future_flags::FutureFlags;
pub use loader::{
    CONFIG_FILE, CONFIG_FILE_JSONC, LoaderError, NoOpUpdater, TASK_ACCESS_CONFIG_PATH,
    TurboJsonLoader, TurboJsonPath, TurboJsonReader, TurboJsonUpdater, load_from_path,
};
pub use parser::{BiomeParseError, parse_turbo_json};
pub use processed::{
    ProcessedDependsOn, ProcessedEnv, ProcessedGlob, ProcessedInputs, ProcessedOutputs,
    ProcessedPassThroughEnv, ProcessedTaskDefinition, ProcessedWith,
};
pub use raw::{
    HasConfigBeyondExtends, Pipeline, RawPackageTurboJson, RawRemoteCacheOptions, RawRootTurboJson,
    RawTaskDefinition, RawTurboJson, SpacesJson,
};
pub use validator::{TOPOLOGICAL_PIPELINE_DELIMITER, Validator};

/// Constant for environment variable delimiter in pipeline dependencies
pub const ENV_PIPELINE_DELIMITER: &str = "$";

// A turbo.json config that is synthesized but not yet resolved.
// This means that we've done the work to synthesize the config from
// package.json, but we haven't yet resolved the workspace
// turbo.json files into a single definition. Therefore we keep the
// `RawTaskDefinition` type so we can determine which fields are actually
// set when we resolve the configuration.
//
// Note that the values here are limited to pipeline configuration.
// Configuration that needs to account for flags, env vars, etc. is
// handled via layered config.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TurboJson {
    text: Option<Arc<str>>,
    path: Option<Arc<str>>,
    pub tags: Option<Spanned<Vec<Spanned<String>>>>,
    pub boundaries: Option<Spanned<BoundariesConfig>>,
    pub extends: Spanned<Vec<String>>,
    pub global_deps: Vec<String>,
    pub global_env: Vec<String>,
    pub global_pass_through_env: Option<Vec<String>>,
    pub tasks: Pipeline,
    pub future_flags: FutureFlags,
}

impl TryFrom<RawTurboJson> for TurboJson {
    type Error = Error;

    fn try_from(raw_turbo: RawTurboJson) -> Result<Self, Error> {
        if let Some(pipeline) = raw_turbo.pipeline {
            let (span, text) = pipeline.span_and_text("turbo.json");
            return Err(Error::PipelineField { span, text });
        }

        // `futureFlags` key is only allowed in root turbo.json
        let is_workspace_config = raw_turbo.extends.is_some();
        if is_workspace_config && raw_turbo.future_flags.is_some() {
            let future_flags = raw_turbo.future_flags.unwrap();
            let (span, text) = future_flags.span_and_text("turbo.json");
            return Err(Error::FutureFlagsInPackage { span, text });
        }
        let mut global_env = HashSet::new();
        let mut global_file_dependencies = HashSet::new();

        if let Some(global_env_from_turbo) = raw_turbo.global_env {
            gather_env_vars(global_env_from_turbo, "globalEnv", &mut global_env)?;
        }

        for global_dep in raw_turbo.global_dependencies.into_iter().flatten() {
            if global_dep.strip_prefix(ENV_PIPELINE_DELIMITER).is_some() {
                let (span, text) = global_dep.span_and_text("turbo.json");
                return Err(Error::InvalidDependsOnValue {
                    field: "globalDependencies",
                    span,
                    text,
                });
            } else if camino::Utf8Path::new(&global_dep.value).is_absolute() {
                let (span, text) = global_dep.span_and_text("turbo.json");
                return Err(Error::AbsolutePathInConfig {
                    field: "globalDependencies",
                    span,
                    text,
                });
            } else {
                global_file_dependencies.insert(global_dep.into_inner().into());
            }
        }

        let tasks = raw_turbo.tasks.clone().unwrap_or_default();

        Ok(TurboJson {
            text: raw_turbo.span.text,
            path: raw_turbo.span.path,
            tags: raw_turbo.tags,
            global_env: {
                let mut global_env: Vec<_> = global_env.into_iter().collect();
                global_env.sort();
                global_env
            },
            global_pass_through_env: raw_turbo
                .global_pass_through_env
                .map(|env| -> Result<Vec<String>, Error> {
                    let mut global_pass_through_env = HashSet::new();
                    gather_env_vars(env, "globalPassThroughEnv", &mut global_pass_through_env)?;
                    let mut global_pass_through_env: Vec<String> =
                        global_pass_through_env.into_iter().collect();
                    global_pass_through_env.sort();
                    Ok(global_pass_through_env)
                })
                .transpose()?,
            global_deps: {
                let mut global_deps: Vec<_> = global_file_dependencies.into_iter().collect();
                global_deps.sort();

                global_deps
            },
            tasks,
            // copy these over, we don't need any changes here.
            extends: raw_turbo
                .extends
                .unwrap_or_default()
                .map(|s| s.into_iter().map(|s| s.into()).collect()),
            boundaries: raw_turbo.boundaries,
            future_flags: raw_turbo
                .future_flags
                .map(|f| f.into_inner())
                .unwrap_or_default(),
            // Remote Cache config is handled through layered config
        })
    }
}

impl TurboJson {
    /// Check if this TurboJson has a task matching the given task name
    pub fn has_task(&self, task_name: &TaskName) -> bool {
        for key in self.tasks.keys() {
            if key == task_name || (key.task() == task_name.task() && !task_name.is_package_task())
            {
                return true;
            }
        }

        false
    }

    /// Check if this is a root turbo.json configuration (not a package config)
    pub fn is_root_config(&self) -> bool {
        self.path
            .as_ref()
            .map(|p| {
                let path_str = p.as_ref();
                path_str == "turbo.json" || path_str == "turbo.jsonc"
            })
            .unwrap_or(false)
    }

    /// Get the text content of the turbo.json file
    pub fn text(&self) -> Option<&Arc<str>> {
        self.text.as_ref()
    }

    /// Get the path of the turbo.json file
    pub fn path(&self) -> Option<&Arc<str>> {
        self.path.as_ref()
    }

    /// Reads a `RawTurboJson` from the given path
    /// and then converts it into `TurboJson`
    ///
    /// Should never be called directly outside of this module.
    /// `TurboJsonReader` should be used instead.
    pub fn read(
        repo_root: &AbsoluteSystemPath,
        path: &AbsoluteSystemPath,
        is_root: bool,
        future_flags: FutureFlags,
    ) -> Result<Option<TurboJson>, Error> {
        let Some(raw_turbo_json) = RawTurboJson::read(repo_root, path, is_root)? else {
            return Ok(None);
        };

        let mut turbo_json = TurboJson::try_from(raw_turbo_json)?;
        // Override with root's future flags (only root turbo.json can define them)
        turbo_json.future_flags = future_flags;
        Ok(Some(turbo_json))
    }

    /// Get a task definition from this TurboJson by task ID or task name
    pub fn task(
        &self,
        task_id: &TaskId,
        task_name: &TaskName,
    ) -> Result<Option<ProcessedTaskDefinition>, Error> {
        match self.tasks.get(&task_id.as_task_name()) {
            Some(entry) => {
                ProcessedTaskDefinition::from_raw(entry.value.clone(), &self.future_flags).map(Some)
            }
            None => self
                .tasks
                .get(task_name)
                .map(|entry| {
                    ProcessedTaskDefinition::from_raw(entry.value.clone(), &self.future_flags)
                })
                .transpose(),
        }
    }

    /// Check if this TurboJson has any root tasks (tasks prefixed with //#)
    pub fn has_root_tasks(&self) -> bool {
        self.tasks
            .iter()
            .any(|(task_name, _)| task_name.package() == Some(ROOT_PKG_NAME))
    }

    /// Adds a local proxy task to a workspace TurboJson
    pub fn with_proxy(&mut self, mfe_package_name: Option<&str>) {
        if self.extends.is_empty() {
            self.extends = Spanned::new(vec!["//".into()]);
        }

        self.tasks.insert(
            TaskName::from("proxy"),
            Spanned::new(RawTaskDefinition {
                cache: Some(Spanned::new(false)),
                depends_on: mfe_package_name.map(|mfe_package_name| {
                    Spanned::new(vec![Spanned::new(UnescapedString::from(format!(
                        "{mfe_package_name}#build"
                    )))])
                }),
                persistent: Some(Spanned::new(true)),
                env_mode: Some(Spanned::new(EnvMode::Loose)),
                ..Default::default()
            }),
        );
    }

    /// Adds a "with" relationship from `task` to `with`
    pub fn with_task(&mut self, task: TaskName<'static>, with: &TaskName) {
        if self.extends.is_empty() {
            self.extends = Spanned::new(vec!["//".into()]);
        }

        let task_definition = self.tasks.entry(task).or_default();

        let with_tasks = task_definition.as_inner_mut().with.get_or_insert_default();

        with_tasks.push(Spanned::new(UnescapedString::from(with.to_string())))
    }

    /// Set the path for this TurboJson (intended for testing)
    pub fn set_path(&mut self, path: Option<Arc<str>>) {
        self.path = path;
    }

    /// Set the text for this TurboJson (intended for testing)
    pub fn set_text(&mut self, text: Option<Arc<str>>) {
        self.text = text;
    }

    /// Create a TurboJson with a specific path (intended for testing)
    pub fn with_path(mut self, path: impl Into<Arc<str>>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Clear text and path fields (intended for testing - useful for
    /// comparison)
    pub fn clear_metadata(&mut self) {
        self.text = None;
        self.path = None;
    }
}

fn gather_env_vars(
    vars: Vec<Spanned<impl Into<String>>>,
    key: &str,
    into: &mut HashSet<String>,
) -> Result<(), Error> {
    for value in vars {
        let value: Spanned<String> = value.map(|v| v.into());
        if value.starts_with(ENV_PIPELINE_DELIMITER) {
            let (span, text) = value.span_and_text("turbo.json");
            // Hard error to help people specify this correctly during migration.
            // TODO: Remove this error after we have run summary.
            return Err(Error::InvalidEnvPrefix(Box::new(InvalidEnvPrefixError {
                key: key.to_string(),
                value: value.into_inner(),
                span,
                text,
                env_pipeline_delimiter: ENV_PIPELINE_DELIMITER,
            })));
        }

        into.insert(value.into_inner());
    }

    Ok(())
}

// ============================================================================
// Extension traits for creating TaskDefinition-related types from processed
// types. These are defined here to allow the types to be defined in other
// crates without circular dependencies.
// ============================================================================

/// Extension trait for creating TaskInputs from ProcessedInputs.
/// This is defined here rather than on the type itself to allow TaskInputs
/// to live in turborepo-types without depending on turbo_json types.
pub trait TaskInputsFromProcessed {
    /// Creates TaskInputs from ProcessedInputs with resolved paths
    fn from_processed(
        inputs: ProcessedInputs,
        turbo_root_path: &RelativeUnixPath,
    ) -> turborepo_types::TaskInputs;
}

impl TaskInputsFromProcessed for turborepo_types::TaskInputs {
    fn from_processed(
        inputs: ProcessedInputs,
        turbo_root_path: &RelativeUnixPath,
    ) -> turborepo_types::TaskInputs {
        // Resolve all globs with the turbo_root path
        // Absolute path validation was already done during ProcessedGlob creation
        turborepo_types::TaskInputs {
            globs: inputs.resolve(turbo_root_path),
            default: inputs.default,
        }
    }
}

/// Creates TaskOutputs from ProcessedOutputs with resolved paths
pub fn task_outputs_from_processed(
    outputs: ProcessedOutputs,
    turbo_root_path: &RelativeUnixPath,
) -> Result<turborepo_types::TaskOutputs, Error> {
    let mut inclusions = Vec::new();
    let mut exclusions = Vec::new();

    // Resolve all globs with the turbo_root path
    // Absolute path validation was already done during ProcessedGlob creation
    let resolved = outputs.resolve(turbo_root_path);

    for glob_str in resolved {
        if let Some(stripped_glob) = glob_str.strip_prefix('!') {
            exclusions.push(stripped_glob.to_string());
        } else {
            inclusions.push(glob_str);
        }
    }

    inclusions.sort();
    exclusions.sort();

    Ok(turborepo_types::TaskOutputs {
        inclusions,
        exclusions,
    })
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
    use turborepo_boundaries::BoundariesConfig;
    use turborepo_task_id::TaskName;
    use turborepo_types::{OutputLogsMode, TaskOutputs, UIMode};
    use turborepo_unescape::UnescapedString;

    use super::*;

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
        assert_eq!(future_flags.as_inner(), &FutureFlags::default());

        // Verify that the futureFlags field doesn't cause errors during conversion to
        // TurboJson
        let turbo_json = TurboJson::try_from(raw_turbo_json);
        assert!(turbo_json.is_ok());
    }

    #[test]
    fn test_deserialize_future_flags_errors_only_show_hash() {
        let json = r#"{
            "tasks": {
                "build": {}
            },
            "futureFlags": {
                "errorsOnlyShowHash": true
            }
        }"#;

        let deserialized_result = deserialize_from_json_str(
            json,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_turbo_json: RawTurboJson = deserialized_result.into_deserialized().unwrap();

        // Verify that futureFlags is parsed correctly
        assert!(raw_turbo_json.future_flags.is_some());
        let future_flags = raw_turbo_json.future_flags.as_ref().unwrap();
        assert!(future_flags.as_inner().errors_only_show_hash);

        // Verify that the futureFlags field doesn't cause errors during conversion to
        // TurboJson
        let turbo_json = TurboJson::try_from(raw_turbo_json);
        assert!(turbo_json.is_ok());
        assert!(turbo_json.unwrap().future_flags.errors_only_show_hash);
    }

    #[test]
    fn test_is_root_config_with_root_path() {
        let turbo_json = TurboJson {
            path: Some("turbo.json".into()),
            ..Default::default()
        };
        assert!(
            turbo_json.is_root_config(),
            "turbo.json should be detected as root config"
        );
    }

    #[test]
    fn test_is_root_config_with_jsonc_extension() {
        let turbo_json = TurboJson {
            path: Some("turbo.jsonc".into()),
            ..Default::default()
        };
        assert!(
            turbo_json.is_root_config(),
            "turbo.jsonc should be detected as root config"
        );
    }

    #[test]
    fn test_is_root_config_with_package_path() {
        let turbo_json = TurboJson {
            path: Some("packages/my-app/turbo.json".into()),
            ..Default::default()
        };
        assert!(
            !turbo_json.is_root_config(),
            "packages/my-app/turbo.json should NOT be detected as root config"
        );
    }

    // Tests moved from turborepo-lib/turbo_json/mod.rs during consolidation

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

        let actual: Option<OutputLogsMode> = json
            .as_ref()
            .ok()
            .and_then(|j| j.tasks.as_ref())
            .and_then(|pipeline| pipeline.0.get(&TaskName::from("build")))
            .and_then(|build| build.value.output_logs.clone())
            .map(|mode| mode.into_inner());
        assert_eq!(actual, expected);
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
}
