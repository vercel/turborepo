use std::env;

use anyhow::{anyhow, Result};
use axum::async_trait;
use reqwest::StatusCode;
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::Deserialize;

use crate::get_version;

#[async_trait]
pub trait UserClient {
    fn set_token(&mut self, token: String);
    async fn get_user(&self) -> Result<UserResponse>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: String,
    pub name: String,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserResponse {
    pub user: User,
}

pub struct APIClient {
    token: String,
    client: reqwest_middleware::ClientWithMiddleware,
    base_url: String,
}

#[async_trait]
impl UserClient for APIClient {
    fn set_token(&mut self, token: String) {
        self.token = token
    }

    async fn get_user(&self) -> Result<UserResponse> {
        let request_builder = self
            .client
            .get(self.make_url("/v2/user"))
            .header("User-Agent", user_agent())
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json");

        let response = request_builder.send().await;

        match response {
            Ok(response) => {
                let user_response = response.json::<UserResponse>().await?;
                Ok(user_response)
            }
            Err(reqwest_middleware::Error::Reqwest(err))
                if err.status() == Some(StatusCode::NOT_FOUND) =>
            {
                Err(anyhow!("404 - Not found"))
            }
            Err(err) => Err(err.into()),
        }
    }
}

impl APIClient {
    pub fn new(token: impl AsRef<str>, base_url: impl AsRef<str>) -> Self {
        let retry_policy = ExponentialBackoff {
            max_n_retries: 2,
            min_retry_interval: std::time::Duration::from_secs(2),
            max_retry_interval: std::time::Duration::from_secs(10),
            // The Go library we were using before, retryablehttp,
            // had the exponent set to 2.
            backoff_exponent: 2,
        };

        let client = ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        APIClient {
            token: token.as_ref().to_string(),
            client,
            base_url: base_url.as_ref().to_string(),
        }
    }

    fn make_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }
}

fn user_agent() -> String {
    format!(
        "turbo {} {} {} {}",
        get_version(),
        rustc_version_runtime::version(),
        env::consts::OS,
        env::consts::ARCH
    )
}
