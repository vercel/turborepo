use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPathBuf;

#[derive(Serialize, Deserialize, Debug)]
/// A set of Turbo profiles containing configuration.
pub struct Profiles {
    #[serde(flatten)]
    profiles: HashMap<String, Profile>,
}

impl Profiles {
    /// Calls the read_existing_to_string_or on the path with a default of an
    /// empty string. Will return empty profiles if no errors happen.
    pub fn read_from_file(
        path: &AbsoluteSystemPathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let content = path.read_existing_to_string_or(Ok(""))?;
        let profiles: Self = toml::from_str(&content)?;
        Ok(profiles)
    }
}

#[derive(Serialize, Deserialize, Debug)]
/// Contains configuration for Turborepo and Turbopack.
pub struct Profile {
    pub active: bool,

    #[serde(rename = "turborepo_login_api", default = "default_login_api")]
    /// API used in `turbo login`. Defaults to `vercel.com/api`.
    pub login_api: String,

    #[serde(
        rename = "turborepo_sso_provider",
        skip_serializing_if = "Option::is_none"
    )]
    /// SSO provider. Example: `SAML/OIDC Single Sign-On`
    pub sso_provider: Option<String>,

    #[serde(rename = "turborepo_sso_team", skip_serializing_if = "Option::is_none")]
    /// SSO Team to log into.
    pub sso_team: Option<String>,

    #[serde(
        rename = "turbopack_random_setting",
        skip_serializing_if = "Option::is_none"
    )]
    /// Some arbitrary turbopack setting.
    pub turbopack_setting: Option<String>,
}

// Used for serde default setting.
fn default_login_api() -> String {
    "vercel.com/api".to_owned()
}
