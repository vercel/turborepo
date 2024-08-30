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
}

impl<'a> ResolvedConfigurationOptions for TurboJsonReader<'a> {
    fn get_configuration_options(
        &self,
        _existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        let turbo_json =
            RawTurboJson::read(self.repo_root, &self.repo_root.join_component("turbo.json"))
                .or_else(|e| {
                    if let Error::Io(e) = &e {
                        if matches!(e.kind(), std::io::ErrorKind::NotFound) {
                            return Ok(Default::default());
                        }
                    }

                    Err(e)
                })?;
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
