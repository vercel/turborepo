use std::future::Future;

use reqwest::Method;
pub use turborepo_vercel_api::{AnalyticsEvent, CacheEvent, CacheSource};

use crate::{retry, APIAuth, APIClient, Error};

pub trait AnalyticsClient {
    fn record_analytics(
        &self,
        api_auth: &APIAuth,
        events: Vec<AnalyticsEvent>,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

impl AnalyticsClient for APIClient {
    #[tracing::instrument(skip_all)]
    async fn record_analytics(
        &self,
        api_auth: &APIAuth,
        events: Vec<AnalyticsEvent>,
    ) -> Result<(), Error> {
        let request_builder = self
            .create_request_builder("/v8/artifacts/events", api_auth, Method::POST)
            .await?
            .json(&events);

        retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
            .await?
            .into_response()
            .error_for_status()?;

        Ok(())
    }
}
