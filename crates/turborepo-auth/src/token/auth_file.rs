use dirs_next::config_dir;
use serde::{Deserialize, Serialize};
use turborepo_api_client::Client;

use crate::{
    error::Error::{FailedToFindConfigDir, FailedToReadAuthFile},
    TURBOREPO_CONFIG_DIR,
};

const TURBOREPO_AUTH_FILE_NAME: &str = "auth.json";

#[derive(Serialize, Deserialize)]
/// AuthFile contains a list of domains, each with a token and a list of teams
/// the token is valid for.
pub struct AuthFile {
    pub tokens: Vec<AuthToken>,
}

impl AuthFile {
    /// Writes the contents of the auth file to disk. Will override whatever is
    /// there with what's in the struct.
    pub fn write_to_disk(&self) -> Result<(), crate::Error> {
        let config_dir = config_dir().ok_or(FailedToFindConfigDir)?;
        let global_auth_path = config_dir
            .join(TURBOREPO_CONFIG_DIR)
            .join(TURBOREPO_AUTH_FILE_NAME);

        let path = turbopath::AbsoluteSystemPathBuf::try_from(global_auth_path)
            .map_err(crate::Error::PathError)?;

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

#[derive(Serialize, Deserialize, Clone)]
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
    pub async fn validate(&self, client: impl Client) -> bool {
        !unimplemented!("validate token")
    }
}

#[derive(Serialize, Deserialize, Clone)]
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

#[derive(Serialize, Deserialize, Clone)]
pub struct Space {
    pub id: String,
}

/// Attempts to read the auth file and returns the parsed json as an AuthFile
/// struct.
pub fn read_auth_file() -> Result<AuthFile, crate::Error> {
    let config_dir = config_dir().ok_or(FailedToFindConfigDir)?;
    let config_path = config_dir
        .join(TURBOREPO_CONFIG_DIR)
        .join(TURBOREPO_AUTH_FILE_NAME);

    let body = std::fs::read_to_string(config_path).map_err(FailedToReadAuthFile)?;
    let parsed_config: AuthFile =
        serde_json::from_str(&body.to_owned()).map_err(|e| FailedToReadAuthFile(e.into()))?;

    Ok(parsed_config)
}
