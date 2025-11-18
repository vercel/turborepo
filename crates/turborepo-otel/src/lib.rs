use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, Meter, MeterProvider as _},
};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig, WithTonicConfig};
use opentelemetry_sdk::{
    Resource,
    metrics::{PeriodicReader, SdkMeterProvider, Temporality},
};
use opentelemetry_semantic_conventions::resource::SERVICE_NAME;
use thiserror::Error;
use tonic::metadata::{MetadataKey, MetadataMap, MetadataValue};
use tracing::warn;

/// Protocol supported by the OTLP exporter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Grpc,
    HttpProtobuf,
}

/// Metric toggle configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MetricsConfig {
    pub run_summary: bool,
    pub task_details: bool,
}

/// Resolved configuration for the exporter.
#[derive(Debug, Clone)]
pub struct Config {
    pub endpoint: String,
    pub protocol: Protocol,
    pub headers: BTreeMap<String, String>,
    pub timeout: Duration,
    pub resource_attributes: BTreeMap<String, String>,
    pub metrics: MetricsConfig,
}

/// Summary of a Turborepo run encoded for metrics export.
#[derive(Debug)]
pub struct RunMetricsPayload {
    pub run_id: String,
    pub turbo_version: String,
    pub duration_ms: f64,
    pub attempted_tasks: u64,
    pub failed_tasks: u64,
    pub cached_tasks: u64,
    pub exit_code: i32,
    pub scm_branch: Option<String>,
    pub scm_revision: Option<String>,
    pub tasks: Vec<TaskMetricsPayload>,
}

/// Per-task metrics details.
#[derive(Debug)]
pub struct TaskMetricsPayload {
    pub task_id: String,
    pub task: String,
    pub package: String,
    pub hash: String,
    pub external_inputs_hash: String,
    pub command: String,
    pub duration_ms: Option<f64>,
    pub cache_status: TaskCacheStatus,
    pub cache_source: Option<String>,
    pub cache_time_saved_ms: Option<u64>,
    pub exit_code: Option<i32>,
}

/// Cache status for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCacheStatus {
    Hit,
    Miss,
}

impl TaskCacheStatus {
    fn as_str(&self) -> &'static str {
        match self {
            TaskCacheStatus::Hit => "hit",
            TaskCacheStatus::Miss => "miss",
        }
    }
}

/// Errors that can occur while configuring or using the exporter.
#[derive(Error, Debug)]
pub enum Error {
    #[error("experimentalOtel requires an endpoint")]
    MissingEndpoint,
    #[error("failed to build OTLP exporter: {0}")]
    Exporter(opentelemetry_otlp::ExporterBuildError),
    #[error("invalid OTLP header `{0}`")]
    InvalidHeader(String),
}

struct Instruments {
    run_duration: Histogram<f64>,
    run_attempted: Counter<u64>,
    run_failed: Counter<u64>,
    run_cached: Counter<u64>,
    task_duration: Histogram<f64>,
    task_cache: Counter<u64>,
}

struct HandleInner {
    provider: SdkMeterProvider,
    instruments: Arc<Instruments>,
    metrics: MetricsConfig,
}

/// Handle to the configured exporter.
#[derive(Clone)]
pub struct Handle {
    inner: Arc<HandleInner>,
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle").finish_non_exhaustive()
    }
}

impl Handle {
    pub fn try_new(config: Config) -> Result<Self, Error> {
        if config.endpoint.trim().is_empty() {
            return Err(Error::MissingEndpoint);
        }

        let provider = build_provider(&config)?;
        let meter = provider.meter("turborepo");
        let instruments = Arc::new(create_instruments(&meter));

        Ok(Self {
            inner: Arc::new(HandleInner {
                provider,
                instruments,
                metrics: config.metrics,
            }),
        })
    }

