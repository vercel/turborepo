use std::future::Future;

use reqwest::Method;
pub use turborepo_vercel_api::{AnalyticsEvent, CacheEvent, CacheSource};

use crate::{APIAuth, APIClient, Error, retry};

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

#[cfg(test)]
mod test {
    use std::time::Duration;

    use turborepo_types::Credential;

    use super::*;
    use crate::build_user_agent;

    #[tokio::test]
    async fn test_record_analytics_sends_user_agent() -> anyhow::Result<()> {
        let server = httpmock::MockServer::start_async().await;
        let events = server
            .mock_async(|w, t| {
                w.method(httpmock::Method::POST)
                    .path("/v8/artifacts/events")
                    .header("user-agent", build_user_agent("2.0.0"));
                t.status(200);
            })
            .await;
        let client = APIClient::new(
            server.base_url(),
            Some(Duration::from_secs(10)),
            None,
            "2.0.0",
            false,
        )?;
        let api_auth = APIAuth {
            team_id: Some("team_id".into()),
            token: Credential::generic("token".into()),
            team_slug: None,
        };

        client.record_analytics(&api_auth, vec![]).await?;

        events.assert_calls_async(1).await;
        Ok(())
    }
}
