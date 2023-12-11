use async_trait::async_trait;
use reqwest::Method;
use turborepo_api_client::retry::make_retryable_request;

use crate::{client::AnonAPIClient, errors::Error, events::TelemetryEvent};

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
        let request_builder = self
            .create_request_builder(
                "/api/turborepo/v1/events",
                Method::POST,
                session_id,
                telemetry_id,
            )
            .await?
            .json(&events);

        make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(())
    }
}
