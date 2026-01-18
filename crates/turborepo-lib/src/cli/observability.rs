use std::collections::BTreeMap;

use clap::Parser;

use crate::config::{
    ExperimentalOtelMetricsOptions, ExperimentalOtelOptions, ExperimentalOtelProtocol,
};

/// CLI arguments for experimental OpenTelemetry metrics export.
///
/// These flags allow configuring OTLP metrics export from the command line.
/// Configuration can also be set via environment variables
/// (`TURBO_EXPERIMENTAL_OTEL_*`) or in `turbo.json` under
/// `experimentalObservability.otel`.
///
/// Note: CLI flags and environment variables work independently of the
/// `futureFlags.experimentalObservability` setting. The future flag only
/// gates the `experimentalObservability` configuration in `turbo.json`.
#[derive(Parser, Clone, Debug, Default, PartialEq)]
pub struct ExperimentalOtelCliArgs {
    /// Enable or disable OpenTelemetry metrics export.
    #[clap(
        long = "experimental-otel-enabled",
        global = true,
        num_args = 0..=1,
        default_missing_value = "true",
        help = "Enable OpenTelemetry metrics export"
    )]
    pub enabled: Option<bool>,

    /// Transport protocol to use for the OTLP exporter.
    /// Supported values: grpc, http-protobuf
    #[clap(
        long = "experimental-otel-protocol",
        value_enum,
        global = true,
        value_name = "PROTOCOL",
        help = "OTLP transport protocol (grpc or http-protobuf)"
    )]
    pub protocol: Option<ExperimentalOtelProtocol>,

    /// OTLP endpoint URL (e.g., http://localhost:4317 for gRPC).
    #[clap(
        long = "experimental-otel-endpoint",
        global = true,
        value_name = "URL",
        help = "OTLP collector endpoint URL"
    )]
    pub endpoint: Option<String>,

    /// Timeout for OTLP export requests in milliseconds.
    #[clap(
        long = "experimental-otel-timeout-ms",
        global = true,
        value_name = "MILLISECONDS",
        help = "OTLP export timeout in milliseconds (default: 10000)"
    )]
    pub timeout_ms: Option<u64>,

    /// Additional headers to send with OTLP requests.
    /// Can be specified multiple times. Useful for authentication.
    #[clap(
        long = "experimental-otel-header",
        global = true,
        value_parser = parse_key_val_pair,
        value_name = "KEY=VALUE",
        help = "Add header to OTLP requests (can be repeated)"
    )]
    pub headers: Vec<(String, String)>,

    /// OpenTelemetry resource attributes to attach to all metrics.
    /// Can be specified multiple times (e.g., service.name=my-app).
    #[clap(
        long = "experimental-otel-resource",
        global = true,
        value_parser = parse_key_val_pair,
        value_name = "KEY=VALUE",
        help = "Add resource attribute to metrics (can be repeated)"
    )]
    pub resource_attributes: Vec<(String, String)>,

    /// Enable run-level summary metrics (duration, task counts).
    /// Enabled by default when OTEL is configured.
    #[clap(
        long = "experimental-otel-metrics-run-summary",
        global = true,
        num_args = 0..=1,
        default_missing_value = "true",
        help = "Emit run-level summary metrics (default: true)"
    )]
    pub metrics_run_summary: Option<bool>,

    /// Enable per-task detail metrics (individual task durations, cache
    /// status). Disabled by default due to higher cardinality.
    #[clap(
        long = "experimental-otel-metrics-task-details",
        global = true,
        num_args = 0..=1,
        default_missing_value = "true",
        help = "Emit per-task detail metrics (default: false)"
    )]
    pub metrics_task_details: Option<bool>,

    /// Use the Vercel remote cache authentication token for OTLP requests.
    /// Automatically adds an Authorization header with the token.
    #[clap(
        long = "experimental-otel-use-remote-cache-token",
        global = true,
        num_args = 0..=1,
        default_missing_value = "true",
        help = "Use remote cache token for OTLP authentication"
    )]
    pub use_remote_cache_token: Option<bool>,
}

