//! `turbo setup` outcome events on the standard command channel. Every value
//! is a closed vocabulary or a counter: never paths, URLs, tool versions, lock
//! contents, credentials, or command arguments. Sent in CI as well, since CI
//! setup is a primary audience.

use std::{fmt, time::Duration};

use super::{Event, EventBuilder, EventType, command::CommandEventBuilder};

// Each variant's wire value sits next to it; the backend aggregates on these.
macro_rules! vocabulary {
    ($(#[$meta:meta])* $name:ident { $($(#[$doc:meta])* $variant:ident => $value:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($(#[$doc])* $variant,)+
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self {
                    $(Self::$variant => $value,)+
                })
            }
        }
    };
}

vocabulary!(SetupMode {
    Provision => "provision",
    Check => "check",
    Plan => "plan",
});

vocabulary!(SetupLock {
    Write => "write",
    Frozen => "frozen",
    NoLock => "no-lock",
});

vocabulary!(
    /// Tools-only setup versus locked dependency installation.
    SetupDependencies {
        Skip => "skip",
        Locked => "locked",
    }
);

vocabulary!(
    /// Coarse failure categories. Local diagnostics carry the details.
    SetupFailure {
        /// Invalid flag combinations.
        Request => "request",
        /// A recognized mode, adapter, or input that is not supported yet.
        Unsupported => "unsupported",
        /// An unqualified OS, architecture, or libc.
        Platform => "platform",
        /// Root, configuration, or native declaration discovery.
        Discovery => "discovery",
        SourcePolicy => "source_policy",
        LockMissing => "lock_missing",
        /// Declarations changed without a lock update (frozen/CI drift).
        LockStale => "lock_stale",
        LockInvalid => "lock_invalid",
        /// Installed tools are missing, stale, or damaged.
        Readiness => "readiness",
        Resolution => "resolution",
        /// Downloading or verifying artifacts.
        Download => "download",
        /// Writing the repository-local installation.
        Install => "install",
        /// Running a native locked dependency install with the managed tools.
        Dependencies => "dependencies",
        Io => "io",
    }
);

/// Aggregate facts about a ready setup. Counts cover managed tools only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetupReady {
    pub elapsed: Duration,
    pub downloaded_bytes: u64,
    pub tools_installed: usize,
    pub tools_reused: usize,
}

fn event(key: &str, value: impl fmt::Display) -> Event {
    Event {
        key: key.to_owned(),
        value: value.to_string(),
        is_sensitive: EventType::NonSensitive,
        send_in_ci: true,
    }
}

fn request_events(mode: SetupMode, lock: SetupLock, deps: SetupDependencies) -> [Event; 3] {
    [
        event("mode", mode),
        event("lock", lock),
        event("dependencies", deps),
    ]
}

fn ready_events(ready: &SetupReady) -> [Event; 5] {
    [
        event("outcome", "ready"),
        event("duration_ms", ready.elapsed.as_millis()),
        event("downloaded_bytes", ready.downloaded_bytes),
        event("tools_installed", ready.tools_installed),
        event("tools_reused", ready.tools_reused),
    ]
}

fn failure_events(failure: SetupFailure, elapsed: Duration) -> [Event; 3] {
    [
        event("outcome", "failed"),
        event("failure", failure),
        event("duration_ms", elapsed.as_millis()),
    ]
}

impl CommandEventBuilder {
    fn track_all<const N: usize>(&self, events: [Event; N]) -> &Self {
        if !crate::is_disabled() {
            events.into_iter().for_each(|event| self.track(event));
        }
        self
    }

    pub fn track_setup_request(
        &self,
        mode: SetupMode,
        lock: SetupLock,
        dependencies: SetupDependencies,
    ) -> &Self {
        self.track_all(request_events(mode, lock, dependencies))
    }

    pub fn track_setup_ready(&self, ready: &SetupReady) -> &Self {
        self.track_all(ready_events(ready))
    }

    pub fn track_setup_failure(&self, failure: SetupFailure, elapsed: Duration) -> &Self {
        self.track_all(failure_events(failure, elapsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows<const N: usize>(events: [Event; N]) -> Vec<String> {
        let rows = events.into_iter().map(|event| {
            assert!(event.send_in_ci && matches!(event.is_sensitive, EventType::NonSensitive));
            format!("{}={}", event.key, event.value)
        });
        rows.collect()
    }

    // Stable names: the backend aggregates on these exact keys and values.
    #[test]
    fn setup_event_names_and_values_are_stable() {
        let request = request_events(SetupMode::Check, SetupLock::NoLock, SetupDependencies::Skip);
        assert_eq!(
            rows(request),
            ["mode=check", "lock=no-lock", "dependencies=skip"]
        );
        let ready = ready_events(&SetupReady {
            elapsed: Duration::from_millis(1234),
            downloaded_bytes: 42,
            tools_installed: 1,
            tools_reused: 2,
        });
        let expected = ["outcome=ready", "duration_ms=1234", "downloaded_bytes=42"];
        assert_eq!(
            rows(ready),
            [&expected[..], &["tools_installed=1", "tools_reused=2"]].concat()
        );
        let failure = failure_events(SetupFailure::LockStale, Duration::from_millis(7));
        assert_eq!(
            rows(failure),
            ["outcome=failed", "failure=lock_stale", "duration_ms=7"]
        );
        let values = [
            SetupMode::Provision.to_string(),
            SetupLock::Frozen.to_string(),
            SetupDependencies::Locked.to_string(),
            SetupFailure::SourcePolicy.to_string(),
        ];
        assert_eq!(values, ["provision", "frozen", "locked", "source_policy"]);
    }
}
