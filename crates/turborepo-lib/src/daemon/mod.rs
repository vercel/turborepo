mod client;
mod connector;
pub(crate) mod endpoint;
mod server;

pub use client::{DaemonClient, DaemonError};
pub use connector::DaemonConnector;
pub use server::DaemonServer;

pub(crate) mod proto {
    tonic::include_proto!("turbodprotocol");
}
