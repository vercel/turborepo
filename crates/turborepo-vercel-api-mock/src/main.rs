use anyhow::Result;
use turborepo_vercel_api_mock::start_test_server;

#[tokio::main]
async fn main() -> Result<()> {
    let port = port_scanner::request_open_port().unwrap();
    tokio::task::block_in_place(|| start_test_server(port)).await?;
    Ok(())
}
