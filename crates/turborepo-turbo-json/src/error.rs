//! Error types for turbo.json parsing and validation
//!
//! This module contains all error types specific to turbo.json configuration
//! parsing, validation, and processing.
//!
//! Note: Many struct/enum fields in this module are read by miette's Diagnostic
//! derive macro for error formatting and display, not directly by code.

use std::backtrace;

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;
use turborepo_errors::ParseDiagnostic;

/// Error type for turbo.json parsing failures
#[derive(Debug, Error, Diagnostic)]
#[error("Failed to parse turbo.json.")]
#[diagnostic(code(turbo_json_parse_error))]
pub struct ParseError {
    #[related]
    pub diagnostics: Vec<ParseDiagnostic>,
    #[backtrace]
    backtrace: backtrace::Backtrace,
}

impl ParseError {
    /// Creates a new ParseError with the given diagnostics
    pub fn new(diagnostics: Vec<ParseDiagnostic>) -> Self {
        Self {
            diagnostics,
            backtrace: backtrace::Backtrace::capture(),
        }
    }
}

/// Error for environment variable prefixes that are not allowed
#[derive(Debug, Error, Diagnostic)]
#[error("Environment variables should not be prefixed with \"{env_pipeline_delimiter}\"")]
#[diagnostic(
    code(invalid_env_prefix),
    url("https://turborepo.dev/messages/invalid-env-prefix")
)]
pub struct InvalidEnvPrefixError {
    /// The invalid value that was found
    pub value: String,
    /// The key/field where the invalid value was found
    pub key: String,
    #[source_code]
    pub text: NamedSource<String>,
    #[label("variable with invalid prefix declared here")]
    pub span: Option<SourceSpan>,
    /// The delimiter that should not be used as a prefix
    pub env_pipeline_delimiter: &'static str,
}

/// Error for unnecessary package task syntax in workspace turbo.json
#[derive(Debug, Error, Diagnostic)]
#[diagnostic(
    code(unnecessary_package_task_syntax),
    url("https://turborepo.dev/messages/unnecessary-package-task-syntax")
)]
#[error("\"{actual}\". Use \"{wanted}\" instead.")]
pub struct UnnecessaryPackageTaskSyntaxError {
    /// The actual task name found
    pub actual: String,
    /// The recommended task name format
    pub wanted: String,
    #[label("unnecessary package syntax found here")]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
}

