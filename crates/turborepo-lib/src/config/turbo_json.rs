use camino::Utf8PathBuf;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};

use super::{ConfigurationOptions, Error, ResolvedConfigurationOptions};
use crate::turbo_json::{RawRemoteCacheOptions, RawRootTurboJson, RawTurboJson};

pub struct TurboJsonReader<'a> {
    repo_root: &'a AbsoluteSystemPath,
}

impl From<&RawRemoteCacheOptions> for ConfigurationOptions {
    fn from(remote_cache_opts: &RawRemoteCacheOptions) -> Self {
        Self {
            api_url: remote_cache_opts
                .api_url
                .as_ref()
                .map(|s| s.as_inner().clone()),
            login_url: remote_cache_opts
                .login_url
                .as_ref()
                .map(|s| s.as_inner().clone()),
            team_slug: remote_cache_opts
                .team_slug
                .as_ref()
                .map(|s| s.as_inner().clone()),
            team_id: remote_cache_opts
                .team_id
                .as_ref()
                .map(|s| s.as_inner().clone()),
            signature: remote_cache_opts.signature.as_ref().map(|s| *s.as_inner()),
            preflight: remote_cache_opts.preflight.as_ref().map(|s| *s.as_inner()),
            timeout: remote_cache_opts.timeout.as_ref().map(|s| *s.as_inner()),
            upload_timeout: remote_cache_opts
                .upload_timeout
                .as_ref()
                .map(|s| *s.as_inner()),
            enabled: remote_cache_opts.enabled.as_ref().map(|s| *s.as_inner()),
            ..Self::default()
        }
    }
}

impl<'a> TurboJsonReader<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self { repo_root }
    }

    fn turbo_json_to_config_options(
        turbo_json: RawTurboJson,
    ) -> Result<ConfigurationOptions, Error> {
        let mut opts = if let Some(remote_cache_options) = &turbo_json.remote_cache {
            remote_cache_options.into()
        } else {
            ConfigurationOptions::default()
        };

        let cache_dir = if let Some(cache_dir) = turbo_json.cache_dir {
            let cache_dir_str: &str = &cache_dir;
            let cache_dir_unix = RelativeUnixPath::new(cache_dir_str).map_err(|_| {
                let (span, text) = cache_dir.span_and_text("turbo.json");
                Error::AbsoluteCacheDir { span, text }
            })?;
            // Convert the relative unix path to an anchored system path
            // For unix/macos this is a no-op
            let cache_dir_system = cache_dir_unix.to_anchored_system_path_buf();
            Some(Utf8PathBuf::from(cache_dir_system.to_string()))
        } else {
            None
        };

        // Don't allow token to be set for shared config.
        opts.token = None;
        opts.ui = turbo_json.ui.map(|ui| *ui.as_inner());
        opts.allow_no_package_manager = turbo_json
            .allow_no_package_manager
            .map(|allow| *allow.as_inner());
        opts.daemon = turbo_json.daemon.map(|daemon| *daemon.as_inner());
        opts.env_mode = turbo_json.env_mode.map(|mode| *mode.as_inner());
        opts.cache_dir = cache_dir;
        opts.concurrency = turbo_json.concurrency.map(|c| c.as_inner().clone());
        opts.future_flags = turbo_json.future_flags.map(|f| *f.as_inner());
        Ok(opts)
    }
}

