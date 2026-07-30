//! Processed task definition types with DSL token handling
//! Processed turbo.json types
//!
//! This module contains types that represent processed/resolved turbo.json
//! configuration after validation and normalization.

use camino::Utf8Path;
use turbopath::RelativeUnixPath;
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;
use turborepo_types::{EnvMode, ExperimentalCIConfig, OutputLogsMode};
use turborepo_unescape::UnescapedString;

use crate::{
    error::Error,
    future_flags::FutureFlags,
    raw::{RawCommand, RawStructuredInput, RawTaskDefinition, RawTaskInput},
};

const TURBO_DEFAULT: &str = "$TURBO_DEFAULT$";
const TURBO_ROOT: &str = "$TURBO_ROOT$";
const TURBO_ROOT_SLASH: &str = "$TURBO_ROOT$/";
const TURBO_EXTENDS: &str = "$TURBO_EXTENDS$";
const ENV_PIPELINE_DELIMITER: &str = "$";
const TOPOLOGICAL_PIPELINE_DELIMITER: &str = "^";

/// Helper function to check for and remove $TURBO_EXTENDS$ from an array
/// Returns (processed_array, extends_found)
fn extract_turbo_extends(
    mut items: Vec<Spanned<UnescapedString>>,
    _future_flags: &FutureFlags,
) -> (Vec<Spanned<UnescapedString>>, bool) {
    if let Some(pos) = items.iter().position(|item| item.as_str() == TURBO_EXTENDS) {
        items.remove(pos);
        (items, true)
    } else {
        (items, false)
    }
}

fn extract_turbo_extends_inputs(
    mut items: Vec<Spanned<RawTaskInput>>,
    _future_flags: &FutureFlags,
) -> (Vec<Spanned<RawTaskInput>>, bool) {
    if let Some(pos) = items.iter().position(|item| match item.as_inner() {
        RawTaskInput::String(value) => value.as_str() == TURBO_EXTENDS,
        RawTaskInput::Structured(_) => false,
    }) {
        items.remove(pos);
        (items, true)
    } else {
        (items, false)
    }
}

/// A processed glob with separated components
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedGlob {
    /// The glob pattern without $TURBO_ROOT$ prefix
    glob: String,
    /// Whether the glob was negated (started with !)
    negated: bool,
    /// Whether the glob needs turbo_root prefix (had $TURBO_ROOT$/)
    turbo_root: bool,
}

impl ProcessedGlob {
    /// Creates a ProcessedGlob from a raw glob string, stripping prefixes
    fn from_spanned_internal(
        value: Spanned<UnescapedString>,
        field: &'static str,
    ) -> Result<Self, Error> {
        let mut negated = false;
        let mut turbo_root = false;

        let without_negation = if let Some(value) = value.strip_prefix('!') {
            negated = true;
            value
        } else {
            value.as_str()
        };

        let glob = if let Some(stripped) = without_negation.strip_prefix(TURBO_ROOT_SLASH) {
            turbo_root = true;
            stripped
        } else if without_negation.starts_with(TURBO_ROOT) {
            // Leading $TURBO_ROOT$ without slash
            let (span, text) = value.span_and_text("turbo.json");
            return Err(Error::InvalidTurboRootNeedsSlash { span, text });
        } else if without_negation.contains(TURBO_ROOT) {
            // non leading $TURBO_ROOT$
            let (span, text) = value.span_and_text("turbo.json");
            return Err(Error::InvalidTurboRootUse { span, text });
        } else {
            without_negation
        };

        // Check for absolute paths (after stripping prefixes)
        if Utf8Path::new(glob).is_absolute() {
            let (span, text) = value.span_and_text("turbo.json");
            return Err(Error::AbsolutePathInConfig { field, span, text });
        }

        Ok(ProcessedGlob {
            glob: glob.to_owned(),
            negated,
            turbo_root,
        })
    }

    /// Creates a ProcessedGlob for outputs (validates as output field)
    pub fn from_spanned_output(value: Spanned<UnescapedString>) -> Result<Self, Error> {
        Self::from_spanned_internal(value, "outputs")
    }

    /// Creates a ProcessedGlob for inputs (validates as input field)
    pub fn from_spanned_input(value: Spanned<UnescapedString>) -> Result<Self, Error> {
        Self::from_spanned_internal(value, "inputs")
    }

    /// Creates a resolved glob string with the actual path
    pub fn resolve(&self, turbo_root_path: &RelativeUnixPath) -> String {
        let prefix = if self.negated { "!" } else { "" };

        let glob = &self.glob;
        if self.turbo_root {
            format!("{prefix}{turbo_root_path}/{glob}")
        } else {
            format!("{prefix}{glob}")
        }
    }
}

/// Processed depends_on field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedDependsOn {
    pub deps: Vec<Spanned<UnescapedString>>,
    pub extends: bool,
}

impl ProcessedDependsOn {
    /// Creates a ProcessedDependsOn, validating that dependencies don't use env
    /// prefix and handling TURBO_EXTENDS if enabled
    pub fn new(
        raw_deps: Spanned<Vec<Spanned<UnescapedString>>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_deps, extends) = extract_turbo_extends(raw_deps.into_inner(), future_flags);

