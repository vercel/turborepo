use std::{sync::Arc, time::Duration};

use turborepo_otel::{RunMetricsPayload, TaskCacheStatus, TaskMetricsPayload};

use super::{Handle, RunObserver};
use crate::{
    config::{ExperimentalOtelMetricsOptions, ExperimentalOtelOptions, ExperimentalOtelProtocol},
    run::summary::{CacheStatus, RunSummary, TaskSummary},
};

/// OpenTelemetry-based observer implementation.
struct OtelObserver {
    handle: turborepo_otel::Handle,
}

impl RunObserver for OtelObserver {
    fn record(&self, summary: &RunSummary<'_>) {
        if let Some(payload) = build_payload(summary) {
            self.handle.record_run(&payload);
        }
    }

    fn shutdown(&self) {
        // `shutdown` consumes the handle. Clone the cheap, Arc-backed handle first.
        self.handle.clone().shutdown();
    }
}

/// Initialize an OpenTelemetry observability handle from configuration options.
/// Returns `None` if observability is disabled or misconfigured.
pub(crate) fn try_init_otel(options: &ExperimentalOtelOptions) -> Option<Handle> {
    let config = config_from_options(options)?;

    match turborepo_otel::Handle::try_new(config) {
        Ok(handle) => Some(Handle {
            inner: Arc::new(OtelObserver { handle }),
        }),
        Err(e) => {
            tracing::warn!("Failed to initialize OTel exporter: {}", e);
            None
        }
    }
}

fn config_from_options(options: &ExperimentalOtelOptions) -> Option<turborepo_otel::Config> {
    if options.enabled.is_some_and(|enabled| !enabled) {
        return None;
    }

    let endpoint = options.endpoint.as_ref()?.trim();
    if endpoint.is_empty() {
        return None;
    }
    let endpoint = endpoint.to_string();

    let protocol = match options.protocol.unwrap_or(ExperimentalOtelProtocol::Grpc) {
        ExperimentalOtelProtocol::Grpc => turborepo_otel::Protocol::Grpc,
        ExperimentalOtelProtocol::HttpProtobuf => turborepo_otel::Protocol::HttpProtobuf,
    };

    let headers = options.headers.clone().unwrap_or_default();
    let resource_attributes = options.resource.clone().unwrap_or_default();
    let metrics = metrics_config(options.metrics.as_ref());
    let timeout = Duration::from_millis(options.timeout_ms.unwrap_or(10_000));

    Some(turborepo_otel::Config {
        endpoint,
        protocol,
        headers,
        timeout,
        resource_attributes,
        metrics,
    })
}

fn metrics_config(
    options: Option<&ExperimentalOtelMetricsOptions>,
) -> turborepo_otel::MetricsConfig {
    let run_summary = options.and_then(|opts| opts.run_summary).unwrap_or(true);
    let task_details = options.and_then(|opts| opts.task_details).unwrap_or(false);
    turborepo_otel::MetricsConfig {
        run_summary,
        task_details,
    }
}

fn build_payload(summary: &RunSummary<'_>) -> Option<RunMetricsPayload> {
    let execution = summary.execution_summary()?;
    let duration_ms = (execution.end_time - execution.start_time) as f64;
    let attempted_tasks = execution.attempted() as u64;
    let failed_tasks = execution.failed() as u64;
    let cached_tasks = execution.cached() as u64;

    let tasks = summary
        .tasks()
        .iter()
        .map(build_task_payload)
        .collect::<Vec<_>>();

    let scm = summary.scm_state();

    Some(RunMetricsPayload {
        run_id: summary.id().to_string(),
        turbo_version: summary.turbo_version().to_string(),
        duration_ms,
        attempted_tasks,
        failed_tasks,
        cached_tasks,
        exit_code: execution.exit_code,
        scm_branch: scm.branch().map(|s| s.to_string()),
        scm_revision: scm.sha().map(|s| s.to_string()),
        tasks,
    })
}

