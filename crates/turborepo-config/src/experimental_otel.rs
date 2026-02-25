use std::{collections::BTreeMap, str::FromStr};

use merge::Merge;
use serde::{Deserialize, Serialize};
// Re-export Protocol from turborepo-otel to avoid duplicating the enum.
pub use turborepo_otel::Protocol as ExperimentalOtelProtocol;

use crate::Error;

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq, Merge)]
#[merge(strategy = merge::option::overwrite_none)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelTaskAttributesOptions {
    pub id: Option<bool>,
    pub hashes: Option<bool>,
}

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq, Merge)]
#[merge(strategy = merge::option::overwrite_none)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelMetricsOptions {
    pub run_summary: Option<bool>,
    pub task_details: Option<bool>,
    pub task_attributes: Option<ExperimentalOtelTaskAttributesOptions>,
}

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq, Merge)]
#[merge(strategy = merge::option::overwrite_none)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelOptions {
    pub enabled: Option<bool>,
    pub protocol: Option<ExperimentalOtelProtocol>,
    pub endpoint: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub timeout_ms: Option<u64>,
    pub interval_ms: Option<u64>,
    pub resource: Option<BTreeMap<String, String>>,
    pub metrics: Option<ExperimentalOtelMetricsOptions>,
    pub use_remote_cache_token: Option<bool>,
}

impl ExperimentalOtelOptions {
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
            && self.protocol.is_none()
            && self.endpoint.is_none()
            && self.headers.as_ref().map(|m| m.is_empty()).unwrap_or(true)
            && self.timeout_ms.is_none()
            && self.interval_ms.is_none()
            && self.resource.as_ref().map(|m| m.is_empty()).unwrap_or(true)
            && self.use_remote_cache_token.is_none()
            && self
                .metrics
                .as_ref()
                .map(|m| {
                    m.run_summary.is_none()
                        && m.task_details.is_none()
                        && m.task_attributes
                            .as_ref()
                            .map(|a| a.id.is_none() && a.hashes.is_none())
                            .unwrap_or(true)
                })
                .unwrap_or(true)
    }

    pub fn from_env_map(
        map: &std::collections::HashMap<&'static str, String>,
    ) -> Result<Option<Self>, Error> {
        let mut options = Self::default();
        let mut touched = false;

        if let Some(raw) = get_non_empty(map, "experimental_otel_enabled") {
            options.enabled = Some(parse_bool_flag(raw, "TURBO_EXPERIMENTAL_OTEL_ENABLED")?);
            touched = true;
        }

        if let Some(raw) = get_non_empty(map, "experimental_otel_protocol") {
            let protocol = <ExperimentalOtelProtocol as FromStr>::from_str(raw).map_err(|_| {
                Error::InvalidExperimentalOtelConfig {
                    message: format!(
                        "Unsupported experimentalObservability.otel protocol `{raw}`. Use `grpc` \
                         or `http/protobuf`."
                    ),
                }
            })?;
            options.protocol = Some(protocol);
            touched = true;
        }

        if let Some(raw) = get_non_empty(map, "experimental_otel_endpoint") {
            options.endpoint = Some(raw.to_string());
            touched = true;
        }

        if let Some(raw) = get_non_empty(map, "experimental_otel_timeout_ms") {
            let timeout = raw
                .parse()
                .map_err(|_| Error::InvalidExperimentalOtelConfig {
                    message: "TURBO_EXPERIMENTAL_OTEL_TIMEOUT_MS must be a number.".to_string(),
                })?;
            options.timeout_ms = Some(timeout);
            touched = true;
        }

        if let Some(raw) = get_non_empty(map, "experimental_otel_interval_ms") {
            let interval = raw
                .parse()
                .map_err(|_| Error::InvalidExperimentalOtelConfig {
                    message: "TURBO_EXPERIMENTAL_OTEL_INTERVAL_MS must be a number.".to_string(),
                })?;
            options.interval_ms = Some(interval);
            touched = true;
        }

        if let Some(raw) = get_non_empty(map, "experimental_otel_headers") {
            options.headers = Some(parse_key_value_pairs(
                raw,
                "TURBO_EXPERIMENTAL_OTEL_HEADERS",
            )?);
            touched = true;
        }

        if let Some(raw) = get_non_empty(map, "experimental_otel_resource") {
            options.resource = Some(parse_key_value_pairs(
                raw,
                "TURBO_EXPERIMENTAL_OTEL_RESOURCE",
            )?);
            touched = true;
        }

        touched |= set_metric_flag(
            map,
            "experimental_otel_metrics_run_summary",
            "TURBO_EXPERIMENTAL_OTEL_METRICS_RUN_SUMMARY",
            |metrics, value| metrics.run_summary = Some(value),
            &mut options,
        )?;

        touched |= set_metric_flag(
            map,
            "experimental_otel_metrics_task_details",
            "TURBO_EXPERIMENTAL_OTEL_METRICS_TASK_DETAILS",
            |metrics, value| metrics.task_details = Some(value),
            &mut options,
        )?;

        touched |= set_metric_flag(
            map,
            "experimental_otel_metrics_task_attributes_id",
            "TURBO_EXPERIMENTAL_OTEL_METRICS_TASK_ATTRIBUTES_ID",
            |metrics, value| {
                metrics
                    .task_attributes
                    .get_or_insert_with(ExperimentalOtelTaskAttributesOptions::default)
                    .id = Some(value)
            },
            &mut options,
        )?;

        if let Some(raw) = get_non_empty(map, "experimental_otel_use_remote_cache_token") {
            options.use_remote_cache_token = Some(parse_bool_flag(
                raw,
                "TURBO_EXPERIMENTAL_OTEL_USE_REMOTE_CACHE_TOKEN",
            )?);
            touched = true;
        }

        touched |= set_metric_flag(
            map,
            "experimental_otel_metrics_task_attributes_hashes",
            "TURBO_EXPERIMENTAL_OTEL_METRICS_TASK_ATTRIBUTES_HASHES",
            |metrics, value| {
                metrics
                    .task_attributes
                    .get_or_insert_with(ExperimentalOtelTaskAttributesOptions::default)
                    .hashes = Some(value)
            },
            &mut options,
        )?;

        Ok(touched.then_some(options))
    }
}