        // Validate that no dependency starts with ENV_PIPELINE_DELIMITER ($)
        for dep in processed_deps.iter() {
            if dep.starts_with(ENV_PIPELINE_DELIMITER) {
                let (span, text) = dep.span_and_text("turbo.json");
                return Err(Error::InvalidDependsOnValue {
                    field: "dependsOn",
                    span,
                    text,
                });
            }
        }
        Ok(ProcessedDependsOn {
            deps: processed_deps,
            extends,
        })
    }
}

/// Processed env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedEnv {
    pub vars: Vec<String>,
    pub extends: bool,
}

impl ProcessedEnv {
    /// Creates a ProcessedEnv, validating that env vars don't use invalid
    /// prefixes and handling TURBO_EXTENDS if enabled
    pub fn new(
        raw_env: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_env, extends) = extract_turbo_extends(raw_env, future_flags);

        Ok(ProcessedEnv {
            vars: extract_env_vars(processed_env, "env")?,
            extends,
        })
    }
}

/// Processed inputs field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedInputs {
    pub globs: Vec<ProcessedGlob>,
    pub default: bool,
    pub jit_globs: Vec<ProcessedGlob>,
    pub jit_default: bool,
    pub dependency_outputs: Option<ProcessedDependencyOutputsInput>,
    pub legacy_startup: bool,
    pub structured_startup: bool,
    pub structured_jit: bool,
    pub structured_dependency_outputs: bool,
    pub extends: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedDependencyOutputsInput {
    pub from: Option<Vec<Spanned<UnescapedString>>>,
    pub globs: Vec<ProcessedGlob>,
}

impl ProcessedInputs {
    pub fn new_legacy(
        raw_globs: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        Self::new(
            raw_globs
                .into_iter()
                .map(|glob| glob.map(RawTaskInput::String))
                .collect(),
            future_flags,
        )
    }

    pub fn new(
        raw_inputs: Vec<Spanned<RawTaskInput>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_inputs, extends) = extract_turbo_extends_inputs(raw_inputs, future_flags);

        let mut globs = Vec::with_capacity(processed_inputs.len());
        let mut jit_globs = Vec::new();
        let mut default = false;
        let mut jit_default = false;
        let mut dependency_outputs = None;
        let mut legacy_startup = false;
        let mut structured_startup = false;
        let mut structured_jit = false;
        let mut structured_dependency_outputs = false;

        for raw_input in processed_inputs {
            let span = raw_input.to(());
            match raw_input.into_inner() {
                RawTaskInput::String(raw_glob) => {
                    legacy_startup = true;
                    if raw_glob.as_str() == TURBO_DEFAULT {
                        default = true;
                        continue;
                    }
                    globs.push(ProcessedGlob::from_spanned_input(Spanned::new(raw_glob))?);
                }
                RawTaskInput::Structured(input) => match structured_input_mode(&input, &span)? {
                    StructuredInputMode::Startup => {
                        if structured_startup {
                            return Err(structured_input_error(
                                &span,
                                "duplicate structured \"startup\" input mode".to_string(),
                            ));
                        }
                        structured_startup = true;
                        reject_from(&input, &span)?;
                        default = input.with_defaults.as_ref().is_some_and(|value| **value);
                        globs = structured_globs(input.globs, &span)?;
                        reject_negative_only_globs("startup", default, &globs, &span)?;
                    }
                    StructuredInputMode::Jit => {
                        if structured_jit {
                            return Err(structured_input_error(
                                &span,
                                "duplicate structured \"jit\" input mode".to_string(),
                            ));
                        }
                        structured_jit = true;
                        reject_from(&input, &span)?;
                        jit_default = input.with_defaults.as_ref().is_some_and(|value| **value);
                        jit_globs = structured_globs(input.globs, &span)?;
                        reject_negative_only_globs("jit", jit_default, &jit_globs, &span)?;
                    }
                    StructuredInputMode::DependencyOutputs => {
                        if structured_dependency_outputs {
                            return Err(structured_input_error(
                                &span,
                                "duplicate structured \"dependencyOutputs\" input mode".to_string(),
                            ));
                        }
                        structured_dependency_outputs = true;
                        if input.with_defaults.is_some() {
                            return Err(structured_input_error(
                                &span,
                                "withDefaults is only valid for startup or jit inputs".to_string(),
                            ));
                        }
                        dependency_outputs = Some(ProcessedDependencyOutputsInput {
                            from: input.from,
                            globs: structured_globs(input.globs, &span)?,
                        });
                    }
                },
            }
        }

        if legacy_startup && structured_startup {
            return Err(duplicate_startup_error(None));
        }

        Ok(ProcessedInputs {
            globs,
            default,
            jit_globs,
            jit_default,
            dependency_outputs,
            legacy_startup,
            structured_startup,
            structured_jit,
            structured_dependency_outputs,
            extends,
        })
    }

    /// Resolves all globs with the given turbo_root path
    pub fn resolve(&self, turbo_root_path: &RelativeUnixPath) -> Vec<String> {
        self.globs
            .iter()
            .map(|glob| glob.resolve(turbo_root_path))
            .collect()
    }

    pub fn resolve_jit(&self, turbo_root_path: &RelativeUnixPath) -> Vec<String> {
        self.jit_globs
            .iter()
            .map(|glob| glob.resolve(turbo_root_path))
            .collect()
    }
}

