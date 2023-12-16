use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Event, EventBuilder, EventType, PubEventBuilder, TelemetryEvent};
use crate::{config::TelemetryConfig, telem};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventData {
    id: String,
    command: String,
    key: String,
    value: String,
    parent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEventBuilder {
    id: String,
    command: String,
    parent: Option<String>,
}

impl EventBuilder<CommandEventBuilder> for CommandEventBuilder {
    fn get_id(&self) -> &String {
        &self.id
    }

    fn with_parent(mut self, parent_event: &CommandEventBuilder) -> Self {
        self.parent = Some(parent_event.get_id().clone());
        self
    }
}

impl PubEventBuilder for CommandEventBuilder {
    fn track(&self, event: Event) {
        let val = match event.is_sensitive {
            EventType::Sensitive => TelemetryConfig::one_way_hash(&event.value),
            EventType::NonSensitive => event.value.to_string(),
        };

        telem(TelemetryEvent::Command(EventData {
            id: self.id.clone(),
            command: self.command.clone(),
            parent: self.parent.clone(),
            key: event.key,
            value: val,
        }));
    }

    fn child(&self) -> Self {
        Self::new(&self.command).with_parent(self)
    }
}

// events
impl CommandEventBuilder {
    pub fn new(command: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            command: command.to_string(),
            parent: None,
        }
    }

    pub fn track_call(self) -> Self {
        self.track(Event {
            key: "command".to_string(),
            value: "called".to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }
}
