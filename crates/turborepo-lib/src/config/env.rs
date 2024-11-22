use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
};

use clap::ValueEnum;
use itertools::Itertools;
use tracing::warn;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_cache::CacheConfig;

use super::{ConfigurationOptions, Error, ResolvedConfigurationOptions};
use crate::{
    cli::{EnvMode, LogOrder},
    turbo_json::UIMode,
};

const TURBO_MAPPING: &[(&str, &str)] = [
    ("turbo_api", "api_url"),
    ("turbo_login", "login_url"),
    ("turbo_team", "team_slug"),
    ("turbo_teamid", "team_id"),
    ("turbo_token", "token"),
    ("turbo_remote_cache_timeout", "timeout"),
    ("turbo_remote_cache_upload_timeout", "upload_timeout"),
    ("turbo_ui", "ui"),
    (
        "turbo_dangerously_disable_package_manager_check",
        "allow_no_package_manager",
    ),
    ("turbo_daemon", "daemon"),
    ("turbo_env_mode", "env_mode"),
    ("turbo_cache_dir", "cache_dir"),
    ("turbo_preflight", "preflight"),
    ("turbo_scm_base", "scm_base"),
    ("turbo_scm_head", "scm_head"),
    ("turbo_root_turbo_json", "root_turbo_json_path"),
    ("turbo_force", "force"),
    ("turbo_log_order", "log_order"),
    ("turbo_remote_only", "remote_only"),
    ("turbo_remote_cache_read_only", "remote_cache_read_only"),
    ("turbo_run_summary", "run_summary"),
    ("turbo_allow_no_turbo_json", "allow_no_turbo_json"),
    ("turbo_cache", "cache"),
]
.as_slice();

pub struct EnvVars {
    output_map: HashMap<&'static str, String>,
}

impl EnvVars {
    pub fn new(environment: &HashMap<OsString, OsString>) -> Result<Self, Error> {
        let turbo_mapping: HashMap<_, _> = TURBO_MAPPING.iter().copied().collect();
        let output_map = map_environment(turbo_mapping, environment)?;
        Ok(Self { output_map })
    }

    fn truthy_value(&self, key: &str) -> Option<Option<bool>> {
        Some(truth_env_var(
            self.output_map.get(key).filter(|s| !s.is_empty())?,
        ))
    }
}

