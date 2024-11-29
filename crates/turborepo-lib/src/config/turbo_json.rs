use camino::Utf8PathBuf;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};

use super::{ConfigurationOptions, Error, ResolvedConfigurationOptions};
use crate::turbo_json::RawTurboJson;

pub struct TurboJsonReader<'a> {
    repo_root: &'a AbsoluteSystemPath,
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
        opts.spaces_id = turbo_json
            .experimental_spaces
            .and_then(|spaces| spaces.id)
            .map(|spaces_id| spaces_id.into());
        opts.ui = turbo_json.ui;
        opts.allow_no_package_manager = turbo_json.allow_no_package_manager;
        opts.daemon = turbo_json.daemon.map(|daemon| *daemon.as_inner());
        opts.env_mode = turbo_json.env_mode;
        opts.cache_dir = cache_dir;
        Ok(opts)
    }
}

impl<'a> ResolvedConfigurationOptions for TurboJsonReader<'a> {
    fn get_configuration_options(
        &self,
        existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        let turbo_json_path = existing_config.root_turbo_json_path(self.repo_root);
        let turbo_json = RawTurboJson::read(self.repo_root, &turbo_json_path).or_else(|e| {
            if let Error::Io(e) = &e {
                if matches!(e.kind(), std::io::ErrorKind::NotFound) {
                    return Ok(Default::default());
                }
            }

            Err(e)
        })?;
        Self::turbo_json_to_config_options(turbo_json)
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_reads_from_default() {
        let tmpdir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::new(tmpdir.path().to_str().unwrap()).unwrap();

        let existing_config = ConfigurationOptions {
            ..Default::default()
        };
        repo_root
            .join_component("turbo.json")
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
        let turbo_json = RawTurboJson::parse(
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
        .unwrap();
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
}
