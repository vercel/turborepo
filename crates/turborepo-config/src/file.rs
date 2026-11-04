use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_auth::{
    TURBO_AUTH_FILE, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE,
};
use turborepo_dirs::{config_dir, vercel_config_dir};
use turborepo_types::Credential;

use crate::{ConfigurationOptions, Error, ResolvedConfigurationOptions};

/// Explicit paths for the global configuration and authentication sources.
/// Repository-local config and the selected root turbo.json are resolved from
/// the resolver's repository root and configuration options.
#[derive(Debug, Clone)]
pub struct ConfigurationFileInputs {
    pub global_config_path: AbsoluteSystemPathBuf,
    pub global_auth_path: AbsoluteSystemPathBuf,
    pub legacy_auth_path: Option<AbsoluteSystemPathBuf>,
}

impl ConfigurationFileInputs {
    /// Resolves the default global configuration and authentication paths from
    /// the current process environment. Production adapters can pass the
    /// resulting paths to the pure configuration resolver.
    pub fn from_process_environment() -> Result<Self, Error> {
        Ok(Self {
            global_config_path: global_config_path()?,
            global_auth_path: global_auth_path()?,
            legacy_auth_path: legacy_auth_path()?,
        })
    }
}

pub struct ConfigFile {
    path: AbsoluteSystemPathBuf,
}

impl ConfigFile {
    pub(crate) fn from_path(path: AbsoluteSystemPathBuf) -> Self {
        Self { path }
    }

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
    path: Option<AbsoluteSystemPathBuf>,
    fallback: Option<Credential>,
    legacy_path: Option<AbsoluteSystemPathBuf>,
}

impl AuthFile {
    pub(crate) fn from_paths(
        path: AbsoluteSystemPathBuf,
        fallback: Option<Credential>,
        legacy_path: Option<AbsoluteSystemPathBuf>,
    ) -> Self {
        Self {
            path: Some(path),
            fallback,
            legacy_path,
        }
    }

    pub fn global_auth(
        override_path: Option<AbsoluteSystemPathBuf>,
        fallback: Option<Credential>,
    ) -> Result<Self, Error> {
        match override_path {
            Some(_) => Ok(Self {
                path: None,
                fallback,
                legacy_path: None,
            }),
            None => Ok(Self {
                path: Some(global_auth_path()?),
                fallback,
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

        let mut token = self
            .path
            .as_deref()
            .map(load_token)
            .transpose()?
            .flatten()
            .map(|token| Credential::saved_vercel(token.into_inner().expose().into()));

        if token.is_none() {
            token = self.fallback.clone();
        }

        if token.is_none()
            && let Some(path) = self.legacy_path.as_ref()
        {
            token = load_token(path)?
                .map(|token| Credential::saved_vercel(token.into_inner().expose().into()));
        }

        Ok(token.map_or_else(ConfigurationOptions::default, |token| {
            ConfigurationOptions {
                token: Some(token),
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
