use std::env;

use anyhow::{anyhow, Result};
use axum::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::get_version;

#[async_trait]
pub trait UserClient {
    fn set_token(&mut self, token: String);
    async fn get_user(&self) -> Result<UserResponse>;
    // fn verify_sso_token(&self, token: String, token_name: String) ->
    // Result<VerifiedSSOUser>; fn set_team_id(&self, team_id: String);
    // fn get_caching_status(&self) -> Result<CachingStatus>;
    // fn get_team(&self, team_id: String) -> Result<Team>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    id: String,
    username: String,
    email: String,
    name: String,
    #[serde(rename = "createdAt")]
    created_at: u64,
}

struct Team {}

#[derive(Debug, Clone, Deserialize)]
pub struct UserResponse {
    user: User,
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
        let request_builder = self.client.get(self.make_url("/v2/user"));
        let response = request_builder
            .header("User-Agent", user_agent())
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(response) => {
                let user_response = response.json::<UserResponse>().await?;
                Ok(user_response)
            }
            Err(err) => {
                if matches!(err.status(), Some(StatusCode::NOT_FOUND)) {
                    Err(anyhow!("404 - Not found"))
                } else {
                    Err(err.into())
                }
            }
        }
    }
}

impl APIClient {
    pub fn new(token: impl AsRef<str>, base_url: impl AsRef<str>) -> Self {
        let client = reqwest::Client::new();
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
