//! Runtime helpers for [turbo-tasks-macro].
pub use once_cell::sync::{Lazy, OnceCell};
pub use tracing;

pub use super::manager::{find_cell_by_type, notify_scheduled_tasks, spawn_detached};
use crate::debug::ValueDebugFormat;

#[inline(never)]
pub async fn value_debug_format_field(value: &dyn ValueDebugFormat, depth: usize) -> String {
    match value
        .value_debug_format(depth.saturating_sub(1))
        .try_to_value_debug_string()
        .await
    {
        Ok(result) => match result.await {
            Ok(result) => result.to_string(),
            Err(err) => {
                format!("{0:?}", err)
            }
        },
        Err(err) => {
            format!("{0:?}", err)
        }
    }
}
