// telemetry events

use serde::{Deserialize, Serialize};

/// All possible telemetry events must be included in this enum.
///
/// These events must be added to the backend (telemetry.vercel.com)
/// before they can be tracked - invalid or unknown events will be
/// ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TelemetryEvent {
    Task(TelemetryTaskEvent),
    Command(TelemetryCommandEvent),
    Repo(TelemetryRepoEvent),
    Generic(TelemetryGenericEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryCommandEvent {
    pub id: String,
    pub command: String,
    pub key: String,
    pub value: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryRepoEvent {
    pub id: String,
    pub repo: String,
    pub key: String,
    pub value: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryTaskEvent {
    pub id: String,
    pub package: String,
    pub task: String,
    pub key: String,
    pub value: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryGenericEvent {
    pub id: String,
    pub key: String,
    pub value: String,
    pub parent_id: Option<String>,
}
