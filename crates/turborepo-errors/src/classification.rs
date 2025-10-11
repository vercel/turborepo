//! Error classification for consistent error handling across turborepo.
//!
//! This module provides a standardized way to classify errors based on their
//! nature, which helps with:
//! - Exit code determination
//! - Error reporting and metrics
//! - User-facing error messages
//! - Debugging and troubleshooting

use std::fmt;

/// Classification of errors by their nature and severity.
///
/// This enum provides a consistent way to categorize errors across different
/// parts of the turborepo codebase. Each variant represents a broad category
/// of error that may require different handling strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorClassification {
    /// Configuration-related errors (invalid config, missing config, etc.)
    Configuration,

    /// Authentication and authorization errors
    Authentication,

    /// Network-related errors (timeouts, connection refused, DNS failures)
    Network,

    /// File system errors (file not found, permission denied, disk full)
    FileSystem,

    /// Process execution errors (spawn failures, non-zero exit codes)
    ProcessExecution,

    /// Invalid user input or arguments
    UserInput,

    /// Internal logic errors or bugs
    Internal,

    /// Errors from external dependencies or packages
    Dependency,

    /// Cache-related errors
    Cache,

    /// Task graph and execution errors
    TaskExecution,

    /// Daemon-related errors
    Daemon,

    /// Environment errors (missing env vars, platform issues)
    Environment,

    /// Parsing errors (JSON, JSONC, package.json, etc.)
    Parsing,

    /// Proxy and networking errors (specific to microfrontends proxy)
    Proxy,
}

impl ErrorClassification {
    /// Returns a suggested exit code for this error classification.
    ///
    /// This helps provide consistent exit codes across different error types.
    pub fn exit_code(&self) -> i32 {
        match self {
            ErrorClassification::Configuration => 1,
            ErrorClassification::Authentication => 1,
            ErrorClassification::Network => 1,
            ErrorClassification::FileSystem => 1,
            ErrorClassification::ProcessExecution => 1,
            ErrorClassification::UserInput => 2,
            ErrorClassification::Internal => 100,
            ErrorClassification::Dependency => 1,
            ErrorClassification::Cache => 1,
            ErrorClassification::TaskExecution => 1,
            ErrorClassification::Daemon => 1,
            ErrorClassification::Environment => 1,
            ErrorClassification::Parsing => 1,
            ErrorClassification::Proxy => 1,
        }
    }

    /// Returns whether this error is retryable.
    ///
    /// Some errors (like network errors or transient daemon issues) may be
    /// worth retrying, while others (like invalid configuration) are not.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ErrorClassification::Network
                | ErrorClassification::Daemon
                | ErrorClassification::Cache
                | ErrorClassification::Proxy
        )
    }

    /// Returns whether this error is likely a user mistake.
    ///
    /// This helps determine whether to show helpful guidance to the user.
    pub fn is_user_error(&self) -> bool {
        matches!(
            self,
            ErrorClassification::Configuration
                | ErrorClassification::UserInput
                | ErrorClassification::Dependency
        )
    }

    /// Returns whether this error indicates an internal bug.
    ///
    /// These errors should be reported and investigated.
    pub fn is_internal_error(&self) -> bool {
        matches!(self, ErrorClassification::Internal)
    }

    /// Returns a human-readable category name for this classification.
    pub fn category_name(&self) -> &'static str {
        match self {
            ErrorClassification::Configuration => "Configuration",
            ErrorClassification::Authentication => "Authentication",
            ErrorClassification::Network => "Network",
            ErrorClassification::FileSystem => "File System",
            ErrorClassification::ProcessExecution => "Process Execution",
            ErrorClassification::UserInput => "User Input",
            ErrorClassification::Internal => "Internal",
            ErrorClassification::Dependency => "Dependency",
            ErrorClassification::Cache => "Cache",
            ErrorClassification::TaskExecution => "Task Execution",
            ErrorClassification::Daemon => "Daemon",
            ErrorClassification::Environment => "Environment",
            ErrorClassification::Parsing => "Parsing",
            ErrorClassification::Proxy => "Proxy",
        }
    }
}

impl fmt::Display for ErrorClassification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.category_name())
    }
}

/// Trait for types that can be classified into error categories.
///
/// Implement this trait to provide error classification for your error types.
pub trait Classify {
    /// Returns the classification for this error.
    fn classify(&self) -> ErrorClassification;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_codes_are_valid() {
        for classification in [
            ErrorClassification::Configuration,
            ErrorClassification::Authentication,
            ErrorClassification::Network,
            ErrorClassification::FileSystem,
            ErrorClassification::ProcessExecution,
            ErrorClassification::UserInput,
            ErrorClassification::Internal,
            ErrorClassification::Dependency,
            ErrorClassification::Cache,
            ErrorClassification::TaskExecution,
            ErrorClassification::Daemon,
            ErrorClassification::Environment,
            ErrorClassification::Parsing,
            ErrorClassification::Proxy,
        ] {
            let exit_code = classification.exit_code();
            assert!(
                exit_code > 0 && exit_code <= 255,
                "Exit code for {:?} should be between 1 and 255",
                classification
            );
        }
    }

    #[test]
    fn test_retryable_classifications() {
        assert!(ErrorClassification::Network.is_retryable());
        assert!(ErrorClassification::Daemon.is_retryable());
        assert!(ErrorClassification::Cache.is_retryable());
        assert!(ErrorClassification::Proxy.is_retryable());

        assert!(!ErrorClassification::UserInput.is_retryable());
        assert!(!ErrorClassification::Configuration.is_retryable());
        assert!(!ErrorClassification::Internal.is_retryable());
    }

    #[test]
    fn test_user_error_classifications() {
        assert!(ErrorClassification::Configuration.is_user_error());
        assert!(ErrorClassification::UserInput.is_user_error());
        assert!(ErrorClassification::Dependency.is_user_error());

        assert!(!ErrorClassification::Internal.is_user_error());
        assert!(!ErrorClassification::Network.is_user_error());
    }

    #[test]
    fn test_internal_error_classification() {
        assert!(ErrorClassification::Internal.is_internal_error());

        assert!(!ErrorClassification::UserInput.is_internal_error());
        assert!(!ErrorClassification::Network.is_internal_error());
    }

    #[test]
    fn test_category_names() {
        assert_eq!(
            ErrorClassification::Configuration.category_name(),
            "Configuration"
        );
        assert_eq!(ErrorClassification::Network.category_name(), "Network");
        assert_eq!(ErrorClassification::Internal.category_name(), "Internal");
    }

    #[test]
    fn test_display() {
        assert_eq!(
            ErrorClassification::Configuration.to_string(),
            "Configuration"
        );
        assert_eq!(ErrorClassification::Network.to_string(), "Network");
    }
}
