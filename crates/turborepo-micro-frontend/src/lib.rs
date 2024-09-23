use error::Error;

mod config;
mod error;

pub const MICROFRONTEND_CONFIG_DEFAULT_FILE_PATH: &str = "micro-fontends.config.json";

pub struct MicroFrontendConfig {}

impl MicroFrontendConfig {
    /// Given a relative path, this function returns the name of the
    /// micro-frontend that serves the path
    pub fn application_for_path(&self, path: &str) -> Result<&str, Error> {
        if !path.starts_with('/') {
            return Err(Error::NonRelative);
        }
        todo!()
    }
}
