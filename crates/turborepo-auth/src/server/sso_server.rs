use std::{net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use axum::{extract::Query, response::Redirect, routing::get, Router};
use reqwest::Url;
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::Error;

#[derive(Debug, Default, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SsoPayload {
    login_error: Option<String>,
    sso_email: Option<String>,
    team_name: Option<String>,
    sso_type: Option<String>,
    token: Option<String>,
    email: Option<String>,
}

#[async_trait]
pub trait SSOLoginServer {
    async fn run(&self, port: u16, verification_token: Arc<OnceCell<String>>) -> Result<(), Error>;
}

/// TODO: Document this.
pub struct DefaultSSOLoginServer;

#[async_trait]
impl SSOLoginServer for DefaultSSOLoginServer {
    async fn run(&self, port: u16, verification_token: Arc<OnceCell<String>>) -> Result<(), Error> {
        let handle = axum_server::Handle::new();
        let route_handle = handle.clone();
        let app = Router::new()
            // `GET /` goes to `root`
            .route(
                "/",
                get(|sso_payload: Query<SsoPayload>| async move {
                    let (token, location) = get_token_and_redirect(sso_payload.0).unwrap();
                    if let Some(token) = token {
                        // If token is already set, it's not a big deal, so we ignore the error.
                        let _ = verification_token.set(token);
                    }
                    route_handle.shutdown();
                    Redirect::to(location.as_str())
                }),
            );
        let addr = SocketAddr::from(([127, 0, 0, 1], port));

        axum_server::bind(addr)
            .handle(handle)
            .serve(app.into_make_service())
            .await
            .expect("failed to start one-shot server");

        Ok(())
    }
}

fn get_token_and_redirect(payload: SsoPayload) -> Result<(Option<String>, Url), Error> {
    let location_stub = "https://vercel.com/notifications/cli-login/turbo/";
    if let Some(login_error) = payload.login_error {
        let mut url = Url::parse(&format!("{}failed", location_stub))?;
        url.query_pairs_mut()
            .append_pair("loginError", login_error.as_str());
        return Ok((None, url));
    }

    if let Some(sso_email) = payload.sso_email {
        let mut url = Url::parse(&format!("{}incomplete", location_stub))?;
        url.query_pairs_mut()
            .append_pair("ssoEmail", sso_email.as_str());
        if let Some(team_name) = payload.team_name {
            url.query_pairs_mut()
                .append_pair("teamName", team_name.as_str());
        }
        if let Some(sso_type) = payload.sso_type {
            url.query_pairs_mut()
                .append_pair("ssoType", sso_type.as_str());
        }

        return Ok((None, url));
    }
    let mut url = Url::parse(&format!("{}success", location_stub))?;
    if let Some(email) = payload.email {
        url.query_pairs_mut().append_pair("email", email.as_str());
    }

    Ok((payload.token, url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_token_and_redirect() {
        assert_eq!(
            get_token_and_redirect(SsoPayload::default()).unwrap(),
            (
                None,
                Url::parse("https://vercel.com/notifications/cli-login/turbo/success").unwrap()
            )
        );

        assert_eq!(
            get_token_and_redirect(SsoPayload {
                login_error: Some("error".to_string()),
                ..SsoPayload::default()
            })
            .unwrap(),
            (
                None,
                Url::parse(
                    "https://vercel.com/notifications/cli-login/turbo/failed?loginError=error"
                )
                .unwrap()
            )
        );

        assert_eq!(
            get_token_and_redirect(SsoPayload {
                sso_email: Some("email".to_string()),
                ..SsoPayload::default()
            })
            .unwrap(),
            (
                None,
                Url::parse(
                    "https://vercel.com/notifications/cli-login/turbo/incomplete?ssoEmail=email"
                )
                .unwrap()
            )
        );

        assert_eq!(
            get_token_and_redirect(SsoPayload {
                sso_email: Some("email".to_string()),
                team_name: Some("team".to_string()),
                ..SsoPayload::default()
            }).unwrap(),
            (
                None,
                Url::parse("https://vercel.com/notifications/cli-login/turbo/incomplete?ssoEmail=email&teamName=team")
                    .unwrap()
            )
        );

        assert_eq!(
            get_token_and_redirect(SsoPayload {
                token: Some("token".to_string()),
                ..SsoPayload::default()
            })
            .unwrap(),
            (
                Some("token".to_string()),
                Url::parse("https://vercel.com/notifications/cli-login/turbo/success").unwrap()
            )
        );
    }
}
