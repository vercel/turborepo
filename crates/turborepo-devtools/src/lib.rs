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

/// Default port for the devtools WebSocket server
pub const DEFAULT_PORT: u16 = 9876;

/// Find an available port, starting from the requested port.
/// If the requested port is in use, finds an open one.
pub fn find_available_port(requested: u16) -> u16 {
    if port_scanner::scan_port(requested) {
        // Port is in use, find another
        port_scanner::request_open_port().unwrap_or(requested + 1)
    } else {
        requested
    }
}
