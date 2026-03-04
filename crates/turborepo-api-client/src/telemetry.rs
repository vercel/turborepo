use std::{future::Future, sync::Arc};

use reqwest::Method;
use tokio::sync::OnceCell;
use turborepo_vercel_api::telemetry::TelemetryEvent;

use crate::{APIClient, AnonAPIClient, Error, build_user_agent, retry};

const TELEMETRY_ENDPOINT: &str = "/api/turborepo/v1/events";

pub trait TelemetryClient {
    fn record_telemetry(
        &self,
        events: Vec<TelemetryEvent>,
        telemetry_id: &str,
        session_id: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

impl TelemetryClient for AnonAPIClient {
    async fn record_telemetry(
        &self,
        events: Vec<TelemetryEvent>,
        telemetry_id: &str,
        session_id: &str,
    ) -> Result<(), Error> {
        let url = self.make_url(TELEMETRY_ENDPOINT);
        let telemetry_request = self
            .client
            .request(Method::POST, url)
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("x-turbo-telemetry-id", telemetry_id)
            .header("x-turbo-session-id", session_id)
            .json(&events);

        retry::make_retryable_request(telemetry_request, retry::RetryStrategy::Timeout)
            .await?
            .into_response()
            .error_for_status()?;

        Ok(())
    }
}

/// A telemetry client backed by an HTTP client that initializes on a
/// background thread. TLS initialization (~100ms) starts as early as
/// possible via `spawn_blocking`; this client shares the `OnceCell` that
/// the background task writes to. By the time telemetry flushes its
/// first batch, TLS init has almost certainly already completed.
#[derive(Clone)]
pub struct DeferredTelemetryClient {
    http_client: Arc<OnceCell<reqwest::Client>>,
    base_url: String,
    user_agent: String,
}

impl DeferredTelemetryClient {
    pub fn new(
        http_client: Arc<OnceCell<reqwest::Client>>,
        base_url: impl Into<String>,
        version: &str,
    ) -> Self {
        Self {
            http_client,
            base_url: base_url.into(),
            user_agent: build_user_agent(version),
        }
    }
}

impl TelemetryClient for DeferredTelemetryClient {
    async fn record_telemetry(
        &self,
        events: Vec<TelemetryEvent>,
        telemetry_id: &str,
        session_id: &str,
    ) -> Result<(), Error> {
        // Fast path: background TLS init already completed.
        // Slow path: initialize inline, but if the runtime is shutting down
        // the spawn_blocking task will be cancelled — return an error instead
        // of panicking. Telemetry is never worth crashing over.
        let maybe_client;
        let client = match self.http_client.get() {
            Some(client) => client,
            None => {
                maybe_client = tokio::task::spawn_blocking(|| APIClient::build_http_client(None))
                    .await
                    .map_err(|_| Error::HttpClientCancelled)??;
                &maybe_client
            }
        };

        let url = format!("{}{}", self.base_url, TELEMETRY_ENDPOINT);
        let telemetry_request = client
            .request(Method::POST, url)
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("x-turbo-telemetry-id", telemetry_id)
            .header("x-turbo-session-id", session_id)
            .json(&events);

        retry::make_retryable_request(telemetry_request, retry::RetryStrategy::Timeout)
            .await?
            .into_response()
            .error_for_status()?;

        Ok(())
    }
}
