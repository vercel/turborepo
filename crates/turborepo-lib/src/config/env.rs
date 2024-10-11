use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
};

use clap::ValueEnum;
use itertools::Itertools;
use turbopath::AbsoluteSystemPathBuf;

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
        _existing_config: Option<&ConfigurationOptions>,
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

        // Process enabled
        let enabled = self
            .truthy_value("enabled")
            .map(|value| value.ok_or_else(|| Error::InvalidRemoteCacheEnabled))
            .transpose()?;

        let force = self.truthy_value("force").flatten();
        let remote_only = self.truthy_value("remote_only").flatten();
        let remote_cache_read_only = self.truthy_value("remote_cache_read_only").flatten();
        let run_summary = self.truthy_value("run_summary").flatten();
        let allow_no_turbo_json = self.truthy_value("allow_no_turbo_json").flatten();

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
            // Processed booleans
            signature,
            preflight,
            enabled,
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

const VERCEL_ARTIFACTS_MAPPING: &[(&str, &str)] = [
    // corresponds to env var TURBO_TOKEN
    ("vercel_artifacts_token", "token"),
    // corresponds to env var TURBO_TEAMID
    ("vercel_artifacts_owner", "team_id"),
    // corresponds to env var TURBO_TEAM
    ("vercel_artifacts_owner", "team_slug"),
]
.as_slice();

pub struct OverrideEnvVars<'a> {
    environment: &'a HashMap<OsString, OsString>,
    output_map: HashMap<&'static str, String>,
}

impl<'a> OverrideEnvVars<'a> {
    pub fn new(environment: &'a HashMap<OsString, OsString>) -> Result<Self, Error> {
        let vercel_artifacts_mapping: HashMap<_, _> = VERCEL_ARTIFACTS_MAPPING
            .iter()
            .filter(|(env_var, _value)| {
                if env_var == &"vercel_artifacts_token" {
                    let has_override = environment.contains_key(OsStr::new("turbo_token"));
                    return !has_override;
                }
                // if env_var == &"vercel_artifacts_owner" {
                //     let contains_team_id =
                // environment.contains_key(OsStr::new("turbo_teamid"));     let
                // contains_team = environment.contains_key(OsStr::new("turbo_team"));
                //     return contains_team_id && contains_team;
                // }
                true
            })
            .copied()
            .collect();

        dbg!(&vercel_artifacts_mapping);
        dbg!(&environment);

        let output_map = map_environment(vercel_artifacts_mapping, environment)?;

        dbg!(&output_map);

        Ok(Self {
            environment,
            output_map,
        })
    }

    fn ui(&self) -> Option<UIMode> {
        let value = self
            .environment
            .get(OsStr::new("ci"))
            .or_else(|| self.environment.get(OsStr::new("no_color")))?;
        match truth_env_var(value.to_str()?)? {
            true => Some(UIMode::Stream),
            false => None,
        }
    }
}

impl<'a> ResolvedConfigurationOptions for OverrideEnvVars<'a> {
    fn get_configuration_options(
        &self,
        _existing_config: Option<&ConfigurationOptions>,
    ) -> Result<ConfigurationOptions, Error> {
        let ui = self.ui();
        // let team_id = if existing_config.team_id.is_none() &&
        // existing_config.team_slug.is_none() {     self.output_map.get("
        // team_id").cloned() } else {
        //     None
        // };
        let output = ConfigurationOptions {
            team_id: self.output_map.get("team_id").cloned(),
            token: self.output_map.get("token").cloned(),
            api_url: None,
            ui,
            ..Default::default()
        };
        dbg!(&output);

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

    dbg!(&output_map);

    Ok(output_map)
}

#[cfg(test)]
mod test {
    use camino::Utf8PathBuf;
    use lazy_static::lazy_static;

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

        let config = EnvVars::new(&env)
            .unwrap()
            .get_configuration_options(None)
            .unwrap();
        assert!(config.preflight());
        assert!(config.force());
        assert_eq!(config.log_order(), LogOrder::Grouped);
        assert!(config.remote_only());
        assert!(config.remote_cache_read_only());
        assert!(config.run_summary());
        assert!(config.allow_no_turbo_json());
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
            .get_configuration_options(None)
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