impl ResolvedConfigurationOptions for EnvVars {
    fn get_configuration_options(
        &self,
        _existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        // Process signature
        let signature = self
            .truthy_value("signature")
            .map(|value| value.ok_or_else(|| Error::InvalidSignature))
            .transpose()?;

        // Process preflight
        let preflight = self
            .truthy_value("preflight")
            .map(|value| value.ok_or_else(|| Error::InvalidPreflight))
            .transpose()?;

        let force = self.truthy_value("force").flatten();
        let mut remote_only = self.truthy_value("remote_only").flatten();

        let mut remote_cache_read_only = self.truthy_value("remote_cache_read_only").flatten();

        let run_summary = self.truthy_value("run_summary").flatten();
        let allow_no_turbo_json = self.truthy_value("allow_no_turbo_json").flatten();
        let mut cache: Option<turborepo_cache::CacheConfig> = self
            .output_map
            .get("cache")
            .map(|c| c.parse())
            .transpose()?;

        // If TURBO_FORCE is set it wins out over TURBO_CACHE
        if force.is_some_and(|t| t) {
            cache = None;
        }

        if remote_only.is_some_and(|t| t) {
            if let Some(cache) = cache {
                // If TURBO_REMOTE_ONLY and TURBO_CACHE result in the same behavior, remove
                // REMOTE_ONLY to avoid deprecation warning or mixing of old/new
                // cache flag error.
                if cache == CacheConfig::remote_only() {
                    remote_only = None;
                }
            }
        }
        if remote_cache_read_only.is_some_and(|t| t) {
            if let Some(cache) = cache {
                // If TURBO_REMOTE_CACHE_READ_ONLY and TURBO_CACHE result in the same behavior,
                // remove REMOTE_CACHE_READ_ONLY to avoid deprecation warning or
                // mixing of old/new cache flag error.
                if cache == CacheConfig::remote_read_only() {
                    remote_cache_read_only = None;
                }
            }
        }

        if remote_only.is_some() {
            warn!(
                "TURBO_REMOTE_ONLY is deprecated and will be removed in a future major version. \
                 Use TURBO_CACHE=remote:rw"
            );
        }

        if remote_cache_read_only.is_some() {
            warn!(
                "TURBO_REMOTE_CACHE_READ_ONLY is deprecated and will be removed in a future major \
                 version. Use TURBO_CACHE=remote:r"
            );
        }

        // Process timeout
        let timeout = self
            .output_map
            .get("timeout")
            .map(|s| s.parse())
            .transpose()
            .map_err(Error::InvalidRemoteCacheTimeout)?;

        let upload_timeout = self
            .output_map
            .get("upload_timeout")
            .map(|s| s.parse())
            .transpose()
            .map_err(Error::InvalidUploadTimeout)?;

        // Process experimentalUI
        let ui =
            self.truthy_value("ui")
                .flatten()
                .map(|ui| if ui { UIMode::Tui } else { UIMode::Stream });

        let allow_no_package_manager = self.truthy_value("allow_no_package_manager").flatten();

        // Process daemon
        let daemon = self.truthy_value("daemon").flatten();

        let env_mode = self
            .output_map
            .get("env_mode")
            .map(|s| s.as_str())
            .and_then(|s| match s {
                "strict" => Some(EnvMode::Strict),
                "loose" => Some(EnvMode::Loose),
                _ => None,
            });

        let cache_dir = self.output_map.get("cache_dir").map(|s| s.clone().into());

        let root_turbo_json_path = self
            .output_map
            .get("root_turbo_json_path")
            .filter(|s| !s.is_empty())
            .map(AbsoluteSystemPathBuf::from_cwd)
            .transpose()?;

        let log_order = self
            .output_map
            .get("log_order")
            .filter(|s| !s.is_empty())
            .map(|s| LogOrder::from_str(s, true))
            .transpose()
            .map_err(|_| {
                Error::InvalidLogOrder(
                    LogOrder::value_variants()
                        .iter()
                        .map(|v| v.to_string())
                        .join(", "),
                )
            })?;

        // We currently don't pick up a Spaces ID via env var, we likely won't
        // continue using the Spaces name, we can add an env var when we have the
        // name we want to stick with.
        let spaces_id = None;

        let output = ConfigurationOptions {
            api_url: self.output_map.get("api_url").cloned(),
            login_url: self.output_map.get("login_url").cloned(),
            team_slug: self.output_map.get("team_slug").cloned(),
            team_id: self.output_map.get("team_id").cloned(),
            token: self.output_map.get("token").cloned(),
            scm_base: self.output_map.get("scm_base").cloned(),
            scm_head: self.output_map.get("scm_head").cloned(),
            cache,
            // Processed booleans
            signature,
            preflight,
            enabled: None,
            ui,
            allow_no_package_manager,
            daemon,
            force,
            remote_only,
            remote_cache_read_only,
            run_summary,
            allow_no_turbo_json,

            // Processed numbers
            timeout,
            upload_timeout,
            spaces_id,
            env_mode,
            cache_dir,
            root_turbo_json_path,
            log_order,
        };

        Ok(output)
    }
}

pub fn truth_env_var(s: &str) -> Option<bool> {
    match s {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn map_environment<'a>(
    // keys are environment variable names
    // values are properties of ConfigurationOptions we want to store the
    // values in
    mapping: HashMap<&str, &'a str>,

    // keys are environment variable names
    // values are the values of those environment variables
    environment: &HashMap<OsString, OsString>,
) -> Result<HashMap<&'a str, String>, Error> {
    let mut output_map = HashMap::new();
    mapping
        .into_iter()
        .try_for_each(|(mapping_key, mapped_property)| -> Result<(), Error> {
            if let Some(value) = environment.get(OsStr::new(mapping_key)) {
                let converted = value
                    .to_str()
                    .ok_or_else(|| Error::Encoding(mapping_key.to_ascii_uppercase()))?;
                output_map.insert(mapped_property, converted.to_owned());
            }
            Ok(())
        })?;
    Ok(output_map)
}

#[cfg(test)]
mod test {
    use camino::Utf8PathBuf;

    use super::*;
    use crate::{
        cli::LogOrder,
        config::{DEFAULT_API_URL, DEFAULT_LOGIN_URL},
    };

