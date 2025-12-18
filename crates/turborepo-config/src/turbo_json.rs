use std::{collections::BTreeMap, str::FromStr};

use camino::Utf8PathBuf;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};
use turborepo_turbo_json::{RawRemoteCacheOptions, RawRootTurboJson, RawTurboJson};

use crate::{
    ConfigurationOptions, Error, ExperimentalObservabilityOptions, ExperimentalOtelMetricsOptions,
    ExperimentalOtelOptions, ExperimentalOtelProtocol, ResolvedConfigurationOptions,
};

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
                Error::TurboJsonError(turborepo_turbo_json::Error::AbsoluteCacheDir { span, text })
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
        opts.no_update_notifier = turbo_json
            .no_update_notifier
            .map(|no_update_notifier| *no_update_notifier.as_inner());
        opts.cache_dir = cache_dir;
        opts.concurrency = turbo_json.concurrency.map(|c| c.as_inner().clone());

        opts.future_flags = turbo_json.future_flags.map(|f| *f.as_inner());

        // Only read observability config if futureFlags.experimentalObservability is
        // enabled
        if opts
            .future_flags
            .map(|f| f.experimental_observability)
            .unwrap_or(false)
        {
            if let Some(raw_observability) = turbo_json.experimental_observability {
                opts.experimental_observability =
                    Some(convert_raw_observability(raw_observability)?);
            }
        }
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
            Some(contents) => RawRootTurboJson::parse(&contents, &root_relative_turbo_json_path)
                .map_err(turborepo_turbo_json::Error::from)?
                .into(),
            None => RawTurboJson::default(),
        };
        Self::turbo_json_to_config_options(turbo_json)
    }
}

fn convert_raw_observability(
    raw: RawExperimentalObservability,
) -> Result<ExperimentalObservabilityOptions, Error> {
    Ok(ExperimentalObservabilityOptions {
        otel: raw.otel.map(convert_raw_observability_otel).transpose()?,
    })
}

