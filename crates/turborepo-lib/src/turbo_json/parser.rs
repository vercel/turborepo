use std::{backtrace, collections::BTreeMap, fmt::Debug, sync::Arc};

use biome_deserialize::{
    json::deserialize_from_json_str, Deserializable, DeserializableValue,
    DeserializationDiagnostic, DeserializationVisitor, VisitableType,
};
use biome_diagnostics::DiagnosticExt;
use biome_json_parser::JsonParserOptions;
use biome_json_syntax::TextRange;
use convert_case::{Case, Casing};
use miette::Diagnostic;
use struct_iterable::Iterable;
use thiserror::Error;
use tracing::log::warn;
use turborepo_errors::{ParseDiagnostic, WithMetadata};
use turborepo_unescape::UnescapedString;

use crate::{
    boundaries::{BoundariesConfig, Permissions, Rule},
    run::task_id::TaskName,
    turbo_json::{Pipeline, RawTaskDefinition, RawTurboJson, Spanned},
};

// Field placement constants for turbo.json validation
// When adding new fields to RawTurboJson, developers MUST add them to one of
// these allowlists, forcing explicit categorization decisions.

/// Fields that can only be used in the root turbo.json
const ROOT_ONLY_FIELDS: &[&str] = &[
    "globalDependencies",
    "globalEnv",
    "globalPassThroughEnv",
    "ui",
    "noUpdateNotifier",
    "concurrency",
    "dangerouslyDisablePackageManagerCheck",
    "cacheDir",
    "daemon",
    "envMode",
    "remoteCache",
    "boundaries",
];

/// Fields that can only be used in Package Configurations
const PACKAGE_ONLY_FIELDS: &[&str] = &["tags", "extends"];

/// Fields that can be used in both root and package configurations
const UNIVERSAL_FIELDS: &[&str] = &[
    "$schema",
    "tasks",
    "experimentalSpaces",
    "pipeline",
    "futureFlags",
];

#[derive(Debug, Error, Diagnostic)]
#[error("Failed to parse turbo.json.")]
#[diagnostic(code(turbo_json_parse_error))]
pub struct Error {
    #[related]
    diagnostics: Vec<ParseDiagnostic>,
    #[backtrace]
    backtrace: backtrace::Backtrace,
}

fn create_unknown_key_diagnostic_from_struct<T: Iterable>(
    struct_iterable: &T,
    unknown_key: &str,
    range: TextRange,
) -> DeserializationDiagnostic {
    let allowed_keys = struct_iterable
        .iter()
        .map(|(k, _)| k.to_case(Case::Camel))
        .collect::<Vec<_>>();
    let allowed_keys_borrowed = allowed_keys.iter().map(|s| s.as_str()).collect::<Vec<_>>();

    DeserializationDiagnostic::new_unknown_key(unknown_key, range, &allowed_keys_borrowed)
}

fn create_field_placement_error_message(field_name: &str, is_root_only: bool) -> String {
    if is_root_only {
        format!(
            "The \"{}\" field can only be used in the root turbo.json. Please remove it from \
             Package Configurations.",
            field_name
        )
    } else {
        format!(
            "The \"{}\" field can only be used in Package Configurations. Please remove it from \
             the root turbo.json.",
            field_name
        )
    }
}

#[derive(Debug)]
pub struct FieldPlacementError {
    pub message: String,
    pub range: Option<std::ops::Range<usize>>,
    pub field_name: String,
}

impl Deserializable for TaskName<'static> {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let task_id: String = UnescapedString::deserialize(value, name, diagnostics)?.into();

        Some(Self::from(task_id))
    }
}

impl Deserializable for Pipeline {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(PipelineVisitor, name, diagnostics)
    }
}

struct PipelineVisitor;

impl DeserializationVisitor for PipelineVisitor {
    type Output = Pipeline;

    const EXPECTED_TYPE: VisitableType = VisitableType::MAP;

