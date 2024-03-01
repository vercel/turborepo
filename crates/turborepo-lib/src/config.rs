use std::{collections::HashMap, ffi::OsString, io};

use convert_case::{Case, Casing};
use miette::{Diagnostic, NamedSource, SourceSpan};
use serde::Deserialize;
use struct_iterable::Iterable;
use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_auth::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE};
use turborepo_dirs::config_dir;
use turborepo_errors::TURBO_SITE;
use turborepo_repository::package_json::{Error as PackageJsonError, PackageJson};

pub use crate::turbo_json::RawTurboJson;
use crate::{commands::CommandBase, turbo_json};

#[derive(Debug, Error, Diagnostic)]
#[error("Environment variables should not be prefixed with \"{env_pipeline_delimiter}\"")]
#[diagnostic(
    code(invalid_env_prefix),
    url("{}/messages/{}", TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab))
)]
pub struct InvalidEnvPrefixError {
    pub value: String,
    pub key: String,
    #[source_code]
    pub text: NamedSource,
    #[label("variable with invalid prefix declared here")]
    pub span: Option<SourceSpan>,
    pub env_pipeline_delimiter: &'static str,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Authentication error: {0}")]
    Auth(#[from] turborepo_auth::Error),
    #[error("Global config path not found")]
    NoGlobalConfigPath,
    #[error("Global auth file path not found")]
    NoGlobalAuthFilePath,
    #[error("Global config directory not found")]
    NoGlobalConfigDir,
    #[error(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(
        "Could not find turbo.json.\nFollow directions at https://turbo.build/repo/docs to create \
         one"
    )]
    NoTurboJSON,
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Camino(#[from] camino::FromPathBufError),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error("Encountered an IO error while attempting to read {config_path}: {error}")]
    FailedToReadConfig {
        config_path: AbsoluteSystemPathBuf,
        error: io::Error,
    },
    #[error("Encountered an IO error while attempting to set {config_path}: {error}")]
    FailedToSetConfig {
        config_path: AbsoluteSystemPathBuf,
        error: io::Error,
    },
    #[error(
        "Package tasks (<package>#<task>) are not allowed in single-package repositories: found \
         {task_id}"
    )]
    #[diagnostic(code(package_task_in_single_package_mode), url("{}/messages/{}", TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab)))]
    PackageTaskInSinglePackageMode {
        task_id: String,
        #[source_code]
        text: NamedSource,
        #[label("package task found here")]
        span: Option<SourceSpan>,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    InvalidEnvPrefix(Box<InvalidEnvPrefixError>),
    #[error(transparent)]
    PathError(#[from] turbopath::PathError),
    #[diagnostic(
        code(unnecessary_package_task_syntax),
        url("{}/messages/{}", TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab))
    )]
    #[error("\"{actual}\". Use \"{wanted}\" instead")]
    UnnecessaryPackageTaskSyntax {
        actual: String,
        wanted: String,
        #[label("unnecessary package syntax found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
    },
    #[error("You can only extend from the root workspace")]
    ExtendFromNonRoot {
        #[label("non-root workspace found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
    },
    #[error("`{field}` cannot contain an absolute path")]
    AbsolutePathInConfig {
        field: &'static str,
        #[label("absolute path found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
    },
    #[error("No \"extends\" key found")]
    NoExtends {
        #[label("add extends key here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
    },
    #[error("Failed to create APIClient: {0}")]
    ApiClient(#[source] turborepo_api_client::Error),
    #[error("{0} is not UTF8.")]
    Encoding(String),
    #[error("TURBO_SIGNATURE should be either 1 or 0.")]
    InvalidSignature,
    #[error("TURBO_REMOTE_CACHE_ENABLED should be either 1 or 0.")]
    InvalidRemoteCacheEnabled,
    #[error("TURBO_REMOTE_CACHE_TIMEOUT: error parsing timeout.")]
    InvalidRemoteCacheTimeout(#[source] std::num::ParseIntError),
    #[error("TURBO_PREFLIGHT should be either 1 or 0.")]
    InvalidPreflight,
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJsonParseError(#[from] turbo_json::parser::Error),
}

macro_rules! create_builder {
    ($func_name:ident, $property_name:ident, $type:ty) => {
        pub fn $func_name(mut self, value: $type) -> Self {
            self.override_config.$property_name = value;
            self
        }
    };
}

const DEFAULT_API_URL: &str = "https://vercel.com/api";
const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
const DEFAULT_TIMEOUT: u64 = 30;

// We intentionally don't derive Serialize so that different parts
// of the code that want to display the config can tune how they
// want to display and what fields they want to include.
#[derive(Deserialize, Default, Debug, PartialEq, Eq, Clone, Iterable)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationOptions {
    #[serde(alias = "apiurl")]
    #[serde(alias = "ApiUrl")]
    #[serde(alias = "APIURL")]
    pub(crate) api_url: Option<String>,
    #[serde(alias = "loginurl")]
    #[serde(alias = "LoginUrl")]
    #[serde(alias = "LOGINURL")]
    pub(crate) login_url: Option<String>,
    #[serde(alias = "teamslug")]
    #[serde(alias = "TeamSlug")]
    #[serde(alias = "TEAMSLUG")]
    pub(crate) team_slug: Option<String>,
    #[serde(alias = "teamid")]
    #[serde(alias = "TeamId")]
    #[serde(alias = "TEAMID")]
    pub(crate) team_id: Option<String>,
    pub(crate) token: Option<String>,
    pub(crate) signature: Option<bool>,
    pub(crate) preflight: Option<bool>,
    pub(crate) timeout: Option<u64>,
    pub(crate) enabled: Option<bool>,
    pub(crate) spaces_id: Option<String>,
}

#[derive(Default)]
pub struct TurborepoConfigBuilder {
    repo_root: AbsoluteSystemPathBuf,
    override_config: ConfigurationOptions,

    #[cfg(test)]
    global_config_path: Option<AbsoluteSystemPathBuf>,
    #[cfg(test)]
    environment: HashMap<OsString, OsString>,
}

// Getters
impl ConfigurationOptions {
    pub fn api_url(&self) -> &str {
        non_empty_str(self.api_url.as_deref()).unwrap_or(DEFAULT_API_URL)
    }

    pub fn login_url(&self) -> &str {
        non_empty_str(self.login_url.as_deref()).unwrap_or(DEFAULT_LOGIN_URL)
    }

    pub fn team_slug(&self) -> Option<&str> {
        self.team_slug
            .as_deref()
            .and_then(|slug| (!slug.is_empty()).then_some(slug))
    }

    pub fn team_id(&self) -> Option<&str> {
        non_empty_str(self.team_id.as_deref())
    }

    pub fn token(&self) -> Option<&str> {
        non_empty_str(self.token.as_deref())
    }

    pub fn signature(&self) -> bool {
        self.signature.unwrap_or_default()
    }

    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn preflight(&self) -> bool {
        self.preflight.unwrap_or_default()
    }

    pub fn timeout(&self) -> u64 {
        self.timeout.unwrap_or(DEFAULT_TIMEOUT)
    }

    pub fn spaces_id(&self) -> Option<&str> {
        self.spaces_id.as_deref()
    }
}

// Maps Some("") to None to emulate how Go handles empty strings
fn non_empty_str(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

trait ResolvedConfigurationOptions {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error>;
}

impl ResolvedConfigurationOptions for PackageJson {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error> {
        match &self.legacy_turbo_config {
            Some(legacy_turbo_config) => {
                let synthetic_raw_turbo_json: RawTurboJson = RawTurboJson::parse(
                    &legacy_turbo_config.to_string(),
                    AnchoredSystemPath::new("package.json").unwrap(),
                )?;
                synthetic_raw_turbo_json.get_configuration_options()
            }
            None => Ok(ConfigurationOptions::default()),
        }
    }
}

impl ResolvedConfigurationOptions for RawTurboJson {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error> {
        let mut opts = if let Some(remote_cache_options) = &self.remote_cache {
            remote_cache_options.into()
        } else {
            ConfigurationOptions::default()
        };
        // Don't allow token to be set for shared config.
        opts.token = None;
        opts.spaces_id = self
            .experimental_spaces
            .and_then(|spaces| spaces.id)
            .map(|spaces_id| spaces_id.into());
        Ok(opts)
    }
}

// Used for global config and local config.
impl ResolvedConfigurationOptions for ConfigurationOptions {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error> {
        Ok(self)
    }
}

fn get_lowercased_env_vars() -> HashMap<OsString, OsString> {
    std::env::vars_os()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

fn get_env_var_config(
    environment: &HashMap<OsString, OsString>,
) -> Result<ConfigurationOptions, Error> {
    let mut turbo_mapping = HashMap::new();
    turbo_mapping.insert(OsString::from("turbo_api"), "api_url");
    turbo_mapping.insert(OsString::from("turbo_login"), "login_url");
    turbo_mapping.insert(OsString::from("turbo_team"), "team_slug");
    turbo_mapping.insert(OsString::from("turbo_teamid"), "team_id");
    turbo_mapping.insert(OsString::from("turbo_token"), "token");
    turbo_mapping.insert(OsString::from("turbo_remote_cache_timeout"), "timeout");

    // We do not enable new config sources:
    // turbo_mapping.insert(String::from("turbo_signature"), "signature"); // new
    // turbo_mapping.insert(String::from("turbo_preflight"), "preflight"); // new
    // turbo_mapping.insert(String::from("turbo_remote_cache_enabled"), "enabled");

    let mut output_map = HashMap::new();

    turbo_mapping.into_iter().try_for_each(
        |(mapping_key, mapped_property)| -> Result<(), Error> {
            if let Some(value) = environment.get(&mapping_key) {
                let converted = value.to_str().ok_or_else(|| {
                    Error::Encoding(
                        // CORRECTNESS: the mapping_key is hardcoded above.
                        mapping_key.to_ascii_uppercase().into_string().unwrap(),
                    )
                })?;
                output_map.insert(mapped_property, converted.to_owned());
                Ok(())
            } else {
                Ok(())
            }
        },
    )?;

    // Process signature
    let signature = if let Some(signature) = output_map.get("signature") {
        match signature.as_str() {
            "0" => Some(false),
            "1" => Some(true),
            _ => return Err(Error::InvalidSignature),
        }
    } else {
        None
    };

    // Process preflight
    let preflight = if let Some(preflight) = output_map.get("preflight") {
        match preflight.as_str() {
            "0" => Some(false),
            "1" => Some(true),
            _ => return Err(Error::InvalidPreflight),
        }
    } else {
        None
    };

    // Process enabled
    let enabled = if let Some(enabled) = output_map.get("enabled") {
        match enabled.as_str() {
            "0" => Some(false),
            "1" => Some(true),
            _ => return Err(Error::InvalidRemoteCacheEnabled),
        }
    } else {
        None
    };

    // Process timeout
    let timeout = if let Some(timeout) = output_map.get("timeout") {
        Some(
            timeout
                .parse::<u64>()
                .map_err(Error::InvalidRemoteCacheTimeout)?,
        )
    } else {
        None
    };

    // We currently don't pick up a Spaces ID via env var, we likely won't
    // continue using the Spaces name, we can add an env var when we have the
    // name we want to stick with.
    let spaces_id = None;

    let output = ConfigurationOptions {
        api_url: output_map.get("api_url").cloned(),
        login_url: output_map.get("login_url").cloned(),
        team_slug: output_map.get("team_slug").cloned(),
        team_id: output_map.get("team_id").cloned(),
        token: output_map.get("token").cloned(),

        // Processed booleans
        signature,
        preflight,
        enabled,

        // Processed numbers
        timeout,
        spaces_id,
    };

    Ok(output)
}

fn get_override_env_var_config(
    environment: &HashMap<OsString, OsString>,
) -> Result<ConfigurationOptions, Error> {
    let mut vercel_artifacts_mapping = HashMap::new();
    vercel_artifacts_mapping.insert(OsString::from("vercel_artifacts_token"), "token");
    vercel_artifacts_mapping.insert(OsString::from("vercel_artifacts_owner"), "team_id");

    let mut output_map = HashMap::new();

    // Process the VERCEL_ARTIFACTS_* next.
    vercel_artifacts_mapping.into_iter().try_for_each(
        |(mapping_key, mapped_property)| -> Result<(), Error> {
            if let Some(value) = environment.get(&mapping_key) {
                let converted = value.to_str().ok_or_else(|| {
                    Error::Encoding(
                        // CORRECTNESS: the mapping_key is hardcoded above.
                        mapping_key.to_ascii_uppercase().into_string().unwrap(),
                    )
                })?;
                output_map.insert(mapped_property, converted.to_owned());
                Ok(())
            } else {
                Ok(())
            }
        },
    )?;

    let output = ConfigurationOptions {
        api_url: None,
        login_url: None,
        team_slug: None,
        team_id: output_map.get("team_id").cloned(),
        token: output_map.get("token").cloned(),

        signature: None,
        preflight: None,
        enabled: None,
        timeout: None,
        spaces_id: None,
    };

    Ok(output)
}

impl TurborepoConfigBuilder {
    pub fn new(base: &CommandBase) -> Self {
        Self {
            repo_root: base.repo_root.to_owned(),
            override_config: Default::default(),
            #[cfg(test)]
            global_config_path: base.global_config_path.clone(),
            #[cfg(test)]
            environment: Default::default(),
        }
    }

    // Getting all of the paths.
    fn global_config_path(&self) -> Result<AbsoluteSystemPathBuf, Error> {
        #[cfg(test)]
        if let Some(global_config_path) = self.global_config_path.clone() {
            return Ok(global_config_path);
        }

        let config_dir = config_dir().ok_or(Error::NoGlobalConfigPath)?;
        let global_config_path = config_dir.join("turborepo").join("config.json");
        AbsoluteSystemPathBuf::try_from(global_config_path).map_err(Error::PathError)
    }
    fn global_auth_path(&self) -> Result<AbsoluteSystemPathBuf, Error> {
        #[cfg(test)]
        if let Some(global_config_path) = self.global_config_path.clone() {
            return Ok(global_config_path);
        }

        let config_dir = config_dir().ok_or(Error::NoGlobalConfigDir)?;

        // Check for both Vercel and Turbo paths. Vercel takes priority.
        let vercel_path = config_dir.join(VERCEL_TOKEN_DIR).join(VERCEL_TOKEN_FILE);
        if vercel_path.try_exists().is_ok_and(|exists| exists) {
            return AbsoluteSystemPathBuf::try_from(vercel_path).map_err(Error::PathError);
        }

        let turbo_path = config_dir.join(TURBO_TOKEN_DIR).join(TURBO_TOKEN_FILE);
        AbsoluteSystemPathBuf::try_from(turbo_path).map_err(Error::PathError)
    }
    fn local_config_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_components(&[".turbo", "config.json"])
    }

    #[allow(dead_code)]
    fn root_package_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("package.json")
    }
    #[allow(dead_code)]
    fn root_turbo_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("turbo.json")
    }

    #[cfg(test)]
    fn get_environment(&self) -> HashMap<OsString, OsString> {
        self.environment.clone()
    }

    #[cfg(not(test))]
    fn get_environment(&self) -> HashMap<OsString, OsString> {
        get_lowercased_env_vars()
    }

    fn get_global_config(&self) -> Result<ConfigurationOptions, Error> {
        let global_config_path = self.global_config_path()?;
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

    fn get_local_config(&self) -> Result<ConfigurationOptions, Error> {
        let local_config_path = self.local_config_path();
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

    fn get_global_auth(&self) -> Result<ConfigurationOptions, Error> {
        let global_auth_path = self.global_auth_path()?;
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

    create_builder!(with_api_url, api_url, Option<String>);
    create_builder!(with_login_url, login_url, Option<String>);
    create_builder!(with_team_slug, team_slug, Option<String>);
    create_builder!(with_team_id, team_id, Option<String>);
    create_builder!(with_token, token, Option<String>);
    create_builder!(with_signature, signature, Option<bool>);
    create_builder!(with_enabled, enabled, Option<bool>);
    create_builder!(with_preflight, preflight, Option<bool>);
    create_builder!(with_timeout, timeout, Option<u64>);

    pub fn build(&self) -> Result<ConfigurationOptions, Error> {
        // Priority, from least significant to most significant:
        // - shared configuration (package.json .turbo)
        // - shared configuration (turbo.json)
        // - global configuration (~/.turbo/config.json)
        // - local configuration (<REPO_ROOT>/.turbo/config.json)
        // - environment variables
        // - CLI arguments
        // - builder pattern overrides.

        let root_package_json = PackageJson::load(&self.repo_root.join_component("package.json"))
            .or_else(|e| {
            if let PackageJsonError::Io(e) = &e {
                if matches!(e.kind(), std::io::ErrorKind::NotFound) {
                    return Ok(Default::default());
                }
            }

            Err(e)
        })?;
        let turbo_json = RawTurboJson::read(
            &self.repo_root,
            AnchoredSystemPath::new("turbo.json").unwrap(),
        )
        .or_else(|e| {
            if let Error::Io(e) = &e {
                if matches!(e.kind(), std::io::ErrorKind::NotFound) {
                    return Ok(Default::default());
                }
            }

            Err(e)
        })?;
        let global_config = self.get_global_config()?;
        let global_auth = self.get_global_auth()?;
        let local_config = self.get_local_config()?;
        let env_vars = self.get_environment();
        let env_var_config = get_env_var_config(&env_vars)?;
        let override_env_var_config = get_override_env_var_config(&env_vars)?;

        let sources = [
            root_package_json.get_configuration_options(),
            turbo_json.get_configuration_options(),
            global_config.get_configuration_options(),
            global_auth.get_configuration_options(),
            local_config.get_configuration_options(),
            env_var_config.get_configuration_options(),
            Ok(self.override_config.clone()),
            override_env_var_config.get_configuration_options(),
        ];

        sources.into_iter().try_fold(
            ConfigurationOptions::default(),
            |mut acc, current_source| {
                current_source.map(|current_source_config| {
                    if let Some(api_url) = current_source_config.api_url.clone() {
                        acc.api_url = Some(api_url);
                    }
                    if let Some(login_url) = current_source_config.login_url.clone() {
                        acc.login_url = Some(login_url);
                    }
                    if let Some(team_slug) = current_source_config.team_slug.clone() {
                        acc.team_slug = Some(team_slug);
                    }
                    if let Some(team_id) = current_source_config.team_id.clone() {
                        acc.team_id = Some(team_id);
                    }
                    if let Some(token) = current_source_config.token.clone() {
                        acc.token = Some(token);
                    }
                    if let Some(signature) = current_source_config.signature {
                        acc.signature = Some(signature);
                    }
                    if let Some(enabled) = current_source_config.enabled {
                        acc.enabled = Some(enabled);
                    }
                    if let Some(preflight) = current_source_config.preflight {
                        acc.preflight = Some(preflight);
                    }
                    if let Some(timeout) = current_source_config.timeout {
                        acc.timeout = Some(timeout);
                    }
                    if let Some(spaces_id) = current_source_config.spaces_id {
                        acc.spaces_id = Some(spaces_id);
                    }

                    acc
                })
            },
        )
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, ffi::OsString};

    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::config::{
        get_env_var_config, get_override_env_var_config, ConfigurationOptions,
        TurborepoConfigBuilder, DEFAULT_API_URL, DEFAULT_LOGIN_URL, DEFAULT_TIMEOUT,
    };

    #[test]
    fn test_defaults() {
        let defaults: ConfigurationOptions = Default::default();
        assert_eq!(defaults.api_url(), DEFAULT_API_URL);
        assert_eq!(defaults.login_url(), DEFAULT_LOGIN_URL);
        assert_eq!(defaults.team_slug(), None);
        assert_eq!(defaults.team_id(), None);
        assert_eq!(defaults.token(), None);
        assert!(!defaults.signature());
        assert!(defaults.enabled());
        assert!(!defaults.preflight());
        assert_eq!(defaults.timeout(), DEFAULT_TIMEOUT);
        assert_eq!(defaults.spaces_id(), None);
    }

    #[test]
    fn test_env_setting() {
        let mut env: HashMap<OsString, OsString> = HashMap::new();

        let turbo_api = "https://example.com/api";
        let turbo_login = "https://example.com/login";
        let turbo_team = "vercel";
        let turbo_teamid = "team_nLlpyC6REAqxydlFKbrMDlud";
        let turbo_token = "abcdef1234567890abcdef";
        let turbo_remote_cache_timeout = 200;

        env.insert("turbo_api".into(), turbo_api.into());
        env.insert("turbo_login".into(), turbo_login.into());
        env.insert("turbo_team".into(), turbo_team.into());
        env.insert("turbo_teamid".into(), turbo_teamid.into());
        env.insert("turbo_token".into(), turbo_token.into());
        env.insert(
            "turbo_remote_cache_timeout".into(),
            turbo_remote_cache_timeout.to_string().into(),
        );

        let config = get_env_var_config(&env).unwrap();
        assert_eq!(turbo_api, config.api_url.unwrap());
        assert_eq!(turbo_login, config.login_url.unwrap());
        assert_eq!(turbo_team, config.team_slug.unwrap());
        assert_eq!(turbo_teamid, config.team_id.unwrap());
        assert_eq!(turbo_token, config.token.unwrap());
        assert_eq!(turbo_remote_cache_timeout, config.timeout.unwrap());
    }

    #[test]
    fn test_empty_env_setting() {
        let mut env: HashMap<OsString, OsString> = HashMap::new();
        env.insert("turbo_api".into(), "".into());
        env.insert("turbo_login".into(), "".into());
        env.insert("turbo_team".into(), "".into());
        env.insert("turbo_teamid".into(), "".into());
        env.insert("turbo_token".into(), "".into());

        let config = get_env_var_config(&env).unwrap();
        assert_eq!(config.api_url(), DEFAULT_API_URL);
        assert_eq!(config.login_url(), DEFAULT_LOGIN_URL);
        assert_eq!(config.team_slug(), None);
        assert_eq!(config.team_id(), None);
        assert_eq!(config.token(), None);
    }

    #[test]
    fn test_override_env_setting() {
        let mut env: HashMap<OsString, OsString> = HashMap::new();

        let vercel_artifacts_token = "correct-horse-battery-staple";
        let vercel_artifacts_owner = "bobby_tables";

        env.insert(
            "vercel_artifacts_token".into(),
            vercel_artifacts_token.into(),
        );
        env.insert(
            "vercel_artifacts_owner".into(),
            vercel_artifacts_owner.into(),
        );

        let config = get_override_env_var_config(&env).unwrap();
        assert_eq!(vercel_artifacts_token, config.token.unwrap());
        assert_eq!(vercel_artifacts_owner, config.team_id.unwrap());
    }

    #[test]
    fn test_env_layering() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let global_config_path = AbsoluteSystemPathBuf::try_from(
            TempDir::new().unwrap().path().join("nonexistent.json"),
        )
        .unwrap();

        repo_root
            .join_component("turbo.json")
            .create_with_contents(r#"{"experimentalSpaces": {"id": "my-spaces-id"}}"#)
            .unwrap();

        let turbo_teamid = "team_nLlpyC6REAqxydlFKbrMDlud";
        let turbo_token = "abcdef1234567890abcdef";
        let vercel_artifacts_owner = "team_SOMEHASH";
        let vercel_artifacts_token = "correct-horse-battery-staple";

        let mut env: HashMap<OsString, OsString> = HashMap::new();
        env.insert("turbo_teamid".into(), turbo_teamid.into());
        env.insert("turbo_token".into(), turbo_token.into());
        env.insert(
            "vercel_artifacts_token".into(),
            vercel_artifacts_token.into(),
        );
        env.insert(
            "vercel_artifacts_owner".into(),
            vercel_artifacts_owner.into(),
        );

        let override_config = ConfigurationOptions {
            token: Some("unseen".into()),
            team_id: Some("unseen".into()),
            ..Default::default()
        };

        let builder = TurborepoConfigBuilder {
            repo_root,
            override_config,
            global_config_path: Some(global_config_path),
            environment: env,
        };

        let config = builder.build().unwrap();
        assert_eq!(config.team_id().unwrap(), vercel_artifacts_owner);
        assert_eq!(config.token().unwrap(), vercel_artifacts_token);
        assert_eq!(config.spaces_id().unwrap(), "my-spaces-id");
    }
}
