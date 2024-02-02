use std::{
    backtrace,
    collections::BTreeMap,
    fmt::{Debug, Display},
    sync::Arc,
};

use biome_deserialize::{
    json::deserialize_from_json_str, Deserializable, DeserializableValue,
    DeserializationDiagnostic, DeserializationVisitor, Text, VisitableType,
};
use biome_diagnostics::DiagnosticExt;
use biome_json_parser::JsonParserOptions;
use biome_json_syntax::TextRange;
use clap::ValueEnum;
use convert_case::{Case, Casing};
use miette::{Diagnostic, SourceSpan};
use struct_iterable::Iterable;
use thiserror::Error;
use turbopath::AnchoredSystemPath;
use turborepo_errors::WithMetadata;

use crate::{
    cli::OutputLogsMode,
    config::ConfigurationOptions,
    run::task_id::TaskName,
    turbo_json::{Pipeline, RawTaskDefinition, RawTurboJson, SpacesJson, Spanned},
    unescape::UnescapedString,
};

#[derive(Debug, Error, Diagnostic)]
#[error("failed to parse turbo json")]
#[diagnostic(code(turbo_json_parse_error))]
pub struct Error {
    #[related]
    diagnostics: Vec<ParseDiagnostic>,
    #[backtrace]
    backtrace: backtrace::Backtrace,
}

struct BiomeMessage<'a>(&'a biome_diagnostics::Error);

impl Display for BiomeMessage<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.description(f)
    }
}

impl From<biome_diagnostics::Error> for ParseDiagnostic {
    fn from(diagnostic: biome_diagnostics::Error) -> Self {
        let location = diagnostic.location();
        let message = BiomeMessage(&diagnostic).to_string();
        Self {
            message,
            source_code: location
                .source_code
                .map(|s| s.text.to_string())
                .unwrap_or_default(),
            label: location.span.map(|span| {
                let start: usize = span.start().into();
                let len: usize = span.len().into();
                (start, len).into()
            }),
        }
    }
}

#[derive(Debug, Error, Diagnostic)]
#[error("{message}")]
#[diagnostic(code(turbo_json_parse_error))]
struct ParseDiagnostic {
    message: String,
    #[source_code]
    source_code: String,
    #[label]
    label: Option<SourceSpan>,
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

impl Deserializable for OutputLogsMode {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let output_logs_str = String::deserialize(value, name, diagnostics)?;
        match OutputLogsMode::from_str(&output_logs_str, false) {
            Ok(result) => Some(result),
            Err(_) => {
                let allowed_variants: Vec<_> = OutputLogsMode::value_variants()
                    .iter()
                    .map(|s| serde_json::to_string(s).unwrap())
                    .collect();

                let allowed_variants_borrowed = allowed_variants
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>();

                diagnostics.push(DeserializationDiagnostic::new_unknown_value(
                    &output_logs_str,
                    value.range(),
                    &allowed_variants_borrowed,
                ));
                None
            }
        }
    }
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

impl Deserializable for RawTaskDefinition {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(RawTaskDefinitionVisitor, name, diagnostics)
    }
}

struct RawTaskDefinitionVisitor;

impl DeserializationVisitor for RawTaskDefinitionVisitor {
    type Output = RawTaskDefinition;

    const EXPECTED_TYPE: VisitableType = VisitableType::MAP;

    fn visit_map(
        self,
        // Iterator of key-value pairs.
        members: impl Iterator<Item = Option<(impl DeserializableValue, impl DeserializableValue)>>,
        // range of the map in the source text.
        _: TextRange,
        _: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        let mut result = RawTaskDefinition::default();
        for (key, value) in members.flatten() {
            let Some(key_text) = Text::deserialize(&key, "", diagnostics) else {
                continue;
            };
            let range = value.range();
            match key_text.text() {
                "cache" => {
                    if let Some(cache) = bool::deserialize(&value, &key_text, diagnostics) {
                        result.cache = Spanned::new(Some(cache)).with_range(range);
                    }
                }
                "dependsOn" => {
                    if let Some(depends_on) = Vec::deserialize(&value, &key_text, diagnostics) {
                        result.depends_on = Some(Spanned::new(depends_on).with_range(range));
                    }
                }
                "dotEnv" => {
                    if let Some(dot_env) = Vec::deserialize(&value, &key_text, diagnostics) {
                        result.dot_env = Some(Spanned::new(dot_env).with_range(range));
                    }
                }
                "env" => {
                    if let Some(env) = Vec::deserialize(&value, &key_text, diagnostics) {
                        result.env = Some(env);
                    }
                }
                "inputs" => {
                    if let Some(inputs) = Vec::deserialize(&value, &key_text, diagnostics) {
                        result.inputs = Some(Spanned::new(inputs).with_range(range));
                    }
                }
                "passThroughEnv" => {
                    if let Some(pass_through_env) = Vec::deserialize(&value, &key_text, diagnostics)
                    {
                        result.pass_through_env = Some(pass_through_env);
                    }
                }
                "persistent" => {
                    if let Some(persistent) = bool::deserialize(&value, &key_text, diagnostics) {
                        result.persistent = Some(Spanned::new(persistent).with_range(range));
                    }
                }
                "outputs" => {
                    if let Some(outputs) = Vec::deserialize(&value, &key_text, diagnostics) {
                        result.outputs = Some(Spanned::new(outputs).with_range(range));
                    }
                }
                "outputMode" => {
                    if let Some(output_mode) =
                        OutputLogsMode::deserialize(&value, &key_text, diagnostics)
                    {
                        result.output_mode = Some(Spanned::new(output_mode).with_range(range));
                    }
                }
                unknown_key => {
                    diagnostics.push(create_unknown_key_diagnostic_from_struct(
                        &result,
                        unknown_key,
                        key.range(),
                    ));
                }
            }
        }

        Some(result)
    }
}

