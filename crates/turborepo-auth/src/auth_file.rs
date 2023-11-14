use std::fmt::Display;

use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::Client;
use turborepo_vercel_api::{Membership, Space, User};

use crate::Error;

#[derive(Serialize, Deserialize, Debug, Default)]
/// AuthFile contains a list of domains, each with a token and a list of teams
/// the token is valid for.
pub struct AuthFile {
    pub tokens: Vec<AuthToken>,
}

impl AuthFile {
    /// Writes the contents of the auth file to disk. Will override whatever is
    /// there with what's in the struct.
    pub fn write_to_disk(&self, path: &AbsoluteSystemPathBuf) -> Result<(), Error> {
        path.ensure_dir().map_err(|e| Error::PathError(e.into()))?;

        let mut pretty_content = serde_json::to_string_pretty(self)
            .map_err(|e| Error::FailedToSerializeAuthFile { source: e })?;
        // to_string_pretty doesn't add terminating line endings, so do that.
        pretty_content.push('\n');

        path.create_with_contents(pretty_content)
            .map_err(|e| crate::Error::FailedToWriteAuth {
                auth_path: path.clone(),
                error: e,
            })?;

        Ok(())
    }
    pub fn get_token(&self, api: &str) -> Option<AuthToken> {
        self.tokens.iter().find(|t| t.api == api).cloned()
    }
    /// Adds a token to the auth file. Attempts to match exclusively on `api`.
    /// If the api matches a token already in the file, it will be updated with
    /// the new token.
    ///
    /// TODO(voz): This should probably be able to match on more than just the
    /// `api` field, like an `id`.
    pub fn add_or_update_token(&mut self, token: AuthToken) {
        if let Some(existing_token) = self.tokens.iter_mut().find(|t| t.api == token.api) {
            // Update existing token.
            *existing_token = token;
        } else {
            self.tokens.push(token);
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
/// Contains the token itself and a list of teams the token is valid for.
pub struct AuthToken {
    /// The token itself.
    pub token: String,
    /// The API URL the token was issued from / for.
    pub api: String,
    /// The date the token was created.
    pub created_at: Option<u64>,
    /// The user the token was issued for.
    pub user: User,
    /// A list of teams the token is valid for.
    pub teams: Vec<Team>,
}

/// Team is re-implemented here because we need to add the `spaces` field to it,
/// and it's not currently returned by the teams endpoint.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Team {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub created_at: u64,
    pub created: chrono::DateTime<chrono::Utc>,
    pub membership: Membership,
    pub spaces: Vec<Space>,
}
impl From<turborepo_vercel_api::Team> for Team {
    fn from(team: turborepo_vercel_api::Team) -> Self {
        Self {
            id: team.id,
            slug: team.slug,
            name: team.name,
            created_at: team.created_at,
            created: team.created,
            membership: team.membership,
            spaces: vec![],
        }
    }
}

impl Team {
    pub fn is_owner(&self) -> bool {
        matches!(self.membership.role, turborepo_vercel_api::Role::Owner)
    }
    /// Search the team to see if it contains the space.
    pub fn contains_space(&self, space: &str) -> bool {
        self.spaces.iter().any(|s| s.id == space)
    }
}

impl AuthToken {
    /// Searches the teams to see if any team ID matches the passed in team.
    pub fn contains_team(&self, team: &str) -> bool {
        self.teams.iter().any(|t| t.slug == team)
    }
    /// Searches the teams to see if any team contains the space ID matching the
    /// passed in space.
    pub fn contains_space(&self, space: &str) -> bool {
        self.teams.iter().any(|t| t.contains_space(space))
    }
    /// Validates the token by checking the expiration date and the signature.
    pub async fn validate(&self, _client: impl Client) -> bool {
        todo!("validate token")
    }
    pub fn friendly_token_display(&self) -> String {
        format!(
            "{}...{}",
            &self.token[..3],
            &self.token[self.token.len() - 3..]
        )
    }
    pub fn friendly_api_display(&self) -> String {
        if self.api.contains("vercel.com") {
            return "▲ Vercel Remote Cache".to_owned();
        }
        self.api.clone()
    }
}
impl Display for AuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let created_at = self
            .created_at
            .map(|t| {
                // This should be safe to unwrap here since we're not setting the time for
                // 262,000 years in the future and we're not putting in more than 2s worth of
                // nanoseconds.
                let timestamp = chrono::NaiveDateTime::from_timestamp_opt(t as i64, 0).unwrap();
                let dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    timestamp,
                    chrono::Utc,
                );
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            })
            .unwrap_or_else(|| "unknown".to_string());
        write!(
            f,
            "Token: {}\nCreated at: {}\nUser: {} ({})\nTeams: {:?}",
            self.token, created_at, self.user.username, self.user.email, self.teams
        )
    }
}