    #[test]
    fn test_env_setting() {
        let mut env: HashMap<OsString, OsString> = HashMap::new();

        let turbo_api = "https://example.com/api";
        let turbo_login = "https://example.com/login";
        let turbo_team = "vercel";
        let turbo_teamid = "team_nLlpyC6REAqxydlFKbrMDlud";
        let turbo_token = "abcdef1234567890abcdef";
        let cache_dir = Utf8PathBuf::from("nebulo9");
        let turbo_remote_cache_timeout = 200;
        let root_turbo_json = if cfg!(windows) {
            "C:\\some\\dir\\yolo.json"
        } else {
            "/some/dir/yolo.json"
        };

        env.insert("turbo_api".into(), turbo_api.into());
        env.insert("turbo_login".into(), turbo_login.into());
        env.insert("turbo_team".into(), turbo_team.into());
        env.insert("turbo_teamid".into(), turbo_teamid.into());
        env.insert("turbo_token".into(), turbo_token.into());
        env.insert(
            "turbo_remote_cache_timeout".into(),
            turbo_remote_cache_timeout.to_string().into(),
        );
        env.insert("turbo_ui".into(), "true".into());
        env.insert(
            "turbo_dangerously_disable_package_manager_check".into(),
            "true".into(),
        );
        env.insert("turbo_daemon".into(), "true".into());
        env.insert("turbo_preflight".into(), "true".into());
        env.insert("turbo_env_mode".into(), "strict".into());
        env.insert("turbo_cache_dir".into(), cache_dir.clone().into());
        env.insert("turbo_root_turbo_json".into(), root_turbo_json.into());
        env.insert("turbo_force".into(), "1".into());
        env.insert("turbo_log_order".into(), "grouped".into());
        env.insert("turbo_remote_only".into(), "1".into());
        env.insert("turbo_remote_cache_read_only".into(), "1".into());
        env.insert("turbo_run_summary".into(), "true".into());
        env.insert("turbo_allow_no_turbo_json".into(), "true".into());
        env.insert("turbo_remote_cache_upload_timeout".into(), "200".into());

        let config = EnvVars::new(&env)
            .unwrap()
            .get_configuration_options(&ConfigurationOptions::default())
            .unwrap();
        assert!(config.preflight());
        assert!(config.force());
        assert_eq!(config.log_order(), LogOrder::Grouped);
        assert!(config.remote_only());
        assert!(config.remote_cache_read_only());
        assert!(config.run_summary());
        assert!(config.allow_no_turbo_json());
        assert_eq!(config.upload_timeout(), 200);
        assert_eq!(turbo_api, config.api_url.unwrap());
        assert_eq!(turbo_login, config.login_url.unwrap());
        assert_eq!(turbo_team, config.team_slug.unwrap());
        assert_eq!(turbo_teamid, config.team_id.unwrap());
        assert_eq!(turbo_token, config.token.unwrap());
        assert_eq!(turbo_remote_cache_timeout, config.timeout.unwrap());
        assert_eq!(Some(UIMode::Tui), config.ui);
        assert_eq!(Some(true), config.allow_no_package_manager);
        assert_eq!(Some(true), config.daemon);
        assert_eq!(Some(EnvMode::Strict), config.env_mode);
        assert_eq!(cache_dir, config.cache_dir.unwrap());
        assert_eq!(
            config.root_turbo_json_path,
            Some(AbsoluteSystemPathBuf::new(root_turbo_json).unwrap())
        );
    }

    #[test]
    fn test_empty_env_setting() {
        let mut env: HashMap<OsString, OsString> = HashMap::new();
        env.insert("turbo_api".into(), "".into());
        env.insert("turbo_login".into(), "".into());
        env.insert("turbo_team".into(), "".into());
        env.insert("turbo_teamid".into(), "".into());
        env.insert("turbo_token".into(), "".into());
        env.insert("turbo_ui".into(), "".into());
        env.insert("turbo_daemon".into(), "".into());
        env.insert("turbo_env_mode".into(), "".into());
        env.insert("turbo_preflight".into(), "".into());
        env.insert("turbo_scm_head".into(), "".into());
        env.insert("turbo_scm_base".into(), "".into());
        env.insert("turbo_root_turbo_json".into(), "".into());
        env.insert("turbo_force".into(), "".into());
        env.insert("turbo_log_order".into(), "".into());
        env.insert("turbo_remote_only".into(), "".into());
        env.insert("turbo_remote_cache_read_only".into(), "".into());
        env.insert("turbo_run_summary".into(), "".into());
        env.insert("turbo_allow_no_turbo_json".into(), "".into());

        let config = EnvVars::new(&env)
            .unwrap()
            .get_configuration_options(&ConfigurationOptions::default())
            .unwrap();
        assert_eq!(config.api_url(), DEFAULT_API_URL);
        assert_eq!(config.login_url(), DEFAULT_LOGIN_URL);
        assert_eq!(config.team_slug(), None);
        assert_eq!(config.team_id(), None);
        assert_eq!(config.token(), None);
        assert_eq!(config.ui, None);
        assert_eq!(config.daemon, None);
        assert_eq!(config.env_mode, None);
        assert!(!config.preflight());
        assert_eq!(config.scm_base(), None);
        assert_eq!(config.scm_head(), None);
        assert_eq!(config.root_turbo_json_path, None);
        assert!(!config.force());
        assert_eq!(config.log_order(), LogOrder::Auto);
        assert!(!config.remote_only());
        assert!(!config.remote_cache_read_only());
        assert!(!config.run_summary());
        assert!(!config.allow_no_turbo_json());
    }
}
