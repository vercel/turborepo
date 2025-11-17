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
