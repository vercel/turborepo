//! Runtime helpers for [turbo-tasks-macro].

use crate::debug::ValueDebugFormat;

pub async fn format_field(value: &dyn ValueDebugFormat, depth: usize) -> String {
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