    fn visit_map(
        self,
        members: impl Iterator<Item = Option<(impl DeserializableValue, impl DeserializableValue)>>,
        _range: TextRange,
        _name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        let mut result = BTreeMap::new();
        for (key, value) in members.flatten() {
            let task_name_range = value.range();
            let task_name = TaskName::deserialize(&key, "", diagnostics)?;
            let task_name_start: usize = task_name_range.start().into();
            let task_name_end: usize = task_name_range.end().into();
            result.insert(
                task_name,
                Spanned::new(RawTaskDefinition::deserialize(&value, "", diagnostics)?)
                    .with_range(task_name_start..task_name_end),
            );
        }

        Some(Pipeline(result))
    }
}

impl WithMetadata for RawTurboJson {
    fn add_text(&mut self, text: Arc<str>) {
        self.span.add_text(text.clone());
        self.extends.add_text(text.clone());
        self.tags.add_text(text.clone());
        if let Some(tags) = &mut self.tags {
            tags.value.add_text(text.clone());
        }
        self.global_dependencies.add_text(text.clone());
        self.global_env.add_text(text.clone());
        self.global_pass_through_env.add_text(text.clone());
        self.boundaries.add_text(text.clone());
        if let Some(boundaries) = &mut self.boundaries {
            boundaries.value.add_text(text.clone());
        }

        self.tasks.add_text(text.clone());
        self.cache_dir.add_text(text.clone());
        self.pipeline.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.span.add_path(path.clone());
        self.extends.add_path(path.clone());
        self.tags.add_path(path.clone());
        if let Some(tags) = &mut self.tags {
            tags.value.add_path(path.clone());
        }
        self.global_dependencies.add_path(path.clone());
        self.global_env.add_path(path.clone());
        self.global_pass_through_env.add_path(path.clone());
        self.boundaries.add_path(path.clone());
        if let Some(boundaries) = &mut self.boundaries {
            boundaries.value.add_path(path.clone());
        }
        self.tasks.add_path(path.clone());
        self.cache_dir.add_path(path.clone());
        self.pipeline.add_path(path);
    }
}

