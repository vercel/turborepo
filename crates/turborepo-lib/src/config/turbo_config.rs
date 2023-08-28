use std::collections::HashMap;

use dirs_next::config_dir;
use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPathBuf;

use crate::{commands::CommandBase, config::RawTurboJSON, package_json::PackageJson};

const DEFAULT_API_URL: &str = "https://vercel.com/api";
const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
const DEFAULT_TIMEOUT: u64 = 20;

use anyhow::{anyhow, Error};

macro_rules! create_builder {
    ($func_name:ident, $property_name:ident, $type:ty) => {
        pub fn $func_name(mut self, value: $type) -> Self {
            self.override_config.$property_name = value;
            self
        }
    };
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq, Clone)]
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
}

#[derive(Default)]
pub struct TurborepoConfigBuilder {
    repo_root: AbsoluteSystemPathBuf,
    override_config: ConfigurationOptions,

    // Used for testing.
    global_config_path: Option<AbsoluteSystemPathBuf>,
}

// Getters
impl ConfigurationOptions {
    #[allow(dead_code)]
    pub fn api_url(&self) -> &str {
        self.api_url.as_deref().unwrap_or(DEFAULT_API_URL)
    }

    #[allow(dead_code)]
    pub fn login_url(&self) -> &str {
        self.login_url.as_deref().unwrap_or(DEFAULT_LOGIN_URL)
    }

    #[allow(dead_code)]
    pub fn team_slug(&self) -> Option<&str> {
        self.team_slug.as_deref()
    }

    #[allow(dead_code)]
    pub fn team_id(&self) -> Option<&str> {
        self.team_id.as_deref()
    }

    #[allow(dead_code)]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    #[allow(dead_code)]
    pub fn signature(&self) -> bool {
        self.signature.unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn preflight(&self) -> bool {
        self.preflight.unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn timeout(&self) -> u64 {
        self.timeout.unwrap_or(DEFAULT_TIMEOUT)
    }
}

trait ResolvedConfigurationOptions {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error>;
}

impl ResolvedConfigurationOptions for PackageJson {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error> {
        match &self.legacy_turbo_config {
            Some(legacy_turbo_config) => {
                let synthetic_raw_turbo_json: RawTurboJSON =
                    serde_json::from_value(legacy_turbo_config.clone())
                        .map_err(|_| anyhow!("global_de"))?;
                synthetic_raw_turbo_json.get_configuration_options()
            }
            None => Ok(ConfigurationOptions::default()),
        }
    }
}

impl ResolvedConfigurationOptions for RawTurboJSON {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error> {
        match &self.remote_cache_options {
            Some(configuration_options) => {
                configuration_options.clone().get_configuration_options()
            }
            None => Ok(ConfigurationOptions::default()),
        }
    }
}

// Used for global config and local config.
impl ResolvedConfigurationOptions for ConfigurationOptions {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error> {
        Ok(self)
    }
}

fn get_lowercased_env_vars() -> HashMap<String, String> {
    std::env::vars()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

fn get_env_var_config(environment: HashMap<String, String>) -> Result<ConfigurationOptions, Error> {
    let mut vercel_artifacts_mapping = HashMap::new();
    vercel_artifacts_mapping.insert(String::from("vercel_artifacts_token"), "token");
    vercel_artifacts_mapping.insert(String::from("vercel_artifacts_owner"), "team_id");

    let mut turbo_mapping = HashMap::new();
    turbo_mapping.insert(String::from("turbo_api"), "api_url");
    turbo_mapping.insert(String::from("turbo_login"), "login_url");
    turbo_mapping.insert(String::from("turbo_team"), "team_slug");
    turbo_mapping.insert(String::from("turbo_teamid"), "team_id");
    turbo_mapping.insert(String::from("turbo_token"), "token");
    turbo_mapping.insert(String::from("turbo_signature"), "signature"); // new
    turbo_mapping.insert(String::from("turbo_preflight"), "preflight"); // new
    turbo_mapping.insert(String::from("turbo_remote_cache_timeout"), "timeout");

    let mut output_map = HashMap::new();

    // Process the VERCEL_ARTIFACTS_* first.
    vercel_artifacts_mapping.iter().for_each(|(k, mapped)| {
        let k = k.to_string();
        if let Some(value) = environment.get(&k) {
            if !value.is_empty() {
                output_map.insert(mapped.to_string(), value.to_string());
            }
        }
    });

    // Process the TURBO_* next.
    turbo_mapping.iter().for_each(|(k, mapped)| {
        if let Some(value) = environment.get(k) {
            if !value.is_empty() {
                output_map.insert(mapped.to_string(), value.to_string());
            }
        }
    });

    // Process signature
    let signature = if let Some(signature) = output_map.get("signature").cloned() {
        match signature.as_str() {
            "0" => Some(false),
            "1" => Some(true),
            _ => return Err(anyhow!("parse_signature")),
        }
    } else {
        None
    };

    // Process preflight
    let preflight = if let Some(preflight) = output_map.get("preflight").cloned() {
        match preflight.as_str() {
            "0" => Some(false),
            "1" => Some(true),
            _ => return Err(anyhow!("parse_preflight")),
        }
    } else {
        None
    };

    // Process timeout
    let timeout = if let Some(timeout) = output_map.get("timeout").cloned() {
        Some(timeout.parse::<u64>().map_err(|e| {
            dbg!(e);
            anyhow!("parse_timeout")
        })?)
    } else {
        None
    };

    let output = ConfigurationOptions {
        api_url: output_map.get("api_url").cloned(),
        login_url: output_map.get("login_url").cloned(),
        team_slug: output_map.get("team_slug").cloned(),
        team_id: output_map.get("team_id").cloned(),
        token: output_map.get("token").cloned(),

        // Processed booleans
        signature,
        preflight,

        // Processed numbers
        timeout,
    };

    Ok(output)
}

impl TurborepoConfigBuilder {
    pub fn new(base: &CommandBase) -> Self {
        Self {
            repo_root: base.repo_root.to_owned(),
            override_config: Default::default(),
            global_config_path: base.global_config_path.clone(),
        }
    }

    // Getting all of the paths.
    fn global_config_path(&self) -> Result<AbsoluteSystemPathBuf, Error> {
        if let Some(global_config_path) = self.global_config_path.clone() {
            return Ok(global_config_path);
        }
        Ok(AbsoluteSystemPathBuf::try_from(
            config_dir()
                .map(|p| p.join("turborepo").join("config.json"))
                .ok_or(anyhow!("No global config path"))?,
        )?)
    }
    fn local_config_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_components(&[".turbo", "config.json"])
    }
    fn root_package_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("package.json")
    }
    fn root_turbo_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("turbo.json")
    }

