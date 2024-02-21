#![feature(cow_is_borrowed)]
#![feature(assert_matches)]
#![deny(clippy::all)]
//! Turborepo's library for authenticating with the Vercel API.
//! Handles logging into Vercel, verifying SSO, and storing the token.

mod auth;
mod error;
mod login_server;
mod ui;

pub use auth::*;
pub use error::Error;
pub use login_server::*;
use turborepo_api_client::TokenClient;
use turborepo_vercel_api::token::ResponseTokenMetadata;

/// Token is the result of a successful login. It contains the token string and
/// potentially metadata about the token.
#[derive(Debug, Clone)]
pub enum Token {
    /// An existing token on the filesystem
    Existing(String),
    /// A token that was just created, but not yet written to the filesystem
    New(String),
}
impl Token {
    pub fn new(token: String) -> Self {
        Self::New(token)
    }
    pub fn existing(token: String) -> Self {
        Self::Existing(token)
    }
    /// Checks if the token is valid. We do a few checks:
    /// 1. Fetch the token metadata.
    /// 2. From the metadata, check if the token is active.
    /// 3. If the token is a SAML SSO token, check if it's expired.
    pub async fn is_valid(&self, client: &impl TokenClient) -> Result<bool, Error> {
        let metadata = self.fetch_metadata(client).await?;
        let current_time = current_unix_time();
        let active = is_token_active(&metadata, current_time);
        Ok(active)
    }

    async fn fetch_metadata(
        &self,
        client: &impl TokenClient,
    ) -> Result<ResponseTokenMetadata, Error> {
        client
            .get_metadata(self.into_inner())
            .await
            .map_err(Error::from)
    }
    /// Returns the underlying token string.
    pub fn into_inner(&self) -> &str {
        match self {
            Self::Existing(token) | Self::New(token) => token.as_str(),
        }
    }
}

fn current_unix_time() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

// As of the time of writing, this should always be true, since a token that
// isn't active returns an error when fetching metadata for the token.
fn is_token_active(metadata: &ResponseTokenMetadata, current_time: u128) -> bool {
    let active_at = metadata.active_at;

    let earliest_expiration = metadata
        .scopes
        .iter()
        .filter_map(|scope| scope.expires_at)
        .min();

    // Not all scopes have an expiration date, so we need to check if all of them
    // are expired. If there isn't an expiration date, we assume they are infinite
    // and therefore cannot be expired.
    let all_scopes_active =
        earliest_expiration.map_or(true, |expiration| current_time < expiration);

    all_scopes_active && (active_at <= current_time)
}

#[cfg(test)]
mod tests {
    use turborepo_vercel_api::token::Scope;

    use super::*;

    #[test]
    fn test_is_token_active() {
        let current_time = current_unix_time();
        let quick_scope = |expiry| Scope {
            expires_at: expiry,
            scope_type: "".to_string(),
            origin: "".to_string(),
            created_at: 0,
            team_id: None,
        };
        let mock_response = |active_at, scopes| ResponseTokenMetadata {
            active_at,
            scopes,
            // These fields don't matter in the test
            id: "".to_string(),
            name: "".to_string(),
            token_type: "".to_string(),
            origin: "".to_string(),
            created_at: 0,
        };

        let cases = vec![
            // Case: Token active, no scopes (implicitly infinite)
            (current_time - 100, vec![], true),
            // Case: Token active, one scope without expiration
            (current_time - 100, vec![quick_scope(None)], true),
            // Case: Token active, one scope expired
            (
                current_time - 100,
                vec![quick_scope(Some(current_time - 1))],
                false,
            ),
            // Case: Token active, one scope not expired
            (
                current_time - 100,
                vec![quick_scope(Some(current_time + 11))],
                true,
            ),
            // Case: Token active, all scopes not expired
            (
                current_time - 100,
                vec![
                    quick_scope(Some(current_time + 11)),
                    quick_scope(Some(current_time + 10)),
                ],
                true,
            ),
            // Case: Token inactive (future `active_at`)
            (
                current_time + 1000,
                vec![quick_scope(Some(current_time + 20))],
                false,
            ),
        ];

        for (active_at, scopes, expected) in cases {
            let metadata = mock_response(active_at, scopes);
            assert_eq!(
                is_token_active(&metadata, current_time),
                expected,
                "Test failed for active_at: {}",
                active_at
            );
        }
    }
}
