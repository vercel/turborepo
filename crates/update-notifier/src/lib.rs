use std::time::Duration;

use colored::*;
use reqwest::Error as ReqwestError;
use semver::{Error as SemVerError, Version};
use serde::{Deserialize, Serialize};
use serde_json::Error as SerdeError;
use thiserror::Error as ThisError;
use tiny_gradient::{GradientStr, RGB};

mod fetch;
mod ui;
mod utils;

// default interval to check for new updates (one per day)
const INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);

#[derive(Debug)]
pub struct UpdateNotifier {
    config: UpdateNotifierConfig,
    package: String,
    tag: Option<String>,
    interval: Duration,
}

// This portion of the config is serialized and persisted to disk
#[derive(Serialize, Deserialize, Debug, Clone)]
struct UpdateNotifierConfig {
    current_version: String,
    latest_version: Option<String>,
    last_checked: Option<u128>,
}

#[derive(ThisError, Debug)]
pub enum UpdateNotifierError {
    #[error("Failed to fetch latest version")]
    FetchError(#[from] ReqwestError),
    #[error("Failed to parse version")]
    ParseError(#[from] SemVerError),
    #[error("Failed to parse JSON")]
    JsonError(#[from] SerdeError),
}

impl UpdateNotifier {
    fn should_refresh(&self) -> bool {
        if self.config.latest_version.is_none() {
            log::debug!("no latest version found in local config");
            return true;
        }
        match self.config.last_checked {
            Some(last_checked) => {
                let now = utils::ms_since_epoch();
                let diff = now - last_checked;
                log::debug!(
                    "last checked {} ms ago (interval={} ms)",
                    diff,
                    self.interval.as_millis()
                );
                log::debug!("refreshing in {} ms", self.interval.as_millis() - diff);
                diff > self.interval.as_millis()
            }
            None => {
                log::debug!("no last checked time found in local config");
                true
            }
        }
    }

    fn update_message(&self) {
        let turbo = "@turborepo";
        let turbo_gradient = turbo.gradient([RGB::new(0, 153, 247), RGB::new(241, 23, 18)]);

        let latest_version = match &self.config.latest_version {
            Some(v) => v,
            None => {
                log::error!("no latest version found in local config");
                return;
            }
        };

        let msg = format!(
            "
            Update available {} ≫ {}
            Changelog: https://github.com/vercel/turbo/releases/tag/v{}Run \
             \"{}\" to update

            Follow {} for updates: {}
            ",
            &self.config.current_version.dimmed(),
            &latest_version.green().bold(),
            &latest_version,
            // TODO: make this package manager aware
            "npm i -g turbo".cyan().bold(),
            turbo_gradient,
            "https://twitter.com/turborepo",
        );

        ui::rectangle(&msg);
    }

    fn first_run(package: String, tag: Option<String>, interval: Option<Duration>) -> Self {
        log::debug!("writing first local version config");
        Self {
            config: UpdateNotifierConfig {
                current_version: utils::get_version().to_string(),
                latest_version: None,
                last_checked: None,
            },
            interval: interval.unwrap_or(INTERVAL),
            package,
            tag,
        }
    }

    pub fn new(package: String, tag: Option<String>, interval: Option<Duration>) -> Self {
        let tmp = utils::get_config_path();

        // ignore if it doesn't exist, we don't care
        if tmp.try_exists().unwrap_or(false) {
            let file = std::fs::File::open(tmp);
            let file = match file {
                Ok(f) => f,
                Err(_) => {
                    log::debug!("failed to open local config, writing first version");
                    return Self::first_run(package, tag, interval);
                }
            };
            let reader = std::io::BufReader::new(file);
            let json_result: Result<UpdateNotifierConfig, SerdeError> =
                serde_json::from_reader(reader);
            match json_result {
                Ok(v) => {
                    log::debug!("found local version config {:?}", v);
                    Self {
                        config: UpdateNotifierConfig {
                            current_version: utils::get_version().to_string(),
                            ..v
                        },
                        package,
                        tag,
                        interval: interval.unwrap_or(INTERVAL),
                    }
                }
                Err(_) => {
                    log::debug!("failed to find version config");
                    Self::first_run(package, tag, interval)
                }
            }
        } else {
            Self::first_run(package, tag, interval)
        }
    }

    fn save(&self) {
        // get directory
        let tmp = utils::get_config_path();
        let config = serde_json::to_string(&self.config);
        let config = match config {
            Ok(v) => v,
            Err(_) => {
                log::debug!("failed to serialize config");
                return;
            }
        };

        let result = std::fs::write(tmp, config);
        match result {
            Ok(_) => log::debug!("saved config to disk {:?}", self.config),
            Err(err) => log::debug!("failed to save config to disk {:?}", err),
        }
    }

    #[tokio::main]
    async fn update(&mut self) -> Result<(), UpdateNotifierError> {
        let latest_version =
            fetch::get_latest_version(&self.package, self.tag.as_deref(), None).await;
        let latest_version = match latest_version {
            Ok(v) => v,
            Err(err) => {
                log::debug!("failed to fetch latest version {:?}", err);
                return Err(err);
            }
        };

        let current_version = String::from(utils::get_version());
        let now = utils::ms_since_epoch();

        // update fields
        self.config.latest_version = Some(latest_version);
        self.config.current_version = current_version;
        self.config.last_checked = Some(now);

        // persist
        self.save();
        return Ok(());
    }

    pub fn check(&mut self) -> Result<(), UpdateNotifierError> {
        if self.should_refresh() {
            log::debug!("refreshing local config");
            self.update()?;
        }

        if let Some(latest_version) = &self.config.latest_version {
            let current_version = Version::parse(&self.config.current_version)?;
            let latest_version = Version::parse(latest_version)?;
            log::debug!("checking if {} > {}", latest_version, current_version);
            if latest_version > current_version {
                log::debug!("update available");
                self.update_message();
                return Ok(());
            } else {
                log::debug!("no update available");
                return Ok(());
            }
        }
        return Ok(());
    }
}
