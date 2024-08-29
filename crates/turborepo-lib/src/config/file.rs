use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_auth::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE};
use turborepo_dirs::{config_dir, vercel_config_dir};

use super::{ConfigurationOptions, Error};

pub fn get_global_config(
    override_path: Option<AbsoluteSystemPathBuf>,
) -> Result<ConfigurationOptions, Error> {
    let global_config_path = override_path.map_or_else(global_config_path, Ok)?;
    let mut contents = global_config_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|error| Error::FailedToReadConfig {
            config_path: global_config_path.clone(),
            error,
        })?;
    if contents.is_empty() {
        contents = String::from("{}");
    }
    let global_config: ConfigurationOptions = serde_json::from_str(&contents)?;
    Ok(global_config)
}

pub fn get_local_config(repo_root: &AbsoluteSystemPath) -> Result<ConfigurationOptions, Error> {
    let local_config_path = local_config_path(repo_root);
    let mut contents = local_config_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|error| Error::FailedToReadConfig {
            config_path: local_config_path.clone(),
            error,
        })?;
    if contents.is_empty() {
        contents = String::from("{}");
    }
    let local_config: ConfigurationOptions = serde_json::from_str(&contents)?;
    Ok(local_config)
}

pub fn get_global_auth(
    override_path: Option<AbsoluteSystemPathBuf>,
) -> Result<ConfigurationOptions, Error> {
    let global_auth_path = override_path.map_or_else(global_auth_path, Ok)?;
    let token = match turborepo_auth::Token::from_file(&global_auth_path) {
        Ok(token) => token,
        // Multiple ways this can go wrong. Don't error out if we can't find the token - it
        // just might not be there.
        Err(e) => {
            if matches!(e, turborepo_auth::Error::TokenNotFound) {
                return Ok(ConfigurationOptions::default());
            }

            return Err(e.into());
        }
    };

    // No auth token found in either Vercel or Turbo config.
    if token.into_inner().is_empty() {
        return Ok(ConfigurationOptions::default());
    }

    let global_auth: ConfigurationOptions = ConfigurationOptions {
        token: Some(token.into_inner().to_owned()),
        ..Default::default()
    };
    Ok(global_auth)
}

fn global_config_path() -> Result<AbsoluteSystemPathBuf, Error> {
    let config_dir = config_dir()?.ok_or(Error::NoGlobalConfigPath)?;

    Ok(config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]))
}

fn local_config_path(repo_root: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
    repo_root.join_components(&[".turbo", "config.json"])
}

fn global_auth_path() -> Result<AbsoluteSystemPathBuf, Error> {
    let vercel_config_dir = vercel_config_dir()?.ok_or(Error::NoGlobalConfigDir)?;
    // Check for both Vercel and Turbo paths. Vercel takes priority.
    let vercel_path = vercel_config_dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]);
    if vercel_path.exists() {
        return Ok(vercel_path);
    }

    let turbo_config_dir = config_dir()?.ok_or(Error::NoGlobalConfigDir)?;

    Ok(turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]))
}