impl WithMetadata for Pipeline {
    fn add_text(&mut self, text: Arc<str>) {
        for (_, entry) in self.0.iter_mut() {
            entry.add_text(text.clone());
            entry.value.add_text(text.clone());
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        for (_, entry) in self.0.iter_mut() {
            entry.add_path(path.clone());
            entry.value.add_path(path.clone());
        }
    }
}

impl WithMetadata for BoundariesConfig {
    fn add_text(&mut self, text: Arc<str>) {
        self.tags.add_text(text.clone());
        if let Some(tags) = &mut self.tags {
            for rule in tags.as_inner_mut().values_mut() {
                rule.add_text(text.clone());
                rule.value.add_text(text.clone());
            }
        }
        self.implicit_dependencies.add_text(text.clone());
        if let Some(implicit_dependencies) = &mut self.implicit_dependencies {
            for dep in implicit_dependencies.as_inner_mut() {
                dep.add_text(text.clone());
            }
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.tags.add_path(path.clone());
        if let Some(tags) = &mut self.tags {
            for rule in tags.as_inner_mut().values_mut() {
                rule.add_path(path.clone());
                rule.value.add_path(path.clone());
            }
        }
        self.implicit_dependencies.add_path(path.clone());
        if let Some(implicit_dependencies) = &mut self.implicit_dependencies {
            for dep in implicit_dependencies.as_inner_mut() {
                dep.add_path(path.clone());
            }
        }
    }
}

impl WithMetadata for Rule {
    fn add_text(&mut self, text: Arc<str>) {
        self.dependencies.add_text(text.clone());
        if let Some(dependencies) = &mut self.dependencies {
            dependencies.value.add_text(text.clone());
        }

        self.dependents.add_text(text.clone());
        if let Some(dependents) = &mut self.dependents {
            dependents.value.add_text(text.clone());
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.dependencies.add_path(path.clone());
        if let Some(dependencies) = &mut self.dependencies {
            dependencies.value.add_path(path.clone());
        }

        self.dependents.add_path(path.clone());
        if let Some(dependents) = &mut self.dependents {
            dependents.value.add_path(path);
        }
    }
}

impl WithMetadata for Permissions {
    fn add_text(&mut self, text: Arc<str>) {
        self.allow.add_text(text.clone());
        if let Some(allow) = &mut self.allow {
            allow.value.add_text(text.clone());
        }

        self.deny.add_text(text.clone());
        if let Some(deny) = &mut self.deny {
            deny.value.add_text(text.clone());
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.allow.add_path(path.clone());
        if let Some(allow) = &mut self.allow {
            allow.value.add_path(path.clone());
        }

        self.deny.add_path(path.clone());
        if let Some(deny) = &mut self.deny {
            deny.value.add_path(path.clone());
        }
    }
}

impl WithMetadata for RawTaskDefinition {
    fn add_text(&mut self, text: Arc<str>) {
        self.depends_on.add_text(text.clone());
        if let Some(depends_on) = &mut self.depends_on {
            depends_on.value.add_text(text.clone());
        }
        self.env.add_text(text.clone());
        self.inputs.add_text(text.clone());
        self.pass_through_env.add_text(text.clone());
        self.persistent.add_text(text.clone());
        self.interruptible.add_text(text.clone());
        self.outputs.add_text(text.clone());
        self.output_logs.add_text(text.clone());
        self.interactive.add_text(text.clone());
        self.with.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.depends_on.add_path(path.clone());
        if let Some(depends_on) = &mut self.depends_on {
            depends_on.value.add_path(path.clone());
        }
        self.env.add_path(path.clone());
        self.inputs.add_path(path.clone());
        self.pass_through_env.add_path(path.clone());
        self.persistent.add_path(path.clone());
        self.interruptible.add_path(path.clone());
        self.outputs.add_path(path.clone());
        self.output_logs.add_path(path.clone());
        self.interactive.add_path(path.clone());
        self.with.add_path(path);
    }
}

impl RawTurboJson {
    // A simple helper for tests
    #[cfg(test)]
    pub fn parse_from_serde(value: serde_json::Value) -> Result<RawTurboJson, Error> {
        let json_string = serde_json::to_string(&value).expect("should be able to serialize");
        Self::parse(&json_string, "turbo.json")
    }

