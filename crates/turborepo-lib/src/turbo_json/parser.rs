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
use turborepo_task_id::TaskName;
use turborepo_unescape::UnescapedString;

use crate::{
    boundaries::{BoundariesConfig, Permissions, Rule},
    turbo_json::{
        Pipeline, RawPackageTurboJson, RawRemoteCacheOptions, RawRootTurboJson, RawTaskDefinition,
        RawTurboJson, Spanned,
    },
};

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
        self.pipeline.add_text(text.clone());
        self.remote_cache.add_text(text.clone());
        self.ui.add_text(text.clone());
        self.allow_no_package_manager.add_text(text.clone());
        self.daemon.add_text(text.clone());
        self.env_mode.add_text(text.clone());
        self.no_update_notifier.add_text(text.clone());
        self.concurrency.add_text(text.clone());
        self.future_flags.add_text(text);
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
        self.pipeline.add_path(path.clone());
        self.remote_cache.add_path(path.clone());
        self.ui.add_path(path.clone());
        self.allow_no_package_manager.add_path(path.clone());
        self.daemon.add_path(path.clone());
        self.env_mode.add_path(path.clone());
        self.no_update_notifier.add_path(path.clone());
        self.concurrency.add_path(path.clone());
        self.future_flags.add_path(path);
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
    pub fn parse(text: &str, file_path: &str) -> Result<Self, Error> {
        let turbo_json = parse_turbo_json::<RawRootTurboJson>(text, file_path)?;

        if turbo_json.experimental_spaces.is_some() {
            warn!("`experimentalSpaces` key in turbo.json is deprecated and does not do anything")
        }

        Ok(turbo_json)
    }
}

impl RawPackageTurboJson {
    pub fn parse(text: &str, file_path: &str) -> Result<Self, Error> {
        parse_turbo_json::<RawPackageTurboJson>(text, file_path)
    }
}

impl RawTurboJson {
    // A simple helper for tests
    #[cfg(test)]
    pub fn parse_from_serde(value: serde_json::Value) -> Result<RawTurboJson, Error> {
        let json_string = serde_json::to_string(&value).expect("should be able to serialize");
        let raw_root = RawRootTurboJson::parse(&json_string, "turbo.json")?;
        Ok(Self::from(raw_root))
    }
}

fn parse_turbo_json<T: Deserializable + WithMetadata>(
    text: &str,
    file_path: &str,
) -> Result<T, Error> {
    let result = deserialize_from_json_str::<T>(
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

    let mut turbo_json = result.into_deserialized().ok_or_else(|| Error {
        diagnostics: vec![],
        backtrace: backtrace::Backtrace::capture(),
    })?;
    turbo_json.add_text(Arc::from(text));
    turbo_json.add_path(Arc::from(file_path));

    Ok(turbo_json)
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use test_case::test_case;

    use super::*;

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
}
