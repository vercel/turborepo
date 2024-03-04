mod login;
mod logout;
mod sso;

pub use login::*;
pub use logout::*;
pub use sso::*;
use turbopath::AbsoluteSystemPath;
use turborepo_api_client::{CacheClient, Client, TokenClient};
use turborepo_ui::UI;

use crate::LoginServer;

const VERCEL_TOKEN_DIR: &str = "com.vercel.cli";
const VERCEL_TOKEN_FILE: &str = "auth.json";

pub struct LoginOptions<'a, T: Client + TokenClient + CacheClient> {
    pub ui: &'a UI,
    pub login_url: &'a str,
    pub api_client: &'a T,
    pub login_server: &'a dyn LoginServer,

    pub sso_team: Option<&'a str>,
    pub existing_token: Option<&'a str>,
    pub force: bool,
}
impl<'a, T: Client + TokenClient + CacheClient> LoginOptions<'a, T> {
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

/// Options for logging out.
pub struct LogoutOptions<'a, T> {
    pub ui: &'a UI,
    pub api_client: &'a T,

    /// The path where we should look for the token to logout.
    pub path: &'a AbsoluteSystemPath,
}

fn extract_vercel_token() -> Result<Option<String>, Error> {
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
        token: Option<String>,
    }

    Ok(serde_json::from_str::<VercelToken>(&contents)?.token)
}