impl Deserializable for SpacesJson {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(SpacesJsonVisitor, name, diagnostics)
    }
}

struct SpacesJsonVisitor;

impl DeserializationVisitor for SpacesJsonVisitor {
    type Output = SpacesJson;

    const EXPECTED_TYPE: VisitableType = VisitableType::MAP;

    fn visit_map(
        self,
        members: impl Iterator<Item = Option<(impl DeserializableValue, impl DeserializableValue)>>,
        _range: TextRange,
        _name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        let mut result = SpacesJson::default();
        for (key, value) in members.flatten() {
            let Some(key_text) = Text::deserialize(&key, "", diagnostics) else {
                continue;
            };
            // We explicitly do not error on unknown keys here,
            // because this is the existing serde behavior
            if key_text.text() == "id" {
                if let Some(id) = UnescapedString::deserialize(&value, &key_text, diagnostics) {
                    result.id = Some(id);
                }
            }
        }
        Some(result)
    }
}

impl Deserializable for ConfigurationOptions {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(ConfigurationOptionsVisitor, name, diagnostics)
    }
}

struct ConfigurationOptionsVisitor;

impl DeserializationVisitor for ConfigurationOptionsVisitor {
    type Output = ConfigurationOptions;

    const EXPECTED_TYPE: VisitableType = VisitableType::MAP;

    fn visit_map(
        self,
        // Iterator of key-value pairs.
        members: impl Iterator<Item = Option<(impl DeserializableValue, impl DeserializableValue)>>,
        // range of the map in the source text.
        _: TextRange,
        _name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        let mut result = ConfigurationOptions::default();
        for (key, value) in members.flatten() {
            // Try to deserialize the key as a string.
            // We use `Text` to avoid an heap-allocation.
            let Some(key_text) = Text::deserialize(&key, "", diagnostics) else {
                // If this failed, then pass to the next key-value pair.
                continue;
            };
            match key_text.text() {
                "apiUrl" | "apiurl" | "ApiUrl" | "APIURL" => {
                    if let Some(api_url) =
                        UnescapedString::deserialize(&value, &key_text, diagnostics)
                    {
                        result.api_url = Some(api_url.into());
                    }
                }
                "loginUrl" | "loginurl" | "LoginUrl" | "LOGINURL" => {
                    if let Some(login_url) =
                        UnescapedString::deserialize(&value, &key_text, diagnostics)
                    {
                        result.login_url = Some(login_url.into());
                    }
                }
                "teamSlug" | "teamslug" | "TeamSlug" | "TEAMSLUG" => {
                    if let Some(team_slug) =
                        UnescapedString::deserialize(&value, &key_text, diagnostics)
                    {
                        result.team_slug = Some(team_slug.into());
                    }
                }
                "teamId" | "teamid" | "TeamId" | "TEAMID" => {
                    if let Some(team_id) =
                        UnescapedString::deserialize(&value, &key_text, diagnostics)
                    {
                        result.team_id = Some(team_id.into());
                    }
                }
                "token" => {
                    if let Some(token) =
                        UnescapedString::deserialize(&value, &key_text, diagnostics)
                    {
                        result.token = Some(token.into());
                    }
                }
                "signature" => {
                    if let Some(signature) = bool::deserialize(&value, &key_text, diagnostics) {
                        result.signature = Some(signature);
                    }
                }
                "preflight" => {
                    if let Some(preflight) = bool::deserialize(&value, &key_text, diagnostics) {
                        result.preflight = Some(preflight);
                    }
                }
                "timeout" => {
                    if let Some(timeout) = u64::deserialize(&value, &key_text, diagnostics) {
                        result.timeout = Some(timeout);
                    }
                }
                "enabled" => {
                    if let Some(enabled) = bool::deserialize(&value, &key_text, diagnostics) {
                        result.enabled = Some(enabled);
                    }
                }
                unknown_key => diagnostics.push(create_unknown_key_diagnostic_from_struct(
                    &result,
                    unknown_key,
                    key.range(),
                )),
            }
        }

        Some(result)
    }
}

impl Deserializable for RawTurboJson {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(RawTurboJsonVisitor, name, diagnostics)
    }
}

struct RawTurboJsonVisitor;

impl DeserializationVisitor for RawTurboJsonVisitor {
    type Output = RawTurboJson;

    const EXPECTED_TYPE: VisitableType = VisitableType::MAP;

