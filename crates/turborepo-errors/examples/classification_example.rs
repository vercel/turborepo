//! Example demonstrating the error classification system
//!
//! This example shows how to implement the `Classify` trait for custom error
//! types and use the classification system for consistent error handling.
//!
//! Run with: cargo run --example classification_example

use thiserror::Error;
use turborepo_errors::{Classify, ErrorClassification};

#[derive(Debug, Error)]
enum CustomError {
    #[error("Configuration file not found: {0}")]
    ConfigNotFound(String),

    #[error("Network connection failed: {0}")]
    NetworkError(String),

    #[error("Internal error occurred: {0}")]
    InternalError(String),

    #[error("Invalid user input: {0}")]
    InvalidInput(String),
}

impl Classify for CustomError {
    fn classify(&self) -> ErrorClassification {
        match self {
            CustomError::ConfigNotFound(_) => ErrorClassification::Configuration,
            CustomError::NetworkError(_) => ErrorClassification::Network,
            CustomError::InternalError(_) => ErrorClassification::Internal,
            CustomError::InvalidInput(_) => ErrorClassification::UserInput,
        }
    }
}

fn handle_error(error: &dyn Classify, error_display: &str) {
    let classification = error.classify();

    println!("Error: {}", error_display);
    println!("Category: {}", classification.category_name());
    println!("Exit code: {}", classification.exit_code());
    println!("Retryable: {}", classification.is_retryable());

    if classification.is_user_error() {
        println!(
            "💡 Tip: This appears to be a user error. Please check your configuration or input."
        );
    }

    if classification.is_internal_error() {
        println!("🐛 This is an internal error. Please report this issue.");
    }

    if classification.is_retryable() {
        println!("🔄 This error may be transient. You might want to retry.");
    }

    println!();
}

fn main() {
    println!("Error Classification System Example\n");
    println!("=====================================\n");

    let errors = vec![
        CustomError::ConfigNotFound("turbo.json".to_string()),
        CustomError::NetworkError("connection timeout".to_string()),
        CustomError::InternalError("unexpected state".to_string()),
        CustomError::InvalidInput("invalid flag --foo".to_string()),
    ];

    for (i, error) in errors.iter().enumerate() {
        println!("Example {}:", i + 1);
        handle_error(error, &error.to_string());
    }

    println!("Classification Categories:");
    println!("=========================\n");

    let classifications = [
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
    ];

    for classification in classifications {
        println!(
            "{:<20} | Exit Code: {:>3} | Retryable: {}",
            classification.category_name(),
            classification.exit_code(),
            if classification.is_retryable() {
                "Yes"
            } else {
                "No "
            }
        );
    }
}
