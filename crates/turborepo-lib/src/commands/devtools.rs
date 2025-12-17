//! `turbo devtools` command implementation.
//!
//! Starts a WebSocket server that serves package graph data
//! and watches for file changes to push updates.

use turbopath::AbsoluteSystemPathBuf;
use turborepo_devtools::{find_available_port, DevtoolsServer};

use crate::{cli, devtools::ProperTaskGraphBuilder};

// In production, use the hosted devtools UI
// For local development, set TURBO_DEVTOOLS_LOCAL=1 to use localhost:3000
const DEVTOOLS_URL: &str = if cfg!(debug_assertions) {
    "http://localhost:3000/devtools"
} else {
    "https://turborepo.com/devtools"
};

/// Run the devtools server.
pub async fn run(
    repo_root: AbsoluteSystemPathBuf,
    port: u16,
    no_open: bool,
) -> Result<(), cli::Error> {
    // Find available port
    let port = find_available_port(port);

    // Create the task graph builder that uses EngineBuilder
    // This ensures the devtools shows the same task graph as `turbo run`
    let task_graph_builder = ProperTaskGraphBuilder::new(repo_root.clone());

    // Create server with the task graph builder
    let server = DevtoolsServer::new(repo_root, port, task_graph_builder);

    let url = format!("{}?port={}", DEVTOOLS_URL, port);

    println!();
    println!("  Turborepo Devtools");
    println!("  ──────────────────────────────────────");
    println!("  WebSocket: ws://localhost:{}", port);
    println!("  Browser:   {}", url);
    println!();
    println!("  Press Ctrl+C to stop");
    println!();

    // Open browser
    if !no_open {
        if let Err(e) = webbrowser::open(&url) {
            eprintln!("  Warning: Could not open browser: {}", e);
        }
    }

    // Run server
    server
        .run()
        .await
        .map_err(|e| cli::Error::Devtools(Box::new(e)))?;

    Ok(())
}
