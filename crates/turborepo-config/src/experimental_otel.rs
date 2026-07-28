use std::{collections::BTreeMap, str::FromStr};

use merge::Merge;
use serde::{Deserialize, Serialize};
// Re-export Protocol from turborepo-otel to avoid duplicating the enum.
pub use turborepo_otel::Protocol as ExperimentalOtelProtocol;

use crate::Error;

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq, Merge)]
#[merge(strategy = merge::option::overwrite_none)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelRunAttributesOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scm_revision: Option<bool>,
}

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq, Merge)]
#[merge(strategy = merge::option::overwrite_none)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelTaskAttributesOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hashes: Option<bool>,
}

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq, Merge)]
#[merge(strategy = merge::option::overwrite_none)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelMetricsOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_summary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_details: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[merge(strategy = merge::option::recurse)]
    pub run_attributes: Option<ExperimentalOtelRunAttributesOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[merge(strategy = merge::option::recurse)]
    pub task_attributes: Option<ExperimentalOtelTaskAttributesOptions>,
}

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<ExperimentalOtelProtocol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<ExperimentalOtelMetricsOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_remote_cache_token: Option<bool>,
}

/// Credential locking: `headers` and `use_remote_cache_token` are security-
/// coupled to `endpoint`. Changing the endpoint resets credentials — you must
/// re-provide them alongside the new endpoint.
///
/// Config sources are merged highest-priority first. Once a source provides
/// an endpoint, all subsequent (lower-priority) sources' credential fields
/// are ignored, even if the higher-priority source left them unset. This
/// prevents auth headers configured for one endpoint from leaking to a
/// different endpoint set by a higher-priority source.
///
/// Non-credential fields (`enabled`, `protocol`, `timeout_ms`, `interval_ms`,
/// `resource`, `metrics`) always merge independently across all sources.
impl Merge for ExperimentalOtelOptions {
    fn merge(&mut self, other: Self) {
        let endpoint_locked = self.endpoint.is_some();
        merge::option::overwrite_none(&mut self.endpoint, other.endpoint);

        if !endpoint_locked {
            merge::option::overwrite_none(&mut self.headers, other.headers);
            merge::option::overwrite_none(
                &mut self.use_remote_cache_token,
                other.use_remote_cache_token,
            );
        }

        merge::option::overwrite_none(&mut self.enabled, other.enabled);
        merge::option::overwrite_none(&mut self.protocol, other.protocol);
        merge::option::overwrite_none(&mut self.timeout_ms, other.timeout_ms);
        merge::option::overwrite_none(&mut self.interval_ms, other.interval_ms);
        merge::option::overwrite_none(&mut self.resource, other.resource);
        merge::option::recurse(&mut self.metrics, other.metrics);
    }
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
                        && m.run_attributes
                            .as_ref()
                            .map(|a| a.id.is_none() && a.scm_revision.is_none())
                            .unwrap_or(true)
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
            "experimental_otel_metrics_run_attributes_id",
            "TURBO_EXPERIMENTAL_OTEL_METRICS_RUN_ATTRIBUTES_ID",
            |metrics, value| {
                metrics
                    .run_attributes
                    .get_or_insert_with(ExperimentalOtelRunAttributesOptions::default)
                    .id = Some(value)
            },
            &mut options,
        )?;

        touched |= set_metric_flag(
            map,
            "experimental_otel_metrics_run_attributes_scm_revision",
            "TURBO_EXPERIMENTAL_OTEL_METRICS_RUN_ATTRIBUTES_SCM_REVISION",
            |metrics, value| {
                metrics
                    .run_attributes
                    .get_or_insert_with(ExperimentalOtelRunAttributesOptions::default)
                    .scm_revision = Some(value)
            },
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
    fn test_from_env_map_metrics_run_attributes_id() {
        let map = build_env_map(&[("experimental_otel_metrics_run_attributes_id", "0")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        let attrs = metrics.run_attributes.unwrap();
        assert_eq!(attrs.id, Some(false));
    }

    #[test]
    fn test_from_env_map_metrics_run_attributes_scm_revision() {
        let map = build_env_map(&[("experimental_otel_metrics_run_attributes_scm_revision", "1")]);
        let result = ExperimentalOtelOptions::from_env_map(&map).unwrap();
        assert!(result.is_some());
        let metrics = result.unwrap().metrics.unwrap();
        let attrs = metrics.run_attributes.unwrap();
        assert_eq!(attrs.scm_revision, Some(true));
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
    fn test_is_empty_with_run_attributes() {
        let opts = ExperimentalOtelOptions {
            metrics: Some(ExperimentalOtelMetricsOptions {
                run_summary: None,
                task_details: None,
                run_attributes: Some(ExperimentalOtelRunAttributesOptions {
                    id: Some(false),
                    scm_revision: None,
                }),
                task_attributes: None,
            }),
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
                run_attributes: None,
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

    // -- Credential-locking merge tests --
    //
    // Config sources are merged highest-priority first. The custom Merge impl
    // on ExperimentalOtelOptions ensures that once a higher-priority source
    // provides an endpoint, credentials (headers, use_remote_cache_token) from
    // lower-priority sources are discarded.

    fn make_headers(pairs: &[(&str, &str)]) -> Option<BTreeMap<String, String>> {
        Some(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn merge_endpoint_override_blocks_lower_priority_headers() {
        // Higher-priority source sets endpoint only.
        let mut high = ExperimentalOtelOptions {
            endpoint: Some("https://staging.example/otel".into()),
            ..Default::default()
        };
        // Lower-priority source sets endpoint + auth headers.
        let low = ExperimentalOtelOptions {
            endpoint: Some("https://internal.corp/otel".into()),
            headers: make_headers(&[("Authorization", "Bearer secret")]),
            use_remote_cache_token: Some(true),
            ..Default::default()
        };

        high.merge(low);

        assert_eq!(
            high.endpoint.as_deref(),
            Some("https://staging.example/otel")
        );
        assert_eq!(
            high.headers, None,
            "headers from lower-priority source must not leak to overridden endpoint"
        );
        assert_eq!(
            high.use_remote_cache_token, None,
            "use_remote_cache_token must not leak"
        );
    }

    #[test]
    fn merge_endpoint_override_still_merges_non_credential_fields() {
        let mut high = ExperimentalOtelOptions {
            endpoint: Some("https://staging.example/otel".into()),
            ..Default::default()
        };
        let low = ExperimentalOtelOptions {
            endpoint: Some("https://internal.corp/otel".into()),
            enabled: Some(true),
            protocol: Some(ExperimentalOtelProtocol::Grpc),
            timeout_ms: Some(5000),
            interval_ms: Some(30000),
            resource: Some([("service.name".into(), "turbo".into())].into()),
            metrics: Some(ExperimentalOtelMetricsOptions {
                run_summary: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };

        high.merge(low);

        assert_eq!(
            high.enabled,
            Some(true),
            "non-credential fields merge independently"
        );
        assert_eq!(high.protocol, Some(ExperimentalOtelProtocol::Grpc));
        assert_eq!(high.timeout_ms, Some(5000));
        assert_eq!(high.interval_ms, Some(30000));
        assert!(high.resource.is_some());
        assert_eq!(
            high.metrics.as_ref().and_then(|m| m.run_summary),
            Some(true)
        );
    }

    #[test]
    fn merge_headers_without_endpoint_are_inherited() {
        // Higher-priority source sets only headers (no endpoint).
        let mut high = ExperimentalOtelOptions {
            headers: make_headers(&[("X-Custom", "value")]),
            ..Default::default()
        };
        // Lower-priority source provides the endpoint.
        let low = ExperimentalOtelOptions {
            endpoint: Some("https://internal.corp/otel".into()),
            headers: make_headers(&[("Authorization", "Bearer secret")]),
            ..Default::default()
        };

        high.merge(low);

        assert_eq!(high.endpoint.as_deref(), Some("https://internal.corp/otel"));
        assert_eq!(
            high.headers
                .as_ref()
                .and_then(|h| h.get("X-Custom"))
                .map(|s| s.as_str()),
            Some("value"),
            "higher-priority headers win"
        );
        assert_eq!(
            high.headers.as_ref().and_then(|h| h.get("Authorization")),
            None,
            "headers use atomic overwrite_none, not key-level merge"
        );
    }

    #[test]
    fn merge_same_source_endpoint_and_headers() {
        let mut high = ExperimentalOtelOptions {
            endpoint: Some("https://prod.example/otel".into()),
            headers: make_headers(&[("Authorization", "Bearer prod")]),
            use_remote_cache_token: Some(true),
            ..Default::default()
        };
        let low = ExperimentalOtelOptions {
            endpoint: Some("https://fallback.example/otel".into()),
            headers: make_headers(&[("Authorization", "Bearer fallback")]),
            ..Default::default()
        };

        high.merge(low);

        assert_eq!(high.endpoint.as_deref(), Some("https://prod.example/otel"));
        assert_eq!(
            high.headers
                .as_ref()
                .and_then(|h| h.get("Authorization"))
                .map(|s| s.as_str()),
            Some("Bearer prod"),
        );
        assert_eq!(high.use_remote_cache_token, Some(true));
    }

    #[test]
    fn merge_three_layers_credential_locking() {
        // Simulates: override → env → turbo.json (highest to lowest priority)
        let mut acc = ExperimentalOtelOptions::default();

        // Highest priority: override sets endpoint + protocol
        let override_source = ExperimentalOtelOptions {
            endpoint: Some("https://override.example/otel".into()),
            protocol: Some(ExperimentalOtelProtocol::HttpProtobuf),
            ..Default::default()
        };
        acc.merge(override_source);

        // Mid priority: env sets headers + timeout
        let env_source = ExperimentalOtelOptions {
            headers: make_headers(&[("X-Env", "from-env")]),
            timeout_ms: Some(9999),
            ..Default::default()
        };
        acc.merge(env_source);

        // Lowest priority: turbo.json sets everything
        let turbo_json = ExperimentalOtelOptions {
            endpoint: Some("https://turbo-json.example/otel".into()),
            headers: make_headers(&[("Authorization", "Bearer turbo-json-secret")]),
            use_remote_cache_token: Some(true),
            enabled: Some(true),
            interval_ms: Some(60000),
            metrics: Some(ExperimentalOtelMetricsOptions {
                run_summary: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        acc.merge(turbo_json);

        // Override's endpoint wins.
        assert_eq!(
            acc.endpoint.as_deref(),
            Some("https://override.example/otel")
        );
        assert_eq!(acc.protocol, Some(ExperimentalOtelProtocol::HttpProtobuf));

        // Endpoint was locked after the override merge, so BOTH env and
        // turbo.json credentials are blocked.
        assert_eq!(
            acc.headers, None,
            "env headers blocked — endpoint already locked"
        );
        assert_eq!(acc.use_remote_cache_token, None, "turbo.json token blocked");

        // Non-credential fields still fill in from lower-priority sources.
        assert_eq!(acc.timeout_ms, Some(9999), "env timeout fills in");
        assert_eq!(acc.enabled, Some(true), "turbo.json enabled fills in");
        assert_eq!(acc.interval_ms, Some(60000), "turbo.json interval fills in");
        assert_eq!(acc.metrics.as_ref().and_then(|m| m.run_summary), Some(true));
    }

    #[test]
    fn merge_no_endpoint_anywhere_credentials_merge_normally() {
        let mut high = ExperimentalOtelOptions {
            headers: make_headers(&[("X-High", "1")]),
            ..Default::default()
        };
        let low = ExperimentalOtelOptions {
            use_remote_cache_token: Some(true),
            enabled: Some(true),
            ..Default::default()
        };

        high.merge(low);

        assert!(high.headers.is_some(), "headers preserved");
        assert_eq!(
            high.use_remote_cache_token,
            Some(true),
            "token fills in when no endpoint set"
        );
        assert_eq!(high.enabled, Some(true));
    }

    #[test]
    fn merge_metrics_deep_merge_unaffected_by_credential_locking() {
        let mut high = ExperimentalOtelOptions {
            endpoint: Some("https://high.example/otel".into()),
            metrics: Some(ExperimentalOtelMetricsOptions {
                run_summary: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };
        let low = ExperimentalOtelOptions {
            endpoint: Some("https://low.example/otel".into()),
            metrics: Some(ExperimentalOtelMetricsOptions {
                task_details: Some(true),
                task_attributes: Some(ExperimentalOtelTaskAttributesOptions {
                    id: Some(true),
                    hashes: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        high.merge(low);

        let metrics = high.metrics.as_ref().unwrap();
        assert_eq!(
            metrics.run_summary,
            Some(false),
            "high's run_summary preserved"
        );
        assert_eq!(
            metrics.task_details,
            Some(true),
            "low's task_details fills in"
        );
        assert_eq!(
            metrics.task_attributes.as_ref().and_then(|ta| ta.id),
            Some(true),
            "low's nested task_attributes fills in via recurse"
        );
    }
}
