use std::collections::HashMap;

use config::Config;
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    config::{default_user_config_path, RawTurboJSON},
    package_json::PackageJson,
};

const DEFAULT_TIMEOUT: u64 = 20;

macro_rules! create_builder {
    ($func_name:ident, $property_name:ident, $type:ty) => {
        pub fn $func_name(mut self, value: $type) -> Self {
            self.override_config.$property_name = value;
            self
        }
    };
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq, Clone)]
pub struct ConfigurationOptions {
    pub(crate) api_url: Option<String>,
    pub(crate) login_url: Option<String>,
    pub(crate) team_slug: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) token: Option<String>,
    pub(crate) signature: Option<bool>,
    pub(crate) preflight: Option<bool>,
    pub(crate) timeout: Option<u64>,
}

#[derive(Default)]
pub struct TurborepoConfigBuilder {
    repo_root: AbsoluteSystemPathBuf,

    // Used for testing.
    override_config: ConfigurationOptions,
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
        self.signature.as_deref().unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn preflight(&self) -> bool {
        self.preflight.as_deref().unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn timeout(&self) -> bool {
        self.timeout.as_deref().unwrap_or(DEFAULT_TIMEOUT)
    }
}

impl config::Source for PackageJson {
    fn clone_into_box(&self) -> Box<dyn config::Source + Send + Sync> {
        todo!()
    }

    fn collect(&self) -> Result<config::Map<String, config::Value>, config::ConfigError> {
        match &self.legacy_turbo_config {
            Some(legacy_turbo_config) => {
                let synthetic_raw_turbo_json: RawTurboJSON =
                    serde_json::from_value(legacy_turbo_config.clone())
                        .map_err(|_| config::ConfigError::Message("()".to_string()))?;
                return synthetic_raw_turbo_json.collect();
            }
            None => Ok(config::Map::new()),
        }
    }
}

impl config::Source for RawTurboJSON {
    fn clone_into_box(&self) -> Box<dyn config::Source + Send + Sync> {
        todo!()
    }

    fn collect(&self) -> Result<config::Map<String, config::Value>, config::ConfigError> {
        match &self.remote_cache_options {
            Some(configuration_options) => configuration_options.collect(),
            None => Ok(config::Map::new()),
        }
    }
}

// Used for global config and local config.
impl config::Source for ConfigurationOptions {
    fn clone_into_box(&self) -> Box<dyn config::Source + Send + Sync> {
        todo!()
    }

    fn collect(&self) -> Result<config::Map<String, config::Value>, config::ConfigError> {
        let mut output = config::Map::new();

        if let Some(api_url) = self.api_url.clone() {
            output.insert(
                String::from("api_url"),
                config::Value::new(None, config::ValueKind::String(api_url)),
            );
        }
        if let Some(login_url) = self.login_url.clone() {
            output.insert(
                String::from("login_url"),
                config::Value::new(None, config::ValueKind::String(login_url)),
            );
        }
        if let Some(team_slug) = self.team_slug.clone() {
            output.insert(
                String::from("team_slug"),
                config::Value::new(None, config::ValueKind::String(team_slug)),
            );
        }
        if let Some(team_id) = self.team_id.clone() {
            output.insert(
                String::from("team_id"),
                config::Value::new(None, config::ValueKind::String(team_id)),
            );
        }
        if let Some(token) = self.token.clone() {
            output.insert(
                String::from("token"),
                config::Value::new(None, config::ValueKind::String(token)),
            );
        }
        if let Some(signature) = self.signature.clone() {
            output.insert(
                String::from("signature"),
                config::Value::new(None, config::ValueKind::Boolean(signature)),
            );
        }
        if let Some(preflight) = self.preflight.clone() {
            output.insert(
                String::from("preflight"),
                config::Value::new(None, config::ValueKind::Boolean(preflight)),
            );
        }
        if let Some(timeout) = self.timeout.clone() {
            output.insert(
                String::from("timeout"),
                config::Value::new(None, config::ValueKind::U64(timeout)),
            );
        }

        Ok(output)
    }
}

fn get_global_config() -> Result<ConfigurationOptions, String> {
    let global_config_path = default_user_config_path().map_err(|e| {
        dbg!(e);
        "global_path"
    })?;
    let contents = std::fs::read_to_string(global_config_path).map_err(|e| {
        dbg!(e);
        "global_read"
    })?;
    let global_config: ConfigurationOptions = serde_json::from_str(&contents).map_err(|e| {
        dbg!(e);
        "global_de"
    })?;
    Ok(global_config)
}

fn get_local_config(repo_root: &AbsoluteSystemPath) -> Result<ConfigurationOptions, String> {
    let local_config_path = repo_root.join_components(&[".turbo", "config.json"]);
    let contents = local_config_path.read_to_string().map_err(|e| {
        dbg!(e);
        "local_read"
    })?;
    let local_config: ConfigurationOptions = serde_json::from_str(&contents).map_err(|e| {
        dbg!(e);
        "local_de"
    })?;
    Ok(local_config)
}

fn get_lowercased_env_vars() -> HashMap<String, String> {
    std::env::vars()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

fn get_env_var_config(
    environment: HashMap<String, String>,
) -> Result<ConfigurationOptions, String> {
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
            _ => return Err(String::from("parse_signature")),
        }
    } else {
        None
    };

