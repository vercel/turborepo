use tracing::error;
use turborepo_api_client::Client;
use turborepo_ui::{cprintln, GREY};

use crate::{Error, LogoutOptions};

pub fn logout<T: Client>(options: &LogoutOptions<T>) -> Result<(), Error> {
    if let Err(err) = remove_token(options) {
        error!("could not logout. Something went wrong: {}", err);
        return Err(err);
    }

    cprintln!(options.ui, GREY, ">>> Logged out");
    Ok(())
}

fn remove_token<T: Client>(options: &LogoutOptions<T>) -> Result<(), Error> {
    // Read the existing content from the global configuration path
    let content = options.path.read_to_string()?;

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
    options.path.create_with_contents(new_content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use reqwest::{RequestBuilder, Response};
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_ui::UI;
    use turborepo_vercel_api::{
        SpacesResponse, Team, TeamsResponse, UserResponse, VerifiedSsoUser,
    };
    use url::Url;

    use super::*;

    struct MockApiClient {}

    impl MockApiClient {
        fn new() -> Self {
            Self {}
        }
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

    #[test]
    fn test_remove_token() {
        let tmp_dir = tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tmp_dir.path().join("config.json"))
            .expect("could not create path");
        let content = r#"{"token":"some-token"}"#;
        path.create_with_contents(content)
            .expect("could not create file");

        let options = LogoutOptions {
            ui: &UI::new(false),
            api_client: &MockApiClient::new(),
            path: &path,
        };

        remove_token(&options).unwrap();

        let new_content = path.read_to_string().unwrap();
        assert_eq!(new_content, "{}");
    }
}