fn build_task_payload(task: &TaskSummary) -> TaskMetricsPayload {
    let duration_ms = task
        .shared
        .execution
        .as_ref()
        .map(|exec| (exec.end_time - exec.start_time) as f64);
    let exit_code = task
        .shared
        .execution
        .as_ref()
        .and_then(|exec| exec.exit_code);
    let cache_status = match task.shared.cache.status() {
        CacheStatus::Hit => TaskCacheStatus::Hit,
        CacheStatus::Miss => TaskCacheStatus::Miss,
    };
    let cache_source = task
        .shared
        .cache
        .cache_source_label()
        .map(|label| label.to_string());
    let cache_time_saved_ms = match cache_status {
        TaskCacheStatus::Hit => {
            let saved = task.shared.cache.time_saved();
            (saved > 0).then_some(saved)
        }
        TaskCacheStatus::Miss => None,
    };

    TaskMetricsPayload {
        task_id: task.task_id.to_string(),
        task: task.task.clone(),
        package: task.package.clone(),
        hash: task.shared.hash.clone(),
        external_inputs_hash: task.shared.hash_of_external_dependencies.clone(),
        command: task.shared.command.clone(),
        duration_ms,
        cache_status,
        cache_source,
        cache_time_saved_ms,
        exit_code,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn test_config_from_options_enabled_false() {
        let options = ExperimentalOtelOptions {
            enabled: Some(false),
            ..Default::default()
        };
        let result = config_from_options(&options);
        assert!(result.is_none());
    }

    #[test]
    fn test_config_from_options_no_endpoint() {
        let options = ExperimentalOtelOptions::default();
        let result = config_from_options(&options);
        assert!(result.is_none());
    }

    #[test]
    fn test_config_from_options_empty_endpoint() {
        let options = ExperimentalOtelOptions {
            endpoint: Some("   ".to_string()),
            ..Default::default()
        };
        let result = config_from_options(&options);
        assert!(result.is_none());
    }

    #[test]
    fn test_config_from_options_defaults() {
        let options = ExperimentalOtelOptions {
            endpoint: Some("https://example.com/otel".to_string()),
            ..Default::default()
        };
        let result = config_from_options(&options);
        assert!(result.is_some());
        let config = result.unwrap();
        assert_eq!(config.endpoint, "https://example.com/otel");
        assert_eq!(config.protocol, turborepo_otel::Protocol::Grpc);
        assert_eq!(config.timeout.as_millis(), 10_000);
        assert!(config.metrics.run_summary);
        assert!(!config.metrics.task_details);
    }

    #[test]
    fn test_config_from_options_http_protobuf() {
        let options = ExperimentalOtelOptions {
            endpoint: Some("https://example.com/otel".to_string()),
            protocol: Some(ExperimentalOtelProtocol::HttpProtobuf),
            ..Default::default()
        };
        let result = config_from_options(&options);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().protocol,
            turborepo_otel::Protocol::HttpProtobuf
        );
    }

    #[test]
    fn test_config_from_options_custom_timeout() {
        let options = ExperimentalOtelOptions {
            endpoint: Some("https://example.com/otel".to_string()),
            timeout_ms: Some(15000),
            ..Default::default()
        };
        let result = config_from_options(&options);
        assert!(result.is_some());
        assert_eq!(result.unwrap().timeout.as_millis(), 15_000);
    }

    #[test]
    fn test_config_from_options_headers() {
        let mut headers = BTreeMap::new();
        headers.insert("auth".to_string(), "token123".to_string());
        let options = ExperimentalOtelOptions {
            endpoint: Some("https://example.com/otel".to_string()),
            headers: Some(headers),
            ..Default::default()
        };
        let result = config_from_options(&options);
        assert!(result.is_some());
        let config = result.unwrap();
        assert_eq!(config.headers.get("auth"), Some(&"token123".to_string()));
    }

    #[test]
    fn test_config_from_options_resource() {
        let mut resource = BTreeMap::new();
        resource.insert("service.name".to_string(), "my-service".to_string());
        resource.insert("env".to_string(), "production".to_string());
        let options = ExperimentalOtelOptions {
            endpoint: Some("https://example.com/otel".to_string()),
            resource: Some(resource),
            ..Default::default()
        };
        let result = config_from_options(&options);
        assert!(result.is_some());
        let config = result.unwrap();
        assert_eq!(
            config.resource_attributes.get("service.name"),
            Some(&"my-service".to_string())
        );
        assert_eq!(
            config.resource_attributes.get("env"),
            Some(&"production".to_string())
        );
    }

    #[test]
    fn test_metrics_config_defaults() {
        let result = metrics_config(None);
        assert!(result.run_summary);
        assert!(!result.task_details);
    }

    #[test]
    fn test_metrics_config_run_summary_override() {
        let metrics = ExperimentalOtelMetricsOptions {
            run_summary: Some(false),
            ..Default::default()
        };
        let result = metrics_config(Some(&metrics));
        assert!(!result.run_summary);
        assert!(!result.task_details);
    }

    #[test]
    fn test_metrics_config_task_details_override() {
        let metrics = ExperimentalOtelMetricsOptions {
            task_details: Some(true),
            ..Default::default()
        };
        let result = metrics_config(Some(&metrics));
        assert!(result.run_summary);
        assert!(result.task_details);
    }
}
