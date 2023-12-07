use std::{env, fs, path::Path};

use chrono::{DateTime, Utc};
pub use config::{Config, ConfigError, File, FileFormat};
use serde::{Deserialize, Serialize};
use serde_json;
use tracing::{debug, error};
use turborepo_ui::{BOLD, GREY, UI, UNDERLINE};
use uuid::Uuid;

static DEBUG_ENV_VAR: &str = "TURBO_TELEMETRY_DEBUG";
static DISABLED_ENV_VAR: &str = "TURBO_TELEMETRY_DISABLED";
static DO_NOT_TRACK_ENV_VAR: &str = "DO_NOT_TRACK";

#[derive(Debug, Deserialize, Serialize)]
pub struct TelemetryConfigContents {
    telemetry_id: String,
    telemetry_enabled: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    telemetry_alerted: Option<DateTime<Utc>>,
}

impl Default for TelemetryConfigContents {
    fn default() -> Self {
        TelemetryConfigContents {
            telemetry_id: Uuid::new_v4().to_string(),
            telemetry_enabled: true,
            telemetry_alerted: None,
        }
    }
}

#[derive(Debug)]
pub struct TelemetryConfig {
    config_path: String,
    ui: UI,
    config: TelemetryConfigContents,
}

fn get_config_path() -> Result<String, ConfigError> {
    let config_dir = dirs_next::config_dir().ok_or(ConfigError::Message(
        "Could find telemetry config directory".to_string(),
    ))?;
    // stored as a sibling to the turbo global config
    let config_path = config_dir.join("turborepo").join("telemetry.json");
    Ok(config_path.to_str().unwrap().to_string())
}

fn write_new_config() -> Result<(), ConfigError> {
    let file_path = &get_config_path()?;
    let serialized = serde_json::to_string_pretty(&TelemetryConfigContents::default())
        .map_err(|e| ConfigError::Message(e.to_string()))?;
    fs::write(file_path, serialized).map_err(|e| ConfigError::Message(e.to_string()))?;
    Ok(())
}

pub fn is_debug() -> bool {
    let debug = env::var(DEBUG_ENV_VAR).unwrap_or("0".to_string());
    return debug == "1" || debug == "true";
}

impl TelemetryConfig {
    pub fn new(ui: UI) -> Result<TelemetryConfig, ConfigError> {
        let file_path = &get_config_path()?;
        debug!("Telemetry config path: {}", file_path);

        if !Path::new(file_path).exists() {
            write_new_config()?
        }

        let mut settings = Config::builder();
        settings = settings.add_source(File::new(file_path, FileFormat::Json));

        let settings = settings.build();

        // Check if this is a FileParse error, remove the config file and write a new
        // one, otherwise return the error
        if let Err(ConfigError::FileParse { .. }) = settings {
            fs::remove_file(file_path).map_err(|e| ConfigError::Message(e.to_string()))?;
            write_new_config()?;
            return Err(settings.unwrap_err());
        } else if settings.is_err() {
            // Propagate other errors
            return Err(settings.unwrap_err());
        }

        // this is safe because we just checked the error case above
        let config = settings
            .unwrap()
            .try_deserialize::<TelemetryConfigContents>()?;

        let config = TelemetryConfig {
            config_path: file_path.to_string(),
            ui,
            config,
        };

        Ok(config)
    }

    fn write(&self) -> Result<(), ConfigError> {
        let serialized = serde_json::to_string_pretty(&self.config)
            .map_err(|e| ConfigError::Message(e.to_string()))?;
        fs::write(&self.config_path, serialized)
            .map_err(|e| ConfigError::Message(e.to_string()))?;
        Ok(())
    }

    pub fn show_alert(&mut self) -> () {
        if !self.has_seen_alert() && self.is_enabled() {
            println!(
                "\n{}\n{}\n{}\n{}\n{}\n",
                self.ui.apply(BOLD.apply_to("Attention:")),
                self.ui.apply(GREY.apply_to(
                    "Turborepo now collects completely anonymous telemetry regarding usage."
                )),
                self.ui.apply(GREY.apply_to(
                    "This information is used to shape the Turborepo roadmap and prioritize \
                     features."
                )),
                self.ui.apply(GREY.apply_to(
                    "You can learn more, including how to opt-out if you'd not like to \
                     participate in this anonymous program, by visiting the following URL:"
                )),
                self.ui.apply(
                    UNDERLINE.apply_to(GREY.apply_to("https://turbo.build/repo/docs/telemetry"))
                ),
            );

            let updated_config = self.alert_shown();
            match updated_config {
                Ok(_) => (),
                Err(err) => error!(
                    "Error saving seen alert event to telemetry config: {:?}",
                    err
                ),
            }
        }
        ()
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

    pub fn get_id(&self) -> &str {
        &self.config.telemetry_id
    }

    // setters
    pub fn enable(&mut self) -> Result<&TelemetryConfigContents, ConfigError> {
        self.config.telemetry_enabled = true;
        self.write()?;
        return Ok(&self.config);
    }

    pub fn disable(&mut self) -> Result<&TelemetryConfigContents, ConfigError> {
        self.config.telemetry_enabled = false;
        self.write()?;
        return Ok(&self.config);
    }

    pub fn alert_shown(&mut self) -> Result<&TelemetryConfigContents, ConfigError> {
        match self.has_seen_alert() {
            true => Ok(&self.config),
            false => {
                self.config.telemetry_alerted = Some(Utc::now());
                self.write()?;
                return Ok(&self.config);
            }
        }
    }
}
