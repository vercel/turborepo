/**
 * This whole file will hopefully go away in the future when we stop writing
 * tokens to `config.json`.
 */
use dirs_next::config_dir;
use turborepo_api_client::{Client, Error as APIError};

use crate::{
    error::Error::{FailedToFindConfigDir, FailedToReadConfigFile},
    token::file::{Space, Team},
    Token, TURBOREPO_CONFIG_DIR,
};

const TURBOREPO_LEGACY_AUTH_FILE_NAME: &str = "config.json";

#[derive(serde::Deserialize)]
pub struct ConfigToken {
    token: String,
}
impl ConfigToken {
    /// This method converts our old style of token held in `config.json` into
    /// the new schema held in `auth.json`
    pub async fn to_auth_token<C: Client>(&self, client: &C) -> Result<Token, APIError> {
        let user_response = client.get_user(self.token.as_str()).await?;
        let teams_response = client.get_teams(self.token.as_str()).await?;

        let mut teams = Vec::new();
        // TODO(voz): This doesn't feel great. Ideally we should async fetch all the
        // teams and their spaces, but this should also only be invoked for a
        // little while (until config.json doesn't have tokens anymore) so the perf hit
        // shouldn't be a worry.
        for team in teams_response.teams {
            let team_id = &team.id;
            let spaces_response = client
                .get_spaces(self.token.as_str(), Some(team_id))
                .await?;
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

        Ok(Token {
            token: self.token.to_string(),
            api: client.base_url().to_owned(),
            created_at: user_response.user.created_at,
            teams,
        })
    }
}

/// Attempts to read the config file for an auth token and returns the token
/// string.
pub fn read_config_auth() -> Result<String, crate::Error> {
    let config_dir = config_dir().ok_or(FailedToFindConfigDir)?;
    let config_path = config_dir
        .join(TURBOREPO_CONFIG_DIR)
        .join(TURBOREPO_LEGACY_AUTH_FILE_NAME);

    let body = std::fs::read_to_string(config_path).map_err(FailedToReadConfigFile)?;
    let parsed_config: ConfigToken =
        serde_json::from_str(&body).map_err(|e| FailedToReadConfigFile(e.into()))?;

    Ok(parsed_config.token.to_owned())
}