fn get_non_empty<'a>(
    map: &'a std::collections::HashMap<&'static str, String>,
    key: &'static str,
) -> Option<&'a str> {
    map.get(key).map(|s| s.as_str()).filter(|s| !s.is_empty())
}

fn parse_bool_flag(raw: &str, var: &str) -> Result<bool, Error> {
    crate::env::truth_env_var(raw).ok_or_else(|| Error::InvalidExperimentalOtelConfig {
        message: format!("{var} should be either 1 or 0."),
    })
}

fn parse_key_value_pairs(raw: &str, context: &str) -> Result<BTreeMap<String, String>, Error> {
    let mut map = BTreeMap::new();
    for entry in raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let Some((key, value)) = entry.split_once('=') else {
            return Err(Error::InvalidExperimentalOtelConfig {
                message: format!("{context} entries must be in key=value format."),
            });
        };
        if key.trim().is_empty() {
            return Err(Error::InvalidExperimentalOtelConfig {
                message: format!("{context} keys cannot be empty."),
            });
        }
        map.insert(key.trim().to_string(), value.trim().to_string());
    }

    Ok(map)
}

fn set_metric_flag(
    map: &std::collections::HashMap<&'static str, String>,
    key: &'static str,
    env_name: &'static str,
    set: impl FnOnce(&mut ExperimentalOtelMetricsOptions, bool),
    options: &mut ExperimentalOtelOptions,
) -> Result<bool, Error> {
    if let Some(raw) = get_non_empty(map, key) {
        let value = parse_bool_flag(raw, env_name)?;
        set(
            options
                .metrics
                .get_or_insert_with(ExperimentalOtelMetricsOptions::default),
            value,
        );
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn build_env_map(entries: &[(&'static str, &str)]) -> HashMap<&'static str, String> {
        entries.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    #[test]
    fn test_from_env_map_empty() {
        let map = HashMap::new();
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_from_env_map_enabled_true() {
        let map = build_env_map(&[("experimental_otel_enabled", "1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().enabled, Some(true));
    }

    #[test]
    fn test_from_env_map_enabled_false() {
        let map = build_env_map(&[("experimental_otel_enabled", "0")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().enabled, Some(false));
    }

    #[test]
    fn test_from_env_map_enabled_true_string() {
        let map = build_env_map(&[("experimental_otel_enabled", "true")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().enabled, Some(true));
    }

    #[test]
    fn test_from_env_map_enabled_invalid() {
        let map = build_env_map(&[("experimental_otel_enabled", "invalid")]);
        let result = ExperimentalOtelOptions::from_env_map(&map);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidExperimentalOtelConfig { message } => {
                assert!(message.contains("TURBO_EXPERIMENTAL_OTEL_ENABLED"));
            }
            _ => panic!("Expected InvalidExperimentalOtelConfig"),
        }
    }

    #[test]
    fn test_from_env_map_protocol_grpc() {
        let map = build_env_map(&[("experimental_otel_protocol", "grpc")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().protocol,
            Some(ExperimentalOtelProtocol::Grpc)
        );
    }

    #[test]
    fn test_from_env_map_protocol_http_protobuf() {
        for protocol_str in ["http/protobuf", "http", "http_protobuf"] {
            let map = build_env_map(&[("experimental_otel_protocol", protocol_str)]);
            let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
            assert!(result.is_some());
            assert_eq!(
                result.unwrap().protocol,
                Some(ExperimentalOtelProtocol::HttpProtobuf)
            );
        }
    }

    #[test]
    fn test_from_env_map_protocol_invalid() {
        let map = build_env_map(&[("experimental_otel_protocol", "invalid")]);
        let result = ExperimentalOtelOptions::from_env_map(&map);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidExperimentalOtelConfig { message } => {
                assert!(message.contains("Unsupported"));
                assert!(message.contains("`invalid`"));
            }
            _ => panic!("Expected InvalidExperimentalOtelConfig"),
        }
    }

    #[test]
    fn test_from_env_map_endpoint() {
        let endpoint = "https://example.com/otel";
        let map = build_env_map(&[("experimental_otel_endpoint", endpoint)]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().endpoint, Some(endpoint.to_string()));
    }

    #[test]
    fn test_from_env_map_endpoint_empty_ignored() {
        let map = build_env_map(&[("experimental_otel_endpoint", "")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_from_env_map_timeout_ms() {
        let map = build_env_map(&[("experimental_otel_timeout_ms", "5000")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().timeout_ms, Some(5000));
    }

    #[test]
    fn test_from_env_map_timeout_ms_invalid() {
        let map = build_env_map(&[("experimental_otel_timeout_ms", "not-a-number")]);
        let result = ExperimentalOtelOptions::from_env_map(&map);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidExperimentalOtelConfig { message } => {
                assert!(message.contains("TURBO_EXPERIMENTAL_OTEL_TIMEOUT_MS must be a number"));
            }
            _ => panic!("Expected InvalidExperimentalOtelConfig"),
        }
    }

    #[test]
    fn test_from_env_map_interval_ms() {
        let map = build_env_map(&[("experimental_otel_interval_ms", "30000")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().interval_ms, Some(30000));
    }

    #[test]
    fn test_from_env_map_interval_ms_invalid() {
        let map = build_env_map(&[("experimental_otel_interval_ms", "not-a-number")]);
        let result = ExperimentalOtelOptions::from_env_map(&map);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidExperimentalOtelConfig { message } => {
                assert!(message.contains("TURBO_EXPERIMENTAL_OTEL_INTERVAL_MS must be a number"));
            }
            _ => panic!("Expected InvalidExperimentalOtelConfig"),
        }
    }

    #[test]
    fn test_from_env_map_headers_single() {
        let map = build_env_map(&[("experimental_otel_headers", "key1=value1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let headers = result.unwrap().headers.unwrap();
        assert_eq!(headers.get("key1"), Some(&"value1".to_string()));
    }

    #[test]
    fn test_from_env_map_headers_multiple() {
        let map = build_env_map(&[("experimental_otel_headers", "key1=value1,key2=value2")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let headers = result.unwrap().headers.unwrap();
        assert_eq!(headers.get("key1"), Some(&"value1".to_string()));
        assert_eq!(headers.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_from_env_map_headers_with_spaces() {
        let map = build_env_map(&[(
            "experimental_otel_headers",
            " key1 = value1 , key2 = value2 ",
        )]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let headers = result.unwrap().headers.unwrap();
        assert_eq!(headers.get("key1"), Some(&"value1".to_string()));
        assert_eq!(headers.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_from_env_map_headers_missing_equals() {
        let map = build_env_map(&[("experimental_otel_headers", "key1value1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidExperimentalOtelConfig { message } => {
                assert!(message.contains("key=value format"));
            }
            _ => panic!("Expected InvalidExperimentalOtelConfig"),
        }
    }

    #[test]
    fn test_from_env_map_headers_empty_key() {
        let map = build_env_map(&[("experimental_otel_headers", "=value1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidExperimentalOtelConfig { message } => {
                assert!(message.contains("keys cannot be empty"));
            }
            _ => panic!("Expected InvalidExperimentalOtelConfig"),
        }
    }

    #[test]
    fn test_from_env_map_resource_single() {
        let map = build_env_map(&[("experimental_otel_resource", "service.name=my-service")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let resource = result.unwrap().resource.unwrap();
        assert_eq!(
            resource.get("service.name"),
            Some(&"my-service".to_string())
        );
    }

    #[test]
    fn test_from_env_map_resource_multiple() {
        let map = build_env_map(&[(
            "experimental_otel_resource",
            "service.name=my-service,env=production",
        )]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let resource = result.unwrap().resource.unwrap();
        assert_eq!(
            resource.get("service.name"),
            Some(&"my-service".to_string())
        );
        assert_eq!(resource.get("env"), Some(&"production".to_string()));
    }

    #[test]
    fn test_from_env_map_metrics_run_summary() {
        let map = build_env_map(&[("experimental_otel_metrics_run_summary", "1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.run_summary, Some(true));
    }

    #[test]
    fn test_from_env_map_metrics_task_details() {
        let map = build_env_map(&[("experimental_otel_metrics_task_details", "1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.task_details, Some(true));
    }

    #[test]
    fn test_from_env_map_metrics_both() {
        let map = build_env_map(&[
            ("experimental_otel_metrics_run_summary", "1"),
            ("experimental_otel_metrics_task_details", "0"),
        ]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.run_summary, Some(true));
        assert_eq!(metrics.task_details, Some(false));
    }

    #[test]
    fn test_from_env_map_metrics_task_attributes_id() {
        let map = build_env_map(&[("experimental_otel_metrics_task_attributes_id", "1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        let attrs = metrics.task_attributes.unwrap();
        assert_eq!(attrs.id, Some(true));
        assert_eq!(attrs.hashes, None);
    }

    #[test]
    fn test_from_env_map_metrics_task_attributes_hashes() {
        let map = build_env_map(&[("experimental_otel_metrics_task_attributes_hashes", "1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        let attrs = metrics.task_attributes.unwrap();
        assert_eq!(attrs.id, None);
        assert_eq!(attrs.hashes, Some(true));
    }

    #[test]
    fn test_from_env_map_metrics_task_attributes_both() {
        let map = build_env_map(&[
            ("experimental_otel_metrics_task_attributes_id", "1"),
            ("experimental_otel_metrics_task_attributes_hashes", "0"),
        ]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        let attrs = metrics.task_attributes.unwrap();
        assert_eq!(attrs.id, Some(true));
        assert_eq!(attrs.hashes, Some(false));
    }

    #[test]
    fn test_from_env_map_enabled_with_endpoint() {
        let map = build_env_map(&[
            ("experimental_otel_enabled", "1"),
            ("experimental_otel_endpoint", "https://example.com/otel"),
        ]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let opts = result.unwrap();
        assert_eq!(opts.enabled, Some(true));
        assert_eq!(opts.endpoint, Some("https://example.com/otel".to_string()));
    }

    #[test]
    fn test_from_env_map_disabled_with_endpoint() {
        let map = build_env_map(&[
            ("experimental_otel_enabled", "0"),
            ("experimental_otel_endpoint", "https://example.com/otel"),
        ]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let opts = result.unwrap();
        assert_eq!(opts.enabled, Some(false));
        assert_eq!(opts.endpoint, Some("https://example.com/otel".to_string()));
    }

    #[test]
    fn test_from_env_map_metrics_run_summary_disabled() {
        let map = build_env_map(&[("experimental_otel_metrics_run_summary", "0")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.run_summary, Some(false));
    }

    #[test]
    fn test_from_env_map_metrics_task_details_disabled() {
        let map = build_env_map(&[("experimental_otel_metrics_task_details", "0")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        assert_eq!(metrics.task_details, Some(false));
    }

    #[test]
    fn test_from_env_map_combined() {
        let map = build_env_map(&[
            ("experimental_otel_enabled", "1"),
            ("experimental_otel_protocol", "grpc"),
            ("experimental_otel_endpoint", "https://example.com/otel"),
            ("experimental_otel_timeout_ms", "15000"),
            ("experimental_otel_interval_ms", "30000"),
            ("experimental_otel_headers", "auth=token123"),
            ("experimental_otel_resource", "service.name=test"),
            ("experimental_otel_metrics_run_summary", "1"),
        ]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let opts = result.unwrap();
        assert_eq!(opts.enabled, Some(true));
        assert_eq!(opts.protocol, Some(ExperimentalOtelProtocol::Grpc));
        assert_eq!(opts.endpoint, Some("https://example.com/otel".to_string()));
        assert_eq!(opts.timeout_ms, Some(15000));
        assert_eq!(opts.interval_ms, Some(30000));
        assert_eq!(
            opts.headers.unwrap().get("auth"),
            Some(&"token123".to_string())
        );
        assert_eq!(
            opts.resource.unwrap().get("service.name"),
            Some(&"test".to_string())
        );
        assert_eq!(opts.metrics.unwrap().run_summary, Some(true));
    }

    #[test]
    fn test_from_env_map_use_remote_cache_token_enabled() {
        let map = build_env_map(&[("experimental_otel_use_remote_cache_token", "1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().use_remote_cache_token, Some(true));
    }

    #[test]
    fn test_from_env_map_use_remote_cache_token_disabled() {
        let map = build_env_map(&[("experimental_otel_use_remote_cache_token", "0")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().use_remote_cache_token, Some(false));
    }

    #[test]
    fn test_from_env_map_use_remote_cache_token_invalid() {
        let map = build_env_map(&[("experimental_otel_use_remote_cache_token", "invalid")]);
        let result = ExperimentalOtelOptions::from_env_map(&map);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidExperimentalOtelConfig { message } => {
                assert!(message.contains("TURBO_EXPERIMENTAL_OTEL_USE_REMOTE_CACHE_TOKEN"));
            }
            _ => panic!("Expected InvalidExperimentalOtelConfig"),
        }
    }

    #[test]
    fn test_is_empty_default() {
        let opts = ExperimentalOtelOptions::default();
        assert!(opts.is_empty());
    }

    #[test]
    fn test_is_empty_with_enabled() {
        let opts = ExperimentalOtelOptions {
            enabled: Some(true),
            ..Default::default()
        };
        assert!(!opts.is_empty());
    }

    #[test]
    fn test_is_empty_with_use_remote_cache_token() {
        let opts = ExperimentalOtelOptions {
            use_remote_cache_token: Some(true),
            ..Default::default()
        };
        assert!(!opts.is_empty());
    }

    #[test]
    fn test_is_empty_with_empty_headers() {
        let opts = ExperimentalOtelOptions {
            headers: Some(BTreeMap::new()),
            ..Default::default()
        };
        assert!(opts.is_empty());
    }

    #[test]
    fn test_is_empty_with_nonempty_headers() {
        let mut headers = BTreeMap::new();
        headers.insert("key".to_string(), "value".to_string());
        let opts = ExperimentalOtelOptions {
            headers: Some(headers),
            ..Default::default()
        };
        assert!(!opts.is_empty());
    }

    #[test]
    fn test_is_empty_with_task_attributes() {
        let opts = ExperimentalOtelOptions {
            metrics: Some(ExperimentalOtelMetricsOptions {
                run_summary: None,
                task_details: None,
                task_attributes: Some(ExperimentalOtelTaskAttributesOptions {
                    id: Some(true),
                    hashes: None,
                }),
            }),
            ..Default::default()
        };
        assert!(!opts.is_empty());
    }

    #[test]
    fn test_parse_key_value_pairs_valid() {
        let result = parse_key_value_pairs("key1=value1,key2=value2", "TEST").unwrap();
        assert_eq!(result.get("key1"), Some(&"value1".to_string()));
        assert_eq!(result.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_parse_key_value_pairs_missing_equals() {
        let result = parse_key_value_pairs("noequals", "TEST");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_key_value_pairs_empty_key() {
        let result = parse_key_value_pairs("=value", "TEST");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_key_value_pairs_empty_string() {
        let result = parse_key_value_pairs("", "TEST").unwrap();
        assert!(result.is_empty());
    }
}
