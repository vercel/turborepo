use serde::{Deserialize, Serialize};
use turborepo_vercel_api::{TelemetryEvent, TelemetryRepoEvent};
use uuid::Uuid;

use super::{Event, EventBuilder, EventType, Identifiable};
use crate::{config::TelemetryConfig, telem};

pub enum RepoType {
    SinglePackage,
    Monorepo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEventBuilder {
    id: String,
    repo: String,
    parent_id: Option<String>,
}

impl Identifiable for RepoEventBuilder {
    fn get_id(&self) -> &String {
        &self.id
    }
}

impl EventBuilder for RepoEventBuilder {
    fn with_parent<U: Identifiable>(mut self, parent_event: &U) -> Self {
        self.parent_id = Some(parent_event.get_id().clone());
        self
    }

    fn track(&self, event: Event) {
        let val = match event.is_sensitive {
            EventType::Sensitive => TelemetryConfig::one_way_hash(&event.value),
            EventType::NonSensitive => event.value.to_string(),
        };

        telem(TelemetryEvent::Repo(TelemetryRepoEvent {
            id: self.id.clone(),
            repo: self.repo.clone(),
            key: event.key,
            value: val,
            parent_id: self.parent_id.clone(),
        }));
    }

    fn child(&self) -> Self {
        Self::new(&self.repo).with_parent(self)
    }
}

// events
impl RepoEventBuilder {
    pub fn new(repo_identifier: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            repo: TelemetryConfig::one_way_hash(repo_identifier),
            parent_id: None,
        }
    }

    pub fn track_package_manager(&self, name: String) -> &Self {
        self.track(Event {
            key: "package_manager".to_string(),
            value: name.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_type(&self, repo_type: RepoType) -> &Self {
        self.track(Event {
            key: "repo_type".to_string(),
            value: match repo_type {
                RepoType::SinglePackage => "single_package".to_string(),
                RepoType::Monorepo => "monorepo".to_string(),
            },
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_size(&self, size: usize) -> &Self {
        self.track(Event {
            key: "workspace_count".to_string(),
            value: size.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }
}
