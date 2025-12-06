use crate::config::ExperimentalOtelOptions;

/// Initialize an OpenTelemetry observability handle from configuration options.
/// Returns `None` when OTel is disabled at compile time.
pub(crate) fn try_init_otel(_options: &ExperimentalOtelOptions) -> Option<super::Handle> {
    None
}