fn convert_raw_observability_otel(
    raw: RawObservabilityOtel,
) -> Result<ExperimentalOtelOptions, Error> {
    let protocol = if let Some(protocol) = raw.protocol {
        let proto_str = protocol.as_inner().as_str();
        Some(ExperimentalOtelProtocol::from_str(proto_str).map_err(|e| {
            Error::InvalidExperimentalOtelConfig {
                message: e.to_string(),
            }
        })?)
    } else {
        None
    };

    let metrics = raw.metrics.map(|metrics| ExperimentalOtelMetricsOptions {
        run_summary: metrics.run_summary.map(|flag| *flag.as_inner()),
        task_details: metrics.task_details.map(|flag| *flag.as_inner()),
    });

    let headers = raw.headers.map(|h| {
        h.into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect::<BTreeMap<String, String>>()
    });

    let resource = raw.resource.map(|r| {
        r.into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect::<BTreeMap<String, String>>()
    });

    Ok(ExperimentalOtelOptions {
        enabled: raw.enabled.map(|flag| *flag.as_inner()),
        protocol,
        endpoint: raw.endpoint.map(|endpoint| endpoint.into_inner().into()),
        headers,
        timeout_ms: raw.timeout_ms.map(|timeout| *timeout.as_inner()),
        resource,
        metrics,
        use_remote_cache_token: raw.use_remote_cache_token.map(|flag| *flag.as_inner()),
    })
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use tempfile::tempdir;
    use test_case::test_case;

    use super::*;
    use crate::CONFIG_FILE;

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

    #[test]
    fn test_no_update_notifier() {
        let turbo_json = RawRootTurboJson::parse(
            &serde_json::to_string_pretty(&json!({
                "noUpdateNotifier": true
            }))
            .unwrap(),
            "turbo.json",
        )
        .unwrap()
        .into();
        let config = TurboJsonReader::turbo_json_to_config_options(turbo_json).unwrap();

        assert!(config.no_update_notifier());
    }

    #[test]
    fn test_experimental_observability_otel_with_future_flag_disabled() {
        let turbo_json = RawRootTurboJson::parse(
            &serde_json::to_string_pretty(&json!({
                "futureFlags": {
                    "experimentalObservability": false
                },
                "experimentalObservability": {
                    "otel": {
                        "enabled": true,
                        "endpoint": "https://example.com/otel"
                    }
                }
            }))
            .unwrap(),
            "turbo.json",
        )
        .unwrap()
        .into();
        let config = TurboJsonReader::turbo_json_to_config_options(turbo_json).unwrap();
        // Should be None because future flag is disabled
        assert_eq!(config.experimental_observability(), None);
    }

    #[test]
    fn test_experimental_observability_otel_with_future_flag_enabled() {
        let endpoint = "https://example.com/otel";
        let turbo_json = RawRootTurboJson::parse(
            &serde_json::to_string_pretty(&json!({
                "futureFlags": {
                    "experimentalObservability": true
                },
                "experimentalObservability": {
                    "otel": {
                        "enabled": true,
                        "endpoint": endpoint,
                        "protocol": "grpc",
                        "timeoutMs": 5000
                    }
                }
            }))
            .unwrap(),
            "turbo.json",
        )
        .unwrap()
        .into();
        let config = TurboJsonReader::turbo_json_to_config_options(turbo_json).unwrap();
        let observability_config = config.experimental_observability();
        assert!(observability_config.is_some());
        let otel_config = observability_config.unwrap().otel.as_ref().unwrap();
        assert_eq!(otel_config.enabled, Some(true));
        assert_eq!(otel_config.endpoint.as_ref(), Some(&endpoint.to_string()));
        assert_eq!(otel_config.protocol, Some(ExperimentalOtelProtocol::Grpc));
        assert_eq!(otel_config.timeout_ms, Some(5000));
    }

    #[test]
    fn test_experimental_observability_without_future_flag() {
        let turbo_json = RawRootTurboJson::parse(
            &serde_json::to_string_pretty(&json!({
                "experimentalObservability": {
                    "otel": {
                        "enabled": true,
                        "endpoint": "https://example.com/otel"
                    }
                }
            }))
            .unwrap(),
            "turbo.json",
        )
        .unwrap()
        .into();
        let config = TurboJsonReader::turbo_json_to_config_options(turbo_json).unwrap();
        // Should be None because future flag is not set (defaults to false)
        assert_eq!(config.experimental_observability(), None);
    }

    #[test]
    fn test_experimental_observability_otel_with_headers_and_resource() {
        let turbo_json = RawRootTurboJson::parse(
            &serde_json::to_string_pretty(&json!({
                "futureFlags": {
                    "experimentalObservability": true
                },
                "experimentalObservability": {
                    "otel": {
                        "enabled": true,
                        "endpoint": "https://example.com/otel",
                        "headers": {
                            "Authorization": "Bearer token123",
                            "X-Custom-Header": "custom-value"
                        },
                        "resource": {
                            "service.name": "turborepo",
                            "service.version": "1.0.0"
                        }
                    }
                }
            }))
            .unwrap(),
            "turbo.json",
        )
        .unwrap()
        .into();
        let config = TurboJsonReader::turbo_json_to_config_options(turbo_json).unwrap();
        let observability_config = config.experimental_observability();
        assert!(observability_config.is_some());
        let otel_config = observability_config.unwrap().otel.as_ref().unwrap();
        assert_eq!(otel_config.enabled, Some(true));

        // Verify headers are parsed correctly
        let headers = otel_config.headers.as_ref().unwrap();
        assert_eq!(
            headers.get("Authorization"),
            Some(&"Bearer token123".to_string())
        );
        assert_eq!(
            headers.get("X-Custom-Header"),
            Some(&"custom-value".to_string())
        );

        // Verify resource is parsed correctly
        let resource = otel_config.resource.as_ref().unwrap();
        assert_eq!(resource.get("service.name"), Some(&"turborepo".to_string()));
        assert_eq!(resource.get("service.version"), Some(&"1.0.0".to_string()));
    }
}