    #[test]
    fn test_override_env_setting() {
        let mut env: HashMap<OsString, OsString> = HashMap::new();

        let vercel_artifacts_token = "correct-horse-battery-staple";
        env.insert(
            "vercel_artifacts_token".into(),
            vercel_artifacts_token.into(),
        );

        let vercel_artifacts_owner = "bobby_tables";
        env.insert(
            "vercel_artifacts_owner".into(),
            vercel_artifacts_owner.into(),
        );

        env.insert("ci".into(), "1".into());

        let config = OverrideEnvVars::new(&env)
            .unwrap()
            .get_configuration_options(None)
            .unwrap();
        assert_eq!(vercel_artifacts_token, config.token.unwrap());
        assert_eq!(vercel_artifacts_owner, config.team_id.unwrap());
        assert_eq!(Some(UIMode::Stream), config.ui);
    }

    // TODO: this should only happen on CI (use vendors crate to detect CI)

    #[test]
    fn test_vercel_artifacts_token_override() {
        let mut env: HashMap<OsString, OsString> = HashMap::new();

        let vercel_artifacts_token = "should ignore";
        env.insert(
            "vercel_artifacts_token".into(),
            vercel_artifacts_token.into(),
        );

        let turbo_token = "should keep";
        env.insert("turbo_token".into(), turbo_token.into());

        let config = OverrideEnvVars::new(&env)
            .unwrap()
            .get_configuration_options(None)
            .unwrap();

        assert_eq!(None, config.token);
    }

    // #[test]
    // fn test_vercel_artifacts_owner_override() {
    //     let mut env: HashMap<OsString, OsString> = HashMap::new();

    //     let vercel_artifacts_owner = "should ignore";
    //     env.insert(
    //         "vercel_artifacts_owner".into(),
    //         vercel_artifacts_owner.into(),
    //     );

    //     let turbo_team = "corresponds to TURBO_TEAM";
    //     env.insert("turbo_team".into(), turbo_team.into());

    //     let turbo_teamid = "corresponds to TURBO_TEAMID";
    //     env.insert("turbo_teamid".into(), turbo_team.into());

    //     let config = OverrideEnvVars::new(&env)
    //         .unwrap()
    //         .get_configuration_options(None)
    //         .unwrap();

    //     assert_eq!(turbo_team, config.team_slug.unwrap());
    //     assert_eq!(turbo_teamid, config.team_id.unwrap());
    // }

    lazy_static! {
        static ref VERCEL_ARTIFACTS_OWNER: String = String::from("valueof:VERCEL_ARTIFACTS_OWNER");
        static ref VERCEL_ARTIFACTS_TOKEN: String = String::from("valueof:VERCEL_ARTIFACTS_TOKEN");
        static ref TURBO_TEAMID: String = String::from("valueof:TURBO_TEAMID");
        static ref TURBO_TEAM: String = String::from("valueof:TURBO_TEAM");
        static ref TURBO_TOKEN: String = String::from("valueof:TURBO_TOKEN");
    }

    #[allow(non_snake_case)]
    struct TestCaseEnv {
        TURBO_TEAM: Option<String>,
        TURBO_TEAMID: Option<String>,
        TURBO_TOKEN: Option<String>,
        VERCEL_ARTIFACTS_OWNER: Option<String>,
        VERCEL_ARTIFACTS_TOKEN: Option<String>,
    }

    impl TestCaseEnv {
        fn new() -> Self {
            Self {
                TURBO_TEAM: None,
                TURBO_TEAMID: None,
                TURBO_TOKEN: None,
                VERCEL_ARTIFACTS_OWNER: None,
                VERCEL_ARTIFACTS_TOKEN: None,
            }
        }
    }

    struct TestCase {
        env: TestCaseEnv,
        expected: ConfigurationOptions,
        reason: &'static str,
    }

    impl TestCase {
        fn new() -> Self {
            Self {
                env: TestCaseEnv::new(),
                expected: Default::default(),
                reason: "missing",
            }
        }

