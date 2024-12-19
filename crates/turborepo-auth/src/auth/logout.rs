use tracing::error;
use turbopath::AbsoluteSystemPath;
use turborepo_api_client::TokenClient;
use turborepo_dirs::{config_dir, vercel_config_dir};
use turborepo_ui::{cprintln, GREY};

use crate::{
    Error, LogoutOptions, Token, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, VERCEL_TOKEN_DIR,
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
    async fn try_remove_token(&self, path: &AbsoluteSystemPath) -> Result<(), Error> {
        // Read the existing content from the global configuration path
        let Ok(content) = path.read_to_string() else {
            return Ok(());
        };

        if self.invalidate {
            match Token::from_file(path) {
                Ok(token) => token.invalidate(&self.api_client).await?,
                // If token doesn't exist, don't do anything.
                Err(Error::TokenNotFound) => {}
                Err(err) => return Err(err),
            }
        }

        // Attempt to deserialize the content into a serde_json::Value
        let mut data: serde_json::Value = serde_json::from_str(&content)?;

        // Check if the data is an object and remove the "token" field if present
        if let Some(obj) = data.as_object_mut() {
            if obj.remove("token").is_none() {
                return Ok(());
            }
        } else {
            return Ok(());
        }

        // Serialize the updated data back to a string
        let new_content = serde_json::to_string_pretty(&data)?;

        // Write the updated content back to the file
        path.create_with_contents(new_content)?;

        Ok(())
    }

    async fn remove_tokens(&self) -> Result<(), Error> {
        #[cfg(test)]
        if let Some(path) = &self.path {
            return self.try_remove_token(path).await;
        }

        if let Some(vercel_config_dir) = vercel_config_dir()? {
            self.try_remove_token(
                &vercel_config_dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]),
            )
            .await?;
        }
        if let Some(turbo_config_dir) = config_dir()? {
            self.try_remove_token(
                &turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]),
            )
            .await?;
        }

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use std::backtrace::Backtrace;

    use reqwest::{RequestBuilder, Response};
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_api_client::Client;
    use turborepo_ui::ColorConfig;
    use turborepo_vercel_api::{
        token::ResponseTokenMetadata, SpacesResponse, Team, TeamsResponse, UserResponse,
        VerifiedSsoUser,
    };
    use url::Url;

    use super::*;

    struct MockApiClient {
        pub succeed_delete_request: bool,
    }

    impl Client for MockApiClient {
        async fn get_user(&self, _token: &str) -> turborepo_api_client::Result<UserResponse> {
            unimplemented!("get_user")
        }
        async fn get_teams(&self, _token: &str) -> turborepo_api_client::Result<TeamsResponse> {
            unimplemented!("get_teams")
        }
        async fn get_team(
            &self,
            _token: &str,
            _team_id: &str,
        ) -> turborepo_api_client::Result<Option<Team>> {
            unimplemented!("get_team")
        }
        fn add_ci_header(_request_builder: RequestBuilder) -> RequestBuilder {
            unimplemented!("add_ci_header")
        }
        async fn get_spaces(
            &self,
            _token: &str,
            _team_id: Option<&str>,
        ) -> turborepo_api_client::Result<SpacesResponse> {
            unimplemented!("get_spaces")
        }
        async fn verify_sso_token(
            &self,
            token: &str,
            _: &str,
        ) -> turborepo_api_client::Result<VerifiedSsoUser> {
            Ok(VerifiedSsoUser {
                token: token.to_string(),
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
        async fn delete_token(&self, _token: &str) -> turborepo_api_client::Result<()> {
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
            _token: &str,
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
}
