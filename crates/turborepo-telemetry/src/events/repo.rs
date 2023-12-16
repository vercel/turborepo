use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Event, EventBuilder, EventType, PubEventBuilder, TelemetryEvent};
use crate::{config::TelemetryConfig, telem};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventData {
    id: String,
    repo: String,
    key: String,
    value: String,
    parent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEventBuilder {
    id: String,
    repo: String,
    parent: Option<String>,
}

impl EventBuilder<RepoEventBuilder> for RepoEventBuilder {
    fn get_id(&self) -> &String {
        &self.id
    }

    fn with_parent(mut self, parent_event: &RepoEventBuilder) -> Self {
        self.parent = Some(parent_event.get_id().clone());
        self
    }
}

impl PubEventBuilder for RepoEventBuilder {
    fn track(&self, event: Event) {
        let val = match event.is_sensitive {
            EventType::Sensitive => TelemetryConfig::one_way_hash(&event.value),
            EventType::NonSensitive => event.value.to_string(),
        };

        telem(TelemetryEvent::Repo(EventData {
            id: self.id.clone(),
            repo: self.repo.clone(),
            key: event.key,
            value: val,
            parent: self.parent.clone(),
        }));
    }

    fn child(&self) -> Self {
        Self::new(&self.repo).with_parent(self)
    }
}

// events
impl RepoEventBuilder {
    pub fn new(repo: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            repo: TelemetryConfig::one_way_hash(repo),
            parent: None,
        }
    }

    pub fn track_package_manager_name(self, name: &str) -> Self {
        self.track(Event {
            key: "package_manager_name".to_string(),
            value: name.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_package_manager_version(self, version: &str) -> Self {
        self.track(Event {
            key: "package_manager_version".to_string(),
            value: version.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_is_monorepo(self, is_monorepo: bool) -> Self {
        self.track(Event {
            key: "is_monorepo".to_string(),
            value: is_monorepo.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }
}
