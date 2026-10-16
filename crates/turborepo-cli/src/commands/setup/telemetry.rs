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

fn native_failure(error: &turborepo_setup::native_baseline::Error) -> SetupFailure {
    use turborepo_setup::native_baseline::Error as Native;
    match error {
        Native::Invalid | Native::Storage(_) => SetupFailure::LockInvalid,
        Native::RuntimeArrayGap(_) => SetupFailure::Unsupported,
        Native::Install(_) => SetupFailure::Install,
    }
}

fn activation_failure(error: &turborepo_setup::activation::Error) -> SetupFailure {
    use turborepo_setup::activation::Error as Activation;
    match error {
        Activation::MissingLock => SetupFailure::LockMissing,
        Activation::DeclarationDrift => SetupFailure::LockStale,
        Activation::InvalidLock => SetupFailure::LockInvalid,
        Activation::UnsupportedAdapter => SetupFailure::Unsupported,
        Activation::UnsupportedPlatform => SetupFailure::Platform,
        _ => SetupFailure::Readiness,
    }
}

// Failures before the installer runs keep their tool-side categories; the
// native install itself, its lifecycle policy, and its host are
// `Dependencies`.
fn dependencies_failure(error: &super::dependencies::Error) -> SetupFailure {
    use super::dependencies::Error as Dependencies;
    match error {
        Dependencies::UndeclaredManager => SetupFailure::Discovery,
        Dependencies::Unlocked(_) | Dependencies::Sources(_) => SetupFailure::LockInvalid,
        Dependencies::NotInstalled(_) | Dependencies::Untrusted => SetupFailure::Readiness,
        // Refused before any installer runs: these installers have no
        // offline mode turbo can enforce.
        Dependencies::OfflineUnsupported(_) => SetupFailure::Unsupported,
        Dependencies::Activation(error) => activation_failure(error),
        Dependencies::Host(_)
        | Dependencies::Install(_)
        | Dependencies::Lifecycle(_)
        | Dependencies::Uv(_)
        | Dependencies::Cargo(_)
        | Dependencies::Go(_)
        | Dependencies::Io(_) => SetupFailure::Dependencies,
    }
}