    // Process preflight
    let preflight = if let Some(preflight) = output_map.get("preflight").cloned() {
        match preflight.as_str() {
            "0" => Some(false),
            "1" => Some(true),
            _ => return Err(String::from("parse_preflight")),
        }
    } else {
        None
    };

    // Process timeout
    let timeout = if let Some(timeout) = output_map.get("timeout").cloned() {
        Some(timeout.parse::<u64>().map_err(|e| {
            dbg!(e);
            "parse_timeout"
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
    pub fn new(repo_root: &AbsoluteSystemPath) -> Self {
        Self {
            repo_root: repo_root.to_owned(),
            override_config: Default::default(),
        }
    }

    create_builder!(with_api_url, api_url, Option<String>);
    create_builder!(with_login_url, login_url, Option<String>);
    create_builder!(with_team_slug, team_slug, Option<String>);
    create_builder!(with_team_id, team_id, Option<String>);
    create_builder!(with_token, token, Option<String>);
    create_builder!(with_signature, signature, Option<bool>);
    create_builder!(with_preflight, preflight, Option<bool>);
    create_builder!(with_timeout, timeout, Option<u64>);

    pub fn build(&self) -> Result<ConfigurationOptions, String> {
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
            "package_json"
        })?;
        let turbo_json =
            RawTurboJSON::read(&self.repo_root.join_component("turbo.json")).map_err(|e| {
                dbg!(e);
                "raw_turbo_json"
            })?;
        let global_config = get_global_config()?;
        let local_config = get_local_config(&self.repo_root)?;
        let env_var_config = get_env_var_config(get_lowercased_env_vars())?;

        let output: ConfigurationOptions = Config::builder()
            .add_source(root_package_json)
            .add_source(turbo_json)
            .add_source(global_config)
            .add_source(local_config)
            .add_source(env_var_config)
            .set_override_option("api_url", self.override_config.api_url.clone())
            .map_err(|e| {
                dbg!(e);
                "api_url"
            })?
            .set_override_option("login_url", self.override_config.login_url.clone())
            .map_err(|e| {
                dbg!(e);
                "login_url"
            })?
            .set_override_option("team_slug", self.override_config.team_slug.clone())
            .map_err(|e| {
                dbg!(e);
                "team_slug"
            })?
            .set_override_option("team_id", self.override_config.team_id.clone())
            .map_err(|e| {
                dbg!(e);
                "team_id"
            })?
            .set_override_option("token", self.override_config.token.clone())
            .map_err(|e| {
                dbg!(e);
                "token"
            })?
            .set_override_option("signature", self.override_config.signature.clone())
            .map_err(|e| {
                dbg!(e);
                "signature"
            })?
            .set_override_option("preflight", self.override_config.preflight.clone())
            .map_err(|e| {
                dbg!(e);
                "preflight"
            })?
            .set_override_option("timeout", self.override_config.timeout.clone())
            .map_err(|e| {
                dbg!(e);
                "timeout"
            })?
            .build()
            .map_err(|e| {
                dbg!(e);
                "build"
            })?
            .try_deserialize()
            .map_err(|e| {
                dbg!(e);
                "try_deserialize"
            })?;

        Ok(output)
    }
}

#[test]
fn test() {
    let repo_root = AbsoluteSystemPathBuf::new("/Users/nathanhammond/repos/vercel/turbo").unwrap();
    let foo = TurborepoConfigBuilder::new(&repo_root);
    dbg!(foo.build());
}
