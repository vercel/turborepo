use async_trait::async_trait;
use reqwest::Method;
use turborepo_vercel_api::telemetry::TelemetryEvent;

use crate::{retry, AnonAPIClient, Error};

const TELEMETRY_ENDPOINT: &str = "/api/turborepo/v1/events";

#[async_trait]
pub trait TelemetryClient {
    async fn record_telemetry(
        &self,
        events: Vec<TelemetryEvent>,
        telemetry_id: &str,
        session_id: &str,
    ) -> Result<(), Error>;
}

#[async_trait]
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

        retry::make_retryable_request(telemetry_request)
            .await?
            .error_for_status()?;

        Ok(())
    }
}
