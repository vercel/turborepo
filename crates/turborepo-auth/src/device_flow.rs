//! OAuth 2.0 Device Authorization Grant (RFC 8628) implementation.
//!
//! This module handles the "device flow" login for Vercel:
//! 1. [`discover`] — fetch OIDC metadata from the issuer
//! 2. [`device_authorization_request`] — request a device code
//! 3. [`poll_for_token`] — poll until the user completes authentication
//!
//! Additionally, [`introspect_token`] (RFC 7662) is used by the SSO flow
//! to extract session metadata from an existing token.

use std::{sync::LazyLock, time::Duration};

use reqwest::header;
use serde::Deserialize;
use url::Url;

use crate::Error;

const DEFAULT_VERCEL_ISSUER: &str = "https://vercel.com";
pub const VERCEL_CLI_CLIENT_ID: &str = "cl_HYyOPBNtFMfHhaUn9L4QPfTZz6TP47bp";
// Only request `offline_access` (refresh token). We intentionally do not
// request `openid` because we don't use or validate the id_token — user
// identity is verified independently via the `get_user` API call.
const DEVICE_FLOW_SCOPE: &str = "offline_access";
// Per-request timeout for token polling. If the server doesn't respond
// within this window, we back off (doubling the interval).
const DEVICE_TOKEN_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// OIDC Authorization Server metadata (RFC 8414).
/// Fetched from the issuer's `.well-known/openid-configuration` endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub device_authorization_endpoint: String,
    pub token_endpoint: String,
    pub revocation_endpoint: String,
    pub introspection_endpoint: String,
}

/// Response from the device authorization endpoint (RFC 8628 §3.2).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    /// The short code the user must confirm at the verification URI.
    /// Per RFC 8628 §3.3 the user code MUST be displayed so users can verify
    /// it matches what the authorization server shows — this is an
    /// anti-phishing measure.
    pub user_code: String,
    pub verification_uri: String,
    /// Per RFC 8628 §3.2 this is OPTIONAL, but Vercel always provides it.
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    /// Polling interval in seconds. Defaults to 5 per the RFC if absent.
    #[serde(default = "default_poll_interval")]
    pub interval: u64,
}

fn default_poll_interval() -> u64 {
    5
}

/// Token set returned on successful device code exchange (RFC 8628 §3.5).
/// Contains the access token, optional refresh token, and expiration.
///
/// Fields are plain `String`s because `SecretString` does not implement
/// `Deserialize`. Use `into_inner` methods to extract and wrap in
/// `SecretString` at call sites. The custom `Debug` impl redacts secrets.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TokenSet {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"***")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "***"))
            .field("scope", &self.scope)
            .finish()
    }
}

/// Token introspection response (RFC 7662 §2.2).
/// Used to extract `session_id` and `client_id` for SSO flows.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IntrospectionResponse {
    pub active: bool,
    pub client_id: Option<String>,
    pub session_id: Option<String>,
}

/// OAuth 2.0 error response body (RFC 6749 §5.2).
/// Used to parse error codes during device code polling.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct OAuthErrorResponse {
    error: String,
    error_description: Option<String>,
}

static USER_AGENT: LazyLock<String> = LazyLock::new(|| {
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{host} @ turbo")
});

fn truncate(s: &str, max: usize) -> &str {
    match s.floor_char_boundary(max) {
        boundary if boundary < s.len() => &s[..boundary],
        _ => s,
    }
}

/// Derive the OIDC issuer URL from a login URL.
/// If the login URL is a Vercel-hosted URL (contains "vercel.com"),
/// use the default Vercel issuer. Otherwise fall back to `https://` + host
/// of the login URL to support self-hosted/enterprise deployments.
/// Non-HTTPS schemes are rejected to prevent token exchange over plaintext.
fn issuer_from_login_url(login_url: &str) -> Result<String, Error> {
    if login_url.contains("vercel.com") {
        return Ok(DEFAULT_VERCEL_ISSUER.to_string());
    }
    if let Ok(parsed) = Url::parse(login_url) {
        if parsed.scheme() != "https" {
            return Err(Error::DiscoveryFailed {
                message: format!("login URL must use https://, got {}://", parsed.scheme()),
            });
        }
        Ok(format!(
            "https://{}",
            parsed.host_str().unwrap_or("vercel.com")
        ))
    } else {
        Ok(DEFAULT_VERCEL_ISSUER.to_string())
    }
}