enum StructuredInputMode {
    Startup,
    Jit,
    DependencyOutputs,
}

fn structured_input_mode(
    input: &RawStructuredInput,
    span: &Spanned<()>,
) -> Result<StructuredInputMode, Error> {
    let Some(mode) = input.mode.as_ref() else {
        return Err(structured_input_error(
            span,
            "Structured input entries must specify mode".to_string(),
        ));
    };

    match mode.as_str() {
        "startup" => Ok(StructuredInputMode::Startup),
        "jit" => Ok(StructuredInputMode::Jit),
        "dependencyOutputs" => Ok(StructuredInputMode::DependencyOutputs),
        unknown => Err(structured_input_error(
            &mode.to(()),
            format!("Unknown input mode \"{unknown}\""),
        )),
    }
}

fn reject_from(input: &RawStructuredInput, span: &Spanned<()>) -> Result<(), Error> {
    if input.from.is_some() {
        return Err(structured_input_error(
            span,
            "from is only valid for dependencyOutputs inputs".to_string(),
        ));
    }
    Ok(())
}

fn structured_globs(
    raw_globs: Option<Vec<Spanned<UnescapedString>>>,
    span: &Spanned<()>,
) -> Result<Vec<ProcessedGlob>, Error> {
    raw_globs
        .unwrap_or_default()
        .into_iter()
        .map(|glob| {
            if matches!(glob.as_str(), TURBO_DEFAULT | TURBO_EXTENDS) {
                return Err(structured_input_error(
                    &glob.to(()),
                    format!(
                        "Sentinel string \"{}\" is not valid inside structured globs",
                        glob.as_str()
                    ),
                ));
            }
            ProcessedGlob::from_spanned_input(glob)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| match err {
            Error::AbsolutePathInConfig { .. }
            | Error::InvalidTurboRootUse { .. }
            | Error::InvalidTurboRootNeedsSlash { .. } => err,
            _ => structured_input_error(span, err.to_string()),
        })
}

fn reject_negative_only_globs(
    mode: &str,
    with_defaults: bool,
    globs: &[ProcessedGlob],
    span: &Spanned<()>,
) -> Result<(), Error> {
    if !with_defaults && !globs.is_empty() && globs.iter().all(|glob| glob.negated) {
        return Err(structured_input_error(
            span,
            format!("negative-only {mode} globs require withDefaults: true"),
        ));
    }
    Ok(())
}

pub fn duplicate_startup_error(span: Option<&Spanned<()>>) -> Error {
    let message = "Legacy input strings normalize to mode \"startup\", but this task also \
                   declares a structured \"startup\" input.\n\nUse either legacy startup \
                   inputs:\n\n  \"inputs\": [\"$TURBO_DEFAULT$\", \"src/**\"]\n\nOr one \
                   structured startup input:\n\n  \"inputs\": [\n    {\n      \"mode\": \
                   \"startup\",\n      \"withDefaults\": true,\n      \"globs\": [\"src/**\"]\n    \
                   }\n  ]"
        .to_string();
    match span {
        Some(span) => structured_input_error(span, message),
        None => Error::StructuredInput {
            message,
            span: None,
            text: miette::NamedSource::new("turbo.json", String::new()),
        },
    }
}

fn structured_input_error(span: &Spanned<()>, message: String) -> Error {
    let (span, text) = span.span_and_text("turbo.json");
    Error::StructuredInput {
        message,
        span,
        text,
    }
}

/// Processed pass_through_env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedPassThroughEnv {
    pub vars: Vec<String>,
    pub extends: bool,
}

impl ProcessedPassThroughEnv {
    /// Creates a ProcessedPassThroughEnv, validating that env vars don't use
    /// invalid prefixes and handling TURBO_EXTENDS if enabled
    pub fn new(
        raw_env: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_env, extends) = extract_turbo_extends(raw_env, future_flags);

        Ok(ProcessedPassThroughEnv {
            vars: extract_env_vars(processed_env, "passThroughEnv")?,
            extends,
        })
    }
}

fn extract_env_vars(
    raw_env: Vec<Spanned<UnescapedString>>,
    field_name: &str,
) -> Result<Vec<String>, Error> {
    use crate::error::InvalidEnvPrefixError;

    let mut env_vars = Vec::with_capacity(raw_env.len());
    // Validate that no env var starts with ENV_PIPELINE_DELIMITER ($)
    for var in raw_env {
        if var.starts_with(ENV_PIPELINE_DELIMITER) {
            let (span, text) = var.span_and_text("turbo.json");
            return Err(Error::InvalidEnvPrefix(Box::new(InvalidEnvPrefixError {
                key: field_name.to_string(),
                value: var.as_str().to_string(),
                span,
                text,
                env_pipeline_delimiter: ENV_PIPELINE_DELIMITER,
            })));
        }

        env_vars.push(String::from(var.into_inner()));
    }
    env_vars.sort();
    Ok(env_vars)
}

/// Processed outputs field with DSL detection
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessedOutputs {
    pub globs: Vec<ProcessedGlob>,
    pub extends: bool,
}

impl ProcessedOutputs {
    pub fn new(
        raw_globs: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_globs, extends) = extract_turbo_extends(raw_globs, future_flags);

