//! Standard opt-out telemetry for setup outcomes. It starts only after the
//! command finished and only when the experimental gate passed, on its own
//! small runtime, so it can never block, fail, or change setup output or exit
//! status. Events carry closed categories and counters only; see
//! `turborepo_telemetry::events::setup`.

use std::time::Duration;

use turborepo_telemetry::events::{
    EventBuilder,
    command::CommandEventBuilder,
    generic::GenericEventBuilder,
    setup::{SetupDependencies, SetupFailure, SetupLock, SetupMode, SetupReady},
};
use turborepo_ui::ColorConfig;

use super::{Error, LockMode, Mode, SetupRequest};

/// What setup observed while running. Nothing here is a path, URL, version,
/// lock entry, or argument.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Report {
    pub(super) gated: bool,
    pub(super) request: Option<(SetupMode, SetupLock, SetupDependencies)>,
    pub(super) downloaded_bytes: u64,
    pub(super) tools_installed: usize,
    pub(super) tools_reused: usize,
}

impl Report {
    pub fn gate_passed(&mut self) {
        self.gated = true;
    }

    pub fn request(&mut self, request: &SetupRequest) {
        self.request = Some((
            match request.mode {
                Mode::Provision => SetupMode::Provision,
                Mode::Check => SetupMode::Check,
                Mode::Plan => SetupMode::Plan,
            },
            match request.lock {
                LockMode::Write => SetupLock::Write,
                LockMode::Frozen => SetupLock::Frozen,
                LockMode::NoLock => SetupLock::NoLock,
            },
            if request.tools_only {
                SetupDependencies::Skip
            } else {
                SetupDependencies::Locked
            },
        ));
    }
}

// `Unsupported` carries fixed messages; unmatched ones stay `Unsupported`.
pub(super) fn failure(error: &Error) -> SetupFailure {
    use turborepo_setup::activation::Error as Activation;
    match error {
        Error::Disabled | Error::FrozenUpdateLock => SetupFailure::Request,
        Error::MuslHost | Error::UnclassifiedLibc(_) => SetupFailure::Platform,
        Error::LockDiagnostics(super::lock_diagnostics::Error::Missing) => {
            SetupFailure::LockMissing
        }
        Error::LockDiagnostics(super::lock_diagnostics::Error::Drift(_)) => SetupFailure::LockStale,
        Error::Offline(_) => SetupFailure::Readiness,
        Error::OfflineMissing(_) => SetupFailure::Download,
        Error::SystemTool(_) => SetupFailure::Readiness,
        Error::Native(_) => SetupFailure::Unsupported,
        Error::Ephemeral(_) => SetupFailure::Resolution,
        Error::NotImplemented => SetupFailure::Unsupported,
        Error::Unsupported(message) => match *message {
            "managed promotion requires macOS or GNU Linux"
            | "cannot select setup check host platform" => SetupFailure::Platform,
            "frozen mode requires turbo.lock" | "locked provisioning requires turbo.lock" => {
                SetupFailure::LockMissing
            }
            "turbo.lock declarations changed; run setup locally and commit the updated lock"
            | "locked Node does not satisfy native declarations" => SetupFailure::LockStale,
            "invalid locked Node version" => SetupFailure::LockInvalid,
            "a native Node declaration is required"
            | "setup discovery changed during check; retry turbo setup"
            | "setup discovery changed during provisioning" => SetupFailure::Discovery,
            "unsafe .turbo storage path"
            | "storage must be untracked and Git-ignored; add /.turbo/ to .gitignore" => {
                SetupFailure::Install
            }
            _ => SetupFailure::Unsupported,
        },
        Error::SourcePolicy(_) => SetupFailure::SourcePolicy,
        Error::Activation(error) => match error {
            Activation::MissingLock => SetupFailure::LockMissing,
            Activation::DeclarationDrift => SetupFailure::LockStale,
            Activation::InvalidLock => SetupFailure::LockInvalid,
            Activation::UnsupportedAdapter => SetupFailure::Unsupported,
            Activation::UnsupportedPlatform => SetupFailure::Platform,
            _ => SetupFailure::Readiness,
        },
        Error::Storage(_) | Error::Lock(_) => SetupFailure::LockInvalid,
        Error::Reconcile(_) => SetupFailure::Resolution,
        Error::Node(_) | Error::Registry(_) => SetupFailure::Download,
        Error::Install(_) => SetupFailure::Install,
        Error::Root(_) | Error::Path(_) => SetupFailure::Discovery,
        Error::Io(_) => SetupFailure::Io,
    }
}

/// Best effort: every failure here is swallowed after a debug log.
pub(super) fn record(
    report: &Report,
    result: &Result<i32, Error>,
    elapsed: Duration,
    color_config: ColorConfig,
) {
    // Without the future flag, setup keeps its previous behavior: no notice,
    // telemetry config, or traffic.
    if !report.gated {
        return;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::debug!("setup telemetry unavailable: {error}");
            return;
        }
    };
    runtime.block_on(async {
        let Some(handle) = crate::cli::initialize_deferred_telemetry_client(
            turborepo_api_client::SharedHttpClient::new(),
            color_config,
            crate::get_version(),
        ) else {
            return;
        };
        // The standard root event, without argument tracking.
        let root = GenericEventBuilder::new();
        root.track_start();
        root.track_platform(turborepo_shim::TurboState::platform_name());
        root.track_version(turborepo_shim::TurboState::version());
        root.track_ai_agent(turborepo_ai_agents::get_agent());
        let event = CommandEventBuilder::new("setup").with_parent(&root);
        event.track_call();
        if let Some((mode, lock, dependencies)) = report.request {
            event.track_setup_request(mode, lock, dependencies);
        }
        match result {
            Ok(_) => event.track_setup_ready(&SetupReady {
                elapsed,
                downloaded_bytes: report.downloaded_bytes,
                tools_installed: report.tools_installed,
                tools_reused: report.tools_reused,
            }),
            Err(error) => event.track_setup_failure(failure(error), elapsed),
        };
        root.track_end();
        handle.close_with_timeout().await;
    });
    // Like every other command, never wait for in-flight telemetry requests.
    runtime.shutdown_background();
}

#[cfg(test)]
mod tests {
    use turborepo_setup::activation::Error as Activation;

    use super::*;

    #[test]
    fn failures_map_to_specific_categories() {
        let unsupported = |message| failure(&Error::Unsupported(message));
        assert_eq!(failure(&Error::FrozenUpdateLock), SetupFailure::Request);
        assert_eq!(failure(&Error::NotImplemented), SetupFailure::Unsupported);
        assert_eq!(
            unsupported("managed promotion requires macOS or GNU Linux"),
            SetupFailure::Platform
        );
        assert_eq!(
            unsupported("frozen mode requires turbo.lock"),
            SetupFailure::LockMissing
        );
        assert_eq!(
            unsupported(
                "turbo.lock declarations changed; run setup locally and commit the updated lock"
            ),
            SetupFailure::LockStale
        );
        assert_eq!(
            unsupported("a native Node declaration is required"),
            SetupFailure::Discovery
        );
        assert_eq!(
            unsupported("npm override provisioning"),
            SetupFailure::Unsupported
        );
        for (error, expected) in [
            (Activation::MissingLock, SetupFailure::LockMissing),
            (Activation::DeclarationDrift, SetupFailure::LockStale),
            (Activation::MissingInventory, SetupFailure::Readiness),
            (Activation::StaleInventory, SetupFailure::Readiness),
        ] {
            assert_eq!(failure(&Error::Activation(error)), expected);
        }
    }
}