    pub fn record_run(&self, payload: &RunMetricsPayload) {
        if self.inner.metrics.run_summary {
            self.inner.instruments.record_run_summary(payload);
        }
        if self.inner.metrics.task_details {
            self.inner.instruments.record_task_details(payload);
        }
    }

    pub fn shutdown(self) {
        match Arc::try_unwrap(self.inner) {
            Ok(inner) => {
                if let Err(err) = inner.provider.shutdown() {
                    warn!("failed to shutdown otel exporter: {err}");
                }
            }
            Err(shared) => {
                if let Err(err) = shared.provider.shutdown() {
                    warn!("failed to shutdown otel exporter: {err}");
                }
            }
        }
    }
}

impl Instruments {
    fn record_run_summary(&self, payload: &RunMetricsPayload) {
        let attrs = build_run_attributes(payload);
        self.run_duration.record(payload.duration_ms, &attrs);
        self.run_attempted.add(payload.attempted_tasks, &attrs);
        self.run_failed.add(payload.failed_tasks, &attrs);
        self.run_cached.add(payload.cached_tasks, &attrs);
    }

    fn record_task_details(&self, payload: &RunMetricsPayload) {
        let base_attrs = build_run_attributes(payload);
        for task in payload.tasks.iter() {
            let mut attrs = base_attrs.clone();
            attrs.push(KeyValue::new("turbo.task.id", task.task_id.clone()));
            attrs.push(KeyValue::new("turbo.task.name", task.task.clone()));
            attrs.push(KeyValue::new("turbo.task.package", task.package.clone()));
            attrs.push(KeyValue::new("turbo.task.hash", task.hash.clone()));
            attrs.push(KeyValue::new(
                "turbo.task.external_inputs_hash",
                task.external_inputs_hash.clone(),
            ));
            attrs.push(KeyValue::new("turbo.task.command", task.command.clone()));
            attrs.push(KeyValue::new(
                "turbo.task.cache_status",
                task.cache_status.as_str(),
            ));
            if let Some(source) = &task.cache_source {
                attrs.push(KeyValue::new("turbo.task.cache_source", source.clone()));
            }
            if let Some(time_saved) = task.cache_time_saved_ms {
                attrs.push(KeyValue::new(
                    "turbo.task.cache_time_saved_ms",
                    time_saved as i64,
                ));
            }
            if let Some(exit_code) = task.exit_code {
                attrs.push(KeyValue::new("turbo.task.exit_code", exit_code as i64));
            }
            if let Some(duration) = task.duration_ms {
                self.task_duration.record(duration, &attrs);
            }
            self.task_cache.add(1, &attrs);
        }
    }
}

fn build_provider(config: &Config) -> Result<SdkMeterProvider, Error> {
    let resource = build_resource(config);

    let temporality = default_temporality();
    let exporter = match config.protocol {
        Protocol::Grpc => {
            let export_config = opentelemetry_otlp::ExportConfig {
                endpoint: Some(config.endpoint.clone()),
                protocol: opentelemetry_otlp::Protocol::Grpc,
                timeout: Some(config.timeout),
            };
            let mut builder = opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_temporality(temporality)
                .with_export_config(export_config);
            if !config.headers.is_empty() {
                builder = builder.with_metadata(build_metadata(&config.headers)?);
            }
            builder.build().map_err(Error::Exporter)?
        }
        Protocol::HttpProtobuf => {
            let export_config = opentelemetry_otlp::ExportConfig {
                endpoint: Some(config.endpoint.clone()),
                protocol: opentelemetry_otlp::Protocol::HttpBinary,
                timeout: Some(config.timeout),
            };
            let mut builder = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_temporality(temporality)
                .with_export_config(export_config);
            if !config.headers.is_empty() {
                let headers: HashMap<_, _> = config.headers.clone().into_iter().collect();
                builder = builder.with_headers(headers);
            }
            builder.build().map_err(Error::Exporter)?
        }
    };

    let reader = PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(15))
        .build();

    Ok(SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(reader)
        .build())
}