        let globs = processed_globs
            .into_iter()
            .map(ProcessedGlob::from_spanned_output)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ProcessedOutputs { globs, extends })
    }

    /// Resolves all globs with the given turbo_root path
    pub fn resolve(&self, turbo_root_path: &RelativeUnixPath) -> Vec<String> {
        self.globs
            .iter()
            .map(|glob| glob.resolve(turbo_root_path))
            .collect()
    }
}

/// Processed with field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedWith {
    pub tasks: Vec<Spanned<TaskName<'static>>>,
    pub extends: bool,
}

impl ProcessedWith {
    /// Creates a ProcessedWith, validating that siblings don't use topological
    /// prefix and handling TURBO_EXTENDS if enabled
    pub fn new(
        raw_with: Vec<Spanned<UnescapedString>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let (processed_with, extends) = extract_turbo_extends(raw_with, future_flags);

        // Validate that no sibling starts with TOPOLOGICAL_PIPELINE_DELIMITER (^)
        let mut tasks = Vec::with_capacity(processed_with.len());
        for sibling in processed_with {
            if sibling.starts_with(TOPOLOGICAL_PIPELINE_DELIMITER) {
                let (span, text) = sibling.span_and_text("turbo.json");
                return Err(Error::InvalidTaskWith { span, text });
            }
            let (sibling, span) = sibling.split();
            tasks.push(span.to(TaskName::from(String::from(sibling))));
        }
        Ok(ProcessedWith { tasks, extends })
    }
}

/// A processed incremental cache partition with validated output and input
/// globs.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedIncrementalPartition {
    pub outputs: ProcessedOutputs,
    pub inputs: Option<ProcessedInputs>,
}

/// The canonical toolchain ids accepted as `command` map keys, alongside
/// their accepted aliases. Kept as literals: this crate sits below the
/// toolchain registry, and these ids are stable public API.
const KNOWN_TOOLCHAINS: [&str; 2] = ["javascript", "rust"];
const TOOLCHAIN_ALIASES: [(&str, &str); 1] = [("typescript", "javascript")];

/// A task `command` after alias resolution and validation: the argv the
/// task runs, an explicit opt-out, or per-toolchain argv defaults keyed by
/// canonical toolchain id.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessedCommand {
    /// Explicitly no command: the task is a no-op for this package.
    OptOut(Spanned<()>),
    /// The argv to execute: program first, arguments after.
    Argv(Spanned<Vec<String>>),
    /// Per-toolchain argv defaults, in source order with canonicalized
    /// keys.
    PerToolchain(Spanned<Vec<(String, Vec<String>)>>),
}

impl ProcessedCommand {
    pub fn from_raw(raw: Spanned<RawCommand>, future_flags: &FutureFlags) -> Result<Self, Error> {
        if !future_flags.experimental_task_command {
            let (span, text) = raw.span_and_text("turbo.json");
            return Err(Error::TaskCommandRequiresFlag { span, text });
        }
        let span_marker = raw.clone().map(|_| ());
        match raw.into_inner() {
            RawCommand::OptOut => Ok(Self::OptOut(span_marker)),
            RawCommand::Argv(items) => {
                let argv = Self::validate_argv(items)?;
                Ok(Self::Argv(span_marker.map(|()| argv)))
            }
            RawCommand::PerToolchain(entries) => {
                let mut canonical_entries: Vec<(String, Vec<String>)> = Vec::new();
                for (key, argv) in entries {
                    let canonical = Self::canonical_toolchain(&key, future_flags)?;
                    if let Some((prior, _)) = canonical_entries
                        .iter()
                        .find(|(existing, _)| existing == &canonical)
                    {
                        let (span, text) = key.span_and_text("turbo.json");
                        return Err(Error::TaskCommandAliasConflict {
                            alias: key.as_inner().clone(),
                            canonical: prior.clone(),
                            span,
                            text,
                        });
                    }
                    canonical_entries.push((canonical, Self::validate_argv(argv)?));
                }
                Ok(Self::PerToolchain(span_marker.map(|()| canonical_entries)))
            }
        }
    }