    fn get_global_config(&self) -> Result<ConfigurationOptions, Error> {
        let global_config_path = self.global_config_path().map_err(|e| {
            dbg!(e);
            anyhow!("global_path")
        })?;
        let contents = std::fs::read_to_string(global_config_path).map_err(|e| {
            dbg!(e);
            anyhow!("global_read")
        })?;
        let global_config: ConfigurationOptions = serde_json::from_str(&contents).map_err(|e| {
            dbg!(e);
            anyhow!("global_de")
        })?;
        Ok(global_config)
    }

    fn get_local_config(&self) -> Result<ConfigurationOptions, Error> {
        let local_config_path = self.local_config_path();
        let contents = local_config_path.read_to_string().map_err(|e| {
            dbg!(e);
            anyhow!("local_read")
        })?;
        let local_config: ConfigurationOptions = serde_json::from_str(&contents).map_err(|e| {
            dbg!(e);
            anyhow!("local_de")
        })?;
        Ok(local_config)
    }

    create_builder!(with_api_url, api_url, Option<String>);
    create_builder!(with_login_url, login_url, Option<String>);
    create_builder!(with_team_slug, team_slug, Option<String>);
    create_builder!(with_team_id, team_id, Option<String>);
    create_builder!(with_token, token, Option<String>);
    create_builder!(with_signature, signature, Option<bool>);
    create_builder!(with_preflight, preflight, Option<bool>);
    create_builder!(with_timeout, timeout, Option<u64>);

    pub fn build(&self) -> Result<ConfigurationOptions, Error> {
        // Priority, from least significant to most significant:
        // - shared configuration (package.json .turbo)
        // - shared configuration (turbo.json)
        // - global configuration (~/.turbo/config.json)
        // - local configuration (<REPO_ROOT>/.turbo/config.json)
        // - environment variables
        // - CLI arguments (deprecated, and to be removed)
        // - builder pattern overrides.

        let root_package_json = PackageJson::load(&self.repo_root.join_component("package.json"))
            .map_err(|e| {
            dbg!(e);
            anyhow!("package_json")
        })?;
        let turbo_json =
            RawTurboJSON::read(&self.repo_root.join_component("turbo.json")).map_err(|e| {
                dbg!(e);
                anyhow!("raw_turbo_json")
            })?;
        let global_config = self.get_global_config()?;
        let local_config = self.get_local_config()?;
        let env_var_config = get_env_var_config(get_lowercased_env_vars())?;

        let sources = [
            root_package_json.get_configuration_options(),
            turbo_json.get_configuration_options(),
            global_config.get_configuration_options(),
            local_config.get_configuration_options(),
            env_var_config.get_configuration_options(),
            Ok(self.override_config.clone()),
        ];

        let output = sources.iter().fold(
            ConfigurationOptions::default(),
            |mut acc, current_source| -> ConfigurationOptions {
                match current_source {
                    Ok(current_source_config) => {
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
                        if let Some(preflight) = current_source_config.preflight {
                            acc.preflight = Some(preflight);
                        }
                        if let Some(timeout) = current_source_config.timeout {
                            acc.timeout = Some(timeout);
                        }
                    }
                    Err(_) => todo!(),
                }

                acc
            },
        );

        Ok(output)
    }
}
