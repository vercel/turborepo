use std::fmt::Display;

use serde::{Deserialize, Serialize};
use turborepo_ci;
use turborepo_vercel_api::telemetry::{TelemetryCommandEvent, TelemetryEvent};
use uuid::Uuid;

use super::{Event, EventBuilder, EventType, Identifiable};
use crate::{config::TelemetryConfig, telem};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEventBuilder {
    id: String,
    command: String,
    parent_id: Option<String>,
    is_ci: bool,
}

impl Identifiable for CommandEventBuilder {
    fn get_id(&self) -> &String {
        &self.id
    }
}

impl EventBuilder for CommandEventBuilder {
    fn with_parent<U: Identifiable>(mut self, parent_event: &U) -> Self {
        self.parent_id = Some(parent_event.get_id().clone());
        self
    }

    fn track(&self, event: Event) {
        if self.is_ci && !event.send_in_ci {
            return;
        }

        let val = match event.is_sensitive {
            EventType::Sensitive => TelemetryConfig::one_way_hash(&event.value),
            EventType::NonSensitive => event.value.to_string(),
        };

        telem(TelemetryEvent::Command(TelemetryCommandEvent {
            id: self.id.clone(),
            command: self.command.clone(),
            parent_id: self.parent_id.clone(),
            key: event.key,
            value: val,
        }));
    }

    fn child(&self) -> Self {
        Self::new(&self.command).with_parent(self)
    }
}

// events

#[derive(Debug, Clone, Serialize, Deserialize, Copy)]
pub enum LoginMethod {
    SSO,
    Standard,
}

impl CommandEventBuilder {
    pub fn new(command: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            command: command.to_string(),
            parent_id: None,
            is_ci: turborepo_ci::is_ci(),
        }
    }

    pub fn track_call(&self) -> &Self {
        self.track(Event {
            key: "command".to_string(),
            value: "called".to_string(),
            is_sensitive: EventType::NonSensitive,
            send_in_ci: true,
        });
        self
    }

    // args
    pub fn track_arg_usage(&self, arg: &str, is_set: bool) -> &Self {
        self.track(Event {
            key: format!("arg:{}", arg),
            value: if is_set { "set" } else { "default" }.to_string(),
            is_sensitive: EventType::NonSensitive,
            send_in_ci: true,
        });
        self
    }

    pub fn track_arg_value(&self, arg: &str, val: impl Display, is_sensitive: EventType) -> &Self {
        self.track(Event {
            key: format!("arg:{}", arg),
            value: val.to_string(),
            is_sensitive,
            send_in_ci: true,
        });
        self
    }

    // telemetry
    pub fn track_telemetry_config(&self, enabled: bool) -> &Self {
        self.track(Event {
            key: "action".to_string(),
            value: if enabled { "enabled" } else { "disabled" }.to_string(),
            is_sensitive: EventType::NonSensitive,
            send_in_ci: false,
        });
        self
    }

    // gen
    pub fn track_generator_option(&self, option: &str) -> &Self {
        self.track(Event {
            key: "option".to_string(),
            value: option.to_string(),
            is_sensitive: EventType::NonSensitive,
            send_in_ci: false,
        });
        self
    }

    pub fn track_generator_tag(&self, tag: &str) -> &Self {
        self.track(Event {
            key: "tag".to_string(),
            value: tag.to_string(),
            is_sensitive: EventType::NonSensitive,
            send_in_ci: false,
        });
        self
    }

    // login
    pub fn track_login_method(&self, method: LoginMethod) -> &Self {
        self.track(Event {
            key: "method".to_string(),
            value: match method {
                LoginMethod::SSO => "sso".to_string(),
                LoginMethod::Standard => "standard".to_string(),
            },
            is_sensitive: EventType::NonSensitive,
            send_in_ci: false,
        });
        self
    }

    // Successful/Failed logins
    pub fn track_login_success(&self, succeeded: bool) -> &Self {
        self.track(Event {
            key: "success".to_string(),
            value: succeeded.to_string(),
            is_sensitive: EventType::NonSensitive,
            send_in_ci: false,
        });
        self
    }
}
