/// Configuration for telemetry.
///
/// This module is responsible for reading and writing the telemetry
/// configuration file.
///
/// NOTE: There is a port of this crate that is used to instrument node
/// projects. Any changes made here should be reflected there as well.
///
/// https://github.com/vercel/turborepo/blob/main/packages/turbo-telemetry/src/config.ts
use std::env;

use chrono::{DateTime, Utc};
pub use config::{Config, ConfigError, File, FileFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{error, trace};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_dirs::config_dir;
use turborepo_ui::{color, ColorConfig, BOLD, GREY, UNDERLINE};
use uuid::Uuid;

static DEBUG_ENV_VAR: &str = "TURBO_TELEMETRY_DEBUG";
static DISABLED_ENV_VAR: &str = "TURBO_TELEMETRY_DISABLED";
static DISABLED_MESSAGE_ENV_VAR: &str = "TURBO_TELEMETRY_MESSAGE_DISABLED";
static DO_NOT_TRACK_ENV_VAR: &str = "DO_NOT_TRACK";

#[derive(Debug, Deserialize, Serialize)]
pub struct TelemetryConfigContents {
    // whether or not telemetry is enabled
    telemetry_enabled: bool,
    // randomized and salted machine id - used for linking events together
    telemetry_id: String,
    // private salt used to anonymize event data (telemetry_id, task names, package names, etc.) -
    // this is generated on first run and never leaves the machine
    telemetry_salt: String,

    // when the alert was shown
    #[serde(skip_serializing_if = "Option::is_none")]
    telemetry_alerted: Option<DateTime<Utc>>,
}

impl Default for TelemetryConfigContents {
    fn default() -> Self {
        let telemetry_salt = Uuid::new_v4().to_string();
        let raw_telemetry_id = Uuid::new_v4().to_string();
        let telemetry_id = one_way_hash_with_salt(&telemetry_salt, &raw_telemetry_id);

        TelemetryConfigContents {
            telemetry_enabled: true,
            telemetry_alerted: None,
            telemetry_salt,
            telemetry_id,
        }
    }
}

#[derive(Debug)]
pub struct TelemetryConfig {
    config_path: AbsoluteSystemPathBuf,
    config: TelemetryConfigContents,
}

impl TelemetryConfig {
    pub fn with_default_config_path() -> Result<TelemetryConfig, ConfigError> {
        let config_path = default_config_path()?;
        TelemetryConfig::new(config_path)
    }

    pub fn new(config_path: AbsoluteSystemPathBuf) -> Result<TelemetryConfig, ConfigError> {
        trace!("Telemetry config path: {}", config_path);
        if !config_path.exists() {
            write_new_config(&config_path)?;
        }

        let mut settings = Config::builder();
        settings = settings.add_source(File::new(config_path.as_str(), FileFormat::Json));
        let settings = settings.build();

        // If this is a FileParse error, we assume something corrupted the file or
        // its structure. In this case, because the telemetry config is intentionally
        // isolated from other turborepo config, try to remove the entire config
        // file and write a new one, otherwise return the error
        let config = match settings {
            Ok(settings) => settings.try_deserialize::<TelemetryConfigContents>()?,
            Err(ConfigError::FileParse { .. }) => {
                config_path
                    .remove_file()
                    .map_err(|e| ConfigError::Message(e.to_string()))?;
                write_new_config(&config_path)?;
                return Err(settings.unwrap_err());
            }
            // Propagate other errors
            Err(err) => return Err(err),
        };

        let config = TelemetryConfig {
            config_path,
            config,
        };

        Ok(config)
    }

    fn write(&self) -> Result<(), ConfigError> {
        let serialized = serde_json::to_string_pretty(&self.config)
            .map_err(|e| ConfigError::Message(e.to_string()))?;
        self.config_path
            .create_with_contents(serialized)
            .map_err(|e| ConfigError::Message(e.to_string()))?;
        Ok(())
    }

    pub fn one_way_hash(input: &str) -> String {
        match TelemetryConfig::with_default_config_path() {
            Ok(config) => config.one_way_hash_with_config_salt(input),
            Err(_) => TelemetryConfig::one_way_hash_with_tmp_salt(input),
        }
    }

    /// Obfuscate with the config salt - this is used for all sensitive event
    /// data
    fn one_way_hash_with_config_salt(&self, input: &str) -> String {
        one_way_hash_with_salt(&self.config.telemetry_salt, input)
    }

    /// Obfuscate with a temporary salt - this is used as a fallback when the
    /// config salt is not available (e.g. config loading failed etc.)
    ///
    /// This is just as secure as the config salt, but it prevents us from
    /// linking together events that include obfuscated data generated with
    /// this method as each call will generate a new salt.
    fn one_way_hash_with_tmp_salt(input: &str) -> String {
        let tmp_salt = Uuid::new_v4().to_string();
        one_way_hash_with_salt(&tmp_salt, input)
    }

    pub fn show_alert(&mut self, color_config: ColorConfig) {
        if !self.has_seen_alert() && self.is_enabled() && Self::is_telemetry_warning_enabled() {
            eprintln!(
                "\n{}\n{}\n{}\n{}\n{}\n",
                color!(color_config, BOLD, "{}", "Attention:"),
                color!(
                    color_config,
                    GREY,
                    "{}",
                    "Turborepo now collects completely anonymous telemetry regarding usage."
                ),
                color!(
                    color_config,
                    GREY,
                    "{}",
                    "This information is used to shape the Turborepo roadmap and prioritize \
                     features."
                ),
                color!(
                    color_config,
                    GREY,
                    "{}",
                    "You can learn more, including how to opt-out if you'd not like to \
                     participate in this anonymous program, by visiting the following URL:"
                ),
                color!(
                    color_config,
                    UNDERLINE,
                    "{}",
                    color!(
                        color_config,
                        GREY,
                        "{}",
                        "https://turbo.build/repo/docs/telemetry"
                    )
                ),
            );

            if let Err(err) = self.alert_shown() {
                error!(
                    "Error saving seen alert event to telemetry config: {:?}",
                    err
                );
            }
        }
    }

    // getters
    pub fn has_seen_alert(&self) -> bool {
        self.config.telemetry_alerted.is_some()
    }

    pub fn is_enabled(&self) -> bool {
        let do_not_track = env::var(DO_NOT_TRACK_ENV_VAR).unwrap_or("0".to_string());
        let turbo_telemetry_disabled = env::var(DISABLED_ENV_VAR).unwrap_or("0".to_string());

        if do_not_track == "1"
            || do_not_track == "true"
            || turbo_telemetry_disabled == "1"
            || turbo_telemetry_disabled == "true"
        {
            return false;
        }

        self.config.telemetry_enabled
    }

    pub fn is_telemetry_warning_enabled() -> bool {
        let turbo_telemetry_msg_disabled =
            env::var(DISABLED_MESSAGE_ENV_VAR).unwrap_or("0".to_string());
        let is_disabled =
            turbo_telemetry_msg_disabled == "1" || turbo_telemetry_msg_disabled == "true";

        !is_disabled
    }

    pub fn get_id(&self) -> &str {
        &self.config.telemetry_id
    }

    // setters
    pub fn enable(&mut self) -> Result<&TelemetryConfigContents, ConfigError> {
        self.config.telemetry_enabled = true;
        self.write()?;
        Ok(&self.config)
    }

    pub fn disable(&mut self) -> Result<&TelemetryConfigContents, ConfigError> {
        self.config.telemetry_enabled = false;
        self.write()?;
        Ok(&self.config)
    }

    pub fn alert_shown(&mut self) -> Result<&TelemetryConfigContents, ConfigError> {
        match self.has_seen_alert() {
            true => Ok(&self.config),
            false => {
                self.config.telemetry_alerted = Some(Utc::now());
                self.write()?;
                Ok(&self.config)
            }
        }
    }
}

fn default_config_path() -> Result<AbsoluteSystemPathBuf, ConfigError> {
    let config_dir = config_dir()
        .map_err(|e| ConfigError::Message(format!("Invalid config directory: {}", e)))?
        .ok_or(ConfigError::Message(
            "Unable to find telemetry config directory".to_string(),
        ))?;
    // stored as a sibling to the turbo global config
    Ok(config_dir.join_components(&["turborepo", "telemetry.json"]))
}

fn write_new_config(file_path: &AbsoluteSystemPath) -> Result<(), ConfigError> {
    let serialized = serde_json::to_string_pretty(&TelemetryConfigContents::default())
        .map_err(|e| ConfigError::Message(e.to_string()))?;

    // Create the directory if it doesn't exist
    file_path
        .ensure_dir()
        .map_err(|_| ConfigError::Message("Failed to create directory".to_string()))?;

    // Write the file
    file_path
        .create_with_contents(serialized)
        .map_err(|e| ConfigError::Message(e.to_string()))?;
    Ok(())
}

pub fn is_debug() -> bool {
    let debug = env::var(DEBUG_ENV_VAR).unwrap_or("0".to_string());
    debug == "1" || debug == "true"
}

fn one_way_hash_with_salt(salt: &str, input: &str) -> String {
    let salted = format!("{}{}", salt, input);
    let mut hasher = Sha256::new();
    hasher.update(salted.as_bytes());
    let generic = hasher.finalize();
    hex::encode(generic)
}
