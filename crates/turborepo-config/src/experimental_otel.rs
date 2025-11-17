use std::{collections::BTreeMap, fmt, str::FromStr};

use clap::ValueEnum;
use merge::Merge;
use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    Hash,
    PartialOrd,
    Ord,
    ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub enum ExperimentalOtelProtocol {
    #[default]
    #[serde(alias = "grpc")]
    Grpc,
    #[serde(alias = "http")]
    #[serde(alias = "http/protobuf")]
    HttpProtobuf,
}

impl fmt::Display for ExperimentalOtelProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExperimentalOtelProtocol::Grpc => write!(f, "grpc"),
            ExperimentalOtelProtocol::HttpProtobuf => write!(f, "http/protobuf"),
        }
    }
}

impl FromStr for ExperimentalOtelProtocol {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "grpc" => Ok(Self::Grpc),
            "http" | "http/protobuf" | "http_protobuf" => Ok(Self::HttpProtobuf),
            _ => Err(()),
        }
    }
}

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq, Merge)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelMetricsOptions {
    pub run_summary: Option<bool>,
    pub task_details: Option<bool>,
}

#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq, Merge)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalOtelOptions {
    pub enabled: Option<bool>,
    pub protocol: Option<ExperimentalOtelProtocol>,
    pub endpoint: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub timeout_ms: Option<u64>,
    pub resource: Option<BTreeMap<String, String>>,
    pub metrics: Option<ExperimentalOtelMetricsOptions>,
}

impl ExperimentalOtelOptions {
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
            && self.protocol.is_none()
            && self.endpoint.is_none()
            && self.headers.as_ref().map(|m| m.is_empty()).unwrap_or(true)
            && self.timeout_ms.is_none()
            && self.resource.as_ref().map(|m| m.is_empty()).unwrap_or(true)
            && self
                .metrics
                .as_ref()
                .map(|m| m.run_summary.is_none() && m.task_details.is_none())
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