impl ExperimentalOtelCliArgs {
    pub fn to_config(&self) -> Option<ExperimentalOtelOptions> {
        let mut options = ExperimentalOtelOptions::default();
        let mut touched = false;

        if let Some(enabled) = self.enabled {
            options.enabled = Some(enabled);
            touched = true;
        }
        if let Some(protocol) = self.protocol {
            options.protocol = Some(protocol);
            touched = true;
        }
        if let Some(endpoint) = &self.endpoint {
            options.endpoint = Some(endpoint.clone());
            touched = true;
        }
        if let Some(timeout) = self.timeout_ms {
            options.timeout_ms = Some(timeout);
            touched = true;
        }
        if !self.headers.is_empty() {
            let mut map = BTreeMap::new();
            for (key, value) in &self.headers {
                map.insert(key.clone(), value.clone());
            }
            options.headers = Some(map);
            touched = true;
        }
        if !self.resource_attributes.is_empty() {
            let mut map = BTreeMap::new();
            for (key, value) in &self.resource_attributes {
                map.insert(key.clone(), value.clone());
            }
            options.resource = Some(map);
            touched = true;
        }
        if let Some(value) = self.metrics_run_summary {
            options
                .metrics
                .get_or_insert_with(ExperimentalOtelMetricsOptions::default)
                .run_summary = Some(value);
            touched = true;
        }
        if let Some(value) = self.metrics_task_details {
            options
                .metrics
                .get_or_insert_with(ExperimentalOtelMetricsOptions::default)
                .task_details = Some(value);
            touched = true;
        }
        if let Some(value) = self.use_remote_cache_token {
            options.use_remote_cache_token = Some(value);
            touched = true;
        }

        touched.then_some(options)
    }
}

