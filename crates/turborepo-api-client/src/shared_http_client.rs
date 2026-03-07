use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::OnceCell;

use crate::{APIClient, Error};

/// Shared reqwest client initialization for all run-time network consumers.
///
/// Call `activate()` as soon as a command knows it will need networking, then
/// use `get_or_init()` at the actual point of use. This overlaps TLS/client
/// setup with other startup work without constructing a client for commands
/// that never touch the network.
#[derive(Clone, Default)]
pub struct SharedHttpClient {
    cell: Arc<OnceCell<reqwest::Client>>,
    warming: Arc<AtomicBool>,
}

impl SharedHttpClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn activate(&self) {
        if self.cell.get().is_some() {
            return;
        }

        if self
            .warming
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let this = self.clone();
        tokio::spawn(async move {
            let _ = this.get_or_init().await;
            this.warming.store(false, Ordering::Release);
        });
    }

    pub async fn get_or_init(&self) -> Result<reqwest::Client, Error> {
        let client = self
            .cell
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(|| {
                    let _span = tracing::info_span!("http_client_init").entered();
                    APIClient::build_http_client(None)
                })
                .await
                .map_err(|_| Error::HttpClientCancelled)?
            })
            .await?;

        Ok(client.clone())
    }
}