        fn reason(mut self, reason: &'static str) -> Self {
            self.reason = reason;
            self
        }

        #[allow(non_snake_case)]
        fn VERCEL_ARTIFACTS_OWNER(mut self) -> Self {
            self.env.VERCEL_ARTIFACTS_OWNER = Some(VERCEL_ARTIFACTS_OWNER.clone());
            self
        }

        #[allow(non_snake_case)]
        fn VERCEL_ARTIFACTS_TOKEN(mut self) -> Self {
            self.env.VERCEL_ARTIFACTS_TOKEN = Some(VERCEL_ARTIFACTS_TOKEN.clone());
            self
        }

        #[allow(non_snake_case)]
        fn TURBO_TEAMID(mut self) -> Self {
            self.env.TURBO_TEAMID = Some(TURBO_TEAMID.clone());
            self
        }

        #[allow(non_snake_case)]
        fn TURBO_TEAM(mut self) -> Self {
            self.env.TURBO_TEAM = Some(TURBO_TEAM.clone());
            self
        }

        #[allow(non_snake_case)]
        fn TURBO_TOKEN(mut self) -> Self {
            self.env.TURBO_TOKEN = Some(TURBO_TOKEN.clone());
            self
        }

        fn team_id(mut self, value: String) -> Self {
            self.expected.team_id = Some(value);
            self
        }

        fn team_slug(mut self, value: String) -> Self {
            self.expected.team_slug = Some(value);
            self
        }

        fn token(mut self, value: String) -> Self {
            self.expected.token = Some(value);
            self
        }
    }