fn parse_key_val_pair(s: &str) -> Result<(String, String), String> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| "must be in key=value format".to_string())?;
    let key = key.trim();
    if key.is_empty() {
        return Err("key cannot be empty".to_string());
    }
    Ok((key.to_string(), value.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_experimental_otel_cli_args_empty() {
        let args = ExperimentalOtelCliArgs::default();
        let result = args.to_config();
        assert_eq!(result, None);
    }

    #[test]
    fn test_experimental_otel_cli_args_enabled() {
        let args = ExperimentalOtelCliArgs {
            enabled: Some(true),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        assert_eq!(result.unwrap().enabled, Some(true));
    }

    #[test]
    fn test_experimental_otel_cli_args_disabled() {
        let args = ExperimentalOtelCliArgs {
            enabled: Some(false),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        assert_eq!(result.unwrap().enabled, Some(false));
    }

    #[test]
    fn test_experimental_otel_cli_args_protocol() {
        let args = ExperimentalOtelCliArgs {
            protocol: Some(ExperimentalOtelProtocol::Grpc),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().protocol,
            Some(ExperimentalOtelProtocol::Grpc)
        );
    }

    #[test]
    fn test_experimental_otel_cli_args_protocol_http_protobuf() {
        let args = ExperimentalOtelCliArgs {
            protocol: Some(ExperimentalOtelProtocol::HttpProtobuf),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().protocol,
            Some(ExperimentalOtelProtocol::HttpProtobuf)
        );
    }

    #[test]
    fn test_experimental_otel_cli_args_endpoint() {
        let args = ExperimentalOtelCliArgs {
            endpoint: Some("https://example.com/otel".to_string()),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().endpoint,
            Some("https://example.com/otel".to_string())
        );
    }

    #[test]
    fn test_experimental_otel_cli_args_timeout_ms() {
        let args = ExperimentalOtelCliArgs {
            timeout_ms: Some(5000),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        assert_eq!(result.unwrap().timeout_ms, Some(5000));
    }

    #[test]
    fn test_experimental_otel_cli_args_headers_single() {
        let args = ExperimentalOtelCliArgs {
            headers: vec![("key1".to_string(), "value1".to_string())],
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let headers = result.unwrap().headers.unwrap();
        assert_eq!(headers.get("key1"), Some(&"value1".to_string()));
    }

    #[test]
    fn test_experimental_otel_cli_args_headers_multiple() {
        let args = ExperimentalOtelCliArgs {
            headers: vec![
                ("key1".to_string(), "value1".to_string()),
                ("key2".to_string(), "value2".to_string()),
            ],
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let headers = result.unwrap().headers.unwrap();
        assert_eq!(headers.get("key1"), Some(&"value1".to_string()));
        assert_eq!(headers.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_experimental_otel_cli_args_headers_empty() {
        let args = ExperimentalOtelCliArgs {
            headers: vec![],
            ..Default::default()
        };
        let result = args.to_config();
        assert_eq!(result, None);
    }

    #[test]
    fn test_experimental_otel_cli_args_resource_single() {
        let args = ExperimentalOtelCliArgs {
            resource_attributes: vec![("service.name".to_string(), "my-service".to_string())],
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let resource = result.unwrap().resource.unwrap();
        assert_eq!(
            resource.get("service.name"),
            Some(&"my-service".to_string())
        );
    }

    #[test]
    fn test_experimental_otel_cli_args_resource_multiple() {
        let args = ExperimentalOtelCliArgs {
            resource_attributes: vec![
                ("service.name".to_string(), "my-service".to_string()),
                ("env".to_string(), "production".to_string()),
            ],
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let resource = result.unwrap().resource.unwrap();
        assert_eq!(
            resource.get("service.name"),
            Some(&"my-service".to_string())
        );
        assert_eq!(resource.get("env"), Some(&"production".to_string()));
    }

    #[test]
    fn test_experimental_otel_cli_args_metrics_run_summary() {
        let args = ExperimentalOtelCliArgs {
            metrics_run_summary: Some(true),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.run_summary, Some(true));
    }

    #[test]
    fn test_experimental_otel_cli_args_metrics_task_details() {
        let args = ExperimentalOtelCliArgs {
            metrics_task_details: Some(true),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.task_details, Some(true));
    }

    #[test]
    fn test_experimental_otel_cli_args_metrics_both() {
        let args = ExperimentalOtelCliArgs {
            metrics_run_summary: Some(true),
            metrics_task_details: Some(false),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.run_summary, Some(true));
        assert_eq!(metrics.task_details, Some(false));
    }

    #[test]
    fn test_experimental_otel_cli_args_metrics_run_summary_disabled() {
        let args = ExperimentalOtelCliArgs {
            metrics_run_summary: Some(false),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.run_summary, Some(false));
    }

    #[test]
    fn test_experimental_otel_cli_args_metrics_task_details_disabled() {
        let args = ExperimentalOtelCliArgs {
            metrics_task_details: Some(false),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.task_details, Some(false));
    }

    #[test]
    fn test_experimental_otel_cli_args_combined() {
        let args = ExperimentalOtelCliArgs {
            enabled: Some(true),
            protocol: Some(ExperimentalOtelProtocol::Grpc),
            endpoint: Some("https://example.com/otel".to_string()),
            timeout_ms: Some(15000),
            headers: vec![("auth".to_string(), "token123".to_string())],
            resource_attributes: vec![("service.name".to_string(), "test".to_string())],
            metrics_run_summary: Some(true),
            metrics_task_details: Some(false),
            use_remote_cache_token: None,
        };
        let result = args.to_config();
        assert!(result.is_some());
        let opts = result.unwrap();
        assert_eq!(opts.enabled, Some(true));
        assert_eq!(opts.protocol, Some(ExperimentalOtelProtocol::Grpc));
        assert_eq!(opts.endpoint, Some("https://example.com/otel".to_string()));
        assert_eq!(opts.timeout_ms, Some(15000));
        assert_eq!(
            opts.headers.unwrap().get("auth"),
            Some(&"token123".to_string())
        );
        assert_eq!(
            opts.resource.unwrap().get("service.name"),
            Some(&"test".to_string())
        );
        let metrics = opts.metrics.unwrap();
        assert_eq!(metrics.run_summary, Some(true));
        assert_eq!(metrics.task_details, Some(false));
    }

    #[test]
    fn test_experimental_otel_cli_args_use_remote_cache_token_enabled() {
        let args = ExperimentalOtelCliArgs {
            use_remote_cache_token: Some(true),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        assert_eq!(result.unwrap().use_remote_cache_token, Some(true));
    }

    #[test]
    fn test_experimental_otel_cli_args_use_remote_cache_token_disabled() {
        let args = ExperimentalOtelCliArgs {
            use_remote_cache_token: Some(false),
            ..Default::default()
        };
        let result = args.to_config();
        assert!(result.is_some());
        assert_eq!(result.unwrap().use_remote_cache_token, Some(false));
    }

    #[test]
    fn test_parse_key_val_pair_valid() {
        let result = super::parse_key_val_pair("key=value");
        assert_eq!(result.unwrap(), ("key".to_string(), "value".to_string()));
    }

    #[test]
    fn test_parse_key_val_pair_with_whitespace() {
        let result = super::parse_key_val_pair("  key  =  value  ");
        assert_eq!(result.unwrap(), ("key".to_string(), "value".to_string()));
    }

    #[test]
    fn test_parse_key_val_pair_multiple_equals() {
        let result = super::parse_key_val_pair("key=value=more");
        assert_eq!(
            result.unwrap(),
            ("key".to_string(), "value=more".to_string())
        );
    }

    #[test]
    fn test_parse_key_val_pair_empty_value() {
        let result = super::parse_key_val_pair("key=");
        assert_eq!(result.unwrap(), ("key".to_string(), "".to_string()));
    }

    #[test]
    fn test_parse_key_val_pair_no_equals() {
        let result = super::parse_key_val_pair("keyvalue");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "must be in key=value format");
    }

    #[test]
    fn test_parse_key_val_pair_empty_key() {
        let result = super::parse_key_val_pair("=value");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "key cannot be empty");
    }

    #[test]
    fn test_parse_key_val_pair_whitespace_only_key() {
        let result = super::parse_key_val_pair("   =value");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "key cannot be empty");
    }
}