/// Validate that all endpoint URLs in the metadata are under the issuer's
/// domain (same host or a subdomain) with the same scheme. This prevents
/// a compromised discovery document from redirecting token requests to
/// an attacker-controlled server.
///
/// For example, issuer `https://vercel.com` allows endpoints on
/// `https://api.vercel.com` but not `https://evil.com`.
fn validate_endpoint_origins(metadata: &AuthorizationServerMetadata) -> Result<(), Error> {
    let issuer_url = Url::parse(&metadata.issuer).map_err(|_| Error::DiscoveryFailed {
        message: format!("invalid issuer URL: {}", metadata.issuer),
    })?;
    let issuer_host = issuer_url.host_str().unwrap_or("");
    let issuer_scheme = issuer_url.scheme();

    for (name, endpoint) in [
        (
            "device_authorization_endpoint",
            &metadata.device_authorization_endpoint,
        ),
        ("token_endpoint", &metadata.token_endpoint),
        ("revocation_endpoint", &metadata.revocation_endpoint),
        ("introspection_endpoint", &metadata.introspection_endpoint),
    ] {
        let endpoint_url = Url::parse(endpoint).map_err(|_| Error::DiscoveryFailed {
            message: format!("invalid {name} URL: {endpoint}"),
        })?;
        let ep_host = endpoint_url.host_str().unwrap_or("");
        let host_ok = ep_host == issuer_host || ep_host.ends_with(&format!(".{issuer_host}"));

        if endpoint_url.scheme() != issuer_scheme || !host_ok {
            return Err(Error::DiscoveryFailed {
                message: format!(
                    "{name} origin mismatch: expected {issuer_scheme}://*{issuer_host}, got \
                     {endpoint}"
                ),
            });
        }
    }
    Ok(())
}

/// Fetch OIDC discovery metadata from the issuer.
///
/// The issuer is derived from `login_url`: Vercel-hosted URLs use
/// `https://vercel.com`, other URLs use their own scheme + host.
pub async fn discover(
    client: &reqwest::Client,
    login_url: &str,
) -> Result<AuthorizationServerMetadata, Error> {
    let issuer = issuer_from_login_url(login_url)?;
    let url = format!("{issuer}/.well-known/openid-configuration");
    let response = client
        .get(&url)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, USER_AGENT.as_str())
        .send()
        .await
        .map_err(|e| Error::DiscoveryFailed {
            message: e.to_string(),
        })?;

    if !response.status().is_success() {
        return Err(Error::DiscoveryFailed {
            message: format!("HTTP {}", response.status()),
        });
    }

    let metadata: AuthorizationServerMetadata =
        response.json().await.map_err(|e| Error::DiscoveryFailed {
            message: e.to_string(),
        })?;

    if metadata.issuer != issuer {
        return Err(Error::DiscoveryFailed {
            message: format!(
                "issuer mismatch: expected {issuer}, got {}",
                metadata.issuer
            ),
        });
    }

    validate_endpoint_origins(&metadata)?;

    Ok(metadata)
}

/// Request device authorization (RFC 8628 §3.1).
pub async fn device_authorization_request(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
) -> Result<DeviceAuthorizationResponse, Error> {
    let response = client
        .post(&metadata.device_authorization_endpoint)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::USER_AGENT, USER_AGENT.as_str())
        .form(&[
            ("client_id", VERCEL_CLI_CLIENT_ID),
            ("scope", DEVICE_FLOW_SCOPE),
        ])
        .send()
        .await
        .map_err(|e| Error::DeviceAuthorizationFailed {
            message: e.to_string(),
        })?;

    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::DeviceAuthorizationFailed {
            message: format!("server returned: {}", truncate(&text, 512)),
        });
    }

    response
        .json()
        .await
        .map_err(|e| Error::DeviceAuthorizationFailed {
            message: e.to_string(),
        })
}

/// Poll for token completion (RFC 8628 §3.4 + §3.5).
///
/// This blocks until the user completes authentication, the device code
/// expires, or the user denies access. The polling interval is increased
/// on `slow_down` errors (+5s per RFC) and doubled on connection timeouts,
/// capped at [`MAX_POLL_INTERVAL`].
pub async fn poll_for_token(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    device_code: &str,
    interval_secs: u64,
    expires_at: u64,
) -> Result<TokenSet, Error> {
    let mut interval = Duration::from_secs(interval_secs);

    loop {
        if crate::current_unix_time_secs() >= expires_at {
            return Err(Error::DeviceCodeExpired);
        }

        tokio::time::sleep(interval).await;

        let result = client
            .post(&metadata.token_endpoint)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::USER_AGENT, USER_AGENT.as_str())
            .timeout(DEVICE_TOKEN_REQUEST_TIMEOUT)
            .form(&[
                ("client_id", VERCEL_CLI_CLIENT_ID),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
            ])
            .send()
            .await;

        let response = match result {
            Ok(r) => r,
            Err(e) if e.is_timeout() => {
                interval = (interval * 2).min(MAX_POLL_INTERVAL);
                continue;
            }
            Err(e) => {
                return Err(Error::DeviceAuthorizationFailed {
                    message: e.to_string(),
                });
            }
        };

        if response.status().is_success() {
            return response.json::<TokenSet>().await.map_err(|e| {
                Error::DeviceAuthorizationFailed {
                    message: format!("failed to parse token response: {e}"),
                }
            });
        }

        let error_body = response.text().await.unwrap_or_default();
        let oauth_error: Result<OAuthErrorResponse, _> = serde_json::from_str(&error_body);

        match oauth_error {
            Ok(err) => match err.error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    // RFC 8628 §3.5: increase polling interval by 5 seconds
                    interval = (interval + Duration::from_secs(5)).min(MAX_POLL_INTERVAL);
                    continue;
                }
                "access_denied" => return Err(Error::AuthorizationDenied),
                "expired_token" => return Err(Error::DeviceCodeExpired),
                _ => {
                    return Err(Error::OAuthError {
                        code: err.error,
                        description: err.error_description,
                    });
                }
            },
            Err(_) => {
                return Err(Error::DeviceAuthorizationFailed {
                    message: format!("unexpected response: {}", truncate(&error_body, 512)),
                });
            }
        }
    }
}

