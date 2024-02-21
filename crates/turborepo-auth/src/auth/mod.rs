mod login;
mod logout;
mod sso;

pub use login::*;
pub use logout::*;
pub use sso::*;
use turborepo_api_client::{Client, TokenClient};
use turborepo_ui::{BOLD, UI};

use crate::{ui, LoginServer, Token};

const VERCEL_TOKEN_DIR: &str = "com.vercel.cli";
const VERCEL_TOKEN_FILE: &str = "auth.json";

pub struct LoginOptions<'a, T>
where
    T: Client + TokenClient,
{
    pub ui: &'a UI,
    pub login_url: &'a str,
    pub api_client: &'a T,
    pub login_server: &'a dyn LoginServer,

    pub sso_team: Option<&'a str>,
    pub existing_token: Option<&'a str>,
    pub force: bool,
}
impl<'a, T> LoginOptions<'a, T>
where
    T: Client + TokenClient,
{
    pub fn new(
        ui: &'a UI,
        login_url: &'a str,
        api_client: &'a T,
        login_server: &'a dyn LoginServer,
    ) -> Self {
        Self {
            ui,
            login_url,
            api_client,
            login_server,
            sso_team: None,
            existing_token: None,
            force: false,
        }
    }
}

async fn check_user_token(
    token: &str,
    ui: &UI,
    api_client: &(impl Client + TokenClient),
    message: &str,
) -> Result<Token, Error> {
    let response_user = api_client.get_user(token).await?;
    println!("{}", ui.apply(BOLD.apply_to(message)));
    ui::print_cli_authorized(&response_user.user.email, ui);
    Ok(Token::Existing(token.to_string()))
}

async fn check_sso_token(
    token: &str,
    sso_team: &str,
    ui: &UI,
    api_client: &(impl Client + TokenClient),
    message: &str,
) -> Result<Token, Error> {
    let (result_user, result_teams) =
        tokio::join!(api_client.get_user(token), api_client.get_teams(token),);

    let token = Token::existing(token.into());

    match (result_user, result_teams) {
        (Ok(response_user), Ok(response_teams)) => {
            if response_teams
                .teams
                .iter()
                .any(|team| team.slug == sso_team)
            {
                if token.is_valid(api_client).await? {
                    println!("{}", ui.apply(BOLD.apply_to(message)));
                    ui::print_cli_authorized(&response_user.user.email, ui);
                    Ok(token)
                } else {
                    Err(Error::SSOTokenExpired(sso_team.to_string()))
                }
            } else {
                Err(Error::SSOTeamNotFound(sso_team.to_string()))
            }
        }
        (Err(e), _) | (_, Err(e)) => Err(Error::APIError(e)),
    }
}

fn extract_vercel_token() -> Result<String, Error> {
    let vercel_config_dir =
        turborepo_dirs::vercel_config_dir().ok_or_else(|| Error::ConfigDirNotFound)?;

    let vercel_token_path = vercel_config_dir
        .join(VERCEL_TOKEN_DIR)
        .join(VERCEL_TOKEN_FILE);
    let contents = std::fs::read_to_string(vercel_token_path)?;

    #[derive(serde::Deserialize)]
    struct VercelToken {
        // This isn't actually dead code, it's used by serde to deserialize the JSON.
        #[allow(dead_code)]
        token: String,
    }

    Ok(serde_json::from_str::<VercelToken>(&contents)?.token)
}
