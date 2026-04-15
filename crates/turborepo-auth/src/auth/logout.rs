use tracing::error;
use turbopath::AbsoluteSystemPath;
use turborepo_api_client::TokenClient;
use turborepo_dirs::{config_dir, vercel_config_dir};
use turborepo_ui::{GREY, cprintln};

use crate::{
    AuthTokens, Error, LogoutOptions, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, Token, VERCEL_TOKEN_DIR,
    VERCEL_TOKEN_FILE,
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
    fn token_at_path(
        path: &AbsoluteSystemPath,
    ) -> Result<Option<turborepo_api_client::SecretString>, Error> {
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
    ) -> Result<(), Error> {
        // Read the existing content from the global configuration path
        if path.read_to_string().is_err() {
            return Ok(());
        }

        if invalidate {
            match Token::from_file(path) {
                Ok(token) => token.invalidate(&self.api_client).await?,
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
            return self.try_remove_token(path, self.invalidate).await;
        }

        let turbo_path =
            config_dir()?.map(|dir| dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]));
        let legacy_path = vercel_config_dir()?
            .map(|dir| dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]));
        let skip_legacy_invalidate = if self.invalidate {
            match (
                turbo_path
                    .as_ref()
                    .map(|path| Self::token_at_path(path))
                    .transpose()?,
                legacy_path
                    .as_ref()
                    .map(|path| Self::token_at_path(path))
                    .transpose()?,
            ) {
                (Some(Some(turbo_token)), Some(Some(legacy_token))) => {
                    turbo_token.expose() == legacy_token.expose()
                }
                _ => false,
            }
        } else {
            false
        };

        if let Some(turbo_path) = turbo_path.as_ref() {
            self.try_remove_token(turbo_path, self.invalidate).await?;
        }
        if let Some(legacy_path) = legacy_path.as_ref() {
            self.try_remove_token(legacy_path, self.invalidate && !skip_legacy_invalidate)
                .await?;
        }

        Ok(())
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
            _token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<UserResponse> {
            unimplemented!("get_user")
        }
        async fn get_teams(
            &self,
            _token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<TeamsResponse> {
            unimplemented!("get_teams")
        }
        async fn get_team(
            &self,
            _token: &turborepo_api_client::SecretString,
            _team_id: &str,
        ) -> turborepo_api_client::Result<Option<Team>> {
            unimplemented!("get_team")
        }
        fn add_ci_header(_request_builder: RequestBuilder) -> RequestBuilder {
            unimplemented!("add_ci_header")
        }
        async fn verify_sso_token(
            &self,
            token: &turborepo_api_client::SecretString,
            _: &str,
        ) -> turborepo_api_client::Result<VerifiedSsoUser> {
            Ok(VerifiedSsoUser {
                token: token.clone(),
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
        async fn delete_token(
            &self,
            _token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<()> {
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
            _token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<ResponseTokenMetadata> {
            unimplemented!("get_metadata")
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