fn build_metadata(headers: &BTreeMap<String, String>) -> Result<MetadataMap, Error> {
    let mut map = MetadataMap::new();
    for (key, value) in headers {
        let metadata_key = MetadataKey::from_bytes(key.as_bytes())
            .map_err(|_| Error::InvalidHeader(key.clone()))?;
        let metadata_value = MetadataValue::try_from(value.as_str())
            .map_err(|_| Error::InvalidHeader(key.clone()))?;
        map.insert(metadata_key, metadata_value);
    }
    Ok(map)
}

fn build_resource(config: &Config) -> Resource {
    let mut attrs = Vec::with_capacity(config.resource_attributes.len() + 1);
    let service_name = config
        .resource_attributes
        .get("service.name")
        .cloned()
        .unwrap_or_else(|| "turborepo".to_string());
    attrs.push(KeyValue::new(SERVICE_NAME, service_name));
    for (key, value) in config.resource_attributes.iter() {
        if key == "service.name" {
            continue;
        }
        attrs.push(KeyValue::new(key.clone(), value.clone()));
    }
    Resource::builder_empty().with_attributes(attrs).build()
}

fn default_temporality() -> Temporality {
    Temporality::Cumulative
}

fn create_instruments(meter: &Meter) -> Instruments {
    let run_duration = meter
        .f64_histogram("turbo.run.duration_ms")
        .with_description("Turborepo run duration in milliseconds")
        .build();
    let run_attempted = meter
        .u64_counter("turbo.run.tasks.attempted")
        .with_description("Tasks attempted per run")
        .build();
    let run_failed = meter
        .u64_counter("turbo.run.tasks.failed")
        .with_description("Tasks failed per run")
        .build();
    let run_cached = meter
        .u64_counter("turbo.run.tasks.cached")
        .with_description("Tasks served from cache per run")
        .build();
    let task_duration = meter
        .f64_histogram("turbo.task.duration_ms")
        .with_description("Task execution duration in milliseconds")
        .build();
    let task_cache = meter
        .u64_counter("turbo.task.cache.events")
        .with_description("Cache hit/miss events")
        .build();

    Instruments {
        run_duration,
        run_attempted,
        run_failed,
        run_cached,
        task_duration,
        task_cache,
    }
}

