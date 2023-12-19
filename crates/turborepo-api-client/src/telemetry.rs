use async_trait::async_trait;
use reqwest::Method;
use serde::Serialize;

use crate::{retry, AnonAPIClient, Error};

const TELEMETRY_ENDPOINT: &str = "/api/turborepo/v1/events";

#[async_trait]
pub trait TelemetryClient {
    async fn record_telemetry<T>(
        &self,
        events: Vec<T>,
        telemetry_id: &str,
        session_id: &str,
    ) -> Result<(), Error>
    where
        T: Serialize + std::marker::Send;
}

#[async_trait]
impl TelemetryClient for AnonAPIClient {
    async fn record_telemetry<T>(
        &self,
        events: Vec<T>,
        telemetry_id: &str,
        session_id: &str,
    ) -> Result<(), Error>
    where
        T: Serialize + std::marker::Send,
    {
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