    /// Validates field placement to ensure root-only and package-only fields
    /// are used in the correct configuration types.
    ///
    /// This uses an allowlist approach - ALL fields must be explicitly
    /// categorized. When adding new fields to RawTurboJson, developers MUST
    /// add them to one of the allowlists below, forcing explicit
    /// categorization decisions.
    pub fn validate_field_placement(&self) -> Result<(), FieldPlacementError> {
        let is_workspace_config = self.extends.is_some();

        // This function ensures ALL fields are explicitly categorized
        // by checking each field individually. If you add a new field to
        // RawTurboJson, you MUST add a check here and put it in one of the
        // allowlists defined at the top of this file, forcing explicit categorization.

        // Helper function to validate field placement and extract range info
        let validate_field_placement = |field_name: &str,
                                        range: Option<std::ops::Range<usize>>|
         -> Result<(), FieldPlacementError> {
            if UNIVERSAL_FIELDS.contains(&field_name) {
                // Universal field - allowed everywhere
            } else if ROOT_ONLY_FIELDS.contains(&field_name) {
                if is_workspace_config {
                    return Err(FieldPlacementError {
                        message: create_field_placement_error_message(field_name, true),
                        range,
                        field_name: field_name.to_string(),
                    });
                }
            } else if PACKAGE_ONLY_FIELDS.contains(&field_name) {
                if !is_workspace_config {
                    return Err(FieldPlacementError {
                        message: create_field_placement_error_message(field_name, false),
                        range,
                        field_name: field_name.to_string(),
                    });
                }
            } else {
                return Err(FieldPlacementError {
                    message: format!(
                        "Field '{}' is not categorized in field placement validation. Please add \
                         it to one of the constants: ROOT_ONLY_FIELDS, PACKAGE_ONLY_FIELDS, or \
                         UNIVERSAL_FIELDS at the top of this file.",
                        field_name
                    ),
                    range,
                    field_name: field_name.to_string(),
                });
            }
            Ok(())
        };

        // Helper function to check fields with span information
        let check_spanned_field = |field_name: &str,
                                   range: Option<std::ops::Range<usize>>|
         -> Result<(), FieldPlacementError> {
            validate_field_placement(field_name, range)
        };

        // Helper function to check fields without span information
        let check_field =
            |field: &Option<_>, field_name: &str| -> Result<(), FieldPlacementError> {
                if field.is_some() {
                    if UNIVERSAL_FIELDS.contains(&field_name) {
                        // Universal field - allowed everywhere
                    } else if ROOT_ONLY_FIELDS.contains(&field_name) {
                        if is_workspace_config {
                            return Err(FieldPlacementError {
                                message: create_field_placement_error_message(field_name, true),
                                range: None,
                                field_name: field_name.to_string(),
                            });
                        }
                    } else if PACKAGE_ONLY_FIELDS.contains(&field_name) {
                        if !is_workspace_config {
                            return Err(FieldPlacementError {
                                message: create_field_placement_error_message(field_name, false),
                                range: None,
                                field_name: field_name.to_string(),
                            });
                        }
                    } else {
                        return Err(FieldPlacementError {
                            message: format!(
                                "Field '{}' is not categorized in field placement validation. \
                                 Please add it to one of the constants: ROOT_ONLY_FIELDS, \
                                 PACKAGE_ONLY_FIELDS, or UNIVERSAL_FIELDS at the top of this file.",
                                field_name
                            ),
                            range: None,
                            field_name: field_name.to_string(),
                        });
                    }
                }
                Ok(())
            };

        // Check every field in RawTurboJson - if you add a field, you MUST add it here
        // Fields with span information:
        if self.schema.is_some() {
            check_spanned_field(
                "$schema",
                self.schema.as_ref().and_then(|f| f.range.clone()),
            )?;
        }
        if self.experimental_spaces.is_some() {
            check_spanned_field(
                "experimentalSpaces",
                self.experimental_spaces
                    .as_ref()
                    .and_then(|f| f.range.clone()),
            )?;
        }
        if self.extends.is_some() {
            check_spanned_field(
                "extends",
                self.extends.as_ref().and_then(|f| f.range.clone()),
            )?;
        }
        if self.tasks.is_some() {
            check_spanned_field("tasks", self.tasks.as_ref().and_then(|f| f.range.clone()))?;
        }
        if self.remote_cache.is_some() {
            check_spanned_field(
                "remoteCache",
                self.remote_cache.as_ref().and_then(|f| f.range.clone()),
            )?;
        }
        if self.ui.is_some() {
            check_spanned_field("ui", self.ui.as_ref().and_then(|f| f.range.clone()))?;
        }
        if self.allow_no_package_manager.is_some() {
            check_spanned_field(
                "dangerouslyDisablePackageManagerCheck",
                self.allow_no_package_manager
                    .as_ref()
                    .and_then(|f| f.range.clone()),
            )?;
        }
        if self.daemon.is_some() {
            check_spanned_field("daemon", self.daemon.as_ref().and_then(|f| f.range.clone()))?;
        }
        if self.env_mode.is_some() {
            check_spanned_field(
                "envMode",
                self.env_mode.as_ref().and_then(|f| f.range.clone()),
            )?;
        }
        if self.cache_dir.is_some() {
            check_spanned_field(
                "cacheDir",
                self.cache_dir.as_ref().and_then(|f| f.range.clone()),
            )?;
        }
        if self.no_update_notifier.is_some() {
            check_spanned_field(
                "noUpdateNotifier",
                self.no_update_notifier
                    .as_ref()
                    .and_then(|f| f.range.clone()),
            )?;
        }
        if self.tags.is_some() {
            check_spanned_field("tags", self.tags.as_ref().and_then(|f| f.range.clone()))?;
        }
        if self.boundaries.is_some() {
            check_spanned_field(
                "boundaries",
                self.boundaries.as_ref().and_then(|f| f.range.clone()),
            )?;
        }
        if self.concurrency.is_some() {
            check_spanned_field(
                "concurrency",
                self.concurrency.as_ref().and_then(|f| f.range.clone()),
            )?;
        }
        if self.future_flags.is_some() {
            check_spanned_field(
                "futureFlags",
                self.future_flags.as_ref().and_then(|f| f.range.clone()),
            )?;
        }
        if self.pipeline.is_some() {
            check_spanned_field(
                "pipeline",
                self.pipeline.as_ref().and_then(|f| f.range.clone()),
            )?;
        }

        // Fields without span information (lists with individual spanned items):
        check_field(&self.global_dependencies, "globalDependencies")?;
        check_field(&self.global_env, "globalEnv")?;
        check_field(&self.global_pass_through_env, "globalPassThroughEnv")?;

        Ok(())
    }

