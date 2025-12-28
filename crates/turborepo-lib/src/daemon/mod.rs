//! Deprecated: Use `turborepo_daemon` crate directly instead.
//!
//! This module re-exports types from the `turborepo_daemon` crate for backward
//! compatibility. New code should depend on `turborepo_daemon` directly.

// These re-exports are intentionally unused within this crate - they exist
// only for backward compatibility with external consumers.
#![allow(unused_imports)]

#[deprecated(since = "2.4.0", note = "use `turborepo_daemon::proto` instead")]
pub use turborepo_daemon::proto;
#[deprecated(since = "2.4.0", note = "use `turborepo_daemon::CloseReason` instead")]
pub use turborepo_daemon::CloseReason;
#[deprecated(since = "2.4.0", note = "use `turborepo_daemon::DaemonClient` instead")]
pub use turborepo_daemon::DaemonClient;
#[deprecated(
    since = "2.4.0",
    note = "use `turborepo_daemon::DaemonConnector` instead"
)]
pub use turborepo_daemon::DaemonConnector;
#[deprecated(
    since = "2.4.0",
    note = "use `turborepo_daemon::DaemonConnectorError` instead"
)]
pub use turborepo_daemon::DaemonConnectorError;
#[deprecated(since = "2.4.0", note = "use `turborepo_daemon::DaemonError` instead")]
pub use turborepo_daemon::DaemonError;
#[deprecated(
    since = "2.4.0",
    note = "use `turborepo_daemon::PackageChangesWatcher` instead"
)]
pub use turborepo_daemon::PackageChangesWatcher as PackageChangesWatcherTrait;
#[deprecated(since = "2.4.0", note = "use `turborepo_daemon::Paths` instead")]
pub use turborepo_daemon::Paths;
#[deprecated(
    since = "2.4.0",
    note = "use `turborepo_daemon::TurboGrpcService` instead"
)]
pub use turborepo_daemon::TurboGrpcService;
