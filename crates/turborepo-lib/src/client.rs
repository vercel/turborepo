use std::env;

use anyhow::Result;
use axum::async_trait;
use serde::Deserialize;

use crate::get_version;

#[async_trait]
trait UserClient {
    fn set_token(&mut self, token: String);
    async fn get_user(&self) -> Result<UserResponse>;
    // fn verify_sso_token(&self, token: String, token_name: String) ->
    // Result<VerifiedSSOUser>; fn set_team_id(&self, team_id: String);
    // fn get_caching_status(&self) -> Result<CachingStatus>;
    // fn get_team(&self, team_id: String) -> Result<Team>;
}

#[derive(Debug, Clone, Deserialize)]
struct User {
    id: String,
    username: String,
    email: String,
    name: String,
    #[serde(rename = "createdAt")]
    created_at: u32,
}

struct Team {}

#[derive(Debug, Clone, Deserialize)]
struct UserResponse {
    user: User,
}

struct APIClient {
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
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;

        let user: UserResponse = response.json().await?;
        Ok(user)
    }

    // fn verify_sso_token(&self, token: String, token_name: String) ->
    // Result<VerifiedSSOUser> {     todo!()
    // }
    //
    // fn set_team_id(&self, team_id: String) {
    //     todo!()
    // }
    //
    // fn get_caching_status(&self) -> Result<CachingStatus> {
    //     todo!()
    // }
    //
    // fn get_team(&self, team_id: String) -> Result<Team> {
    //     todo!()
    // }
}

impl APIClient {
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

#[test]
fn test_user_agent() {
    println!("{}", user_agent());
}
