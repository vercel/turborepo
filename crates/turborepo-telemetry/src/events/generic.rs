use std::fmt::Display;

use serde::{Deserialize, Serialize};
use turborepo_vercel_api::{TelemetryEvent, TelemetryGenericEvent};
use uuid::Uuid;

use super::{Event, EventBuilder, EventType, Identifiable, TrackedErrors};
use crate::{config::TelemetryConfig, telem};

// Remote cache URL's that will be passed through to the API without obfuscation
const RC_URL_ALLOWLIST: [&str; 1] = ["https://vercel.com/api"];

pub enum DaemonInitStatus {
    // skipped due to context (running in CI etc)
    Skipped,
    /// daemon was started
    Started,
    /// daemon failed to start
    Failed,
    /// daemon was manually disabled by user
    Disabled,
}

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

    // args
    pub fn track_arg_usage(&self, arg: &str, is_set: bool) -> &Self {
        self.track(Event {
            key: format!("arg:{}", arg),
            value: if is_set { "set" } else { "default" }.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_arg_value(&self, arg: &str, val: impl Display, is_sensitive: EventType) -> &Self {
        self.track(Event {
            key: format!("arg:{}", arg),
            value: val.to_string(),
            is_sensitive,
        });
        self
    }

    // run data
    pub fn track_is_linked(&self, is_linked: bool) -> &Self {
        self.track(Event {
            key: "is_linked".to_string(),
            value: if is_linked { "true" } else { "false" }.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_remote_cache(&self, cache_url: &str) -> &Self {
        self.track(Event {
            key: "remote_cache_url".to_string(),
            value: cache_url.to_string(),
            is_sensitive: if RC_URL_ALLOWLIST.contains(&cache_url) {
                EventType::NonSensitive
            } else {
                EventType::Sensitive
            },
        });
        self
    }

    pub fn track_ci(&self, ci: Option<&'static str>) -> &Self {
        if let Some(ci) = ci {
            self.track(Event {
                key: "ci".to_string(),
                value: ci.to_string(),
                is_sensitive: EventType::NonSensitive,
            });
        }
        self
    }

    pub fn track_run_type(&self, is_dry: bool) -> &Self {
        self.track(Event {
            key: "run_type".to_string(),
            value: if is_dry { "dry" } else { "full" }.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    pub fn track_daemon_init(&self, status: DaemonInitStatus) -> &Self {
        self.track(Event {
            key: "daemon_status".to_string(),
            value: match status {
                DaemonInitStatus::Skipped => "skipped".to_string(),
                DaemonInitStatus::Started => "started".to_string(),
                DaemonInitStatus::Failed => "failed".to_string(),
                DaemonInitStatus::Disabled => "disabled".to_string(),
            },
            is_sensitive: EventType::NonSensitive,
        });
        self
    }

    // errors
    pub fn track_error(&self, error: TrackedErrors) -> &Self {
        self.track(Event {
            key: "error".to_string(),
            value: error.to_string(),
            is_sensitive: EventType::NonSensitive,
        });
        self
    }
}
