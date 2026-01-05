use turborepo_config::ExperimentalOtelOptions;

/// Initialize an OpenTelemetry observability handle from configuration options.
/// Returns `None` when OTel is disabled at compile time.
pub(crate) fn try_init_otel(
    _options: &ExperimentalOtelOptions,
    _token: Option<&str>,
) -> Option<super::Handle> {
    None
}
