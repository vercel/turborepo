/**
 * This whole file will hopefully go away in the future when we stop writing
 * tokens to `config.json`.
 */
use dirs_next::config_dir;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::Client;

use crate::{
    error::Error::{FailedToFindConfigDir, FailedToReadConfigFile},
    AuthFile, AuthToken, Error, Space, Team, TURBOREPO_AUTH_FILE_NAME, TURBOREPO_CONFIG_DIR,
    TURBOREPO_LEGACY_AUTH_FILE_NAME,
};

#[derive(serde::Deserialize)]
/// ConfigToken describes the legacy token format. It should only be used as a
/// way to store the underlying token as a Token trait, and then converted to an
/// AuthToken.
pub struct ConfigToken {
    pub token: String,
}

/// Attempts to read the config file for an auth token and returns the token
/// string.
pub fn read_config_auth() -> Result<ConfigToken, crate::Error> {
    let config_dir = config_dir().ok_or(FailedToFindConfigDir)?;
    let config_path = config_dir
        .join(TURBOREPO_CONFIG_DIR)
        .join(TURBOREPO_LEGACY_AUTH_FILE_NAME);

    let body = std::fs::read_to_string(config_path).map_err(FailedToReadConfigFile)?;
    let parsed_config: ConfigToken =
        serde_json::from_str(&body).map_err(|e| FailedToReadConfigFile(e.into()))?;

    Ok(parsed_config)
}

/// Converts our old style of token held in `config.json` into the new schema.
///
/// Uses the client to get information not readily available in the current
/// token.
pub async fn convert_to_auth_file(
    token: &str,
    client: &impl Client,
) -> Result<AuthFile, crate::Error> {
    // Get the path to the auth file.
    let absolute_auth_path = AbsoluteSystemPathBuf::try_from(
        config_dir()
            .ok_or(Error::FailedToFindConfigDir)?
            .join(TURBOREPO_CONFIG_DIR)
            .join(TURBOREPO_AUTH_FILE_NAME),
    )
    .map_err(Error::PathError)?;

    // Fill in auth file data.
    let user_response = client.get_user(token).await?;
    let teams_response = client.get_teams(token).await?;

    let mut teams = Vec::new();
    let mut af: AuthFile = AuthFile { tokens: Vec::new() };
    // NOTE(voz): This doesn't feel great. Ideally we should async fetch all the
    // teams and their spaces, but this should also only be invoked once in a while
    // (until config.json doesn't have tokens anymore) so the perf hit shouldn't be
    // a worry.
    for team in teams_response.teams {
        let team_id = &team.id;
        let spaces_response = client.get_spaces(token, Some(team_id)).await?;
        let spaces = spaces_response
            .spaces
            .into_iter()
            .map(|space_data| Space { id: space_data.id })
            .collect();
        teams.push(Team {
            id: team_id.to_string(),
            spaces,
        })
    }

    // Add the token to the list of tokens.
    af.add_token(AuthToken {
        token: token.to_string(),
        api: client.base_url().to_owned(),
        created_at: user_response.user.created_at,
        teams,
    });

    // Write this to disk, overwriting anything that may be in `auth.json`.
    af.write_to_disk(absolute_auth_path)?;

    Ok(af)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::mocks::*;

    #[tokio::test]
    async fn test_read_config_auth() {
        // Setup: Create a temporary directory with a fake config file
        let temp_dir = tempdir().unwrap();
        let config_file_path = temp_dir
            .path()
            .join(TURBOREPO_CONFIG_DIR)
            .join(TURBOREPO_LEGACY_AUTH_FILE_NAME);
        fs::create_dir_all(temp_dir.path().join(TURBOREPO_CONFIG_DIR)).unwrap();
        fs::write(config_file_path, r#"{ "token": "test-token" }"#).unwrap();

        // Test: Call the read_config_auth function and check the result
        let result = read_config_auth();
        assert!(result.is_ok());
        let config_token = result.unwrap();
        assert_eq!(config_token.token, "test-token");
    }

    #[tokio::test]
    async fn test_convert_to_auth_file() {
        // Setup: Create a mock client and a fake token
        let mock_client = MockApiClient::new();
        let token = "test-token";

        // Test: Call the convert_to_auth_file function and check the result
        let result = convert_to_auth_file(token, &mock_client).await;
        assert!(result.is_ok());
        let auth_file = result.unwrap();

        // Check that the AuthFile contains the correct data
        assert_eq!(auth_file.tokens[0].token, "test-token".to_string());
        assert_eq!(auth_file.tokens[0].api, "custom-domain".to_string());
    }
}