/// Introspect a token to get session info (RFC 7662).
/// Used by the SSO flow to get `session_id` and `client_id`.
///
/// Includes `client_id` in the request body for RFC 7662 §2.1 compliance.
/// Vercel's endpoint accepts this for public clients.
pub async fn introspect_token(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    token: &str,
) -> Result<IntrospectionResponse, Error> {
    let response = client
        .post(&metadata.introspection_endpoint)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::USER_AGENT, USER_AGENT.as_str())
        .form(&[("token", token), ("client_id", VERCEL_CLI_CLIENT_ID)])
        .send()
        .await
        .map_err(|e| Error::IntrospectionFailed {
            message: e.to_string(),
        })?;

    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::IntrospectionFailed {
            message: format!("HTTP error: {}", truncate(&text, 512)),
        });
    }

    response
        .json()
        .await
        .map_err(|e| Error::IntrospectionFailed {
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_agent_format() {
        assert!(USER_AGENT.contains("@ turbo"));
    }

    #[test]
    fn test_current_unix_time_secs_reasonable() {
        let now = crate::current_unix_time_secs();
        assert!(now > 1_700_000_000);
    }

    #[test]
    fn token_set_debug_redacts_secrets() {
        let ts = TokenSet {
            access_token: "super-secret-access-token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            refresh_token: Some("super-secret-refresh-token".to_string()),
            scope: Some("offline_access".to_string()),
        };
        let debug = format!("{:?}", ts);
        assert!(
            !debug.contains("super-secret-access-token"),
            "Debug output must not contain the raw access token"
        );
        assert!(
            !debug.contains("super-secret-refresh-token"),
            "Debug output must not contain the raw refresh token"
        );
        assert!(debug.contains("***"));
        assert!(debug.contains("Bearer"));
        assert!(debug.contains("3600"));
    }

    #[test]
    fn test_issuer_from_login_url_vercel() {
        assert_eq!(
            issuer_from_login_url("https://vercel.com/api").unwrap(),
            "https://vercel.com"
        );
        assert_eq!(
            issuer_from_login_url("https://api.vercel.com").unwrap(),
            "https://vercel.com"
        );
    }

    #[test]
    fn test_issuer_from_login_url_self_hosted() {
        assert_eq!(
            issuer_from_login_url("https://my-company.example.com/api").unwrap(),
            "https://my-company.example.com"
        );
    }

    #[test]
    fn test_issuer_from_login_url_rejects_http() {
        let result = issuer_from_login_url("http://my-company.example.com/api");
        assert!(result.is_err());
    }

    #[test]
    fn test_issuer_from_login_url_fallback() {
        assert_eq!(
            issuer_from_login_url("not-a-url").unwrap(),
            "https://vercel.com"
        );
    }

    #[test]
    fn test_default_poll_interval() {
        assert_eq!(default_poll_interval(), 5);
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello");
    }

    #[test]
    fn test_deserialize_token_set() {
        let json = r#"{
            "access_token": "tok_abc",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "ref_xyz",
            "scope": "offline_access"
        }"#;
        let ts: TokenSet = serde_json::from_str(json).unwrap();
        assert_eq!(ts.access_token, "tok_abc");
        assert_eq!(ts.token_type, "Bearer");
        assert_eq!(ts.expires_in, 3600);
        assert_eq!(ts.refresh_token.as_deref(), Some("ref_xyz"));
        assert_eq!(ts.scope.as_deref(), Some("offline_access"));
    }

    #[test]
    fn test_deserialize_token_set_minimal() {
        let json = r#"{
            "access_token": "tok",
            "token_type": "Bearer",
            "expires_in": 60
        }"#;
        let ts: TokenSet = serde_json::from_str(json).unwrap();
        assert_eq!(ts.access_token, "tok");
        assert!(ts.refresh_token.is_none());
        assert!(ts.scope.is_none());
    }

    #[test]
    fn test_deserialize_device_auth_response_defaults() {
        let json = r#"{
            "device_code": "dc_abc",
            "user_code": "ABCD-1234",
            "verification_uri": "https://vercel.com/device",
            "expires_in": 900
        }"#;
        let resp: DeviceAuthorizationResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.device_code, "dc_abc");
        assert!(resp.verification_uri_complete.is_none());
        assert_eq!(resp.interval, 5); // default
    }

    #[test]
    fn test_deserialize_introspection_response() {
        let json = r#"{"active": true, "client_id": "cl_abc", "session_id": "sess_123"}"#;
        let resp: IntrospectionResponse = serde_json::from_str(json).unwrap();
        assert!(resp.active);
        assert_eq!(resp.client_id.as_deref(), Some("cl_abc"));
        assert_eq!(resp.session_id.as_deref(), Some("sess_123"));
    }

    #[test]
    fn test_deserialize_introspection_response_minimal() {
        let json = r#"{"active": false}"#;
        let resp: IntrospectionResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.active);
        assert!(resp.client_id.is_none());
    }

    #[test]
    fn test_validate_endpoint_origins_same_host() {
        let metadata = AuthorizationServerMetadata {
            issuer: "https://vercel.com".to_string(),
            device_authorization_endpoint: "https://vercel.com/api/device".to_string(),
            token_endpoint: "https://vercel.com/api/token".to_string(),
            revocation_endpoint: "https://vercel.com/api/revoke".to_string(),
            introspection_endpoint: "https://vercel.com/api/introspect".to_string(),
        };
        assert!(validate_endpoint_origins(&metadata).is_ok());
    }

    #[test]
    fn test_validate_endpoint_origins_subdomain() {
        // Vercel's real setup: issuer is vercel.com, endpoints on api.vercel.com
        let metadata = AuthorizationServerMetadata {
            issuer: "https://vercel.com".to_string(),
            device_authorization_endpoint:
                "https://api.vercel.com/login/oauth/device-authorization".to_string(),
            token_endpoint: "https://api.vercel.com/login/oauth/token".to_string(),
            revocation_endpoint: "https://api.vercel.com/login/oauth/token/revoke".to_string(),
            introspection_endpoint: "https://api.vercel.com/login/oauth/token/introspect"
                .to_string(),
        };
        assert!(validate_endpoint_origins(&metadata).is_ok());
    }

    #[test]
    fn test_validate_endpoint_origins_mismatch() {
        let metadata = AuthorizationServerMetadata {
            issuer: "https://vercel.com".to_string(),
            device_authorization_endpoint: "https://evil.com/api/device".to_string(),
            token_endpoint: "https://vercel.com/api/token".to_string(),
            revocation_endpoint: "https://vercel.com/api/revoke".to_string(),
            introspection_endpoint: "https://vercel.com/api/introspect".to_string(),
        };
        assert!(validate_endpoint_origins(&metadata).is_err());
    }

    #[test]
    fn test_validate_endpoint_origins_rejects_suffix_match() {
        // Verify that the subdomain check uses a proper domain boundary (the
        // leading `.`), so `el.com` and `notvercel.com` don't pass for issuer
        // `vercel.com`.
        let make = |ep_host: &str| AuthorizationServerMetadata {
            issuer: "https://vercel.com".to_string(),
            device_authorization_endpoint: format!("https://{ep_host}/api/device"),
            token_endpoint: "https://vercel.com/api/token".to_string(),
            revocation_endpoint: "https://vercel.com/api/revoke".to_string(),
            introspection_endpoint: "https://vercel.com/api/introspect".to_string(),
        };
        assert!(validate_endpoint_origins(&make("el.com")).is_err());
        assert!(validate_endpoint_origins(&make("notvercel.com")).is_err());
        assert!(validate_endpoint_origins(&make("evil-vercel.com")).is_err());
        // But a proper subdomain should pass
        assert!(validate_endpoint_origins(&make("api.vercel.com")).is_ok());
    }

    #[test]
    fn test_validate_endpoint_origins_scheme_mismatch() {
        let metadata = AuthorizationServerMetadata {
            issuer: "https://vercel.com".to_string(),
            device_authorization_endpoint: "http://api.vercel.com/api/device".to_string(),
            token_endpoint: "https://vercel.com/api/token".to_string(),
            revocation_endpoint: "https://vercel.com/api/revoke".to_string(),
            introspection_endpoint: "https://vercel.com/api/introspect".to_string(),
        };
        assert!(validate_endpoint_origins(&metadata).is_err());
    }
}
