use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
};

use super::{ConfigurationOptions, Error, ResolvedConfigurationOptions};
use crate::{cli::EnvMode, turbo_json::UIMode};

pub struct EnvVars {
    output_map: HashMap<&'static str, String>,
}

impl EnvVars {
    pub fn new(environment: &HashMap<OsString, OsString>) -> Result<Self, Error> {
        let mut turbo_mapping = HashMap::new();
        turbo_mapping.insert(OsString::from("turbo_api"), "api_url");
        turbo_mapping.insert(OsString::from("turbo_login"), "login_url");
        turbo_mapping.insert(OsString::from("turbo_team"), "team_slug");
        turbo_mapping.insert(OsString::from("turbo_teamid"), "team_id");
        turbo_mapping.insert(OsString::from("turbo_token"), "token");
        turbo_mapping.insert(OsString::from("turbo_remote_cache_timeout"), "timeout");
        turbo_mapping.insert(
            OsString::from("turbo_remote_cache_upload_timeout"),
            "upload_timeout",
        );
        turbo_mapping.insert(OsString::from("turbo_ui"), "ui");
        turbo_mapping.insert(
            OsString::from("turbo_dangerously_disable_package_manager_check"),
            "allow_no_package_manager",
        );
        turbo_mapping.insert(OsString::from("turbo_daemon"), "daemon");
        turbo_mapping.insert(OsString::from("turbo_env_mode"), "env_mode");
        turbo_mapping.insert(OsString::from("turbo_cache_dir"), "cache_dir");
        turbo_mapping.insert(OsString::from("turbo_preflight"), "preflight");
        turbo_mapping.insert(OsString::from("turbo_scm_base"), "scm_base");
        turbo_mapping.insert(OsString::from("turbo_scm_head"), "scm_head");

        // We do not enable new config sources:
        // turbo_mapping.insert(String::from("turbo_signature"), "signature"); // new
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
        Ok(Self { output_map })
    }
}

impl ResolvedConfigurationOptions for EnvVars {
    fn get_configuration_options(&self) -> Result<ConfigurationOptions, Error> {
        // Process signature
        let signature = if let Some(signature) = self.output_map.get("signature") {
            match signature.as_str() {
                "0" => Some(false),
                "1" => Some(true),
                _ => return Err(Error::InvalidSignature),
            }
        } else {
            None
        };

        // Process preflight
        let preflight = if let Some(preflight) = self.output_map.get("preflight") {
            match preflight.as_str() {
                "0" | "false" => Some(false),
                "1" | "true" => Some(true),
                "" => None,
                _ => return Err(Error::InvalidPreflight),
            }
        } else {
            None
        };

        // Process enabled
        let enabled = if let Some(enabled) = self.output_map.get("enabled") {
            match enabled.as_str() {
                "0" => Some(false),
                "1" => Some(true),
                _ => return Err(Error::InvalidRemoteCacheEnabled),
            }
        } else {
            None
        };

        // Process timeout
        let timeout = if let Some(timeout) = self.output_map.get("timeout") {
            Some(
                timeout
                    .parse::<u64>()
                    .map_err(Error::InvalidRemoteCacheTimeout)?,
            )
        } else {
            None
        };

        let upload_timeout = if let Some(upload_timeout) = self.output_map.get("upload_timeout") {
            Some(
                upload_timeout
                    .parse::<u64>()
                    .map_err(Error::InvalidUploadTimeout)?,
            )
        } else {
            None
        };

        // Process experimentalUI
        let ui = self
            .output_map
            .get("ui")
            .map(|s| s.as_str())
            .and_then(truth_env_var)
            .map(|ui| if ui { UIMode::Tui } else { UIMode::Stream });

        let allow_no_package_manager = self
            .output_map
            .get("allow_no_package_manager")
            .map(|s| s.as_str())
            .and_then(truth_env_var);

        // Process daemon
        let daemon = self
            .output_map
            .get("daemon")
            .and_then(|val| match val.as_str() {
                "1" | "true" => Some(true),
                "0" | "false" => Some(false),
                _ => None,
            });

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
            // Processed booleans
            signature,
            preflight,
            enabled,
            ui,
            allow_no_package_manager,
            daemon,

            // Processed numbers
            timeout,
            upload_timeout,
            spaces_id,
            env_mode,
            cache_dir,
        };

        Ok(output)
    }
}

pub struct OverrideEnvVars<'a> {
    environment: &'a HashMap<OsString, OsString>,
    output_map: HashMap<&'static str, String>,
}

impl<'a> OverrideEnvVars<'a> {
    pub fn new(environment: &'a HashMap<OsString, OsString>) -> Result<Self, Error> {
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
        Ok(Self {
            environment,
            output_map,
        })
    }
}

impl<'a> ResolvedConfigurationOptions for OverrideEnvVars<'a> {
    fn get_configuration_options(&self) -> Result<ConfigurationOptions, Error> {
        let ui = self
            .environment
            .get(OsStr::new("ci"))
            .or_else(|| self.environment.get(OsStr::new("no_color")))
            .and_then(|value| {
                // If either of these are truthy, then we disable the TUI
                if value == "true" || value == "1" {
                    Some(UIMode::Stream)
                } else {
                    None
                }
            });

        let output = ConfigurationOptions {
            api_url: None,
            login_url: None,
            team_slug: None,
            team_id: self.output_map.get("team_id").cloned(),
            token: self.output_map.get("token").cloned(),
            scm_base: None,
            scm_head: None,

            signature: None,
            preflight: None,
            enabled: None,
            ui,
            daemon: None,
            timeout: None,
            upload_timeout: None,
            spaces_id: None,
            allow_no_package_manager: None,
            env_mode: None,
            cache_dir: None,
        };

        Ok(output)
    }
}

fn truth_env_var(s: &str) -> Option<bool> {
    match s {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use camino::Utf8PathBuf;

    use super::*;
    use crate::config::{DEFAULT_API_URL, DEFAULT_LOGIN_URL};

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

        let config = EnvVars::new(&env)
            .unwrap()
            .get_configuration_options()
            .unwrap();
        assert!(config.preflight());
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

        let config = EnvVars::new(&env)
            .unwrap()
            .get_configuration_options()
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
        assert_eq!(config.scm_head(), "HEAD");
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
        env.insert("ci".into(), "1".into());

        let config = OverrideEnvVars::new(&env)
            .unwrap()
            .get_configuration_options()
            .unwrap();
        assert_eq!(vercel_artifacts_token, config.token.unwrap());
        assert_eq!(vercel_artifacts_owner, config.team_id.unwrap());
        assert_eq!(Some(UIMode::Stream), config.ui);
    }
}
