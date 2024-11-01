use std::{env, io};

use sysinfo::{System, SystemExt};
use thiserror::Error;
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};

use super::CommandBase;
use crate::{DaemonConnector, DaemonConnectorError};

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not get path to turbo binary: {0}")]
    NoCurrentExe(#[from] io::Error),
}

pub async fn run(base: CommandBase) -> Result<(), Error> {
    let system = System::new_all();
    let connector = DaemonConnector::new(false, false, &base.repo_root);
    let daemon_status = match connector.connect().await {
        Ok(_status) => "Running",
        Err(DaemonConnectorError::NotRunning) => "Not running",
        Err(_e) => "Error getting status",
    };

    let package_manager = match PackageJson::load(&base.repo_root.join_component("package.json")) {
        Ok(package_json) => {
            match PackageManager::read_or_detect_package_manager(&package_json, &base.repo_root) {
                Ok(pm) => pm.to_string(),
                Err(_) => "Not found".to_owned(),
            }
        }
        Err(_) => "Not found".to_owned(),
    };

    println!("CLI:");
    println!("   Version: {}", base.version);
    println!(
        "   Location: {}",
        std::env::current_exe()?.to_string_lossy()
    );
    println!("   Daemon status: {}", daemon_status);
    println!("   Package manager: {}", package_manager);
    println!();

    println!("Platform:");
    println!("   Architecture: {}", std::env::consts::ARCH);
    println!("   Operating system: {}", std::env::consts::OS);
    println!(
        "   Available memory (MB): {}",
        system.available_memory() / 1024 / 1024
    );
    println!("   Available CPU cores: {}", num_cpus::get());
    println!();

    println!("Environment:");
    println!("   CI: {:#?}", turborepo_ci::Vendor::get_name());
    println!(
        "   Terminal (TERM): {}",
        env::var("TERM").unwrap_or_else(|_| "unknown".to_owned())
    );

    println!(
        "   Terminal program (TERM_PROGRAM): {}",
        env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!(
        "   Terminal program version (TERM_PROGRAM_VERSION): {}",
        env::var("TERM_PROGRAM_VERSION").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!(
        "   Shell (SHELL): {}",
        env::var("SHELL").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!("   stdin: {}", turborepo_ci::is_ci());
    println!();

    println!("Turborepo System Environment Variables:");
    for (key, value) in env::vars() {
        // Don't print sensitive information
        if key == "TURBO_TEAM"
            || key == "TURBO_TEAMID"
            || key == "TURBO_TOKEN"
            || key == "TURBO_API"
        {
            continue;
        }
        if key.starts_with("TURBO_") {
            println!("   {}: {}", key, value);
        }
    }

    Ok(())
}
