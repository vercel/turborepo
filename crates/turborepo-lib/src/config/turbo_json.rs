use camino::Utf8PathBuf;
use turbopath::RelativeUnixPath;

use super::{ConfigurationOptions, Error, ResolvedConfigurationOptions};
use crate::turbo_json::RawTurboJson;

impl ResolvedConfigurationOptions for RawTurboJson {
    fn get_configuration_options(self) -> Result<ConfigurationOptions, Error> {
        let mut opts = if let Some(remote_cache_options) = &self.remote_cache {
            remote_cache_options.into()
        } else {
            ConfigurationOptions::default()
        };

        let cache_dir = if let Some(cache_dir) = self.cache_dir {
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
        opts.spaces_id = self
            .experimental_spaces
            .and_then(|spaces| spaces.id)
            .map(|spaces_id| spaces_id.into());
        opts.ui = self.ui;
        opts.allow_no_package_manager = self.allow_no_package_manager;
        opts.daemon = self.daemon.map(|daemon| *daemon.as_inner());
        opts.env_mode = self.env_mode;
        opts.cache_dir = cache_dir;
        Ok(opts)
    }
}
