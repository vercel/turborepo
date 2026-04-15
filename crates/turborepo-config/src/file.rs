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
}

impl AuthFile {
    pub fn global_auth(override_path: Option<AbsoluteSystemPathBuf>) -> Result<Self, Error> {
        let path = override_path.map_or_else(global_auth_path, Ok)?;
        Ok(Self { path })
    }
}

impl ResolvedConfigurationOptions for AuthFile {
    fn get_configuration_options(
        &self,
        _existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        let contents =
            self.path
                .read_existing_to_string()
                .map_err(|error| Error::FailedToReadConfig {
                    config_path: self.path.clone(),
                    error,
                })?;

        if contents.as_deref().is_none_or(str::is_empty) {
            return Ok(ConfigurationOptions::default());
        }

        let token = match turborepo_auth::Token::from_file(&self.path) {
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
        if token.into_inner().expose().is_empty() {
            return Ok(ConfigurationOptions::default());
        }

        let global_auth: ConfigurationOptions = ConfigurationOptions {
            token: Some(token.into_inner().expose().to_owned()),
            ..Default::default()
        };
        Ok(global_auth)
    }
}

fn global_config_path() -> Result<AbsoluteSystemPathBuf, Error> {
    let config_dir = config_dir()?.ok_or(Error::NoGlobalConfigPath)?;

    Ok(config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]))
}

fn global_auth_path() -> Result<AbsoluteSystemPathBuf, Error> {
    let turbo_config_dir = config_dir()?;
    if let Some(config_dir) = turbo_config_dir.as_ref() {
        let turbo_auth_path = config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
        if turbo_auth_path.exists() {
            return Ok(turbo_auth_path);
        }
    }

    let vercel_config_dir = vercel_config_dir()?;
    let Some(vercel_config_dir) = vercel_config_dir.as_ref() else {
        let turbo_config_dir = turbo_config_dir.ok_or(Error::NoGlobalConfigDir)?;
        return Ok(turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]));
    };

    let vercel_path = vercel_config_dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]);
    if vercel_path.exists() {
        return Ok(vercel_path);
    }

    let turbo_config_dir = turbo_config_dir.ok_or(Error::NoGlobalConfigDir)?;

    Ok(turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]))
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    use tempfile::tempdir;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct ConfigDirEnvGuard;

    impl ConfigDirEnvGuard {
        fn set(turbo_dir: &AbsoluteSystemPathBuf, vercel_dir: &AbsoluteSystemPathBuf) -> Self {
            unsafe {
                std::env::set_var("TURBO_CONFIG_DIR_PATH", turbo_dir.as_str());
                std::env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_dir.as_str());
            }

            Self
        }
    }

    impl Drop for ConfigDirEnvGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("TURBO_CONFIG_DIR_PATH");
                std::env::remove_var("VERCEL_CONFIG_DIR_PATH");
            }
        }
    }

    fn write_file(path: &camino::Utf8Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("path should have a parent"))
            .expect("failed to create parent dir");
        std::fs::write(path, contents).expect("failed to write file");
    }

    fn expiry_secs_from_now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs()
            + 3600
    }

    #[test]
    fn auth_file_prefers_turbo_auth_over_shared_auth() {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let turbo_dir = tempdir().expect("failed to create turbo temp dir");
        let vercel_dir = tempdir().expect("failed to create vercel temp dir");
        let turbo_dir = AbsoluteSystemPathBuf::try_from(turbo_dir.path().to_path_buf())
            .expect("failed to create turbo path");
        let vercel_dir = AbsoluteSystemPathBuf::try_from(vercel_dir.path().to_path_buf())
            .expect("failed to create vercel path");
        let _guard = ConfigDirEnvGuard::set(&turbo_dir, &vercel_dir);

        write_file(
            turbo_dir
                .join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE])
                .as_path(),
            &format!(
                r#"{{"token":"vca_turbo_auth","refreshToken":"refresh-token","expiresAt":{}}}"#,
                expiry_secs_from_now()
            ),
        );
        write_file(
            vercel_dir
                .join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE])
                .as_path(),
            r#"{"token":"vercel_shared_token"}"#,
        );
        write_file(
            turbo_dir
                .join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE])
                .as_path(),
            r#"{"token":"legacy_config_token"}"#,
        );

        let auth_file = AuthFile::global_auth(None).expect("failed to construct auth file");
        let config = auth_file
            .get_configuration_options(&ConfigurationOptions::default())
            .expect("failed to load auth config");

        assert_eq!(config.token.as_deref(), Some("vca_turbo_auth"));
    }

    #[test]
    fn auth_file_falls_back_to_legacy_config() {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let turbo_dir = tempdir().expect("failed to create turbo temp dir");
        let vercel_dir = tempdir().expect("failed to create vercel temp dir");
        let turbo_dir = AbsoluteSystemPathBuf::try_from(turbo_dir.path().to_path_buf())
            .expect("failed to create turbo path");
        let vercel_dir = AbsoluteSystemPathBuf::try_from(vercel_dir.path().to_path_buf())
            .expect("failed to create vercel path");
        let _guard = ConfigDirEnvGuard::set(&turbo_dir, &vercel_dir);

        write_file(
            turbo_dir
                .join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE])
                .as_path(),
            r#"{"token":"legacy_config_token"}"#,
        );

        let auth_file = AuthFile::global_auth(None).expect("failed to construct auth file");
        let config = auth_file
            .get_configuration_options(&ConfigurationOptions::default())
            .expect("failed to load auth config");

        assert_eq!(config.token.as_deref(), Some("legacy_config_token"));
    }
}
