//! Parser for turbo.json configuration files
//! Turbo.json parsing module
//!
//! This module provides parsing functionality for turbo.json using biome's
//! JSON parser with support for comments and trailing commas.

use std::{backtrace, collections::BTreeMap, fmt::Debug, sync::Arc};

use biome_deserialize::{
    Deserializable, DeserializableValue, DeserializationDiagnostic, DeserializationVisitor,
    VisitableType,
};
use biome_diagnostics::DiagnosticExt;
use biome_json_parser::JsonParserOptions;
use biome_json_syntax::TextRange;
use convert_case::{Case, Casing};
use miette::Diagnostic;
use struct_iterable::Iterable;
use thiserror::Error;
use turborepo_errors::{ParseDiagnostic, Spanned, WithMetadata};
use turborepo_task_id::TaskName;
use turborepo_unescape::UnescapedString;

use crate::raw::{
    Pipeline, RawCommand, RawExperimentalObservability, RawGlobalConfig, RawObservabilityOtel,
    RawObservabilityOtelMetrics, RawObservabilityOtelRunAttributes,
    RawObservabilityOtelTaskAttributes, RawPackageTurboJson, RawRemoteCacheOptions,
    RawRootTurboJson, RawStructuredInput, RawTaskDefinition, RawTaskInput, RawTurboJson,
};

/// Error type for turbo.json parsing failures using biome parser
#[derive(Debug, Error, Diagnostic)]
#[error("Failed to parse turbo.json.")]
#[diagnostic(code(turbo_json_parse_error))]
pub struct BiomeParseError {
    #[related]
    pub diagnostics: Vec<ParseDiagnostic>,
    #[backtrace]
    pub backtrace: backtrace::Backtrace,
}

impl BiomeParseError {
    /// Creates a new BiomeParseError with the given diagnostics
    pub fn new(diagnostics: Vec<ParseDiagnostic>) -> Self {
        Self {
            diagnostics,
            backtrace: backtrace::Backtrace::capture(),
        }
    }

    /// Creates an empty error (for cases where deserialization fails without
    /// diagnostics)
    pub fn empty() -> Self {
        Self {
            diagnostics: vec![],
            backtrace: backtrace::Backtrace::capture(),
        }
    }
}

/// Creates an unknown key diagnostic from a struct that implements Iterable
#[allow(dead_code)]
pub fn create_unknown_key_diagnostic_from_struct<T: Iterable>(
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

impl Deserializable for Pipeline {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(PipelineVisitor, name, diagnostics)
    }
}

impl Deserializable for RawTaskInput {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(RawTaskInputVisitor, name, diagnostics)
    }
}

impl Deserializable for RawCommand {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(RawCommandVisitor, name, diagnostics)
    }
}

/// Dispatches the three JSON shapes of a task `command`: an argv array, an
/// explicit `null` opt-out, or a per-toolchain map of argv arrays.
struct RawCommandVisitor;

impl DeserializationVisitor for RawCommandVisitor {
    type Output = RawCommand;

    const EXPECTED_TYPE: VisitableType = VisitableType::ARRAY
        .union(VisitableType::NULL)
        .union(VisitableType::MAP);

    fn visit_null(
        self,
        _range: TextRange,
        _name: &str,
        _diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        Some(RawCommand::OptOut)
    }

    fn visit_array(
        self,
        values: impl Iterator<Item = Option<impl DeserializableValue>>,
        _range: TextRange,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        let items: Vec<Spanned<UnescapedString>> = values
            .flatten()
            .filter_map(|value| Spanned::deserialize(&value, name, diagnostics))
            .collect();
        // An empty array is the same opt-out as `null`.
        if items.is_empty() {
            return Some(RawCommand::OptOut);
        }
        Some(RawCommand::Argv(items))
    }

