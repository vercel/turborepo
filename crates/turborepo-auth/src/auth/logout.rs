use tracing::error;
use turbopath::AbsoluteSystemPath;
use turborepo_api_client::TokenClient;
use turborepo_dirs::{config_dir, vercel_config_dir};
use turborepo_types::Credential;
use turborepo_ui::{GREY, cprintln};

use crate::{
    AuthTokens, Error, LogoutOptions, TURBO_AUTH_FILE, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, Token,
    VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE,
};

pub async fn logout<T: TokenClient>(options: &LogoutOptions<T>) -> Result<(), Error> {
    if let Err(err) = options.remove_tokens().await {
        error!("could not logout. Something went wrong: {}", err);
        return Err(err);
    }

    cprintln!(options.color_config, GREY, ">>> Logged out");
    Ok(())
}

impl<T: TokenClient> LogoutOptions<T> {
    fn token_at_path(path: &AbsoluteSystemPath) -> Result<Option<Credential>, Error> {
        match Token::from_file(path) {
            Ok(token) => Ok(Some(token.into_inner().clone())),
            Err(Error::TokenNotFound) => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn try_remove_token(
        &self,
        path: &AbsoluteSystemPath,
        invalidate: bool,
        saved_vercel: bool,
        revoker: &impl TokenClient,
    ) -> Result<(), Error> {
        // Read the existing content from the global configuration path
        if path.read_to_string().is_err() {
            return Ok(());
        }

        if invalidate {
            match Token::from_file(path) {
                Ok(token) => {
                    let token = if saved_vercel {
                        Token::existing_secret(Credential::saved_vercel(
                            token.into_inner().expose().into(),
                        ))
                    } else {
                        token
                    };
                    token.invalidate(revoker).await?
                }
                // If token doesn't exist, don't do anything.
                Err(Error::TokenNotFound | Error::InvalidTokenFileFormat { .. }) => {}
                Err(err) => return Err(err),
            }
        }

        match AuthTokens::clear_from_config_file(path) {
            Ok(()) => {}
            Err(Error::JsonRewrite(_)) => path.create_with_contents_secret("{}")?,
            Err(err) => return Err(err),
        }

        Ok(())
    }

    async fn remove_tokens(&self) -> Result<(), Error> {
        #[cfg(test)]
        if let Some(path) = &self.path {
            return self
                .try_remove_token(path, self.invalidate, false, &self.api_client)
                .await;
        }

        if !self.invalidate {
            return self.remove_tokens_with_vercel(&self.api_client).await;
        }
        let vercel_client = turborepo_api_client::APIClient::new(
            "https://vercel.com/api",
            Some(std::time::Duration::from_secs(30)),
            None,
            env!("CARGO_PKG_VERSION"),
            false,
        )?;
        self.remove_tokens_with_vercel(&vercel_client).await
    }

    async fn remove_tokens_with_vercel(
        &self,
        vercel_client: &impl TokenClient,
    ) -> Result<(), Error> {
        let turbo_auth_path =
            config_dir()?.map(|dir| dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]));
        let turbo_config_path =
            config_dir()?.map(|dir| dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]));
        let legacy_path = vercel_config_dir()?
            .map(|dir| dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]));
        let mut revoked = std::collections::HashMap::<String, bool>::new();
        let mut first_error = None;
        for (path, saved_vercel) in [
            (turbo_auth_path, true),
            (legacy_path, true),
            (turbo_config_path, false),
        ] {
            let Some(path) = path else { continue };
            let token = if self.invalidate {
                match Self::token_at_path(&path) {
                    Ok(token) => token,
                    Err(Error::InvalidTokenFileFormat { .. }) => None,
                    Err(err) => {
                        first_error.get_or_insert(err);
                        continue;
                    }
                }
            } else {
                None
            };
            let previous = token
                .as_ref()
                .and_then(|token| revoked.get(token.expose()))
                .copied();
            if previous == Some(false) {
                continue;
            }
            let invalidate = self.invalidate && previous.is_none();
            let result = if saved_vercel {
                self.try_remove_token(&path, invalidate, true, vercel_client)
                    .await
            } else {
                self.try_remove_token(&path, invalidate, false, &self.api_client)
                    .await
            };
            if let Some(token) = token {
                revoked.insert(token.expose().to_owned(), result.is_ok());
            }
            if let Err(err) = result {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
#[cfg(test)]
mod tests {
    use std::{backtrace::Backtrace, env, fs};

    use reqwest::{RequestBuilder, Response};
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_api_client::Client;
    use turborepo_ui::ColorConfig;
    use turborepo_vercel_api::{
        Team, TeamsResponse, UserResponse, VerifiedSsoUser, token::ResponseTokenMetadata,
    };
    use url::Url;

    use super::*;

    struct MockApiClient {
        pub succeed_delete_request: bool,
    }

    impl Client for MockApiClient {
        async fn get_user(
            &self,
            _token: &Credential,
        ) -> turborepo_api_client::Result<UserResponse> {
            unimplemented!("get_user")
        }
        async fn get_teams(
            &self,
            _token: &Credential,
        ) -> turborepo_api_client::Result<TeamsResponse> {
            unimplemented!("get_teams")
        }
        async fn get_team(
            &self,
            _token: &Credential,
            _team_id: &str,
        ) -> turborepo_api_client::Result<Option<Team>> {
            unimplemented!("get_team")
        }
        fn add_ci_header(_request_builder: RequestBuilder) -> RequestBuilder {
            unimplemented!("add_ci_header")
        }
        async fn verify_sso_token(
            &self,
            token: &Credential,
            _: &str,
        ) -> turborepo_api_client::Result<VerifiedSsoUser> {
            Ok(VerifiedSsoUser {
                token: token.expose().into(),
                team_id: Some("team_id".to_string()),
            })
        }
        async fn handle_403(_response: Response) -> turborepo_api_client::Error {
            unimplemented!("handle_403")
        }
        fn make_url(&self, _endpoint: &str) -> turborepo_api_client::Result<Url> {
            unimplemented!("make_url")
        }
    }

    impl TokenClient for MockApiClient {
        async fn delete_token(&self, _token: &Credential) -> turborepo_api_client::Result<()> {
            if self.succeed_delete_request {
                Ok(())
            } else {
                Err(turborepo_api_client::Error::UnknownStatus {
                    code: "code".to_string(),
                    message: "this failed".to_string(),
                    backtrace: Backtrace::capture(),
                })
            }
        }
        async fn get_metadata(
            &self,
            _token: &Credential,
        ) -> turborepo_api_client::Result<ResponseTokenMetadata> {
            unimplemented!("get_metadata")
        }
    }

    struct VercelRevoker {
        calls: std::sync::Mutex<Vec<Credential>>,
        fail: bool,
    }

    impl TokenClient for VercelRevoker {
        async fn delete_token(&self, token: &Credential) -> turborepo_api_client::Result<()> {
            let url = Url::parse("https://vercel.com/v3/user/tokens/current").unwrap();
            turborepo_api_client::validate_credential_destination(token, &url)?;
            assert!(token.is_saved_vercel());
            self.calls.lock().unwrap().push(token.clone());
            if self.fail {
                Err(turborepo_api_client::Error::SavedVercelCredentialDestination)
            } else {
                Ok(())
            }
        }

        async fn get_metadata(
            &self,
            _: &Credential,
        ) -> turborepo_api_client::Result<ResponseTokenMetadata> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn mixed_store_logout_routes_revocations_and_continues_after_failure() {
        for (fail, duplicate_generic) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let turbo_dir = tempdir().unwrap();
            let vercel_dir = tempdir().unwrap();
            let auth_path =
                AbsoluteSystemPathBuf::try_from(turbo_dir.path().join("turborepo/auth.json"))
                    .unwrap();
            let config_path =
                AbsoluteSystemPathBuf::try_from(turbo_dir.path().join("turborepo/config.json"))
                    .unwrap();
            let legacy_path =
                AbsoluteSystemPathBuf::try_from(vercel_dir.path().join("com.vercel.cli/auth.json"))
                    .unwrap();
            fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
            fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
            auth_path
                .create_with_contents(r#"{"token":"synthetic-saved"}"#)
                .unwrap();
            config_path
                .create_with_contents(if duplicate_generic {
                    r#"{"token":"synthetic-saved","other":true}"#
                } else {
                    r#"{"token":"synthetic-generic","other":true}"#
                })
                .unwrap();
            legacy_path
                .create_with_contents(r#"{"token":"synthetic-saved"}"#)
                .unwrap();
            unsafe {
                env::set_var("TURBO_CONFIG_DIR_PATH", turbo_dir.path());
                env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_dir.path());
            }
            let server = httpmock::MockServer::start_async().await;
            let generic = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::DELETE)
                        .path("/v3/user/tokens/current")
                        .header("authorization", "Bearer synthetic-generic");
                    then.status(200);
                })
                .await;
            let vercel = VercelRevoker {
                calls: Default::default(),
                fail,
            };
            let options = LogoutOptions {
                color_config: ColorConfig::new(false),
                api_client: turborepo_api_client::APIClient::new(
                    server.base_url(),
                    None,
                    None,
                    "test",
                    false,
                )
                .unwrap(),
                invalidate: true,
                path: None,
            };
            let result = options.remove_tokens_with_vercel(&vercel).await;
            unsafe {
                env::remove_var("TURBO_CONFIG_DIR_PATH");
                env::remove_var("VERCEL_CONFIG_DIR_PATH");
            }
            assert_eq!(
                result.is_err(),
                fail,
                "real revocation failures must propagate"
            );
            generic
                .assert_calls_async(usize::from(!duplicate_generic))
                .await;
            let config: serde_json::Value =
                serde_json::from_str(&config_path.read_to_string().unwrap()).unwrap();
            assert_eq!(config.get("other"), Some(&serde_json::json!(true)));
            assert_eq!(config.get("token").is_some(), duplicate_generic && fail);
            assert_eq!(
                vercel.calls.lock().unwrap().len(),
                1,
                "duplicate saved tokens must be revoked once"
            );
            for path in [&auth_path, &legacy_path] {
                assert_eq!(
                    path.read_to_string().unwrap().contains("synthetic-saved"),
                    fail,
                    "failed revocations remain retryable; successful revocations clear all \
                     duplicate stores"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_remove_token() {
        let tmp_dir = tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp_dir.path().join("config.json"))
            .expect("could not create path");
        let content = r#"{"token":"some-token"}"#;
        path.create_with_contents(content)
            .expect("could not create file");

        let logout_options = LogoutOptions {
            color_config: ColorConfig::new(false),
            api_client: MockApiClient {
                succeed_delete_request: true,
            },
            invalidate: false,
            path: Some(path.clone()),
        };

        logout_options.remove_tokens().await.unwrap();

        let new_content = path.read_to_string().unwrap();
        assert_eq!(new_content, "{}");
    }

    #[tokio::test]
    async fn test_invalidate_token() {
        let tmp_dir = tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp_dir.path().join("config.json"))
            .expect("could not create path");
        let content = r#"{"token":"some-token"}"#;
        path.create_with_contents(content)
            .expect("could not create file");

        let api_client = MockApiClient {
            succeed_delete_request: true,
        };

        let options = LogoutOptions {
            color_config: ColorConfig::new(false),
            api_client,
            path: Some(path.clone()),
            invalidate: true,
        };

        logout(&options).await.unwrap();

        let new_content = path.read_to_string().unwrap();
        assert_eq!(new_content, "{}");
    }

    #[tokio::test]
    async fn failed_revocation_preserves_local_token() {
        let tmp_dir = tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp_dir.path().join("config.json")).unwrap();
        let content = r#"{"token":"vca_synthetic-sentinel"}"#;
        path.create_with_contents(content).unwrap();
        let options = LogoutOptions {
            color_config: ColorConfig::new(false),
            api_client: MockApiClient {
                succeed_delete_request: false,
            },
            path: Some(path.clone()),
            invalidate: true,
        };

        assert!(logout(&options).await.is_err());
        assert_eq!(path.read_to_string().unwrap(), content);
    }

    #[tokio::test]
    async fn test_remove_token_with_malformed_file() {
        let tmp_dir = tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp_dir.path().join("config.json"))
            .expect("could not create path");
        path.create_with_contents("{not-json")
            .expect("could not create malformed file");

        let logout_options = LogoutOptions {
            color_config: ColorConfig::new(false),
            api_client: MockApiClient {
                succeed_delete_request: true,
            },
            invalidate: true,
            path: Some(path.clone()),
        };

        logout_options.remove_tokens().await.unwrap();

        let new_content = path.read_to_string().unwrap();
        assert_eq!(new_content, "{}");
    }

    #[tokio::test]
    async fn test_remove_tokens_clears_legacy_and_turbo_auth_files() {
        let turbo_dir = tempdir().expect("Failed to create turbo dir");
        let vercel_dir = tempdir().expect("Failed to create vercel dir");
        let turbo_path =
            AbsoluteSystemPathBuf::try_from(turbo_dir.path().join("turborepo/config.json"))
                .expect("could not create turbo path");
        let legacy_path =
            AbsoluteSystemPathBuf::try_from(vercel_dir.path().join("com.vercel.cli/auth.json"))
                .expect("could not create legacy path");

        fs::create_dir_all(turbo_dir.path().join("turborepo"))
            .expect("Failed to create turbo auth dir");
        fs::create_dir_all(vercel_dir.path().join("com.vercel.cli"))
            .expect("Failed to create legacy auth dir");
        turbo_path
            .create_with_contents(r#"{"token":"turbo-token"}"#)
            .expect("could not create turbo auth file");
        legacy_path
            .create_with_contents(r#"{"token":"legacy-token"}"#)
            .expect("could not create legacy auth file");

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_dir.path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_dir.path());
        }

        let logout_options = LogoutOptions {
            color_config: ColorConfig::new(false),
            api_client: MockApiClient {
                succeed_delete_request: true,
            },
            invalidate: false,
            path: None,
        };

        logout_options.remove_tokens().await.unwrap();

        assert_eq!(turbo_path.read_to_string().unwrap(), "{}");
        assert_eq!(legacy_path.read_to_string().unwrap(), "{}");

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }
}