    /// Parses a turbo.json file into the raw representation with span info
    /// attached.
    ///
    /// # Arguments
    ///
    /// * `text`: The text contents of the turbo.json file
    /// * `file_path`: The path to the turbo.json file. Just used for error
    ///   display, so doesn't need to actually be a correct path.
    ///
    /// returns: Result<RawTurboJson, Error>
    pub fn parse(text: &str, file_path: &str) -> Result<RawTurboJson, Error> {
        let result = deserialize_from_json_str::<RawTurboJson>(
            text,
            JsonParserOptions::default()
                .with_allow_comments()
                .with_allow_trailing_commas(),
            file_path,
        );

        if !result.diagnostics().is_empty() {
            let diagnostics = result
                .into_diagnostics()
                .into_iter()
                .map(|d| {
                    d.with_file_source_code(text)
                        .with_file_path(file_path)
                        .as_ref()
                        .into()
                })
                .collect();

            return Err(Error {
                diagnostics,
                backtrace: backtrace::Backtrace::capture(),
            });
        }

        // It's highly unlikely that biome would fail to produce a deserialized value
        // *and* not return any errors, but it's still possible. In that case, we
        // just print that there is an error and return.
        let mut turbo_json = result.into_deserialized().ok_or_else(|| Error {
            diagnostics: vec![],
            backtrace: backtrace::Backtrace::capture(),
        })?;

        if turbo_json.experimental_spaces.is_some() {
            warn!("`experimentalSpaces` key in turbo.json is deprecated and does not do anything")
        }

        turbo_json.add_text(Arc::from(text));
        turbo_json.add_path(Arc::from(file_path));

        // Validate field placement
        if let Err(field_placement_error) = turbo_json.validate_field_placement() {
            // Create a proper diagnostic with the field placement error message and span
            let diagnostic = if let Some(range) = field_placement_error.range {
                // Convert Range<usize> to TextRange (u32)
                let text_range =
                    TextRange::new((range.start as u32).into(), (range.end as u32).into());
                DeserializationDiagnostic::new(field_placement_error.message)
                    .with_range(text_range)
                    .with_file_source_code(text)
                    .with_file_path(file_path)
            } else {
                DeserializationDiagnostic::new(field_placement_error.message)
                    .with_file_source_code(text)
                    .with_file_path(file_path)
            };

            return Err(Error {
                diagnostics: vec![diagnostic.as_ref().into()],
                backtrace: std::backtrace::Backtrace::capture(),
            });
        }

        Ok(turbo_json)
    }
}
