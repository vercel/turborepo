use std::{sync::Arc, time::Duration};

use turborepo_config::{ExperimentalOtelMetricsOptions, ExperimentalOtelOptions};
use turborepo_otel::{RunMetricsPayload, TaskCacheStatus, TaskMetricsPayload};

use super::{Handle, RunObserver};
use crate::{
    RunSummary,
    task::{CacheStatus, TaskSummary},
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
pub(crate) fn try_init_otel(
    options: &ExperimentalOtelOptions,
    token: Option<&str>,
) -> Option<Handle> {
    let config = config_from_options(options, token)?;

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

fn config_from_options(
    options: &ExperimentalOtelOptions,
    token: Option<&str>,
) -> Option<turborepo_otel::Config> {
    if options.enabled.is_some_and(|enabled| !enabled) {
        return None;
    }

    let endpoint = options.endpoint.as_ref()?.trim();
    if endpoint.is_empty() {
        return None;
    }
    let endpoint = endpoint.to_string();

    // ExperimentalOtelProtocol is a re-export of turborepo_otel::Protocol,
    // so no conversion is needed.
    let protocol = options.protocol.unwrap_or_default();

    let headers = options.headers.clone().unwrap_or_default();
    let resource_attributes = options.resource.clone().unwrap_or_default();
    let metrics = metrics_config(options.metrics.as_ref());
    let timeout = Duration::from_millis(options.timeout_ms.unwrap_or(10_000));
    let interval = Duration::from_millis(options.interval_ms.unwrap_or(15_000));

    let mut config = turborepo_otel::Config {
        endpoint,
        protocol,
        headers,
        timeout,
        interval,
        resource_attributes,
        metrics,
    };

    apply_auth_token(&mut config, token);

    Some(config)
}

fn apply_auth_token(config: &mut turborepo_otel::Config, token: Option<&str>) {
    if config
        .headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("Authorization"))
    {
        return;
    }
    if let Some(token) = token {
        config
            .headers
            .insert("Authorization".to_string(), format!("Bearer {}", token));
    }
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

    use turborepo_config::ExperimentalOtelProtocol;

    use super::*;

    fn make_config(headers: BTreeMap<String, String>) -> turborepo_otel::Config {
        turborepo_otel::Config {
            endpoint: "https://example.com/otel".to_string(),
            protocol: turborepo_otel::Protocol::Grpc,
            headers,
            timeout: Duration::from_millis(10_000),
            interval: Duration::from_millis(15_000),
            resource_attributes: BTreeMap::new(),
            metrics: turborepo_otel::MetricsConfig {
                run_summary: true,
                task_details: false,
            },
        }
    }

    #[test]
    fn config_from_options_returns_none_for_invalid_configs() {
        let cases: &[(&str, ExperimentalOtelOptions)] = &[
            (
                "explicitly disabled",
                ExperimentalOtelOptions {
                    enabled: Some(false),
                    endpoint: Some("https://example.com".to_string()),
                    ..Default::default()
                },
            ),
            ("no endpoint", ExperimentalOtelOptions::default()),
            (
                "empty endpoint",
                ExperimentalOtelOptions {
                    endpoint: Some("".to_string()),
                    ..Default::default()
                },
            ),
            (
                "whitespace endpoint",
                ExperimentalOtelOptions {
                    endpoint: Some("   ".to_string()),
                    ..Default::default()
                },
            ),
        ];

        for (name, options) in cases {
            assert!(
                config_from_options(options, None).is_none(),
                "case '{}': expected None",
                name
            );
        }
    }

    #[test]
    fn config_from_options_applies_defaults() {
        let options = ExperimentalOtelOptions {
            endpoint: Some("https://example.com/otel".to_string()),
            ..Default::default()
        };

        let config = config_from_options(&options, None).expect("should create config");

        assert_eq!(config.endpoint, "https://example.com/otel");
        assert_eq!(config.protocol, turborepo_otel::Protocol::Grpc);
        assert_eq!(config.timeout.as_millis(), 10_000);
        assert_eq!(config.interval.as_millis(), 15_000);
        assert!(config.headers.is_empty());
        assert!(config.resource_attributes.is_empty());
        assert!(config.metrics.run_summary);
        assert!(!config.metrics.task_details);
    }

    #[test]
    fn config_from_options_respects_custom_values() {
        let mut headers = BTreeMap::new();
        headers.insert("X-Custom".to_string(), "value".to_string());
        let mut resource = BTreeMap::new();
        resource.insert("service.name".to_string(), "my-service".to_string());

        let options = ExperimentalOtelOptions {
            endpoint: Some("  https://example.com/otel  ".to_string()),
            protocol: Some(ExperimentalOtelProtocol::HttpProtobuf),
            timeout_ms: Some(5000),
            interval_ms: Some(30000),
            headers: Some(headers),
            resource: Some(resource),
            metrics: Some(ExperimentalOtelMetricsOptions {
                run_summary: Some(false),
                task_details: Some(true),
            }),
            ..Default::default()
        };

        let config = config_from_options(&options, None).expect("should create config");

        assert_eq!(config.endpoint, "https://example.com/otel");
        assert_eq!(config.protocol, turborepo_otel::Protocol::HttpProtobuf);
        assert_eq!(config.timeout.as_millis(), 5000);
        assert_eq!(config.interval.as_millis(), 30000);
        assert_eq!(config.headers.get("X-Custom"), Some(&"value".to_string()));
        assert_eq!(
            config.resource_attributes.get("service.name"),
            Some(&"my-service".to_string())
        );
        assert!(!config.metrics.run_summary);
        assert!(config.metrics.task_details);
    }

    #[test]
    fn metrics_config_applies_defaults_and_overrides() {
        let cases: &[(&str, Option<ExperimentalOtelMetricsOptions>, bool, bool)] = &[
            ("defaults", None, true, false),
            (
                "both overridden",
                Some(ExperimentalOtelMetricsOptions {
                    run_summary: Some(false),
                    task_details: Some(true),
                }),
                false,
                true,
            ),
            (
                "only run_summary overridden",
                Some(ExperimentalOtelMetricsOptions {
                    run_summary: Some(false),
                    task_details: None,
                }),
                false,
                false,
            ),
            (
                "only task_details overridden",
                Some(ExperimentalOtelMetricsOptions {
                    run_summary: None,
                    task_details: Some(true),
                }),
                true,
                true,
            ),
        ];

        for (name, options, expected_run_summary, expected_task_details) in cases {
            let result = metrics_config(options.as_ref());
            assert_eq!(
                result.run_summary, *expected_run_summary,
                "case '{}': run_summary mismatch",
                name
            );
            assert_eq!(
                result.task_details, *expected_task_details,
                "case '{}': task_details mismatch",
                name
            );
        }
    }

    #[test]
    fn config_from_options_applies_auth_token() {
        let options = ExperimentalOtelOptions {
            endpoint: Some("https://example.com/otel".to_string()),
            ..Default::default()
        };

        let config = config_from_options(&options, Some("my-token")).expect("should create config");

        assert_eq!(
            config.headers.get("Authorization"),
            Some(&"Bearer my-token".to_string())
        );
    }

    #[test]
    fn config_from_options_preserves_existing_auth_header() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer user-token".to_string());

        let options = ExperimentalOtelOptions {
            endpoint: Some("https://example.com/otel".to_string()),
            headers: Some(headers),
            ..Default::default()
        };

        let config = config_from_options(&options, Some("remote-cache-token"))
            .expect("should create config");

        assert_eq!(
            config.headers.get("Authorization"),
            Some(&"Bearer user-token".to_string()),
            "should preserve user-provided Authorization header"
        );
    }

    #[test]
    fn apply_auth_token_behavior() {
        struct Case {
            name: &'static str,
            existing_header: Option<(&'static str, &'static str)>,
            token: Option<&'static str>,
            expect_auth: Option<&'static str>,
        }

        let cases = [
            Case {
                name: "adds token when no existing header",
                existing_header: None,
                token: Some("my-token"),
                expect_auth: Some("Bearer my-token"),
            },
            Case {
                name: "skips when no token provided",
                existing_header: None,
                token: None,
                expect_auth: None,
            },
            Case {
                name: "preserves existing Authorization header",
                existing_header: Some(("Authorization", "Bearer existing")),
                token: Some("new-token"),
                expect_auth: Some("Bearer existing"),
            },
            Case {
                name: "case-insensitive: skips when lowercase authorization exists",
                existing_header: Some(("authorization", "Bearer existing")),
                token: Some("new-token"),
                // Case-insensitive check finds "authorization", so no new header is added.
                // We check for "Authorization" (capitalized) which won't exist since
                // the original was lowercase and we didn't add a new one.
                expect_auth: None,
            },
            Case {
                name: "case-insensitive: skips when AUTHORIZATION exists",
                existing_header: Some(("AUTHORIZATION", "Bearer existing")),
                token: Some("new-token"),
                // Case-insensitive check finds "AUTHORIZATION", so no new header is added.
                // We check for "Authorization" (mixed case) which won't exist since
                // the original was uppercase and we didn't add a new one.
                expect_auth: None,
            },
        ];

        for case in cases {
            let mut headers = BTreeMap::new();
            if let Some((key, value)) = case.existing_header {
                headers.insert(key.to_string(), value.to_string());
            }
            let mut config = make_config(headers);

            apply_auth_token(&mut config, case.token);

            assert_eq!(
                config.headers.get("Authorization").map(|s| s.as_str()),
                case.expect_auth,
                "case '{}': Authorization header mismatch",
                case.name
            );

            if let Some((key, value)) = case.existing_header {
                assert_eq!(
                    config.headers.get(key),
                    Some(&value.to_string()),
                    "case '{}': existing header should be preserved",
                    case.name
                );
            }
        }
    }
}
