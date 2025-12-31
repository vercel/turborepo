//! Validation utilities for task names and definitions.

use thiserror::Error;
use turborepo_errors::Spanned;

use crate::InvalidTaskNameError;

/// Invalid tokens that are not allowed in task names.
const INVALID_TOKENS: &[&str] = &["$colon$"];

/// Error type for validation operations.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    InvalidTaskName(#[from] Box<InvalidTaskNameError>),
}

/// Result of checking if a task has a definition in the current run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskDefinitionResult {
    /// True if the task has a valid definition.
    has_definition: bool,
    /// True if the task was excluded via `extends: false` somewhere in the
    /// chain.
    is_excluded: bool,
}

impl TaskDefinitionResult {
    fn new(has_definition: bool, is_excluded: bool) -> Self {
        Self {
            has_definition,
            is_excluded,
        }
    }

    /// Creates a result indicating no definition was found.
    pub fn not_found() -> Self {
        Self::new(false, false)
    }

    /// Creates a result indicating the task was explicitly excluded.
    pub fn excluded() -> Self {
        Self::new(false, true)
    }

    /// Creates a result indicating a definition was found.
    pub fn found() -> Self {
        Self::new(true, false)
    }

    /// Returns true if the task has a valid definition.
    pub fn has_definition(&self) -> bool {
        self.has_definition
    }

    /// Returns true if the task was excluded.
    pub fn is_excluded(&self) -> bool {
        self.is_excluded
    }
}

/// Validates a task name, returning an error if it contains invalid tokens.
pub fn validate_task_name(task: Spanned<&str>) -> Result<(), Error> {
    INVALID_TOKENS
        .iter()
        .find(|token| task.contains(**token))
        .map(|found_token| {
            let (span, text) = task.span_and_text("turbo.json");
            Err(Error::InvalidTaskName(Box::new(InvalidTaskNameError::new(
                span,
                text,
                task.to_string(),
                format!("task contains invalid string '{found_token}'"),
            ))))
        })
        .unwrap_or(Ok(()))
}

#[cfg(test)]
mod tests {
    use test_case::test_case;
    use turborepo_errors::Spanned;

    use super::validate_task_name;

    #[allow(clippy::duplicated_attributes)]
    #[test_case("build", None ; "simple_task_name")]
    #[test_case("build:prod", None ; "task_name_with_colon")]
    #[test_case("build$colon$prod", Some("task contains invalid string '$colon$'") ; "task_with_invalid_colon_token")]
    fn test_validate_task_name(task_name: &str, expected_error: Option<&str>) {
        let result = validate_task_name(Spanned::new(task_name))
            .map_err(|e| e.to_string())
            .err();

        if let Some(expected_reason) = expected_error {
            let error_msg = result.as_ref().expect("Expected an error but got Ok");
            assert!(
                error_msg.contains(expected_reason),
                "Error message '{}' should contain '{}'",
                error_msg,
                expected_reason
            );
        } else {
            assert!(result.is_none(), "Expected Ok but got error: {:?}", result);
        }
    }
}
