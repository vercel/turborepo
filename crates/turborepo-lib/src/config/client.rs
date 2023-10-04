use std::collections::HashMap;

use anyhow::Result;
use config::{Config, Environment};
use serde::{Deserialize, Serialize};

use crate::config::Error;

const DEFAULT_TIMEOUT: u64 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    config: ClientConfigValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct ClientConfigValue {
    remote_cache_timeout: u64,
}

#[derive(Debug, Clone)]
pub struct ClientConfigLoader {
    remote_cache_timeout: Option<u64>,
    environment: Option<HashMap<String, String>>,
}

impl ClientConfig {
    pub fn remote_cache_timeout(&self) -> u64 {
        self.config.remote_cache_timeout
    }
}

impl ClientConfigLoader {
    /// Creates a loader that will load the client config
    pub fn new() -> Self {
        Self {
            remote_cache_timeout: None,
            environment: None,
        }
    }

    /// Set an override for token that the user provided via the command line
    pub fn with_remote_cache_timeout(mut self, remote_cache_timeout: Option<u64>) -> Self {
        self.remote_cache_timeout = remote_cache_timeout;
        self
    }

    #[allow(dead_code)]
    pub fn with_environment(mut self, environment: Option<HashMap<String, String>>) -> Self {
        self.environment = environment;
        self
    }

    pub fn load(self) -> Result<ClientConfig, Error> {
        let Self {
            remote_cache_timeout,
            environment,
        } = self;

        let config_attempt = Config::builder()
            .set_default("remote_cache_timeout", DEFAULT_TIMEOUT)?
            .add_source(Environment::with_prefix("turbo").source(environment))
            .set_override_option("remote_cache_timeout", remote_cache_timeout)?
            .build()?
            // Deserialize is the only user-input-fallible step.
            // Everything else is programmer error.
            // This goes wrong when TURBO_REMOTE_CACHE_TIMEOUT can't be deserialized to u64
            .try_deserialize();

        config_attempt
            .map_err(Error::Config)
            .map(|config| ClientConfig { config })
    }
}

#[cfg(test)]
mod test {
    use std::env::set_var;

    use super::*;

    // We group these test functions under one test to
    // avoid race conditions with environment variables
    #[test]
    fn all_tests() -> Result<()> {
        test_client_default()?;
        test_client_arg_variable()?;
        test_client_env_variable()?;

        Ok(())
    }

    fn test_client_default() -> Result<()> {
        let config = ClientConfigLoader::new().load()?;

        assert_eq!(config.remote_cache_timeout(), DEFAULT_TIMEOUT);

        Ok(())
    }

    fn test_client_arg_variable() -> Result<()> {
        let arg_value: u64 = 1;

        let config = ClientConfigLoader::new()
            .with_remote_cache_timeout(Some(arg_value))
            .load()?;

        assert_eq!(config.remote_cache_timeout(), arg_value);

        Ok(())
    }

    fn test_client_env_variable() -> Result<()> {
        let env_value = String::from("2");

        let config = ClientConfigLoader::new()
            .with_environment({
                let mut env = HashMap::new();
                env.insert("TURBO_REMOTE_CACHE_TIMEOUT".into(), env_value.clone());
                Some(env)
            })
            .load()?;

        assert_eq!(
            config.remote_cache_timeout(),
            env_value.parse::<u64>().unwrap()
        );

        Ok(())
    }

    #[test]
    fn test_client_arg_env_variable() -> Result<()> {
        #[derive(Debug)]
        struct TestCase {
            arg: Option<u64>,
            env: String,
            output: u64,
            want_err: Option<&'static str>,
        }

        let tests = [
            TestCase {
                arg: Some(0),
                env: String::from("0"),
                output: 0,
                want_err: None,
            },
            TestCase {
                arg: Some(0),
                env: String::from("2"),
                output: 0,
                want_err: None,
            },
            TestCase {
                arg: Some(0),
                env: String::from("garbage"),
                output: 0,
                want_err: None,
            },
            TestCase {
                arg: Some(0),
                env: String::from(""),
                output: 0,
                want_err: None,
            },
            TestCase {
                arg: Some(1),
                env: String::from("0"),
                output: 1,
                want_err: None,
            },
            TestCase {
                arg: Some(1),
                env: String::from("2"),
                output: 1,
                want_err: None,
            },
            TestCase {
                arg: Some(1),
                env: String::from("garbage"),
                output: 1,
                want_err: None,
            },
            TestCase {
                arg: Some(1),
                env: String::from(""),
                output: 1,
                want_err: None,
            },
            TestCase {
                arg: None,
                env: String::from("0"),
                output: 0,
                want_err: None,
            },
            TestCase {
                arg: None,
                env: String::from("2"),
                output: 2,
                want_err: None,
            },
            TestCase {
                arg: None,
                env: String::from("garbage"),
                output: DEFAULT_TIMEOUT,
                want_err: Some(
                    "invalid type: string \"garbage\", expected an integer for key \
                     `remote_cache_timeout` in the environment",
                ),
            },
            TestCase {
                arg: None,
                env: String::from(""),
                output: DEFAULT_TIMEOUT,
                want_err: Some(
                    "invalid type: string \"\", expected an integer for key \
                     `remote_cache_timeout` in the environment",
                ),
            },
        ];

        for test in &tests {
            println!("{:?}", test);
            let config = ClientConfigLoader::new()
                .with_remote_cache_timeout(test.arg)
                .with_environment({
                    let mut env = HashMap::new();
                    env.insert("TURBO_REMOTE_CACHE_TIMEOUT".into(), test.env.clone());
                    Some(env)
                })
                .load();

            if test.want_err.is_none() {
                assert_eq!(config.unwrap().remote_cache_timeout(), test.output);
            } else {
                assert_eq!(config.err().unwrap().to_string(), test.want_err.unwrap());
            }
        }

        // We can only hit the actual system for env vars in a single test
        // without triggering race conditions.
        for test in &tests {
            set_var("TURBO_REMOTE_CACHE_TIMEOUT", test.env.clone());
            let config = ClientConfigLoader::new()
                .with_remote_cache_timeout(test.arg)
                .load();

            if test.want_err.is_none() {
                assert_eq!(config.unwrap().remote_cache_timeout(), test.output);
            } else {
                assert_eq!(config.err().unwrap().to_string(), test.want_err.unwrap());
            }
        }

        Ok(())
    }
}
