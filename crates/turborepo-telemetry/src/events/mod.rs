use serde::{Deserialize, Serialize};

// all event builders and their event methods
pub mod command;
pub mod repo;
pub mod task;

/// All possible telemetry events must be included in this enum.
///
/// These events must be added to the backend (telemetry.vercel.com)
/// before they can be tracked - invalid or unknown events will be
/// ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TelemetryEvent {
    PackageTask(task::EventData),
    Command(command::EventData),
    Repo(repo::EventData),
    #[cfg(test)]
    TestVariant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventType {
    Sensitive,
    NonSensitive,
}

/// Key-value pairs that are sent with each even - if the value is
/// sensitive, it will be hashed and anonymized before being sent
/// using the users private salt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    key: String,
    value: String,
    is_sensitive: EventType,
}

/// Private trait that can be used for building telemetry events.
///
/// Supports connecting events via a parent-child relationship
/// to aid in connecting events together.
trait EventBuilder<T> {
    fn get_id(&self) -> &String;
    fn with_parent(self, parent_event: &T) -> Self;
}

/// Public trait that can be used for building telemetry events.
pub trait PubEventBuilder {
    fn track(&self, event: Event);
    fn child(&self) -> Self;
}
