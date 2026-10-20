//! Turborepo Devtools
//!
//! A WebSocket-based devtools server that allows visualization of package
//! and task graphs in real-time. Changes to the repository are detected
//! via file watching and pushed to connected clients.

#![deny(clippy::all)]

mod graph;
mod server;
mod types;
mod watcher;

pub use server::{DevtoolsServer, ServerError};
pub use types::*;
pub use watcher::{DevtoolsWatcher, WatchError, WatchEvent};

pub use crate::graph::package_graph_to_data;

/// Default port for the devtools WebSocket server
pub const DEFAULT_PORT: u16 = 9876;

/// Find an available port, starting from the requested port.
/// If the requested port is in use, finds an open one.
pub fn find_available_port(requested: u16) -> u16 {
    // Probe the same loopback address the devtools server binds to. Connecting
    // to 0.0.0.0 does not reliably detect an occupied port on Windows.
    if std::net::TcpListener::bind(("127.0.0.1", requested)).is_err() {
        std::net::TcpListener::bind("127.0.0.1:0")
            .ok()
            .and_then(|listener| listener.local_addr().ok().map(|addr| addr.port()))
            .unwrap_or(requested.saturating_add(1))
    } else {
        requested
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::find_available_port;

    #[test]
    fn keeps_unoccupied_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        assert_eq!(find_available_port(port), port);
    }

    #[test]
    fn chooses_another_port_when_occupied() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let selected = find_available_port(port);
        assert_ne!(selected, port);
        assert_ne!(selected, 0);
    }
}
