//! Runtime helpers for [turbo-tasks-macro].
use anyhow::{Error, Result};
pub use once_cell::sync::{Lazy, OnceCell};
pub use tracing;

pub use super::manager::{find_cell_by_type, notify_scheduled_tasks, spawn_detached};
use crate::debug::ValueDebugFormatString;

#[inline(never)]
pub async fn value_debug_format_field(value: ValueDebugFormatString<'_>) -> String {
    match value.try_to_value_debug_string().await {
        Ok(result) => match result.await {
            Ok(result) => result.to_string(),
            Err(err) => format!("{0:?}", err),
        },
        Err(err) => format!("{0:?}", err),
    }
}

pub fn ok_or_else_for_missing_element<T>(result: Option<T>, field_ident: &str) -> Result<T> {
    match result {
        Some(result) => Ok(result),
        None => Err(error_for_missing_element(field_ident)),
    }
}

#[cold]
#[inline(never)]
fn error_for_missing_element(field_ident: &str) -> Error {
    anyhow::anyhow!("missing element for {}", field_ident)
}