// `Unsupported` carries fixed messages; unmatched ones stay `Unsupported`.
pub(super) fn failure(error: &Error) -> SetupFailure {
    match error {
        Error::Disabled | Error::FrozenUpdateLock => SetupFailure::Request,
        Error::NotImplemented => SetupFailure::Unsupported,
        // #14701's --no-lock refusals: like "a native Node declaration is
        // required", and an unsupported clone-local tool.
        Error::NoLockNode => SetupFailure::Discovery,
        Error::NoLockTool(_) => SetupFailure::Unsupported,
        Error::MuslHost | Error::UnclassifiedLibc(_) => SetupFailure::Platform,
        Error::LockDiagnostics(super::lock_diagnostics::Error::Missing) => {
            SetupFailure::LockMissing
        }
        Error::LockDiagnostics(super::lock_diagnostics::Error::Drift(_)) => SetupFailure::LockStale,
        Error::Offline(_) => SetupFailure::Readiness,
        Error::OfflineMissing(_) => SetupFailure::Download,
        Error::Unsupported(message) => match *message {
            "managed promotion requires macOS or GNU Linux"
            | "managed promotion requires macOS, GNU Linux or Windows x64"
            | "managed setup is not qualified for Windows arm64, including x64 turbo under \
             emulation; use WSL or a Windows x64 machine"
            | "managed setup could not confirm a Windows x64 host"
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
        Error::Activation(error) => activation_failure(error),
        Error::Dependencies(error) => dependencies_failure(error),
        Error::Storage(_) | Error::Lock(_) => SetupFailure::LockInvalid,
        Error::Reconcile(_) => SetupFailure::Resolution,
        Error::UvResolution(_) | Error::PythonResolution(_) | Error::GoResolution(_) => {
            SetupFailure::Resolution
        }
        Error::Node(_)
        | Error::Registry(_)
        | Error::Rustup(_)
        | Error::Rust(_)
        | Error::Uv(_)
        | Error::Python(_)
        | Error::Go(_)
        | Error::Bun(_)
        | Error::Artifact(_) => SetupFailure::Download,
        Error::AdditionalToolDrift(_) => SetupFailure::LockStale,
        Error::Mirror(_) => SetupFailure::Request,
        Error::Native(error) => native_failure(error),
        Error::Ephemeral(error) => {
            use turborepo_setup::ephemeral_reconcile::Error as Ephemeral;
            match error {
                Ephemeral::Unsupported => SetupFailure::Unsupported,
                Ephemeral::Native(error) => native_failure(error),
                Ephemeral::Reconcile(_) => SetupFailure::Resolution,
                Ephemeral::Storage(_) => SetupFailure::LockInvalid,
                Ephemeral::Install(_) => SetupFailure::Install,
                Ephemeral::Node(_) | Ephemeral::Pnpm(_) => SetupFailure::Download,
                Ephemeral::Check(_) => SetupFailure::Io,
            }
        }
        Error::Install(_) => SetupFailure::Install,
        Error::Root(_) | Error::Path(_) => SetupFailure::Discovery,
        Error::Io(_) => SetupFailure::Io,
        // The Windows Ctrl+C relay failed to register.
        #[cfg(windows)]
        Error::Signal(_) => SetupFailure::Io,
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
        for message in [
            "managed promotion requires macOS or GNU Linux",
            "managed promotion requires macOS, GNU Linux or Windows x64",
            "managed setup is not qualified for Windows arm64, including x64 turbo under \
             emulation; use WSL or a Windows x64 machine",
            "managed setup could not confirm a Windows x64 host",
        ] {
            assert_eq!(unsupported(message), SetupFailure::Platform);
        }
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
            (
                Error::Registry(turborepo_setup::npm_provision::Error::InvalidLock),
                SetupFailure::Download,
            ),
            (
                Error::Native(turborepo_setup::native_baseline::Error::Invalid),
                SetupFailure::LockInvalid,
            ),
            (
                Error::Native(turborepo_setup::native_baseline::Error::RuntimeArrayGap(42)),
                SetupFailure::Unsupported,
            ),
            (
                Error::Ephemeral(turborepo_setup::ephemeral_reconcile::Error::Unsupported),
                SetupFailure::Unsupported,
            ),
            (
                Error::Ephemeral(turborepo_setup::ephemeral_reconcile::Error::Check(
                    std::io::Error::other("private repository path and credentials"),
                )),
                SetupFailure::Io,
            ),
        ] {
            assert_eq!(failure(&error), expected);
        }
        for (error, expected) in [
            (Activation::MissingLock, SetupFailure::LockMissing),
            (Activation::DeclarationDrift, SetupFailure::LockStale),
            (Activation::MissingInventory, SetupFailure::Readiness),
            (Activation::StaleInventory, SetupFailure::Readiness),
        ] {
            assert_eq!(failure(&Error::Activation(error)), expected);
        }
        use super::super::dependencies::Error as Dependencies;
        for (error, expected) in [
            (Dependencies::UndeclaredManager, SetupFailure::Discovery),
            (Dependencies::Unlocked("pnpm"), SetupFailure::LockInvalid),
            (Dependencies::Untrusted, SetupFailure::Readiness),
            (
                Dependencies::OfflineUnsupported("private installer detail".into()),
                SetupFailure::Unsupported,
            ),
            (
                Dependencies::Activation(Activation::DeclarationDrift),
                SetupFailure::LockStale,
            ),
            (
                Dependencies::Host("private host detail".into()),
                SetupFailure::Dependencies,
            ),
            (
                Dependencies::Io(std::io::Error::other("private path")),
                SetupFailure::Dependencies,
            ),
            (
                Dependencies::Uv(turborepo_setup::uv_sync::Error::Failed(2)),
                SetupFailure::Dependencies,
            ),
            (
                Dependencies::Cargo(turborepo_setup::cargo_prefetch::Error::StaleLock),
                SetupFailure::Dependencies,
            ),
            (
                Dependencies::Go(turborepo_setup::go_prefetch::Error::MissingSums(
                    "private module".into(),
                )),
                SetupFailure::Dependencies,
            ),
        ] {
            assert_eq!(failure(&Error::Dependencies(error)), expected);
        }
    }
}
