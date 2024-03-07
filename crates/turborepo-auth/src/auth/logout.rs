use tracing::error;
use turbopath::AbsoluteSystemPath;
use turborepo_api_client::TokenClient;
use turborepo_ui::{cprintln, GREY};

use crate::{Error, LogoutOptions, Token};

pub async fn logout<T: TokenClient>(options: &LogoutOptions<'_, T>) -> Result<(), Error> {
    let LogoutOptions {
        ui,
        api_client,
        path,
        invalidate,
    } = *options;

    if invalidate {
        Token::from_file(path)?.invalidate(api_client).await?;
    }

    if let Err(err) = remove_token(path) {
        error!("could not logout. Something went wrong: {}", err);
        return Err(err);
    }

    cprintln!(ui, GREY, ">>> Logged out");
    Ok(())
}

fn remove_token(path: &AbsoluteSystemPath) -> Result<(), Error> {
    let content = path.read_to_string()?;

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
    path.create_with_contents(new_content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::backtrace::Backtrace;

    use async_trait::async_trait;
    use reqwest::{RequestBuilder, Response};
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_api_client::Client;
    use turborepo_vercel_api::{
        token::ResponseTokenMetadata, SpacesResponse, Team, TeamsResponse, UserResponse,
        VerifiedSsoUser,
    };
    use url::Url;

    use super::*;

    struct MockApiClient {
        pub succeed_delete_request: bool,
    }

    #[async_trait]
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

    #[async_trait]
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

    #[test]
    fn test_remove_token() {
        let tmp_dir = tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp_dir.path().join("config.json"))
            .expect("could not create path");
        let content = r#"{"token":"some-token"}"#;
        path.create_with_contents(content)
            .expect("could not create file");

        remove_token(&path).unwrap();

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
            ui: &turborepo_ui::UI::new(false),
            api_client: &api_client,
            path: &path,
            invalidate: true,
        };

        logout(&options).await.unwrap();

        let new_content = path.read_to_string().unwrap();
        assert_eq!(new_content, "{}");
    }
}
