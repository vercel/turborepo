use serde::{Deserialize, Serialize};
use turborepo_vercel_api::{TelemetryEvent, TelemetryGenericEvent};
use uuid::Uuid;

use super::{Event, EventBuilder, EventType, Identifiable};
use crate::{config::TelemetryConfig, telem};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericEventBuilder {
    id: String,
    parent_id: Option<String>,
}

impl Identifiable for GenericEventBuilder {
    fn get_id(&self) -> &String {
        &self.id
    }
}

impl EventBuilder for GenericEventBuilder {
    fn with_parent<U: Identifiable>(mut self, parent_event: &U) -> Self {
        self.parent_id = Some(parent_event.get_id().clone());
        self
    }

    fn track(&self, event: Event) {
        let val = match event.is_sensitive {
            EventType::Sensitive => TelemetryConfig::one_way_hash(&event.value),
            EventType::NonSensitive => event.value.to_string(),
        };

        telem(TelemetryEvent::Generic(TelemetryGenericEvent {
            id: self.id.clone(),
            parent_id: self.parent_id.clone(),
            key: event.key,
            value: val,
        }));
    }

    fn child(&self) -> Self {
        Self::new().with_parent(self)
    }
}

impl Default for GenericEventBuilder {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            parent_id: None,
        }
    }
}

// events
impl GenericEventBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn track_start(&self) -> &Self {
        self.track(Event {
            key: "execution".to_string(),
            value: "started".to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_end(&self) -> &Self {
        self.track(Event {
            key: "execution".to_string(),
            value: "ended".to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_success(&self) -> &Self {
        self.track(Event {
            key: "execution".to_string(),
            value: "succeeded".to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_failure(&self) -> &Self {
        self.track(Event {
            key: "execution".to_string(),
            value: "failed".to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_platform(&self, platform: &str) -> &Self {
        self.track(Event {
            key: "platform".to_string(),
            value: platform.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_cpus(&self, cpus: usize) -> &Self {
        self.track(Event {
            key: "cpu_count".to_string(),
            value: cpus.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_version(&self, version: &str) -> &Self {
        self.track(Event {
            key: "turbo_version".to_string(),
            value: version.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }
}
