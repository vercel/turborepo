use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Event, EventBuilder, EventType, PubEventBuilder, TelemetryEvent};
use crate::{config::TelemetryConfig, telem};

// task names that will be passed through to the API without obfuscation
const ALLOWLIST: [&str; 8] = [
    "build",
    "test",
    "lint",
    "typecheck",
    "checktypes",
    "check-types",
    "type-check",
    "check",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventData {
    id: String,
    package: String,
    task: String,
    key: String,
    value: String,
    parent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageTaskEventBuilder {
    id: String,
    package: String,
    task: String,
    parent: Option<String>,
}

impl EventBuilder<PackageTaskEventBuilder> for PackageTaskEventBuilder {
    fn get_id(&self) -> &String {
        &self.id
    }

    fn with_parent(mut self, parent_event: &PackageTaskEventBuilder) -> Self {
        self.parent = Some(parent_event.get_id().clone());
        self
    }
}

impl PubEventBuilder for PackageTaskEventBuilder {
    fn track(&self, event: Event) {
        let val = match event.is_sensitive {
            EventType::Sensitive => {
                let config = TelemetryConfig::new().unwrap();
                config.salt(&event.value)
            }
            EventType::NonSensitive => event.value.to_string(),
        };

        telem(TelemetryEvent::PackageTask(EventData {
            id: self.id.clone(),
            package: self.package.clone(),
            task: self.task.clone(),
            parent: self.parent.clone(),
            key: event.key,
            value: val,
        }));
    }

    fn child(&self) -> Self {
        Self::new(&self.package, &self.task).with_parent(self)
    }
}

impl PackageTaskEventBuilder {
    pub fn new(package: &str, task: &str) -> Self {
        // TODO don't unwrap this
        let config = TelemetryConfig::new().unwrap();

        // don't obfuscate the package in development mode
        let package = if cfg!(debug_assertions) {
            package.to_string()
        } else {
            config.salt(package)
        };

        // don't obfuscate the task in development mode or if it's in the allowlist
        let task = if cfg!(debug_assertions) || ALLOWLIST.contains(&task) {
            task.to_string()
        } else {
            config.salt(task)
        };

        Self {
            id: Uuid::new_v4().to_string(),
            parent: None,
            package,
            task,
        }
    }

    // event methods
    pub fn track_recursive_error(self) -> Self {
        self.track(Event {
            key: "error".to_string(),
            value: "recursive".to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_hash(self, hash: &str) -> Self {
        self.track(Event {
            key: "hash".to_string(),
            value: hash.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_framework(self, framework: &str) -> Self {
        self.track(Event {
            key: "framework".to_string(),
            value: framework.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }
}
