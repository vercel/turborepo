use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_auth::{
    TURBO_AUTH_FILE, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE,
};
use turborepo_dirs::{config_dir, vercel_config_dir};

use crate::{ConfigurationOptions, Error, ResolvedConfigurationOptions};

pub struct ConfigFile {
    path: AbsoluteSystemPathBuf,
}

impl ConfigFile {
    pub fn global_config(override_path: Option<AbsoluteSystemPathBuf>) -> Result<Self, Error> {
        let path = override_path.map_or_else(global_config_path, Ok)?;
        Ok(Self { path })
    }

    pub fn local_config(repo_root: &AbsoluteSystemPath) -> Self {
        let path = repo_root.join_components(&[".turbo", "config.json"]);
        Self { path }
    }
}

impl ResolvedConfigurationOptions for ConfigFile {
    fn get_configuration_options(
        &self,
        _existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        let contents = self
            .path
            .read_existing_to_string()
            .map_err(|error| Error::FailedToReadConfig {
                config_path: self.path.clone(),
                error,
            })?
            .filter(|s| !s.is_empty());

        let global_config = contents
            .as_deref()
            .map_or_else(|| Ok(ConfigurationOptions::default()), serde_json::from_str)?;
        Ok(global_config)
    }
}

pub struct AuthFile {
    path: AbsoluteSystemPathBuf,
    fallback_path: Option<AbsoluteSystemPathBuf>,
    legacy_path: Option<AbsoluteSystemPathBuf>,
}

impl AuthFile {
    pub fn global_auth(override_path: Option<AbsoluteSystemPathBuf>) -> Result<Self, Error> {
        match override_path {
            Some(path) => Ok(Self {
                path,
                fallback_path: None,
                legacy_path: None,
            }),
            None => Ok(Self {
                path: global_auth_path()?,
                fallback_path: Some(global_config_path()?),
                legacy_path: legacy_auth_path()?,
            }),
        }
    }
}

impl ResolvedConfigurationOptions for AuthFile {
    fn get_configuration_options(
        &self,
        _existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        let load_token =
            |path: &AbsoluteSystemPath| -> Result<Option<turborepo_auth::Token>, Error> {
                let contents =
                    path.read_existing_to_string()
                        .map_err(|error| Error::FailedToReadConfig {
                            config_path: path.to_owned(),
                            error,
                        })?;

                if contents.as_deref().is_none_or(str::is_empty) {
                    return Ok(None);
                }

                match turborepo_auth::Token::from_file(path) {
                    Ok(token) if !token.into_inner().expose().is_empty() => Ok(Some(token)),
                    Ok(_)
                    | Err(turborepo_auth::Error::TokenNotFound)
                    | Err(turborepo_auth::Error::InvalidTokenFileFormat { .. }) => Ok(None),
                    Err(e) => Err(e.into()),
                }
            };

        let mut token = load_token(&self.path)?;

        if token.is_none()
            && let Some(path) = self.fallback_path.as_ref()
        {
            token = load_token(path)?;
        }

        if token.is_none()
            && let Some(path) = self.legacy_path.as_ref()
        {
            token = load_token(path)?;
        }

        Ok(token.map_or_else(ConfigurationOptions::default, |token| {
            ConfigurationOptions {
                token: Some(token.into_inner().expose().to_owned()),
                ..Default::default()
            }
        }))
    }
}

fn global_config_path() -> Result<AbsoluteSystemPathBuf, Error> {
    let config_dir = config_dir()?.ok_or(Error::NoGlobalConfigPath)?;

    Ok(config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]))
}

fn global_auth_path() -> Result<AbsoluteSystemPathBuf, Error> {
    let config_dir = config_dir()?.ok_or(Error::NoGlobalConfigPath)?;

    Ok(config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]))
}

fn legacy_auth_path() -> Result<Option<AbsoluteSystemPathBuf>, Error> {
    Ok(vercel_config_dir()?.map(|dir| dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE])))
}

#[cfg(test)]
mod tests {
    use std::{env, fs, sync::Mutex};

