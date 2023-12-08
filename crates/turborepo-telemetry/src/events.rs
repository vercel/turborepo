use serde::{Deserialize, Serialize};

/// All possible telemetry events must be included in this enum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TelemetryEvent {
    Fallback(Fallback),
    KeyVal(KeyVal),
}

/// Individual events are defined here
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyVal {
    key: String,
    value: String,
}

impl KeyVal {
    pub fn framework(value: &str) -> Self {
        Self {
            key: "framework".to_string(),
            value: value.to_string(),
        }
    }

    pub fn command(value: &str) -> Self {
        Self {
            key: "command".to_string(),
            value: value.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fallback {
    pub go_arg: bool,
    pub rust_env_var: bool,
}