    fn visit_map(
        self,
        // Iterator of key-value pairs.
        members: impl Iterator<Item = Option<(impl DeserializableValue, impl DeserializableValue)>>,
        // range of the map in the source text.
        _range: TextRange,
        _name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        let mut result = RawTurboJson::default();
        for (key, value) in members.flatten() {
            // Try to deserialize the key as a string.
            // We use `Text` to avoid an heap-allocation.
            let Some(key_text) = Text::deserialize(&key, "", diagnostics) else {
                // If this failed, then pass to the next key-value pair.
                continue;
            };
            let range = value.range();
            match key_text.text() {
                "$schema" => {
                    if let Some(name) = UnescapedString::deserialize(&value, &key_text, diagnostics)
                    {
                        result.schema = Some(name);
                    }
                }
                "extends" => {
                    if let Some(extends) = Vec::deserialize(&value, &key_text, diagnostics) {
                        result.extends = Some(Spanned::new(extends).with_range(range));
                    }
                }
                "globalDependencies" => {
                    if let Some(global_dependencies) =
                        Vec::deserialize(&value, &key_text, diagnostics)
                    {
                        result.global_dependencies =
                            Some(Spanned::new(global_dependencies).with_range(range));
                    }
                }
                "globalEnv" => {
                    if let Some(global_env) = Vec::deserialize(&value, &key_text, diagnostics) {
                        result.global_env = Some(global_env);
                    }
                }
                "globalPassThroughEnv" => {
                    if let Some(global_pass_through_env) =
                        Vec::deserialize(&value, &key_text, diagnostics)
                    {
                        result.global_pass_through_env = Some(global_pass_through_env);
                    }
                }
                "globalDotEnv" => {
                    if let Some(global_dot_env) = Vec::deserialize(&value, &key_text, diagnostics) {
                        result.global_dot_env = Some(global_dot_env);
                    }
                }
                "experimentalSpaces" => {
                    if let Some(spaces) = SpacesJson::deserialize(&value, &key_text, diagnostics) {
                        result.experimental_spaces = Some(spaces);
                    }
                }
                "pipeline" => {
                    if let Some(pipeline) = Pipeline::deserialize(&value, &key_text, diagnostics) {
                        result.pipeline = Some(pipeline);
                    }
                }
                "remoteCache" => {
                    if let Some(remote_cache) =
                        ConfigurationOptions::deserialize(&value, &key_text, diagnostics)
                    {
                        result.remote_cache = Some(remote_cache);
                    }
                }
                unknown_key => {
                    diagnostics.push(create_unknown_key_diagnostic_from_struct(
                        &result,
                        unknown_key,
                        key.range(),
                    ));
                }
            }
        }
        Some(result)
    }
}

impl WithMetadata for RawTurboJson {
    fn add_text(&mut self, text: Arc<str>) {
        self.text = Some(text.clone());
        self.extends.add_text(text.clone());
        self.global_dependencies.add_text(text.clone());
        self.global_env.add_text(text.clone());
        self.global_pass_through_env.add_text(text.clone());
        self.pipeline.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.path = Some(path.clone());
        self.extends.add_path(path.clone());
        self.global_dependencies.add_path(path.clone());
        self.global_env.add_path(path.clone());
        self.global_pass_through_env.add_path(path.clone());
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

impl WithMetadata for RawTaskDefinition {
    fn add_text(&mut self, text: Arc<str>) {
        self.depends_on.add_text(text.clone());
        self.dot_env.add_text(text.clone());
        self.env.add_text(text.clone());
        self.inputs.add_text(text.clone());
        self.pass_through_env.add_text(text.clone());
        self.persistent.add_text(text.clone());
        self.outputs.add_text(text.clone());
        self.output_mode.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.depends_on.add_path(path.clone());
        self.dot_env.add_path(path.clone());
        self.env.add_path(path.clone());
        self.inputs.add_path(path.clone());
        self.pass_through_env.add_path(path.clone());
        self.persistent.add_path(path.clone());
        self.outputs.add_path(path.clone());
        self.output_mode.add_path(path);
    }
}

impl RawTurboJson {
    // A simple helper for tests
    #[cfg(test)]
    pub fn parse_from_serde(value: serde_json::Value) -> Result<RawTurboJson, Error> {
        let json_string = serde_json::to_string(&value).expect("should be able to serialize");
        Self::parse(&json_string, AnchoredSystemPath::new("turbo.json").unwrap())
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
    pub fn parse(text: &str, file_path: &AnchoredSystemPath) -> Result<RawTurboJson, Error> {
        let result = deserialize_from_json_str::<RawTurboJson>(
            text,
            JsonParserOptions::default().with_allow_comments(),
        );

        if !result.diagnostics().is_empty() {
            let diagnostics = result
                .into_diagnostics()
                .into_iter()
                .map(|d| {
                    d.with_file_source_code(text)
                        .with_file_path(file_path.as_str())
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

        turbo_json.add_text(Arc::from(text));
        turbo_json.add_path(Arc::from(file_path.as_str()));

        Ok(turbo_json)
    }
}
