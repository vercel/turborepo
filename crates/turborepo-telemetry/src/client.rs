use std::env;

use async_trait::async_trait;
use reqwest::{Method, RequestBuilder};

use crate::errors::{Error, Result};

#[async_trait]
pub trait Client {
    fn make_url(&self, endpoint: &str) -> String;
}

#[derive(Clone)]
pub struct AnonAPIClient {
    client: reqwest::Client,
    base_url: String,
    user_agent: String,
}

impl Client for AnonAPIClient {
    fn make_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }
}

impl AnonAPIClient {
    pub fn new(base_url: impl AsRef<str>, timeout: u64, version: &str) -> Result<Self> {
        let client_build = if timeout != 0 {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout))
                .build()
        } else {
            reqwest::Client::builder().build()
        };

        let client = client_build.map_err(Error::TlsError)?;

        let user_agent = format!(
            "turbo {} {} {} {}",
            version,
            rustc_version_runtime::version(),
            env::consts::OS,
            env::consts::ARCH
        );
        Ok(AnonAPIClient {
            client,
            base_url: base_url.as_ref().to_string(),
            user_agent,
        })
    }

    pub(crate) async fn create_request_builder(
        &self,
        url: &str,
        method: Method,
        session_id: &str,
        telemetry_id: &str,
    ) -> Result<RequestBuilder> {
        let url = self.make_url(url);

        let request_builder = self
            .client
            .request(method, url)
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("x-turbo-telemetry-id", telemetry_id)
            .header("x-turbo-session-id", session_id);

        Ok(request_builder)
    }
}