/// Main error enum for turbo.json operations
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    // ============================================================
    // File existence and location errors
    // ============================================================
    #[error(
        "Could not find turbo.json or turbo.jsonc.\nFollow directions at https://turborepo.dev/docs \
         to create one."
    )]
    NoTurboJSON,

    #[error(
        "Found both turbo.json and turbo.jsonc in the same directory: {directory}\nRemove either \
         turbo.json or turbo.jsonc so there is only one."
    )]
    MultipleTurboConfigs {
        /// The directory containing both config files
        directory: String,
    },

    // ============================================================
    // Parsing errors
    // ============================================================
    #[error(transparent)]
    #[diagnostic(transparent)]
    Parse(#[from] ParseError),

    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    // ============================================================
    // Task definition validation errors
    // ============================================================
    #[error(
        "Package tasks (<package>#<task>) are not allowed in single-package repositories: found \
         {task_id}"
    )]
    #[diagnostic(
        code(package_task_in_single_package_mode),
        url("https://turborepo.dev/messages/package-task-in-single-package-mode")
    )]
    PackageTaskInSinglePackageMode {
        task_id: String,
        #[source_code]
        text: NamedSource<String>,
        #[label("package task found here")]
        span: Option<SourceSpan>,
    },

    #[error("Interruptible tasks must be persistent.")]
    InterruptibleButNotPersistent {
        #[source_code]
        text: NamedSource<String>,
        #[label("`interruptible` set here")]
        span: Option<SourceSpan>,
    },

    #[error("Tasks cannot be marked as interactive and cacheable.")]
    InteractiveNoCacheable {
        #[label("marked interactive here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(transparent)]
    #[diagnostic(transparent)]
    InvalidEnvPrefix(Box<InvalidEnvPrefixError>),

    #[error(transparent)]
    #[diagnostic(transparent)]
    UnnecessaryPackageTaskSyntax(Box<UnnecessaryPackageTaskSyntaxError>),

    // ============================================================
    // Extends validation errors
    // ============================================================
    #[error("You must extend from the root of the workspace first.")]
    ExtendsRootFirst {
        #[label("'//' should be first")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "The \"extends\" key on task \"{task_name}\" can only be used in Package Configurations."
    )]
    TaskExtendsInRoot {
        task_name: String,
        #[label("\"extends\" found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "Cannot set \"extends\": false on task \"{task_name}\" because it is not defined in the \
         extends chain."
    )]
    #[diagnostic(help("{extends_chain}"))]
    TaskNotInExtendsChain {
        task_name: String,
        extends_chain: String,
        #[label("task is not inherited")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error("No \"extends\" key found.")]
    NoExtends {
        #[label("add extends key here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    // ============================================================
    // Field value validation errors
    // ============================================================
    #[error("`{field}` cannot contain an environment variable.")]
    InvalidDependsOnValue {
        field: &'static str,
        #[label("environment variable found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error("`{field}` cannot contain an absolute path.")]
    AbsolutePathInConfig {
        field: &'static str,
        #[label("absolute path found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error("`outputs` cannot contain path traversal.")]
    #[diagnostic(help("Use `$TURBO_ROOT$` to declare outputs relative to the repository root."))]
    OutputPathTraversal {
        #[label("path traversal found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error("found absolute path in `cacheDir`")]
    #[diagnostic(help("If absolute paths are required, use `--cache-dir` or `TURBO_CACHE_DIR`."))]
    AbsoluteCacheDir {
        #[label("Make `cacheDir` value a relative unix path.")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    // ============================================================
    // Pipeline/tasks field errors
    // ============================================================
    #[error("Found `pipeline` field instead of `tasks`.")]
    #[diagnostic(help("Changed in 2.0: `pipeline` has been renamed to `tasks`."))]
    PipelineField {
        #[label("Rename `pipeline` field to `tasks`")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    // ============================================================
    // $TURBO_ROOT$ DSL errors
    // ============================================================
    #[error("\"$TURBO_ROOT$\" must be used at the start of glob.")]
    InvalidTurboRootUse {
        #[label("\"$TURBO_ROOT$\" must be used at the start of glob.")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error("\"$TURBO_ROOT$\" must be followed by a '/'.")]
    InvalidTurboRootNeedsSlash {
        #[label("\"$TURBO_ROOT$\" must be followed by a '/'.")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    // ============================================================
    // Task `with` field errors
    // ============================================================
    #[error("`with` cannot use dependency relationships.")]
    InvalidTaskWith {
        #[label("Remove `^` from start of task name.")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    // ============================================================
    // Incremental configuration errors
    // ============================================================
    #[error(
        "`{value}` is not supported in incremental partition inputs. Incremental inputs are \
         independent of the task's regular input configuration."
    )]
    InvalidIncrementalInput {
        value: String,
        #[label("unsupported token in incremental inputs")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "The `command` field requires `futureFlags.experimentalTaskCommand` in the root \
         turbo.json."
    )]
    TaskCommandRequiresFlag {
        #[label("`command` used here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error("Unknown toolchain {key:?} in `command`. {hint}")]
    TaskCommandUnknownToolchain {
        key: String,
        hint: String,
        #[label("unknown toolchain")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "`command` defines both {alias:?} and {canonical:?} — these are the same toolchain. Use \
         one key."
    )]
    TaskCommandAliasConflict {
        alias: String,
        canonical: String,
        #[label("same toolchain as {canonical}")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "The {key:?} toolchain in `command` requires `futureFlags.experimentalCargoWorkspaces`."
    )]
    TaskCommandToolchainRequiresFlag {
        key: String,
        #[label("toolchain is not enabled")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "`command` arguments cannot be empty strings. Remove the empty element or replace it with \
         a real argument."
    )]
    TaskCommandEmptyArgument {
        #[label("empty argument")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "`{token}` is not supported in `command`. A command is atomic: define the whole argv in \
         one place."
    )]
    TaskCommandNoExtends {
        token: String,
        #[label("unsupported token")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "The per-toolchain map form of `command` is only valid on unscoped root tasks. \
         \"{task_name}\" already determines its package's toolchain — use an argv array."
    )]
    TaskCommandMapInScopedPosition {
        task_name: String,
        #[label("use an array here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "A `command` opt-out (`null` or `[]`) is only valid in package-scoped positions. An \
         unscoped default of nothing is meaningless — omit `command` instead, or opt out \
         per-package (e.g. \"my-package#{task_name}\")."
    )]
    TaskCommandOptOutUnscoped {
        task_name: String,
        #[label("opt-out on an unscoped task")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error("Invalid inputs for task.\n\n{message}")]
    StructuredInput {
        message: String,
        #[label("invalid input declared here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    // ============================================================
    // Root-only field errors
    // ============================================================
    #[error(
        "The \"futureFlags\" key can only be used in the root turbo.json. Please remove it from \
         Package Configurations."
    )]
    FutureFlagsInPackage {
        #[label("futureFlags key found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error("The \"global\" key requires \"futureFlags.globalConfiguration\" to be enabled.")]
    #[diagnostic(help(
        "Add `\"futureFlags\": {{ \"globalConfiguration\": true }}` to your root turbo.json."
    ))]
    GlobalKeyWithoutFlag {
        #[label("\"global\" key found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },

    #[error(
        "When \"futureFlags.globalConfiguration\" is enabled, \"{key}\" should be placed inside \
         the \"global\" key{rename_hint}."
    )]
    #[diagnostic(help("Move the value to \"global\" and remove it from the top level."))]
    TopLevelGlobalKeyWithFlag {
        key: &'static str,
        rename_hint: String,
        #[label("move this inside \"global\"")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
}

impl From<crate::parser::BiomeParseError> for Error {
    fn from(err: crate::parser::BiomeParseError) -> Self {
        // BiomeParseError has the same structure as ParseError
        // We convert it directly
        Error::Parse(ParseError {
            diagnostics: err.diagnostics,
            backtrace: backtrace::Backtrace::capture(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::NoTurboJSON;
        assert!(err.to_string().contains("Could not find turbo.json"));

        let err = Error::MultipleTurboConfigs {
            directory: "/path/to/dir".to_string(),
        };
        assert!(err.to_string().contains("turbo.json and turbo.jsonc"));
        assert!(err.to_string().contains("/path/to/dir"));
    }

    #[test]
    fn test_parse_error_creation() {
        let parse_err = ParseError::new(vec![]);
        assert!(parse_err.diagnostics.is_empty());
    }

    #[test]
    fn test_invalid_env_prefix_error() {
        let err = InvalidEnvPrefixError {
            value: "$NODE_ENV".to_string(),
            key: "env".to_string(),
            text: NamedSource::new("turbo.json", String::new()),
            span: None,
            env_pipeline_delimiter: "$",
        };
        assert!(
            err.to_string()
                .contains("Environment variables should not be prefixed")
        );
    }
}
