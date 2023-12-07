use serde::{Deserialize, Serialize};

/// All possible telemetry events must be included in this enum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TelemetryEvent {
    Fallback(Fallback),
    Framework(Framework),
}

/// Individual events are defined here
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Framework {
    pub framework: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fallback {
    pub go_arg: bool,
    pub rust_env_var: bool,
}