    fn visit_map(
        self,
        members: impl Iterator<Item = Option<(impl DeserializableValue, impl DeserializableValue)>>,
        _range: TextRange,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        let mut entries = Vec::new();
        for (key, value) in members.flatten() {
            let Some(key) = Spanned::<String>::deserialize(&key, name, diagnostics) else {
                continue;
            };
            let Some(argv) =
                Vec::<Spanned<UnescapedString>>::deserialize(&value, key.as_inner(), diagnostics)
            else {
                continue;
            };
            entries.push((key, argv));
        }
        Some(RawCommand::PerToolchain(entries))
    }
}

struct RawTaskInputVisitor;

impl DeserializationVisitor for RawTaskInputVisitor {
    type Output = RawTaskInput;

    const EXPECTED_TYPE: VisitableType = VisitableType::STR.union(VisitableType::MAP);

    fn visit_str(
        self,
        value: biome_deserialize::Text,
        _range: TextRange,
        _name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        match UnescapedString::from_escaped(value.text().to_string()) {
            Ok(value) => Some(RawTaskInput::String(value)),
            Err(error) => {
                diagnostics.push(DeserializationDiagnostic::new(format!("{error}")));
                None
            }
        }
    }

    fn visit_map(
        self,
        members: impl Iterator<Item = Option<(impl DeserializableValue, impl DeserializableValue)>>,
        _range: TextRange,
        _name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        let mut structured = RawStructuredInput {
            mode: None,
            globs: None,
            with_defaults: None,
            from: None,
        };

        for (key, value) in members.flatten() {
            let key = String::deserialize(&key, "", diagnostics)?;
            match key.as_str() {
                "mode" => {
                    structured.mode = Spanned::deserialize(&value, "mode", diagnostics);
                }
                "globs" => {
                    structured.globs =
                        Vec::<Spanned<UnescapedString>>::deserialize(&value, "globs", diagnostics);
                }
                "withDefaults" => {
                    structured.with_defaults =
                        Spanned::<bool>::deserialize(&value, "withDefaults", diagnostics);
                }
                "from" => {
                    structured.from =
                        Vec::<Spanned<UnescapedString>>::deserialize(&value, "from", diagnostics);
                }
                _ => diagnostics.push(DeserializationDiagnostic::new_unknown_key(
                    &key,
                    value.range(),
                    &["mode", "globs", "withDefaults", "from"],
                )),
            }
        }

        Some(RawTaskInput::Structured(structured))
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
            let task_name = TaskName::from(String::from(UnescapedString::deserialize(
                &key,
                "",
                diagnostics,
            )?));
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

impl WithMetadata for RawRemoteCacheOptions {
    fn add_text(&mut self, text: Arc<str>) {
        self.api_url.add_text(text.clone());
        self.login_url.add_text(text.clone());
        self.team_slug.add_text(text.clone());
        self.team_id.add_text(text.clone());
        self.signature.add_text(text.clone());
        self.preflight.add_text(text.clone());
        self.timeout.add_text(text.clone());
        self.enabled.add_text(text.clone());
        self.upload_timeout.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.api_url.add_path(path.clone());
        self.login_url.add_path(path.clone());
        self.team_slug.add_path(path.clone());
        self.team_id.add_path(path.clone());
        self.signature.add_path(path.clone());
        self.preflight.add_path(path.clone());
        self.timeout.add_path(path.clone());
        self.enabled.add_path(path.clone());
        self.upload_timeout.add_path(path);
    }
}

impl WithMetadata for RawObservabilityOtelRunAttributes {
    fn add_text(&mut self, text: Arc<str>) {
        self.id.add_text(text.clone());
        self.scm_revision.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.id.add_path(path.clone());
        self.scm_revision.add_path(path);
    }
}

impl WithMetadata for RawObservabilityOtelTaskAttributes {
    fn add_text(&mut self, text: Arc<str>) {
        self.id.add_text(text.clone());
        self.hashes.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.id.add_path(path.clone());
        self.hashes.add_path(path);
    }
}

impl WithMetadata for RawObservabilityOtelMetrics {
    fn add_text(&mut self, text: Arc<str>) {
        self.run_summary.add_text(text.clone());
        self.task_details.add_text(text.clone());
        self.run_attributes.add_text(text.clone());
        if let Some(attrs) = &mut self.run_attributes {
            attrs.add_text(text.clone());
        }
        self.task_attributes.add_text(text.clone());
        if let Some(attrs) = &mut self.task_attributes {
            attrs.add_text(text);
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.run_summary.add_path(path.clone());
        self.task_details.add_path(path.clone());
        self.run_attributes.add_path(path.clone());
        if let Some(attrs) = &mut self.run_attributes {
            attrs.add_path(path.clone());
        }
        self.task_attributes.add_path(path.clone());
        if let Some(attrs) = &mut self.task_attributes {
            attrs.add_path(path);
        }
    }
}

impl WithMetadata for RawObservabilityOtel {
    fn add_text(&mut self, text: Arc<str>) {
        self.enabled.add_text(text.clone());
        self.protocol.add_text(text.clone());
        self.endpoint.add_text(text.clone());
        self.timeout_ms.add_text(text.clone());
        self.interval_ms.add_text(text.clone());
        self.use_remote_cache_token.add_text(text.clone());
        self.metrics.add_text(text.clone());
        if let Some(metrics) = &mut self.metrics {
            metrics.add_text(text);
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.enabled.add_path(path.clone());
        self.protocol.add_path(path.clone());
        self.endpoint.add_path(path.clone());
        self.timeout_ms.add_path(path.clone());
        self.interval_ms.add_path(path.clone());
        self.use_remote_cache_token.add_path(path.clone());
        self.metrics.add_path(path.clone());
        if let Some(metrics) = &mut self.metrics {
            metrics.add_path(path);
        }
    }
}

impl WithMetadata for RawExperimentalObservability {
    fn add_text(&mut self, text: Arc<str>) {
        self.otel.add_text(text.clone());
        if let Some(otel) = &mut self.otel {
            otel.add_text(text);
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.otel.add_path(path.clone());
        if let Some(otel) = &mut self.otel {
            otel.add_path(path);
        }
    }
}

impl WithMetadata for RawGlobalConfig {
    fn add_text(&mut self, text: Arc<str>) {
        self.inputs.add_text(text.clone());
        self.env.add_text(text.clone());
        self.pass_through_env.add_text(text.clone());
        self.ui.add_text(text.clone());
        self.allow_no_package_manager.add_text(text.clone());
        self.daemon.add_text(text.clone());
        self.env_mode.add_text(text.clone());
        self.cache_dir.add_text(text.clone());
        self.cache_max_age.add_text(text.clone());
        self.cache_max_size.add_text(text.clone());
        self.no_update_notifier.add_text(text.clone());
        self.concurrency.add_text(text.clone());
        self.remote_cache.add_text(text.clone());
        self.experimental_observability.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.inputs.add_path(path.clone());
        self.env.add_path(path.clone());
        self.pass_through_env.add_path(path.clone());
        self.ui.add_path(path.clone());
        self.allow_no_package_manager.add_path(path.clone());
        self.daemon.add_path(path.clone());
        self.env_mode.add_path(path.clone());
        self.cache_dir.add_path(path.clone());
        self.cache_max_age.add_path(path.clone());
        self.cache_max_size.add_path(path.clone());
        self.no_update_notifier.add_path(path.clone());
        self.concurrency.add_path(path.clone());
        self.remote_cache.add_path(path.clone());
        self.experimental_observability.add_path(path);
    }
}

impl WithMetadata for RawRootTurboJson {
    fn add_text(&mut self, text: Arc<str>) {
        self.span.add_text(text.clone());
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
        self.cache_max_age.add_text(text.clone());
        self.cache_max_size.add_text(text.clone());
        self.pipeline.add_text(text.clone());
        self.remote_cache.add_text(text.clone());
        self.ui.add_text(text.clone());
        self.allow_no_package_manager.add_text(text.clone());
        self.daemon.add_text(text.clone());
        self.env_mode.add_text(text.clone());
        self.no_update_notifier.add_text(text.clone());
        self.concurrency.add_text(text.clone());
        self.future_flags.add_text(text.clone());
        self.global.add_text(text.clone());
        if let Some(global) = &mut self.global {
            global.value.add_text(text);
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.span.add_path(path.clone());
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
        self.cache_max_age.add_path(path.clone());
        self.cache_max_size.add_path(path.clone());
        self.pipeline.add_path(path.clone());
        self.remote_cache.add_path(path.clone());
        self.ui.add_path(path.clone());
        self.allow_no_package_manager.add_path(path.clone());
        self.daemon.add_path(path.clone());
        self.env_mode.add_path(path.clone());
        self.no_update_notifier.add_path(path.clone());
        self.concurrency.add_path(path.clone());
        self.future_flags.add_path(path.clone());
        self.global.add_path(path.clone());
        if let Some(global) = &mut self.global {
            global.value.add_path(path);
        }
    }
}

impl WithMetadata for RawPackageTurboJson {
    fn add_text(&mut self, text: Arc<str>) {
        self.span.add_text(text.clone());
        self.extends.add_text(text.clone());
        self.tags.add_text(text.clone());
        if let Some(tags) = &mut self.tags {
            tags.value.add_text(text.clone());
        }
        self.boundaries.add_text(text.clone());
        if let Some(boundaries) = &mut self.boundaries {
            boundaries.value.add_text(text.clone());
        }
        self.tasks.add_text(text.clone());
        self.pipeline.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.span.add_path(path.clone());
        self.extends.add_path(path.clone());
        self.tags.add_path(path.clone());
        if let Some(tags) = &mut self.tags {
            tags.value.add_path(path.clone());
        }
        self.boundaries.add_path(path.clone());
        if let Some(boundaries) = &mut self.boundaries {
            boundaries.value.add_path(path.clone());
        }
        self.tasks.add_path(path.clone());
        self.pipeline.add_path(path);
    }
}

impl RawRootTurboJson {
    pub fn parse(text: &str, file_path: &str) -> Result<Self, BiomeParseError> {
        parse_turbo_json::<RawRootTurboJson>(text, file_path)
    }
}

impl RawPackageTurboJson {
    pub fn parse(text: &str, file_path: &str) -> Result<Self, BiomeParseError> {
        parse_turbo_json::<RawPackageTurboJson>(text, file_path)
    }
}

impl RawTurboJson {
    /// Parse RawTurboJson from a serde_json::Value
    ///
    /// This is a convenience helper for constructing RawTurboJson from
    /// serde_json::json! macro in tests.
    pub fn parse_from_serde(value: serde_json::Value) -> Result<RawTurboJson, crate::error::Error> {
        let json_string = serde_json::to_string(&value)?;
        let raw_root = RawRootTurboJson::parse(&json_string, "turbo.json")?;
        raw_root.try_into()
    }
}

/// Generic function to parse turbo.json content using biome parser
///
/// This function handles the common logic for parsing turbo.json files:
/// - Deserializes JSON with comments and trailing commas allowed
/// - Collects and converts diagnostics to ParseDiagnostic
/// - Adds source text and path metadata to the result
pub fn parse_turbo_json<T: Deserializable + WithMetadata>(
    text: &str,
    file_path: &str,
) -> Result<T, BiomeParseError> {
    let (deserialized, errors) = turborepo_errors::json::deserialize_from_json_str::<T>(
        text,
        JsonParserOptions::default()
            .with_allow_comments()
            .with_allow_trailing_commas(),
        file_path,
    );

    if !errors.is_empty() {
        let diagnostics = errors
            .into_iter()
            .map(|d| {
                d.with_file_source_code(text)
                    .with_file_path(file_path)
                    .as_ref()
                    .into()
            })
            .collect();

        return Err(BiomeParseError::new(diagnostics));
    }

    let mut turbo_json = deserialized.ok_or_else(BiomeParseError::empty)?;
    turbo_json.add_text(Arc::from(text));
    turbo_json.add_path(Arc::from(file_path));

    Ok(turbo_json)
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use test_case::test_case;

    use super::*;

    #[test]
    fn test_biome_parse_error_new() {
        let err = BiomeParseError::new(vec![]);
        assert!(err.diagnostics.is_empty());
    }

    #[test]
    fn test_biome_parse_error_empty() {
        let err = BiomeParseError::empty();
        assert!(err.diagnostics.is_empty());
    }

    #[test]
    fn test_biome_parse_error_display() {
        let err = BiomeParseError::empty();
        assert_eq!(err.to_string(), "Failed to parse turbo.json.");
    }

    // Regression tests for https://github.com/vercel/turborepo/issues/13197
    // Unterminated string literals used to panic inside biome during
    // deserialization instead of producing a parse error.
    #[test_case("{\"tasks\": {\"build\": {\"persistent\": \"\n}}}"; "quote before newline")]
    #[test_case("{\"tasks\": {\"build\": {\"dependsOn\": [\""; "quote at eof")]
    fn test_unterminated_string_reports_parse_error(json: &str) {
        assert!(RawRootTurboJson::parse(json, "turbo.json").is_err());
    }

    #[test_case(r#"{"daemon": true}"#; "daemon in package turbo.json")]
    fn test_root_only_fields_in_package_turbo_json(json: &str) {
        let result = RawPackageTurboJson::parse(json, "packages/foo/turbo.json");
        assert!(result.is_err());

        let report = miette::Report::new(result.unwrap_err());
        let mut msg = String::new();
        miette::NarratableReportHandler::new()
            .render_report(&mut msg, report.as_ref())
            .unwrap();
        assert_snapshot!(msg);
    }

    #[test]
    fn test_command_parses_three_shapes() {
        let json = r#"{"tasks": {
            "a": {"command": ["cargo", "nextest", "run"]},
            "b": {"command": null},
            "c": {"command": []},
            "d": {"command": {"rust": ["cargo", "test"], "javascript": ["vitest"]}}
        }}"#;
        let parsed = RawRootTurboJson::parse(json, "turbo.json").unwrap();
        let tasks = parsed.tasks.unwrap();
        let command = |name: &str| {
            tasks.0[&TaskName::from(name.to_string())]
                .command
                .clone()
                .unwrap()
                .into_inner()
        };

        let RawCommand::Argv(argv) = command("a") else {
            panic!("expected argv");
        };
        assert_eq!(
            argv.iter().map(|a| a.as_str()).collect::<Vec<_>>(),
            vec!["cargo", "nextest", "run"]
        );
        // `null` and `[]` are the same explicit opt-out; absent is absent.
        assert_eq!(command("b"), RawCommand::OptOut);
        assert_eq!(command("c"), RawCommand::OptOut);
        let RawCommand::PerToolchain(entries) = command("d") else {
            panic!("expected map");
        };
        // Entries keep source order.
        assert_eq!(
            entries
                .iter()
                .map(|(key, _)| key.as_inner().as_str())
                .collect::<Vec<_>>(),
            vec!["rust", "javascript"]
        );
    }

    #[test]
    fn test_command_rejects_other_shapes() {
        for json in [
            r#"{"tasks": {"a": {"command": "cargo test"}}}"#,
            r#"{"tasks": {"a": {"command": true}}}"#,
            r#"{"tasks": {"a": {"command": 42}}}"#,
        ] {
            assert!(
                RawRootTurboJson::parse(json, "turbo.json").is_err(),
                "should reject: {json}"
            );
        }
    }

    #[test]
    fn test_no_update_notifier_parsed_from_root_turbo_json() {
        let json = r#"{"noUpdateNotifier": true}"#;
        let result = RawRootTurboJson::parse(json, "turbo.json").unwrap();
        assert_eq!(
            result.no_update_notifier.as_ref().map(|v| *v.as_inner()),
            Some(true),
            "noUpdateNotifier should be parsed from root turbo.json"
        );
    }

    #[test]
    fn test_no_update_notifier_parsed_from_full_turbo_json() {
        let json = r#"{
          "$schema": "https://turborepo.dev/schema.json",
          "noUpdateNotifier": true,
          "tasks": {
            "build": {
              "dependsOn": ["prebuild", "^build"],
              "outputs": ["output-file.txt", "dist/**"]
            },
            "prebuild": {},
            "lint": {},
            "check-types": {}
          }
        }"#;
        let result = RawRootTurboJson::parse(json, "turbo.json").unwrap();
        assert_eq!(
            result.no_update_notifier.as_ref().map(|v| *v.as_inner()),
            Some(true),
            "noUpdateNotifier should be parsed from a full turbo.json"
        );
    }

    #[test]
    fn test_unknown_key_in_task_definition() {
        // Task definitions should reject unknown keys
        let json = r#"{"tasks": {"build": {"lol": true}}}"#;
        let result = RawPackageTurboJson::parse(json, "packages/foo/turbo.json");
        assert!(
            result.is_err(),
            "expected parsing to fail due to unknown key 'lol' in task definition, but got: {:?}",
            result
        );
    }

    #[test]
    fn test_structured_task_inputs_accept_mixed_entries() {
        let json = r#"{
          "tasks": {
            "build": {
              "inputs": [
                "$TURBO_DEFAULT$",
                "!src/generated/**",
                {
                  "mode": "jit",
                  "globs": ["src/generated/**"],
                  "withDefaults": true
                },
                {
                  "mode": "dependencyOutputs",
                  "from": ["codegen"],
                  "globs": ["dist/**", "!dist/**/*.map"]
                }
              ]
            }
          }
        }"#;

        let result = RawRootTurboJson::parse(json, "turbo.json");

        assert!(result.is_ok(), "structured task inputs should parse");
    }

    #[test]
    fn test_experimental_ci_accepts_boolean() {
        let json = r#"{"tasks": {"build": {"experimentalCI": true}}}"#;
        let result = RawRootTurboJson::parse(json, "turbo.json").unwrap();
        let task = result
            .tasks
            .as_ref()
            .unwrap()
            .get(&TaskName::from("build"))
            .unwrap();

        assert_eq!(
            task.experimental_ci.as_ref().map(|v| v.as_inner()),
            Some(&turborepo_types::ExperimentalCIConfig::Enabled(true))
        );
    }

    #[test]
    fn test_experimental_ci_accepts_object_with_arbitrary_keys() {
        let json = r#"{
            "tasks": {
                "build": {
                    "experimentalCI": {
                        "provider": "github",
                        "enabled": true,
                        "attempts": 3,
                        "nested": { "key": ["value"] }
                    }
                }
            }
        }"#;
        let result = RawRootTurboJson::parse(json, "turbo.json").unwrap();
        let task = result
            .tasks
            .as_ref()
            .unwrap()
            .get(&TaskName::from("build"))
            .unwrap();

        assert_eq!(
            task.experimental_ci.as_ref().map(|v| v.as_inner()),
            Some(&turborepo_types::ExperimentalCIConfig::Options(
                serde_json::json!({
                    "provider": "github",
                    "enabled": true,
                    "attempts": 3,
                    "nested": { "key": ["value"] }
                })
                .as_object()
                .unwrap()
                .clone()
            ))
        );
    }
}
