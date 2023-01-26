use std::{env, future::Future};

use anyhow::{anyhow, Result};
use axum::async_trait;
use lazy_static::lazy_static;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::{get_version, retry::retry_future};

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
    client: reqwest::Client,
    base_url: String,
}

#[async_trait]
impl UserClient for APIClient {
    fn set_token(&mut self, token: String) {
        self.token = token
    }

    async fn get_user(&self) -> Result<UserResponse> {
        let response = self
            .make_retryable_request(|| {
                let request_builder = self
                    .client
                    .get(self.make_url("/v2/user"))
                    .header("User-Agent", USER_AGENT.clone())
                    .header("Authorization", format!("Bearer {}", self.token))
                    .header("Content-Type", "application/json");

                request_builder.send()
            })
            .await;

        match response {
            Ok(response) => {
                let user_response = response.json::<UserResponse>().await?;
                Ok(user_response)
            }
            Err(error) => {
                if let Some(error) = error.downcast_ref::<reqwest::Error>() {
                    if error.status() == Some(StatusCode::NOT_FOUND) {
                        return Err(anyhow!("404 - Not found"));
                    }
                }

                Err(error)
            }
        }
    }
}

const RETRY_MAX: u32 = 2;

impl APIClient {
    async fn make_retryable_request<
        F: Future<Output = Result<reqwest::Response, reqwest::Error>>,
    >(
        &self,
        request_builder: impl Fn() -> F,
    ) -> Result<reqwest::Response> {
        retry_future(RETRY_MAX, request_builder, Self::should_retry_request).await
    }

    fn should_retry_request(error: &reqwest::Error) -> bool {
        if let Some(status) = error.status() {
            if status == StatusCode::TOO_MANY_REQUESTS {
                return true;
            }

            if status.as_u16() >= 500 && status.as_u16() != 501 {
                return true;
            }
        }

        false
    }

    pub fn new(token: impl AsRef<str>, base_url: impl AsRef<str>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()?;

        Ok(APIClient {
            token: token.as_ref().to_string(),
            client,
            base_url: base_url.as_ref().to_string(),
        })
    }

    fn make_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }
}

lazy_static! {
    static ref USER_AGENT: String = format!(
        "turbo {} {} {} {}",
        get_version(),
        rustc_version_runtime::version(),
        env::consts::OS,
        env::consts::ARCH
    );
}
