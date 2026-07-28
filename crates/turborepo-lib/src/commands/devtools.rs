//! `turbo devtools` command implementation.
//!
//! Starts a WebSocket server that serves package graph data
//! and watches for file changes to push updates.

use turbopath::AbsoluteSystemPathBuf;
use turborepo_devtools::{find_available_port, DevtoolsServer};

use crate::{cli, config::resolve_configuration_from_args, devtools::ProperTaskGraphBuilder, Args};

// In production, use the hosted devtools UI
// For local development, set TURBO_DEVTOOLS_LOCAL=1 to use localhost:3000
const DEVTOOLS_URL: &str = if cfg!(debug_assertions) {
    "http://localhost:3000/devtools"
} else {
    "https://turborepo.dev/devtools"
};
const DEVTOOLS_ORIGIN: &str = if cfg!(debug_assertions) {
    "http://localhost:3000"
} else {
    "https://turborepo.dev"
};

/// Run the devtools server.
pub async fn run(
    repo_root: AbsoluteSystemPathBuf,
    args: Args,
    port: u16,
    no_open: bool,
) -> Result<(), cli::Error> {
    // Find available port
    let port = find_available_port(port);

    // Create the task graph builder that uses EngineBuilder
    // This ensures the devtools shows the same task graph as `turbo run`
    // Only snapshot an explicit path. Default turbo.json/turbo.jsonc selection
    // remains refreshable through the normal filename watches.
    let config = resolve_configuration_from_args(&repo_root, &args)?;
    let watcher_paths = config.root_turbo_json_path.into_iter().collect();
    let task_graph_builder = ProperTaskGraphBuilder::new(repo_root.clone(), args);

    // Create server with the task graph builder
    let server = DevtoolsServer::new_with_paths(
        repo_root,
        port,
        task_graph_builder,
        DEVTOOLS_ORIGIN,
        watcher_paths,
    );

    let url = format!(
        "{}?port={}#token={}",
        DEVTOOLS_URL,
        port,
        server.auth_token()
    );

    println!();
    println!("  Turborepo Devtools");
    println!("  ──────────────────────────────────────");
    println!("  WebSocket: ws://localhost:{} (token required)", port);
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