async fn convert_sso_to_auth_token(
    token: &str,
    client: &impl Client,
    team_id: &str,
) -> Result<AuthToken, Error> {
    let user_response = client.get_user(token).await.map_err(Error::APIError)?;
    let team = client
        .get_team(token, team_id)
        .await
        .map_err(Error::APIError)?;

    let auth_token = AuthToken {
        token: token.to_string(),
        api: client.base_url().to_owned(),
        created_at: user_response.user.created_at,
        user: user_response.user,
        teams: match team {
            Some(t) => vec![t.into()],
            None => Vec::new(),
        },
    };
    Ok(auth_token)
}

/// Converts our old style of token held in `config.json` into the new schema.
///
/// Uses the client to get information not readily available in the current
/// token. Will write the new token to disk immediately and return the AuthFile
/// for use.
pub async fn convert_to_auth_token(
    token: &str,
    client: &impl Client,
    team_id: Option<&str>,
) -> Result<AuthToken, Error> {
    // Converting the SSO token is a bit different than the normal token. It uses
    // `get_team` and skips the loop.
    if let Some(team) = team_id {
        return convert_sso_to_auth_token(token, client, team).await;
    }

    // Fill in auth file data.
    let user_response = client.get_user(token).await.map_err(Error::APIError)?;
    let teams_response = client.get_teams(token).await.map_err(Error::APIError)?;

    let mut teams = Vec::new();
    // NOTE(voz): This doesn't feel great. Ideally we should async fetch all the
    // teams and their spaces, but this should also only be invoked once in a while
    // (until config.json doesn't have tokens anymore) so the perf hit shouldn't be
    // a worry.
    for team in teams_response.teams {
        let team_id = &team.id;
        let spaces_response = client
            .get_spaces(token, Some(team_id))
            .await
            .map_err(Error::APIError)?;
        let spaces = spaces_response.spaces;

        // Because the team endpoint doesn't return the spaces associated for the team,
        // we need to use our custom `Team` struct and apply what we got from the teams
        // endpoint to it.
        let mut team: Team = team.into();
        team.spaces = spaces;
        teams.push(team)
    }

    let auth_token = AuthToken {
        token: token.to_string(),
        api: client.base_url().to_owned(),
        created_at: user_response.user.created_at,
        user: user_response.user,
        teams,
    };
    Ok(auth_token)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use turborepo_vercel_api::{Membership, Role, Space};

    use super::*;
    use crate::{mocks::*, TURBOREPO_AUTH_FILE_NAME, TURBOREPO_CONFIG_DIR};

    #[tokio::test]
    async fn test_convert_to_auth_token() {
        // Setup: Create a mock client and a fake token
        let mock_client = MockApiClient::new();
        let token = "test-token";
        let temp_dir = tempfile::tempdir().unwrap();

        // Create the temp dir files.
        fs::create_dir_all(temp_dir.path().join(TURBOREPO_CONFIG_DIR)).unwrap();

        // Test: Call the convert_to_auth_file function and check the result
        let result = convert_to_auth_token(token, &mock_client, None).await;
        assert!(result.is_ok());
        let auth_token = result.unwrap();

        // Check that the AuthFile contains the correct data
        assert_eq!(auth_token.token, "test-token".to_string());
        assert_eq!(auth_token.api, "custom-domain".to_string());
    }

    #[tokio::test]
    async fn test_write_to_disk_and_read_back() {
        // Setup: Use temp dirs to avoid polluting the user's config dir
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
        auth_file.add_or_update_token(AuthToken {
            token: "test-token".to_string(),
            api: "test-api".to_string(),
            created_at: Some(1634851200),
            teams: Vec::new(),
            user: User {
                id: "user id".to_owned(),
                username: "voz".to_owned(),
                email: "mitch.vostrez@vercel.com".to_owned(),
                name: Some("voz".to_owned()),
                created_at: None,
            },
        });

        // Test: Write the auth file to disk and then read it back.
        auth_file.write_to_disk(&absolute_auth_path).unwrap();

        let read_back: AuthFile =
            serde_json::from_str(&fs::read_to_string(absolute_auth_path.clone()).unwrap()).unwrap();
        assert_eq!(read_back.tokens.len(), 1);
        assert_eq!(read_back.tokens[0].token, "test-token");
    }

    #[tokio::test]
    async fn test_get_token() {
        let mut auth_file = AuthFile { tokens: Vec::new() };
        auth_file.add_or_update_token(AuthToken {
            token: "test-token".to_string(),
            api: "test-api".to_string(),
            created_at: Some(1634851200),
            teams: Vec::new(),
            user: User {
                id: "user id".to_owned(),
                username: "voz".to_owned(),
                email: "mitch.vostrez@vercel.com".to_owned(),
                name: Some("voz".to_owned()),
                created_at: None,
            },
        });

        let token = auth_file.get_token("test-api");
        assert!(token.is_some());
        assert_eq!(token.unwrap().token, "test-token");
    }

    #[tokio::test]
    async fn test_add_token() {
        let mut auth_file = AuthFile { tokens: Vec::new() };
        assert_eq!(auth_file.tokens.len(), 0);

        // Add the first token to the file.
        auth_file.add_or_update_token(AuthToken {
            token: "test-token".to_string(),
            api: "test-api".to_string(),
            created_at: Some(1634851200),
            teams: Vec::new(),
            user: User {
                id: "user id".to_owned(),
                username: "voz".to_owned(),
                email: "mitch.vostrez@vercel.com".to_owned(),
                name: Some("voz".to_owned()),
                created_at: None,
            },
        });

        // Do it twice to make sure it doesn't add duplicates.
        auth_file.add_or_update_token(AuthToken {
            token: "some new token".to_string(),
            api: "test-api".to_string(),
            created_at: Some(1634851200),
            teams: Vec::new(),
            user: User {
                id: "user id".to_owned(),
                username: "voz".to_owned(),
                email: "mitch.vostrez@vercel.com".to_owned(),
                name: Some("voz".to_owned()),
                created_at: None,
            },
        });

        assert_eq!(auth_file.tokens.len(), 1);
        assert!(auth_file.tokens[0].token == *"some new token");

        auth_file.add_or_update_token(AuthToken {
            token: "a second token".to_string(),
            api: "some vercel api".to_string(),
            created_at: Some(1634851200),
            teams: Vec::new(),
            user: User {
                id: "user id".to_owned(),
                username: "voz".to_owned(),
                email: "mitch.vostrez@vercel.com".to_owned(),
                name: Some("voz".to_owned()),
                created_at: None,
            },
        });

        assert_eq!(auth_file.tokens.len(), 2);
        assert!(auth_file.tokens[1].token == *"a second token");
    }

    #[tokio::test]
    async fn test_contains_team_and_space() {
        let team = Team {
            id: "team1".to_string(),
            spaces: vec![Space {
                id: "space1".to_string(),
                name: "space1 name".to_string(),
            }],
            slug: "team1 slug".to_string(),
            name: "maximum effort".to_string(),
            created_at: 0,
            created: chrono::Utc::now(),
            membership: Membership::new(Role::Developer),
        };
        let auth_token = AuthToken {
            token: "token1".to_string(),
            api: "api1".to_string(),
            created_at: None,
            teams: vec![team.clone()],
            user: User {
                id: "user id".to_owned(),
                username: "voz".to_owned(),
                email: "mitch.vostrez@vercel.com".to_owned(),
                name: Some("voz".to_owned()),
                created_at: None,
            },
        };

        assert!(auth_token.contains_team("team1 slug"));
        assert!(!auth_token.contains_team("team2"));
        assert!(team.contains_space("space1"));
        assert!(!team.contains_space("space2"));
    }
}