impl<'a> ResolvedConfigurationOptions for TurboJsonReader<'a> {
    fn get_configuration_options(
        &self,
        existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        let turbo_json_path = existing_config.root_turbo_json_path(self.repo_root)?;
        let root_relative_turbo_json_path = self.repo_root.anchor(&turbo_json_path).map_or_else(
            |_| turbo_json_path.as_str().to_owned(),
            |relative| relative.to_string(),
        );
        let turbo_json = match turbo_json_path.read_existing_to_string()? {
            Some(contents) => {
                RawRootTurboJson::parse(&contents, &root_relative_turbo_json_path)?.into()
            }
            None => RawTurboJson::default(),
        };
        Self::turbo_json_to_config_options(turbo_json)
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use tempfile::tempdir;
    use test_case::test_case;

    use super::*;
    use crate::config::CONFIG_FILE;

    #[test]
    fn test_reads_from_default() {
        let tmpdir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::new(tmpdir.path().to_str().unwrap()).unwrap();

        let existing_config = ConfigurationOptions {
            ..Default::default()
        };
        repo_root
            .join_component(CONFIG_FILE)
            .create_with_contents(
                serde_json::to_string_pretty(&serde_json::json!({
                    "daemon": false
                }))
                .unwrap(),
            )
            .unwrap();

        let reader = TurboJsonReader::new(repo_root);
        let config = reader.get_configuration_options(&existing_config).unwrap();
        // Make sure we read the default turbo.json
        assert_eq!(config.daemon(), Some(false));
    }

    #[test]
    fn test_respects_root_turbo_json_config() {
        let tmpdir = tempdir().unwrap();
        let tmpdir_path = AbsoluteSystemPath::new(tmpdir.path().to_str().unwrap()).unwrap();
        let root_turbo_json_path = tmpdir_path.join_component("yolo.json");
        let repo_root = AbsoluteSystemPath::new(if cfg!(windows) {
            "C:\\my\\repo"
        } else {
            "/my/repo"
        })
        .unwrap();
        let existing_config = ConfigurationOptions {
            root_turbo_json_path: Some(root_turbo_json_path.to_owned()),
            ..Default::default()
        };
        root_turbo_json_path
            .create_with_contents(
                serde_json::to_string_pretty(&json!({
                    "daemon": false
                }))
                .unwrap(),
            )
            .unwrap();

        let reader = TurboJsonReader::new(repo_root);
        let config = reader.get_configuration_options(&existing_config).unwrap();
        // Make sure we read the correct turbo.json
        assert_eq!(config.daemon(), Some(false));
    }

    #[test]
    fn test_remote_cache_options() {
        let timeout = 100;
        let upload_timeout = 200;
        let api_url = "localhost:3000";
        let login_url = "localhost:3001";
        let team_slug = "acme-packers";
        let team_id = "id-123";
        let turbo_json = RawRootTurboJson::parse(
            &serde_json::to_string_pretty(&json!({
                "remoteCache": {
                    "enabled": true,
                    "timeout": timeout,
                    "uploadTimeout": upload_timeout,
                    "apiUrl": api_url,
                    "loginUrl": login_url,
                    "teamSlug": team_slug,
                    "teamId": team_id,
                    "signature": true,
                    "preflight": true
                }
            }))
            .unwrap(),
            "junk",
        )
        .unwrap()
        .into();
        let config = TurboJsonReader::turbo_json_to_config_options(turbo_json).unwrap();
        assert!(config.enabled());
        assert_eq!(config.timeout(), timeout);
        assert_eq!(config.upload_timeout(), upload_timeout);
        assert_eq!(config.api_url(), api_url);
        assert_eq!(config.login_url(), login_url);
        assert_eq!(config.team_slug(), Some(team_slug));
        assert_eq!(config.team_id(), Some(team_id));
        assert!(config.signature());
        assert!(config.preflight());
    }

    #[test_case(None, false)]
    #[test_case(Some(false), false)]
    #[test_case(Some(true), true)]
    fn test_dangerously_disable_package_manager_check(value: Option<bool>, expected: bool) {
        let turbo_json = RawRootTurboJson::parse(
            &serde_json::to_string_pretty(
                &(if let Some(value) = value {
                    json!({
                        "dangerouslyDisablePackageManagerCheck": value
                    })
                } else {
                    json!({})
                }),
            )
            .unwrap(),
            "turbo.json",
        )
        .unwrap()
        .into();
        let config = TurboJsonReader::turbo_json_to_config_options(turbo_json).unwrap();
        assert_eq!(config.allow_no_package_manager(), expected);
    }
}