    /// Resolve a `command` map key to its canonical toolchain id, erroring
    /// on unknown keys (with a did-you-mean) and on toolchains whose
    /// feature flag is off.
    fn canonical_toolchain(
        key: &Spanned<String>,
        future_flags: &FutureFlags,
    ) -> Result<String, Error> {
        let raw_key = key.as_inner().as_str();
        let canonical = TOOLCHAIN_ALIASES
            .iter()
            .find(|(alias, _)| *alias == raw_key)
            .map(|(_, canonical)| *canonical)
            .unwrap_or(raw_key);
        if !KNOWN_TOOLCHAINS.contains(&canonical) {
            let (span, text) = key.span_and_text("turbo.json");
            let hint = if raw_key == "cargo" {
                r#"Rust crates are the "rust" toolchain."#.to_string()
            } else {
                format!(
                    "Known toolchains: {}.",
                    KNOWN_TOOLCHAINS
                        .iter()
                        .map(|t| format!("{t:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err(Error::TaskCommandUnknownToolchain {
                key: raw_key.to_string(),
                hint,
                span,
                text,
            });
        }
        if canonical == "rust" && !future_flags.experimental_cargo_workspaces {
            let (span, text) = key.span_and_text("turbo.json");
            return Err(Error::TaskCommandToolchainRequiresFlag {
                key: raw_key.to_string(),
                span,
                text,
            });
        }
        Ok(canonical.to_string())
    }

    /// Validate argv elements: no empty strings, no `$TURBO_EXTENDS$` (a
    /// command is atomic), and warn on shell-variable lookalikes — there is
    /// no shell, so `$VAR` is passed literally.
    fn validate_argv(items: Vec<Spanned<UnescapedString>>) -> Result<Vec<String>, Error> {
        items
            .into_iter()
            .map(|item| {
                let value = item.as_str().to_string();
                if value.is_empty() {
                    let (span, text) = item.span_and_text("turbo.json");
                    return Err(Error::TaskCommandEmptyArgument { span, text });
                }
                if value == TURBO_EXTENDS {
                    let (span, text) = item.span_and_text("turbo.json");
                    return Err(Error::TaskCommandNoExtends {
                        token: value,
                        span,
                        text,
                    });
                }
                if looks_like_shell_variable(&value) {
                    tracing::warn!(
                        "`command` arguments are not shell-interpolated; {value:?} will be passed \
                         literally"
                    );
                }
                Ok(value)
            })
            .collect()
    }
}

/// `$IDENTIFIER` or `%IDENTIFIER%`: almost always someone expecting shell
/// interpolation that does not exist.
fn looks_like_shell_variable(value: &str) -> bool {
    let unix_style = value
        .strip_prefix('$')
        .is_some_and(|rest| !rest.is_empty() && is_identifier(rest));
    let windows_style = value
        .strip_prefix('%')
        .and_then(|rest| rest.strip_suffix('%'))
        .is_some_and(|inner| !inner.is_empty() && is_identifier(inner));
    unix_style || windows_style
}

fn is_identifier(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Intermediate representation for task definitions with DSL processing
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessedTaskDefinition {
    pub extends: Option<Spanned<bool>>,
    pub description: Option<Spanned<UnescapedString>>,
    pub cache: Option<Spanned<bool>>,
    pub depends_on: Option<ProcessedDependsOn>,
    pub env: Option<ProcessedEnv>,
    pub inputs: Option<ProcessedInputs>,
    pub pass_through_env: Option<ProcessedPassThroughEnv>,
    pub persistent: Option<Spanned<bool>>,
    pub interruptible: Option<Spanned<bool>>,
    pub outputs: Option<ProcessedOutputs>,
    pub output_logs: Option<Spanned<OutputLogsMode>>,
    pub interactive: Option<Spanned<bool>>,
    pub env_mode: Option<Spanned<EnvMode>>,
    pub with: Option<ProcessedWith>,
    pub incremental: Option<Vec<ProcessedIncrementalPartition>>,
    pub experimental_ci: Option<Spanned<ExperimentalCIConfig>>,
    pub command: Option<ProcessedCommand>,
}

impl ProcessedTaskDefinition {
    /// Creates a processed task definition from raw task
    pub fn from_raw(
        raw_task: RawTaskDefinition,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let incremental = raw_task
            .incremental
            .map(|partitions| {
                partitions
                    .into_iter()
                    .filter_map(|partition| {
                        let outputs = match partition
                            .outputs
                            .map(|o| ProcessedOutputs::new(o, future_flags))
                            .transpose()
                        {
                            Ok(o) => o.unwrap_or_default(),
                            Err(e) => return Some(Err(e)),
                        };
                        // Skip partitions with no output globs — they'd never
                        // match any files and are almost certainly a config error.
                        if outputs.globs.is_empty() {
                            return None;
                        }
                        // Reject task-input DSL tokens in
                        // incremental inputs — these DSL tokens only apply to
                        // regular task inputs and have no meaning here.
                        if let Some(ref raw_inputs) = partition.inputs {
                            for input in raw_inputs {
                                if input.as_str() == TURBO_DEFAULT
                                    || input.as_str() == TURBO_EXTENDS
                                {
                                    let (span, text) = input.span_and_text("turbo.json");
                                    return Some(Err(Error::InvalidIncrementalInput {
                                        value: input.as_str().to_string(),
                                        span,
                                        text,
                                    }));
                                }
                            }
                        }
                        let inputs = match partition
                            .inputs
                            .map(|i| ProcessedInputs::new_legacy(i, future_flags))
                            .transpose()
                        {
                            Ok(i) => i,
                            Err(e) => return Some(Err(e)),
                        };
                        Some(Ok(ProcessedIncrementalPartition { outputs, inputs }))
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
            .transpose()?;

        Ok(ProcessedTaskDefinition {
            extends: raw_task.extends,
            description: raw_task.description,
            cache: raw_task.cache,
            depends_on: raw_task
                .depends_on
                .map(|deps| ProcessedDependsOn::new(deps, future_flags))
                .transpose()?,
            env: raw_task
                .env
                .map(|env| ProcessedEnv::new(env, future_flags))
                .transpose()?,
            inputs: raw_task
                .inputs
                .map(|inputs| ProcessedInputs::new(inputs, future_flags))
                .transpose()?,
            pass_through_env: raw_task
                .pass_through_env
                .map(|env| ProcessedPassThroughEnv::new(env, future_flags))
                .transpose()?,
            persistent: raw_task.persistent,
            interruptible: raw_task.interruptible,
            outputs: raw_task
                .outputs
                .map(|outputs| ProcessedOutputs::new(outputs, future_flags))
                .transpose()?,
            output_logs: raw_task.output_logs,
            interactive: raw_task.interactive,
            env_mode: raw_task.env_mode,
            with: raw_task
                .with
                .map(|with| ProcessedWith::new(with, future_flags))
                .transpose()?,
            incremental,
            experimental_ci: raw_task.experimental_ci,
            command: raw_task
                .command
                .map(|command| ProcessedCommand::from_raw(command, future_flags))
                .transpose()?,
        })
    }

    /// Check if a task definition has any configuration beyond just the
    /// `extends` field.
    pub fn has_config_beyond_extends(&self) -> bool {
        self.cache.is_some()
            || self.depends_on.is_some()
            || self.env.is_some()
            || self.inputs.is_some()
            || self.pass_through_env.is_some()
            || self.persistent.is_some()
            || self.interruptible.is_some()
            || self.outputs.is_some()
            || self.output_logs.is_some()
            || self.interactive.is_some()
            || self.with.is_some()
            || self.incremental.is_some()
            || self.experimental_ci.is_some()
            || self.command.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches, sync::Arc};

    use test_case::test_case;
    use turborepo_errors::Spanned;
    use turborepo_unescape::UnescapedString;

    use super::*;

    fn command_flags() -> FutureFlags {
        FutureFlags {
            experimental_task_command: true,
            experimental_cargo_workspaces: true,
            ..Default::default()
        }
    }

    fn spanned_argv(items: &[&str]) -> Spanned<RawCommand> {
        Spanned::new(RawCommand::Argv(
            items
                .iter()
                .map(|item| Spanned::new(UnescapedString::from(item.to_string())))
                .collect(),
        ))
    }

    fn spanned_map(entries: &[(&str, &[&str])]) -> Spanned<RawCommand> {
        Spanned::new(RawCommand::PerToolchain(
            entries
                .iter()
                .map(|(key, argv)| {
                    (
                        Spanned::new(key.to_string()),
                        argv.iter()
                            .map(|item| Spanned::new(UnescapedString::from(item.to_string())))
                            .collect(),
                    )
                })
                .collect(),
        ))
    }

    #[test]
    fn test_command_requires_flag() {
        let err = ProcessedCommand::from_raw(spanned_argv(&["vitest"]), &FutureFlags::default())
            .unwrap_err();
        assert!(matches!(err, Error::TaskCommandRequiresFlag { .. }));
    }

    #[test]
    fn test_command_argv_processed() {
        let command = ProcessedCommand::from_raw(
            spanned_argv(&["cargo", "nextest", "run"]),
            &command_flags(),
        )
        .unwrap();
        let ProcessedCommand::Argv(argv) = command else {
            panic!("expected argv");
        };
        assert_eq!(argv.as_inner(), &["cargo", "nextest", "run"]);
    }

    #[test]
    fn test_command_opt_out_processed() {
        let command =
            ProcessedCommand::from_raw(Spanned::new(RawCommand::OptOut), &command_flags()).unwrap();
        assert!(matches!(command, ProcessedCommand::OptOut(_)));
    }

    #[test]
    fn test_command_alias_resolves_to_canonical() {
        let command = ProcessedCommand::from_raw(
            spanned_map(&[("typescript", &["vitest"])]),
            &command_flags(),
        )
        .unwrap();
        let ProcessedCommand::PerToolchain(entries) = command else {
            panic!("expected map");
        };
        assert_eq!(
            entries.as_inner(),
            &[("javascript".to_string(), vec!["vitest".to_string()])]
        );
    }

    #[test]
    fn test_command_alias_conflict() {
        let err = ProcessedCommand::from_raw(
            spanned_map(&[("javascript", &["vitest"]), ("typescript", &["jest"])]),
            &command_flags(),
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                Error::TaskCommandAliasConflict { ref alias, ref canonical, .. }
                    if alias == "typescript" && canonical == "javascript"
            ),
            "got: {err}"
        );
    }

    #[test]
    fn test_command_unknown_toolchain_hints() {
        let err = ProcessedCommand::from_raw(spanned_map(&[("go", &["go"])]), &command_flags())
            .unwrap_err();
        assert!(
            matches!(err, Error::TaskCommandUnknownToolchain { ref hint, .. }
                if hint.contains("javascript") && hint.contains("rust")),
            "got: {err}"
        );

        // `cargo` gets a targeted correction, not accepted as an alias.
        let err = ProcessedCommand::from_raw(
            spanned_map(&[("cargo", &["cargo", "test"])]),
            &command_flags(),
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::TaskCommandUnknownToolchain { ref hint, .. }
                if hint.contains(r#""rust" toolchain"#)),
            "got: {err}"
        );
    }

    #[test]
    fn test_command_rust_key_requires_cargo_flag() {
        let flags = FutureFlags {
            experimental_task_command: true,
            ..Default::default()
        };
        let err = ProcessedCommand::from_raw(spanned_map(&[("rust", &["cargo", "test"])]), &flags)
            .unwrap_err();
        assert!(matches!(
            err,
            Error::TaskCommandToolchainRequiresFlag { .. }
        ));
    }

    #[test]
    fn test_command_rejects_empty_argument_and_extends_token() {
        let err =
            ProcessedCommand::from_raw(spanned_argv(&["cargo", ""]), &command_flags()).unwrap_err();
        assert!(matches!(err, Error::TaskCommandEmptyArgument { .. }));

        let err = ProcessedCommand::from_raw(
            spanned_argv(&["$TURBO_EXTENDS$", "cargo"]),
            &command_flags(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::TaskCommandNoExtends { .. }));
    }

    #[test]
    fn test_extract_turbo_extends_with_marker() {
        let items = vec![
            Spanned::new(UnescapedString::from("item1")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("item2")),
        ];

        let (processed, extends) = extract_turbo_extends(items, &FutureFlags::default());

        assert!(extends);
        assert_eq!(processed.len(), 2);
        assert_eq!(processed[0].as_str(), "item1");
        assert_eq!(processed[1].as_str(), "item2");
    }

    #[test]
    fn test_extract_turbo_extends_no_marker() {
        let items = vec![
            Spanned::new(UnescapedString::from("item1")),
            Spanned::new(UnescapedString::from("item2")),
        ];

        let (processed, extends) = extract_turbo_extends(items, &FutureFlags::default());

        assert!(!extends);
        assert_eq!(processed.len(), 2);
    }

    #[test_case("$TURBO_ROOT$/config.txt", Ok((true, false)) ; "detects turbo root")]
    #[test_case("!$TURBO_ROOT$/README.md", Ok((true, true)) ; "detects negated turbo root")]
    #[test_case("src/**/*.ts", Ok((false, false)) ; "no turbo root")]
    fn test_processed_glob_detection(input: &str, expected: Result<(bool, bool), &str>) {
        // Test with input variant
        let result = ProcessedGlob::from_spanned_input(Spanned::new(UnescapedString::from(
            input.to_string(),
        )));

        match expected {
            Ok((turbo_root, negated)) => {
                let glob = result.unwrap();
                assert_eq!(glob.turbo_root, turbo_root);
                assert_eq!(glob.negated, negated);
            }
            Err(_) => {
                assert!(result.is_err());
            }
        }
    }

    #[test_case("$TURBO_ROOT$config.txt", "must be followed by a '/'" ; "missing slash")]
    #[test_case("../$TURBO_ROOT$/config.txt", "must be used at the start of glob" ; "middle turbo root")]
    fn test_processed_glob_validation_errors(input: &str, expected_error: &str) {
        // Test with input variant
        let result = ProcessedGlob::from_spanned_input(
            Spanned::new(UnescapedString::from(input.to_string()))
                .with_path(Arc::from("turbo.json"))
                .with_text(format!("\"{}\"", input))
                .with_range(1..input.len() + 1),
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains(expected_error));
    }

    #[test_case("$TURBO_ROOT$/config.txt", "../..", "../../config.txt" ; "replace turbo root")]
    #[test_case("!$TURBO_ROOT$/README.md", "../..", "!../../README.md" ; "replace negated turbo root")]
    #[test_case("src/**/*.ts", "../..", "src/**/*.ts" ; "no replacement needed")]
    fn test_processed_glob_resolution(input: &str, replacement: &str, expected: &str) {
        let replacement = RelativeUnixPath::new(replacement).unwrap();
        // Test with output variant
        let glob = ProcessedGlob::from_spanned_output(Spanned::new(UnescapedString::from(
            input.to_string(),
        )))
        .unwrap();

        let resolved = glob.resolve(replacement);
        assert_eq!(resolved, expected);
    }

    #[test_case("../../target.txt" ; "direct traversal")]
    #[test_case("..///../target.txt" ; "extra slashes")]
    #[test_case("extra/../../target.txt" ; "intermediate directory")]
    #[test_case("./../../target.txt" ; "leading current directory")]
    #[test_case("!../../target.txt" ; "negated traversal")]
    #[test_case("../../{file1,file2,fileN}" ; "brace expansion")]
    fn test_processed_outputs_allow_parent_directory_segments(input: &str) {
        let result = ProcessedGlob::from_spanned_output(Spanned::new(UnescapedString::from(
            input.to_string(),
        )));

        assert!(result.is_ok());
    }

    // A leading absolute path is rejected, but as an absolute-path error rather
    // than a traversal error.
    #[test]
    fn test_processed_outputs_reject_absolute_path() {
        let absolute_path = if cfg!(windows) {
            "C:\\win32\\target.txt"
        } else {
            "/etc/passwd"
        };
        let result =
            ProcessedGlob::from_spanned_output(Spanned::new(UnescapedString::from(absolute_path)));

        assert_matches!(result, Err(Error::AbsolutePathInConfig { .. }));
    }

    #[test]
    fn test_incremental_outputs_allow_parent_directory_segments() {
        let raw_task = RawTaskDefinition {
            incremental: Some(vec![crate::raw::RawIncrementalPartition {
                outputs: Some(vec![Spanned::new(UnescapedString::from("../target.txt"))]),
                ..Default::default()
            }]),
            ..Default::default()
        };

        let result = ProcessedTaskDefinition::from_raw(raw_task, &FutureFlags::default());

        assert!(result.is_ok());
    }

    #[test]
    fn test_incremental_negated_outputs_allow_parent_directory_segments() {
        let raw_task = RawTaskDefinition {
            incremental: Some(vec![crate::raw::RawIncrementalPartition {
                outputs: Some(vec![
                    Spanned::new(UnescapedString::from("dist/**")),
                    Spanned::new(UnescapedString::from("!../../secret.txt")),
                ]),
                ..Default::default()
            }]),
            ..Default::default()
        };

        let result = ProcessedTaskDefinition::from_raw(raw_task, &FutureFlags::default());

        assert!(result.is_ok());
    }

    #[test]
    fn test_from_raw_carries_experimental_ci() {
        let raw_task = RawTaskDefinition {
            experimental_ci: Some(Spanned::new(ExperimentalCIConfig::Enabled(true))),
            ..Default::default()
        };

        let processed =
            ProcessedTaskDefinition::from_raw(raw_task, &FutureFlags::default()).unwrap();

        assert_eq!(
            processed.experimental_ci,
            Some(Spanned::new(ExperimentalCIConfig::Enabled(true)))
        );
        assert!(processed.has_config_beyond_extends());
    }

    #[test]
    fn test_processed_task_definition_resolve() {
        // Create a raw task definition with TURBO_ROOT tokens
        let raw_task = RawTaskDefinition {
            inputs: Some(vec![
                Spanned::new(RawTaskInput::String(UnescapedString::from(
                    "$TURBO_ROOT$/config.txt",
                ))),
                Spanned::new(RawTaskInput::String(UnescapedString::from("src/**/*.ts"))),
            ]),
            outputs: Some(vec![
                Spanned::new(UnescapedString::from("!$TURBO_ROOT$/README.md")),
                Spanned::new(UnescapedString::from("dist/**")),
            ]),
            ..Default::default()
        };

        // Convert to processed task definition
        let processed =
            ProcessedTaskDefinition::from_raw(raw_task, &FutureFlags::default()).unwrap();
        let turbo_root = RelativeUnixPath::new("../..").unwrap();

        // Verify TURBO_ROOT detection
        let inputs = processed.inputs.as_ref().unwrap();
        assert!(inputs.globs[0].turbo_root);
        assert!(!inputs.globs[0].negated);
        assert!(!inputs.globs[1].turbo_root);

        let outputs = processed.outputs.as_ref().unwrap();
        assert!(outputs.globs[0].turbo_root);
        assert!(outputs.globs[0].negated);
        assert!(!outputs.globs[1].turbo_root);

        // Resolve with turbo_root path
        let resolved_inputs = inputs.resolve(turbo_root);
        assert_eq!(resolved_inputs[0], "../../config.txt");
        assert_eq!(resolved_inputs[1], "src/**/*.ts");

        let resolved_outputs = outputs.resolve(turbo_root);
        assert_eq!(resolved_outputs[0], "!../../README.md");
        assert_eq!(resolved_outputs[1], "dist/**");
    }

    #[test]
    fn test_detects_turbo_default() {
        let raw_globs = vec![Spanned::new(UnescapedString::from(TURBO_DEFAULT))];

        let inputs = ProcessedInputs::new_legacy(raw_globs, &FutureFlags::default()).unwrap();
        assert!(inputs.default);
        assert!(inputs.globs.is_empty());
    }

    #[test]
    fn test_absolute_paths_error_in_inputs() {
        let absolute_path = if cfg!(windows) {
            "C:\\win32"
        } else {
            "/dev/null"
        };

        // The error should be caught when creating the ProcessedGlob
        let result =
            ProcessedGlob::from_spanned_input(Spanned::new(UnescapedString::from(absolute_path)));

        assert_matches!(result, Err(Error::AbsolutePathInConfig { .. }));
    }

    // Test that demonstrates the extends field is properly set when the helper is
    // used
    #[test]
    fn test_processed_inputs_with_turbo_extends() {
        let raw_globs: Vec<Spanned<UnescapedString>> = vec![
            Spanned::new(UnescapedString::from("src/**")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("lib/**")),
        ];

        let inputs = ProcessedInputs::new_legacy(raw_globs, &FutureFlags::default()).unwrap();

        assert!(inputs.extends);
        assert_eq!(inputs.globs.len(), 2);
        assert_eq!(inputs.globs[0].glob, "src/**");
        assert_eq!(inputs.globs[1].glob, "lib/**");
    }

    #[test]
    fn test_processed_env_with_turbo_extends() {
        // $TURBO_EXTENDS$ is now always processed
        let raw_env: Vec<Spanned<UnescapedString>> = vec![
            Spanned::new(UnescapedString::from("NODE_ENV")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("API_KEY")),
        ];

        let result = ProcessedEnv::new(raw_env, &FutureFlags::default());
        assert!(result.is_ok());
        let env = result.unwrap();
        assert!(env.extends);
        assert_eq!(env.vars.len(), 2);
    }

    #[test]
    fn test_processed_depends_on_with_turbo_extends() {
        // $TURBO_EXTENDS$ is now always processed
        let raw_deps: Vec<Spanned<UnescapedString>> = vec![
            Spanned::new(UnescapedString::from("build")),
            Spanned::new(UnescapedString::from("$TURBO_EXTENDS$")),
            Spanned::new(UnescapedString::from("test")),
        ];

        let result = ProcessedDependsOn::new(Spanned::new(raw_deps), &FutureFlags::default());
        assert!(result.is_ok());
        let depends_on = result.unwrap();
        assert!(depends_on.extends);
        assert_eq!(depends_on.deps.len(), 2);
    }
}