    use tempfile::tempdir;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn write_auth_file(path: &AbsoluteSystemPathBuf, token: &str) {
        path.create_with_contents(format!(r#"{{"token":"{token}"}}"#))
            .expect("Failed to write auth file");
    }

    #[test]
    fn global_auth_prefers_turbo_auth_token() {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let turbo_dir = tempdir().expect("Failed to create turbo dir");
        let vercel_dir = tempdir().expect("Failed to create vercel dir");
        let turbo_auth_path = turbo_dir.path().join("turborepo/auth.json");
        let turbo_config_path = turbo_dir.path().join("turborepo/config.json");
        let legacy_auth_path = vercel_dir.path().join("com.vercel.cli/auth.json");

        let turbo_auth_path = AbsoluteSystemPathBuf::try_from(turbo_auth_path.clone())
            .expect("Failed to create turbo auth path");
        let turbo_config_path = AbsoluteSystemPathBuf::try_from(turbo_config_path.clone())
            .expect("Failed to create turbo config path");
        let legacy_auth_path = AbsoluteSystemPathBuf::try_from(legacy_auth_path.clone())
            .expect("Failed to create legacy auth path");

        fs::create_dir_all(turbo_dir.path().join("turborepo")).expect("Failed to create turbo dir");
        fs::create_dir_all(vercel_dir.path().join("com.vercel.cli"))
            .expect("Failed to create legacy auth dir");
        write_auth_file(&turbo_auth_path, "turbo-auth-token");
        turbo_config_path
            .create_with_contents(r#"{"token":"turbo-token"}"#)
            .expect("Failed to write turbo config");
        legacy_auth_path
            .create_with_contents(r#"{"token":"legacy-token"}"#)
            .expect("Failed to write legacy auth");

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_dir.path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_dir.path());
        }

        let auth_file = AuthFile::global_auth(None).expect("Failed to create auth file");
        let config = auth_file
            .get_configuration_options(&ConfigurationOptions::default())
            .expect("Failed to load auth config");

        assert_eq!(config.token(), Some("turbo-auth-token"));

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn global_auth_falls_back_to_turbo_config_token() {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let turbo_dir = tempdir().expect("Failed to create turbo dir");
        let vercel_dir = tempdir().expect("Failed to create vercel dir");
        let turbo_config_path = turbo_dir.path().join("turborepo/config.json");

        let turbo_config_path = AbsoluteSystemPathBuf::try_from(turbo_config_path.clone())
            .expect("Failed to create turbo config path");

        fs::create_dir_all(turbo_dir.path().join("turborepo")).expect("Failed to create turbo dir");

        turbo_config_path
            .create_with_contents(r#"{"token":"turbo-token"}"#)
            .expect("Failed to write turbo config");

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_dir.path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_dir.path());
        }

        let auth_file = AuthFile::global_auth(None).expect("Failed to create auth file");
        let config = auth_file
            .get_configuration_options(&ConfigurationOptions::default())
            .expect("Failed to load auth config");

        assert_eq!(config.token(), Some("turbo-token"));

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn global_auth_falls_back_to_legacy_vercel_auth() {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let turbo_dir = tempdir().expect("Failed to create turbo dir");
        let vercel_dir = tempdir().expect("Failed to create vercel dir");
        let legacy_auth_path = vercel_dir.path().join("com.vercel.cli/auth.json");

        let legacy_auth_path = AbsoluteSystemPathBuf::try_from(legacy_auth_path.clone())
            .expect("Failed to create legacy auth path");

        fs::create_dir_all(vercel_dir.path().join("com.vercel.cli"))
            .expect("Failed to create legacy auth dir");
        legacy_auth_path
            .create_with_contents(r#"{"token":"legacy-token"}"#)
            .expect("Failed to write legacy auth");

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_dir.path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_dir.path());
        }

        let auth_file = AuthFile::global_auth(None).expect("Failed to create auth file");
        let config = auth_file
            .get_configuration_options(&ConfigurationOptions::default())
            .expect("Failed to load auth config");

        assert_eq!(config.token(), Some("legacy-token"));

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }
}
