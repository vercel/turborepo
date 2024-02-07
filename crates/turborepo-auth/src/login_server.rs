use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use axum::{extract::Query, response::Redirect, routing::get, Router};
use serde::Deserialize;
use tokio::sync::OnceCell;
use url::Url;

use crate::Error;

pub enum LoginType {
    Basic { login_url_configuration: String },
    SSO,
}

#[derive(Debug, Clone, Deserialize)]
struct LoginPayload {
    token: String,
}

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
pub trait LoginServer {
    async fn run(
        &self,
        port: u16,
        login_type: LoginType,
        token: Arc<OnceCell<String>>,
    ) -> Result<(), Error>;
}

/// A struct that implements LoginServer.
///
/// Listens on 127.0.0.1 and a port that's passed in.
pub struct DefaultLoginServer;

#[async_trait]
impl LoginServer for DefaultLoginServer {
    async fn run(
        &self,
        port: u16,
        login_type: LoginType,
        login_token: Arc<OnceCell<String>>,
    ) -> Result<(), Error> {
        let handle = axum_server::Handle::new();
        let route_handle = handle.clone();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        match login_type {
            LoginType::Basic {
                login_url_configuration,
            } => {
                let app = Router::new().route(
                    "/",
                    get(|login_payload: Query<LoginPayload>| async move {
                        let _ = login_token.set(login_payload.0.token);
                        route_handle.shutdown();
                        Redirect::to(&format!("{login_url_configuration}/turborepo/success"))
                    }),
                );

                axum_server::bind(addr)
                    .handle(handle)
                    .serve(app.into_make_service())
                    .await
                    .expect("failed to start one-shot server");
            }
            LoginType::SSO => {
                let app = Router::new().route(
                    "/",
                    get(|sso_payload: Query<SsoPayload>| async move {
                        let (token, location) = get_token_and_redirect(sso_payload.0).unwrap();
                        if let Some(token) = token {
                            // If token is already set, it's not a big deal, so we ignore the error.
                            let _ = login_token.set(token);
                        }
                        route_handle.shutdown();
                        Redirect::to(location.as_str())
                    }),
                );

                axum_server::bind(addr)
                    .handle(handle)
                    .serve(app.into_make_service())
                    .await
                    .expect("failed to start one-shot server");
            }
        }

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