    #[test]
    fn test_all_the_combos() {
        let cases: &[TestCase] = &[
            //
            // Get nothing back
            // ------------------------------
            TestCase::new().reason("no env vars set"),
            TestCase::new()
                .reason("just VERCEL_ARTIFACTS_TOKEN")
                .VERCEL_ARTIFACTS_TOKEN(),
            TestCase::new().reason("just TURBO_TOKEN").TURBO_TOKEN(),
            TestCase::new()
                .reason("we don't mix-and-match, TURBO_TEAM, VERCEL_ARTIFACTS_TOKEN")
                .TURBO_TEAM()
                .VERCEL_ARTIFACTS_TOKEN(),
            //
            // Just get a team_id
            // ------------------------------
            TestCase::new()
                .reason("just VERCEL_ARTIFACTS_OWNER")
                .VERCEL_ARTIFACTS_OWNER()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone()),
            TestCase::new()
                .reason("just TURBO_TEAMID")
                .TURBO_TEAMID()
                .team_id(TURBO_TEAMID.clone()),
            TestCase::new()
                .reason("if it's just between VERCEL_ARTIFACTS_OWNER and TURBO_TEAMID, Vercel wins")
                .TURBO_TEAMID()
                .VERCEL_ARTIFACTS_OWNER()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone()),
            TestCase::new()
                .reason(
                    "if we have a TURBO_TEAMID and a TURBO_TEAM but also a \
                     VERCEL_ARTIFACTS_OWNER, we disregard the token the user set because we were \
                     expecting a VERCEL_ARTIFACTS_TOKEN as well",
                )
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone()),
            //
            // Just get a team_slug
            // ------------------------------
            TestCase::new()
                .reason("just TURBO_TEAM")
                .TURBO_TEAM()
                .team_slug(TURBO_TEAM.clone()),
            //
            // just team_slug and team_id
            // ------------------------------
            TestCase::new()
                .reason("if we just have VERCEL_ARTIFACTS_OWNER and TURBO_TEAM, vercel wins")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .team_slug(TURBO_TEAM.clone())
                .team_id(TURBO_TEAMID.clone()),
            TestCase::new()
                .reason("if we just have VERCEL_ARTIFACTS_OWNER and TURBO_TEAM, vercel wins")
                .TURBO_TEAM()
                .VERCEL_ARTIFACTS_OWNER()
                .team_slug(VERCEL_ARTIFACTS_OWNER.clone())
                .team_id(VERCEL_ARTIFACTS_OWNER.clone()),
            //
            // When 3rd Party Wins with team_slug
            // ------------------------------
            TestCase::new()
                .reason("golden path for 3rd party, not deployed on Vercel")
                .TURBO_TEAM()
                .TURBO_TOKEN()
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason(
                    "a TURBO_TEAM+TURBO_TOKEN pair wins against an incomplete Vercel (just \
                     artifacts token)",
                )
                .TURBO_TEAM()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_TOKEN() // disregarded
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("golden path for 3rd party, deployed on Vercel")
                .TURBO_TEAM()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER() // normally this would map to team_id, but not with a complete 3rd party pair
                .VERCEL_ARTIFACTS_TOKEN()
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            //
            // When 3rd Party Wins with team_id
            // ------------------------------
            TestCase::new()
                .reason("if they pass a TURBO_TEAMID and a TURBO_TOKEN, we use them")
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("a TURBO_TEAMID+TURBO_TOKEN pair will also win against a Vercel pair")
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason(
                    "a TURBO_TEAMID+TURBO_TOKEN pair wins against an incomplete Vercel (just \
                     artifacts token)",
                )
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .token(TURBO_TOKEN.clone()),
            //
            // When 3rd Party Wins with all three
            // ------------------------------
            TestCase::new()
                .reason("we can use all of TURBO_TEAM, TURBO_TEAMID, and TURBO_TOKEN")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("if we have a 3rd party trifecta, that wins, even against a Vercel Pair")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("a 3rd party trifecta wins against a partial Vercel (just artifacts token)")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            TestCase::new()
                .reason("a 3rd party trifecta wins against a partial Vercel (just artifacts owner)")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone())
                .token(TURBO_TOKEN.clone()),
            //
            // just set team_id and team_slug
            // ------------------------------
            TestCase::new()
                .reason("if we have a TURBO_TEAM and TURBO_TEAMID we can use them both")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .team_id(TURBO_TEAMID.clone())
                .team_slug(TURBO_TEAM.clone()),
            //
            // When Vercel Wins
            // ------------------------------
            TestCase::new()
                .reason("golden path on Vercel zero config")
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            TestCase::new()
                .reason("Vercel wins: disregard just TURBO_TOKEN")
                .TURBO_TOKEN()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            TestCase::new()
                .reason("Vercel wins: disregard just TURBO_TEAM")
                .TURBO_TEAM()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            TestCase::new()
                .reason("Vercel wins: disregard just TURBO_TEAMID")
                .TURBO_TEAMID()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
            TestCase::new()
                .reason("Vercel wins if TURBO_TOKEN is missing")
                .TURBO_TEAM()
                .TURBO_TEAMID()
                .VERCEL_ARTIFACTS_OWNER()
                .VERCEL_ARTIFACTS_TOKEN()
                .team_id(VERCEL_ARTIFACTS_OWNER.clone())
                .token(VERCEL_ARTIFACTS_TOKEN.clone()),
        ];

        for case in cases {
            let mut env: HashMap<OsString, OsString> = HashMap::new();

            if let Some(value) = &case.env.TURBO_TEAM {
                env.insert("turbo_team".into(), value.into());
            }
            if let Some(value) = &case.env.TURBO_TEAMID {
                env.insert("turbo_teamid".into(), value.into());
            }
            if let Some(value) = &case.env.TURBO_TOKEN {
                env.insert("turbo_token".into(), value.into());
            }
            if let Some(value) = &case.env.VERCEL_ARTIFACTS_OWNER {
                env.insert("vercel_artifacts_owner".into(), value.into());
            }
            if let Some(value) = &case.env.VERCEL_ARTIFACTS_TOKEN {
                env.insert("vercel_artifacts_token".into(), value.into());
            }

            let config = OverrideEnvVars::new(&env)
                .unwrap()
                .get_configuration_options(None)
                .unwrap();

            assert_eq!(case.expected.team_id, config.team_id);
            assert_eq!(case.expected.team_slug, config.team_slug);
            assert_eq!(case.expected.token, config.token);
        }
    }
}
