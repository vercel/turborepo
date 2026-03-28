use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use crate::{APIClient, Error};

/// Shared reqwest client initialization for all run-time network consumers.
///
/// Uses two-phase initialization to avoid blocking on macOS Keychain
/// enumeration (~200ms). Phase 1 builds an instant client with bundled
/// Mozilla CAs (webpki-roots). Phase 2 builds a full client with system
/// CAs (native-roots) in the background. Consumers get whichever is
/// best available at the time of use.
#[derive(Clone)]
pub struct SharedHttpClient {
    /// Instant client with bundled Mozilla CAs only (~0ms to build).
    fast_client: Arc<OnceLock<reqwest::Client>>,
    /// Full client with system Keychain CAs (~200ms on macOS).
    /// Built in the background; preferred once ready.
    native_client: Arc<OnceLock<reqwest::Client>>,
    warming: Arc<AtomicBool>,
}

impl Default for SharedHttpClient {
    fn default() -> Self {
        Self {
            fast_client: Arc::new(OnceLock::new()),
            native_client: Arc::new(OnceLock::new()),
            warming: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl SharedHttpClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn activate(&self) {
        if self.native_client.get().is_some() {
            return;
        }

        if self
            .warming
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        // Phase 1: build fast client in background (webpki-roots only, ~0ms).
        let fast = self.fast_client.clone();
        tokio::task::spawn_blocking(move || {
            let _span = tracing::info_span!("http_client_init_fast").entered();
            let _ = fast.get_or_init(|| {
                APIClient::build_http_client_webpki_only(None)
                    .expect("failed to build webpki HTTP client")
            });
        });

        // Phase 2: build full client in background (native-roots, ~200ms on macOS).
        let native = self.native_client.clone();
        let fast_fallback = self.fast_client.clone();
        let warming = self.warming.clone();
        tokio::task::spawn_blocking(move || {
            let _span = tracing::info_span!("http_client_init").entered();
            match APIClient::build_http_client(None) {
                Ok(client) => {
                    let _ = native.set(client);
                }
                Err(e) => {
                    tracing::warn!("Native HTTP client init failed ({e}), using fast client");
                    // Ensure the fast (webpki-only) client is available as fallback.
                    if fast_fallback.get().is_none()
                        && let Ok(client) = APIClient::build_http_client_webpki_only(None)
                    {
                        let _ = fast_fallback.set(client);
                    }
                }
            }
            warming.store(false, Ordering::Release);
        });
    }

    pub async fn get_or_init(&self) -> Result<reqwest::Client, Error> {
        // Prefer the full client (includes system CAs for corporate proxies)
        if let Some(client) = self.native_client.get() {
            return Ok(client.clone());
        }

        // If the fast client is ready, use it while native is still building
        if let Some(client) = self.fast_client.get() {
            return Ok(client.clone());
        }

        // Neither is ready — build the fast client synchronously as fallback
        let fast = self.fast_client.clone();
        let client = tokio::task::spawn_blocking(move || {
            let _span = tracing::info_span!("http_client_init_fast").entered();
            fast.get_or_init(|| {
                APIClient::build_http_client_webpki_only(None)
                    .expect("failed to build webpki HTTP client")
            })
            .clone()
        })
        .await
        .map_err(|_| Error::HttpClientCancelled)?;

        Ok(client)
    }
}
