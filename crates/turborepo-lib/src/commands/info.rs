use std::{env, io, path::Path};

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

// https://superuser.com/questions/1749781/how-can-i-check-if-the-environment-is-wsl-from-a-shell-script/1749811#1749811
fn is_wsl() -> bool {
    Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
}

pub async fn run(base: CommandBase) {
    let system = System::new_all();
    let connector = DaemonConnector::new(false, false, &base.repo_root);
    let daemon_status = match connector.connect().await {
        Ok(_status) => "Running",
        Err(DaemonConnectorError::NotRunning) => "Not running",
        Err(_e) => "Error getting status",
    };
    let package_manager = PackageJson::load(&base.repo_root.join_component("package.json"))
        .ok()
        .and_then(|package_json| {
            PackageManager::read_or_detect_package_manager(&package_json, &base.repo_root).ok()
        })
        .map_or_else(|| "Not found".to_owned(), |pm| pm.to_string());

    println!("CLI:");
    println!("   Version: {}", base.version);

    let exe_path = std::env::current_exe().map_or_else(
        |e| format!("Cannot determine current binary: {e}").to_owned(),
        |path| path.to_string_lossy().into_owned(),
    );

    println!("   Path to executable: {}", exe_path);
    println!("   Daemon status: {}", daemon_status);
    println!("   Package manager: {}", package_manager);
    println!();

    println!("Platform:");
    println!("   Architecture: {}", std::env::consts::ARCH);
    println!("   Operating system: {}", std::env::consts::OS);
    println!("   WSL: {}", is_wsl());
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
}
