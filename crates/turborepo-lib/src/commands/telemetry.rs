use turborepo_telemetry::config::TelemetryConfig;
use turborepo_ui::{BOLD, BOLD_GREEN, BOLD_RED};

use super::CommandBase;
use crate::cli::TelemetryCommand;

fn log_status(config: TelemetryConfig, base: &CommandBase) {
    let status = config.is_enabled();
    match status {
        true => {
            println!(
                "\nStatus: {}",
                base.ui.apply(BOLD_GREEN.apply_to("Enabled"))
            );
            println!("\nTurborepo telemetry is completely anonymous. Thank you for participating!");
        }
        false => {
            println!("\nStatus: {}", base.ui.apply(BOLD_RED.apply_to("Disabled")));
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
        base.ui.apply(BOLD_RED.apply_to(message)),
        base.ui.apply(BOLD_RED.apply_to(error.to_string()))
    );
}

pub fn configure(command: &Option<Box<TelemetryCommand>>, base: &mut CommandBase) {
    let config = TelemetryConfig::new(base.ui);
    let mut config = match config {
        Ok(config) => config,
        Err(e) => {
            log_error("Failed to load telemetry config", &e.to_string(), base);
            return ();
        }
    };

    match command {
        Some(box TelemetryCommand::Enable) => {
            let result = config.enable();
            match result {
                Ok(_) => {
                    println!("{}", base.ui.apply(BOLD.apply_to("Success!")));
                    log_status(config, base);
                }
                Err(e) => log_error("Failed to enable telemetry", &e.to_string(), base),
            }
        }
        Some(box TelemetryCommand::Disable) => {
            let result = config.disable();
            match result {
                Ok(_) => {
                    println!("{}", base.ui.apply(BOLD.apply_to("Success!")));
                    log_status(config, base);
                }
                Err(e) => log_error("Failed to disable telemetry", &e.to_string(), base),
            }
        }
        _ => {
            log_status(config, base);
        }
    }

    ()
}
