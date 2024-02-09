mod login;
mod logout;
mod sso;

pub use login::*;
pub use logout::*;
pub use sso::*;
use turborepo_api_client::Client;
use turborepo_ui::{BOLD, UI};

use crate::{ui, LoginServer, Token};

const VERCEL_TOKEN_DIR: &str = "com.vercel.cli";
const VERCEL_TOKEN_FILE: &str = "auth.json";

pub struct LoginOptions<'a, T>
where
    T: Client,
{
    pub ui: &'a UI,
    pub login_url: &'a str,
    pub api_client: &'a T,
    pub login_server: &'a dyn LoginServer,

    pub sso_team: Option<&'a str>,
    pub existing_token: Option<&'a str>,
}
impl<'a, T> LoginOptions<'a, T>
where
    T: Client,
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
        }
    }
}

async fn check_token(
    token: &str,
    ui: &UI,
    api_client: &impl Client,
    message: &str,
) -> Result<Token, Error> {
    let response = api_client.get_user(token).await?;
    println!("{}", ui.apply(BOLD.apply_to(message)));
    ui::print_cli_authorized(&response.user.email, ui);
    Ok(Token::Existing(token.to_string()))
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
