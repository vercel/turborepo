use anyhow::Result;
use config::{Config, Environment};
use serde::{Deserialize, Serialize};

const DEFAULT_TIMEOUT: u64 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    config: ClientConfigValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct ClientConfigValue {
    remote_cache_timeout: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ClientConfigLoader {
    remote_cache_timeout: Option<u64>,
}

impl ClientConfig {
    #[allow(dead_code)]
    pub fn remote_cache_timeout(&self) -> u64 {
        self.config.remote_cache_timeout.unwrap_or(DEFAULT_TIMEOUT)
    }
}

impl ClientConfigLoader {
    /// Creates a loader that will load the client config
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            remote_cache_timeout: None,
        }
    }

    /// Set an override for token that the user provided via the command line
    #[allow(dead_code)]
    pub fn with_remote_cache_timeout(mut self, remote_cache_timeout: Option<u64>) -> Self {
        self.remote_cache_timeout = remote_cache_timeout;
        self
    }

    #[allow(dead_code)]
    pub fn load(self) -> Result<ClientConfig> {
        let Self {
            remote_cache_timeout,
        } = self;

        let config: ClientConfigValue = Config::builder()
            .add_source(Environment::with_prefix("turbo"))
            .set_override_option("remote_cache_timeout", remote_cache_timeout)?
            .build()?
            .try_deserialize()?;

        Ok(ClientConfig { config })
    }
}