fn build_run_attributes(payload: &RunMetricsPayload) -> Vec<KeyValue> {
    let mut attrs = Vec::with_capacity(6);
    attrs.push(KeyValue::new("turbo.run.id", payload.run_id.clone()));
    attrs.push(KeyValue::new(
        "turbo.run.exit_code",
        payload.exit_code.to_string(),
    ));
    attrs.push(KeyValue::new(
        "turbo.version",
        payload.turbo_version.clone(),
    ));
    if let Some(branch) = &payload.scm_branch {
        attrs.push(KeyValue::new("turbo.scm.branch", branch.clone()));
    }
    if let Some(revision) = &payload.scm_revision {
        attrs.push(KeyValue::new("turbo.scm.revision", revision.clone()));
    }
    attrs
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn test_handle_try_new_empty_endpoint() {
        let config = Config {
            endpoint: "".to_string(),
            protocol: Protocol::Grpc,
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(10),
            resource_attributes: BTreeMap::new(),
            metrics: MetricsConfig::default(),
        };
        let result = Handle::try_new(config);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::MissingEndpoint => {}
            _ => panic!("Expected MissingEndpoint error"),
        }
    }

    #[test]
    fn test_handle_try_new_whitespace_endpoint() {
        let config = Config {
            endpoint: "   ".to_string(),
            protocol: Protocol::Grpc,
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(10),
            resource_attributes: BTreeMap::new(),
            metrics: MetricsConfig::default(),
        };
        let result = Handle::try_new(config);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::MissingEndpoint => {}
            _ => panic!("Expected MissingEndpoint error"),
        }
    }

    #[test]
    fn test_build_metadata_valid() {
        let mut headers = BTreeMap::new();
        headers.insert("authorization".to_string(), "Bearer token123".to_string());
        headers.insert("x-custom-header".to_string(), "value".to_string());

        let result = build_metadata(&headers);
        assert!(result.is_ok());
        let metadata = result.unwrap();
        assert_eq!(metadata.len(), 2);
    }

    #[test]
    fn test_build_metadata_invalid_key() {
        let mut headers = BTreeMap::new();
        headers.insert("\0invalid".to_string(), "value".to_string());

        let result = build_metadata(&headers);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidHeader(key) => {
                assert_eq!(key, "\0invalid");
            }
            _ => panic!("Expected InvalidHeader error"),
        }
    }

    #[test]
    fn test_build_metadata_invalid_value() {
        let mut headers = BTreeMap::new();
        headers.insert("valid-key".to_string(), "\0invalid-value".to_string());

        let result = build_metadata(&headers);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidHeader(key) => {
                assert_eq!(key, "valid-key");
            }
            _ => panic!("Expected InvalidHeader error"),
        }
    }

    #[test]
    fn test_build_resource_default_service_name() {
        let config = Config {
            endpoint: "https://example.com".to_string(),
            protocol: Protocol::Grpc,
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(10),
            resource_attributes: BTreeMap::new(),
            metrics: MetricsConfig::default(),
        };
        let resource = build_resource(&config);
        let attrs: Vec<_> = resource
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert!(
            attrs
                .iter()
                .any(|(k, v)| *k == SERVICE_NAME && *v == "turborepo")
        );
    }

    #[test]
    fn test_build_resource_custom_service_name() {
        let mut resource_attrs = BTreeMap::new();
        resource_attrs.insert("service.name".to_string(), "my-service".to_string());
        let config = Config {
            endpoint: "https://example.com".to_string(),
            protocol: Protocol::Grpc,
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(10),
            resource_attributes: resource_attrs,
            metrics: MetricsConfig::default(),
        };
        let resource = build_resource(&config);
        let attrs: Vec<_> = resource
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert!(
            attrs
                .iter()
                .any(|(k, v)| *k == SERVICE_NAME && *v == "my-service")
        );
    }

    #[test]
    fn test_build_resource_additional_attributes() {
        let mut resource_attrs = BTreeMap::new();
        resource_attrs.insert("env".to_string(), "production".to_string());
        resource_attrs.insert("version".to_string(), "1.0.0".to_string());
        let config = Config {
            endpoint: "https://example.com".to_string(),
            protocol: Protocol::Grpc,
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(10),
            resource_attributes: resource_attrs,
            metrics: MetricsConfig::default(),
        };
        let resource = build_resource(&config);
        let attrs: Vec<_> = resource
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(attrs.len(), 3);
        assert!(
            attrs
                .iter()
                .any(|(k, v)| *k == SERVICE_NAME && *v == "turborepo")
        );
        assert!(attrs.iter().any(|(k, v)| *k == "env" && *v == "production"));
        assert!(attrs.iter().any(|(k, v)| *k == "version" && *v == "1.0.0"));
    }

    #[test]
    fn test_build_resource_no_duplicate_service_name() {
        let mut resource_attrs = BTreeMap::new();
        resource_attrs.insert("service.name".to_string(), "custom".to_string());
        resource_attrs.insert("env".to_string(), "production".to_string());
        let config = Config {
            endpoint: "https://example.com".to_string(),
            protocol: Protocol::Grpc,
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(10),
            resource_attributes: resource_attrs,
            metrics: MetricsConfig::default(),
        };
        let resource = build_resource(&config);
        let attrs: Vec<_> = resource
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let service_name_count = attrs.iter().filter(|(k, _)| *k == SERVICE_NAME).count();
        assert_eq!(service_name_count, 1);
        assert!(
            attrs
                .iter()
                .any(|(k, v)| *k == SERVICE_NAME && *v == "custom")
        );
    }
}
