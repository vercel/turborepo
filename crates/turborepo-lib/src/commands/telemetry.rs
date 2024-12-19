use turborepo_telemetry::{config::TelemetryConfig, events::command::CommandEventBuilder};
use turborepo_ui::{color, BOLD, BOLD_GREEN, BOLD_RED};

use super::CommandBase;
use crate::cli::TelemetryCommand;

fn log_status(config: TelemetryConfig, base: &CommandBase) {
    let status = config.is_enabled();
    match status {
        true => {
            println!(
                "\nStatus: {}",
                base.color_config.apply(BOLD_GREEN.apply_to("Enabled"))
            );
            println!("\nTurborepo telemetry is completely anonymous. Thank you for participating!");
        }
        false => {
            println!(
                "\nStatus: {}",
                base.color_config.apply(BOLD_RED.apply_to("Disabled"))
            );
            println!(
                "\nYou have opted-out of Turborepo anonymous telemetry. No data will be collected \
                 from your machine."
            );
        }
    }
    println!("Learn more: https://turbo.build/repo/docs/telemetry");
}

fn log_error(message: &str, error: &str, base: &CommandBase) {
    println!(
        "{}: {}",
        color!(base.color_config, BOLD_RED, "{}", message),
        color!(base.color_config, BOLD_RED, "{}", error)
    );
}

pub fn configure(
    command: &Option<TelemetryCommand>,
    base: &mut CommandBase,
    telemetry: CommandEventBuilder,
) {
    let config = TelemetryConfig::with_default_config_path();
    let mut config = match config {
        Ok(config) => config,
        Err(e) => {
            log_error("Failed to load telemetry config", &e.to_string(), base);
            return;
        }
    };

    match command {
        Some(TelemetryCommand::Enable) => {
            let result = config.enable();
            match result {
                Ok(_) => {
                    println!("{}", color!(base.color_config, BOLD, "{}", "Success!"));
                    log_status(config, base);
                    telemetry.track_telemetry_config(true);
                }
                Err(e) => log_error("Failed to enable telemetry", &e.to_string(), base),
            }
        }
        Some(TelemetryCommand::Disable) => {
            let result = config.disable();
            match result {
                Ok(_) => {
                    println!("{}", color!(base.color_config, BOLD, "{}", "Success!"));
                    log_status(config, base);
                    telemetry.track_telemetry_config(false);
                }
                Err(e) => log_error("Failed to disable telemetry", &e.to_string(), base),
            }
        }
        _ => {
            log_status(config, base);
        }
    }
}
