mod bump_timeout;
mod bump_timeout_layer;
mod client;
mod connector;
pub(crate) mod endpoint;
mod server;

pub use client::{DaemonClient, DaemonError};
pub use connector::{DaemonConnector, DaemonConnectorError};
pub use server::{serve, CloseReason};

pub(crate) mod proto {
    tonic::include_proto!("turbodprotocol");

    /// The version of the protocol that this library implements.
    ///
    /// Protocol buffers aim to be backward and forward compatible at a protocol
    /// level, however that doesn't mean that our daemon will have the same
    /// logical API. We may decide to change the API in the future, and this
    /// version number will be used to indicate that.
    ///
    /// Changes are driven by the server changing its implementation.
    ///
    /// Guideline for bumping the daemon protocol version:
    /// - Bump the major version if making backwards incompatible changes.
    /// - Bump the minor version if adding new features, such that clients can
    ///   mandate at least some set of features on the target server.
    /// - Bump the patch version if making backwards compatible bug fixes.
    pub const VERSION: &str = "1.10.17";
}
