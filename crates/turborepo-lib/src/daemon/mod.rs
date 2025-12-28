//! The Turborepo daemon watches files and pre-computes data to speed up turbo's
//! execution. Each repository has a separate daemon instance.
//!
//! # Architecture
//! The daemon consists of a gRPC server that can be queried by a client.

//! The server spins up a `FileWatching` struct, which contains a struct
//! responsible for watching the repository (`FileSystemWatcher`), and the
//! various consumers of that file change data such as `GlobWatcher` and
//! `PackageWatcher`.
//!
//! We use cookie files to ensure proper event synchronization, i.e.
//! that we don't get stale file system events while handling queries.
//!
//! # Naming Conventions
//! `recv` is a receiver of file system events. Structs such as `GlobWatcher`
//! or `PackageWatcher` consume these file system events and either derive state
//! or produce new events.
//!
//! `_tx`/`_rx` suffixes indicate that this variable is respectively a `Sender`
//! or `Receiver`.

// Re-export everything from turborepo-daemon crate
pub use turborepo_daemon::{
    proto, CloseReason, DaemonClient, DaemonConnector, DaemonConnectorError, DaemonError,
    PackageChangesWatcher as PackageChangesWatcherTrait, Paths, TurboGrpcService,
};

// Keep endpoint accessible for internal use
pub(crate) mod endpoint {
    pub use turborepo_daemon::endpoint::*;
}
