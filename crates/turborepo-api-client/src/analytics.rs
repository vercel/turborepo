use async_trait::async_trait;
use reqwest::Method;
pub use turborepo_vercel_api::{AnalyticsEvent, CacheEvent, CacheSource};

use crate::{retry, APIAuth, APIClient, Error};

#[async_trait]
pub trait AnalyticsClient {
    async fn record_analytics(
        &self,
        api_auth: &APIAuth,
        events: Vec<AnalyticsEvent>,
    ) -> Result<(), Error>;
}

#[async_trait]
impl AnalyticsClient for APIClient {
    async fn record_analytics(
        &self,
        api_auth: &APIAuth,
        events: Vec<AnalyticsEvent>,
    ) -> Result<(), Error> {
        let request_builder = self
            .create_request_builder("/v8/artifacts/events", api_auth, Method::POST)
            .await?
            .json(&events);

        retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(())
    }
}
