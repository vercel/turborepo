use async_trait::async_trait;
use reqwest::Method;
use serde::Serialize;

use crate::{retry, AnonAPIClient, Error};

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
        let request_builder = self
            .create_telemetry_request_builder(
                "/api/turborepo/v1/events",
                Method::POST,
                session_id,
                telemetry_id,
            )
            .await?
            .json(&events);

        retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(())
    }
}
