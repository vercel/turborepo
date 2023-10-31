use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::Client;

use crate::error::Error::FailedToReadAuthFile;

#[derive(Serialize, Deserialize, Debug)]
/// AuthFile contains a list of domains, each with a token and a list of teams
/// the token is valid for.
pub struct AuthFile {
    pub tokens: Vec<AuthToken>,
}

impl AuthFile {
    /// Writes the contents of the auth file to disk. Will override whatever is
    /// there with what's in the struct.
    pub fn write_to_disk(&self, path: AbsoluteSystemPathBuf) -> Result<(), crate::Error> {
        path.ensure_dir()
            .map_err(|e| crate::Error::FailedToWriteAuth {
                auth_path: path.clone(),
                error: e,
            })?;

        path.create_with_contents(
            serde_json::to_string_pretty(self)
                .map_err(|e| crate::Error::FailedToSerializeAuthFile { error: e })?,
        )
        .map_err(|e| crate::Error::FailedToWriteAuth {
            auth_path: path.clone(),
            error: e,
        })?;

        Ok(())
    }
    pub fn get_token(&self, api: &str) -> Option<AuthToken> {
        self.tokens.iter().find(|t| t.api == api).cloned()
    }
    pub fn add_token(&mut self, token: AuthToken) {
        self.tokens.push(token);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
/// Contains the token itself and a list of teams the token is valid for.
pub struct AuthToken {
    /// The token itself.
    pub token: String,
    /// The API URL the token was issued from.
    pub api: String,
    /// The date the token was created.
    pub created_at: Option<u64>,
    /// A list of teams the token is valid for.
    pub teams: Vec<Team>,
}
impl AuthToken {
    /// Searches the teams to see if any team ID matches the passed in team.
    pub fn contains_team(&self, team: &str) -> bool {
        self.teams.iter().any(|t| t.id == team)
    }
    /// Searches the teams to see if any team contains the space ID matching the
    /// passed in space.
    pub fn contains_space(&self, space: &str) -> bool {
        self.teams.iter().any(|t| t.contains_space(space))
    }
    /// Validates the token by checking the expiration date and the signature.
    pub async fn validate(&self, _client: impl Client) -> bool {
        unimplemented!("validate token")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Team {
    pub id: String,
    pub spaces: Vec<Space>,
}
impl Team {
    // Search the team to see if it contains the space.
    pub fn contains_space(&self, space: &str) -> bool {
        self.spaces.iter().any(|s| s.id == space)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Space {
    pub id: String,
}

/// Attempts to read the auth file and returns the parsed json as an AuthFile
/// struct.
pub fn read_auth_file(path: AbsoluteSystemPathBuf) -> Result<AuthFile, crate::Error> {
    let body = std::fs::read_to_string(path).map_err(FailedToReadAuthFile)?;
    let parsed_config: AuthFile =
        serde_json::from_str(&body.to_owned()).map_err(|e| FailedToReadAuthFile(e.into()))?;

    Ok(parsed_config)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::{TURBOREPO_AUTH_FILE_NAME, TURBOREPO_CONFIG_DIR};

    #[tokio::test]
    async fn test_write_to_disk_and_read_back() {
        // Use temp dirs to avoid polluting the user's config dir
        let temp_dir = tempdir().unwrap();
        let auth_file_path = temp_dir
            .path()
            .join(TURBOREPO_CONFIG_DIR)
            .join(TURBOREPO_AUTH_FILE_NAME);

        // unwrapping is fine because we know the path exists
        let absolute_auth_path = AbsoluteSystemPathBuf::try_from(auth_file_path).unwrap();

        // Make sure the temp dir exists before writing to it.
        fs::create_dir_all(temp_dir.path().join(TURBOREPO_CONFIG_DIR)).unwrap();

        // Add a token to auth file
        let mut auth_file = AuthFile { tokens: Vec::new() };
        auth_file.add_token(AuthToken {
            token: "test-token".to_string(),
            api: "test-api".to_string(),
            created_at: Some(1634851200),
            teams: Vec::new(),
        });

        auth_file.write_to_disk(absolute_auth_path.clone()).unwrap();

        // Read back from disk
        let read_back: AuthFile =
            serde_json::from_str(&fs::read_to_string(absolute_auth_path.clone()).unwrap()).unwrap();
        assert_eq!(read_back.tokens.len(), 1);
        assert_eq!(read_back.tokens[0].token, "test-token");
    }

    #[tokio::test]
    async fn test_get_token() {
        let mut auth_file = AuthFile { tokens: Vec::new() };
        auth_file.add_token(AuthToken {
            token: "test-token".to_string(),
            api: "test-api".to_string(),
            created_at: Some(1634851200),
            teams: Vec::new(),
        });

        let token = auth_file.get_token("test-api");
        assert!(token.is_some());
        assert_eq!(token.unwrap().token, "test-token");
    }

    #[tokio::test]
    async fn test_add_token() {
        let mut auth_file = AuthFile { tokens: Vec::new() };
        assert_eq!(auth_file.tokens.len(), 0);

        auth_file.add_token(AuthToken {
            token: "test-token".to_string(),
            api: "test-api".to_string(),
            created_at: Some(1634851200),
            teams: Vec::new(),
        });

        assert_eq!(auth_file.tokens.len(), 1);
    }

    #[tokio::test]
    async fn test_contains_team_and_space() {
        let team = Team {
            id: "team1".to_string(),
            spaces: vec![Space {
                id: "space1".to_string(),
            }],
        };
        let auth_token = AuthToken {
            token: "token1".to_string(),
            api: "api1".to_string(),
            created_at: None,
            teams: vec![team.clone()],
        };

        assert!(auth_token.contains_team("team1"));
        assert!(!auth_token.contains_team("team2"));
        assert!(team.contains_space("space1"));
        assert!(!team.contains_space("space2"));
    }

    #[tokio::test]
    async fn test_read_auth_file() {
        // Setup: Create a temporary directory with a fake auth file
        let temp_dir = tempdir().unwrap();
        let auth_file_path = temp_dir
            .path()
            .join(TURBOREPO_CONFIG_DIR)
            .join(TURBOREPO_AUTH_FILE_NAME);

        // Write a dummy auth file
        fs::create_dir_all(temp_dir.path().join(TURBOREPO_CONFIG_DIR)).unwrap();
        fs::write(
            &auth_file_path,
            r#"{ "tokens": [ { "token": "test-token", "api": "test-api", "created_at": 1634851200, "teams": [] } ] }"#,
        )
        .unwrap();

        let absolute_path = AbsoluteSystemPathBuf::try_from(auth_file_path).unwrap();

        // Test: Ensure the auth file has been read correctly
        let auth_file = read_auth_file(absolute_path).unwrap();
        assert_eq!(auth_file.tokens.len(), 1);
        assert_eq!(auth_file.tokens[0].token, "test-token");
    }
}
