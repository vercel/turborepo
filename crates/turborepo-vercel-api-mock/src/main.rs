use anyhow::{Context, Result};
use turborepo_vercel_api_mock::start_test_server;

#[tokio::main]
async fn main() -> Result<()> {
    // Use the port given as the first argument, or find an open one.
    let port = match std::env::args().nth(1) {
        Some(arg) => arg.parse().context("port argument must be a number")?,
        None => port_scanner::request_open_port().context("failed to find open port")?,
    };
    eprintln!("vercel api mock listening on port {port}");
    tokio::task::block_in_place(|| start_test_server(port, None)).await?;
    Ok(())
}
