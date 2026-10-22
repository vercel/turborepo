use std::{
    env,
    io::{self, Write},
    path::Path,
    process,
};

use sysinfo::{RefreshKind, System, SystemExt};
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};

use super::{CommandBase, ignore_broken_pipe};
use crate::{DaemonConnector, DaemonConnectorError, cli};

// https://superuser.com/questions/1749781/how-can-i-check-if-the-environment-is-wsl-from-a-shell-script/1749811#1749811
fn is_wsl() -> bool {
    Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
}

pub async fn run(base: CommandBase) -> Result<(), cli::Error> {
    // Only available memory is reported. Refreshing everything would also
    // enumerate every process, disk, network interface, and sensor.
    let system = System::new_with_specifics(RefreshKind::new().with_memory());
    let daemon_status = match DaemonConnector::new(false, false, &base.repo_root, None) {
        Ok(connector) => match connector.connect().await {
            Ok(_status) => "Running",
            Err(DaemonConnectorError::NotRunning) => "Not running",
            Err(_e) => "Error getting status",
        },
        Err(_e) => "Error getting status",
    };
    let package_manager = PackageJson::load(&base.repo_root.join_component("package.json"))
        .ok()
        .and_then(|package_json| {
            PackageManager::read_or_detect_package_manager_info(&package_json, &base.repo_root)
                .ok()
                .map(|info| info.to_string())
        })
        .unwrap_or_else(|| "Not found".to_owned());

    ignore_broken_pipe(
        print_info(
            &mut io::stdout().lock(),
            &base,
            &system,
            daemon_status,
            &package_manager,
        )
        .map_err(Into::into),
    )
}

fn print_info(
    out: &mut impl Write,
    base: &CommandBase,
    system: &System,
    daemon_status: &str,
    package_manager: &str,
) -> io::Result<()> {
    writeln!(out, "CLI:")?;
    writeln!(out, "   Version: {}", base.version)?;

    let exe_path = std::env::current_exe().map_or_else(
        |e| format!("Cannot determine current binary: {e}").to_owned(),
        |path| path.to_string_lossy().into_owned(),
    );

    writeln!(out, "   Path to executable: {exe_path}")?;
    writeln!(out, "   Daemon status: {daemon_status}")?;
    writeln!(out, "   Package manager: {package_manager}")?;
    writeln!(out)?;

    writeln!(out, "Platform:")?;
    writeln!(out, "   Architecture: {}", std::env::consts::ARCH)?;
    writeln!(out, "   Operating system: {}", std::env::consts::OS)?;
    writeln!(out, "   WSL: {}", is_wsl())?;
    writeln!(
        out,
        "   Available memory (MB): {}",
        system.available_memory() / 1024 / 1024
    )?;
    writeln!(
        out,
        "   Available CPU cores: {}",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    )?;
    writeln!(out)?;

    let node_version = process::Command::new("node")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| {
            output
                .status
                .success()
                .then(|| String::from_utf8(output.stdout).ok())
                .flatten()
                .map(|v| v.trim().to_owned())
        })
        .unwrap_or_else(|| "Not found".to_owned());

    writeln!(out, "Environment:")?;
    writeln!(out, "   CI: {:#?}", turborepo_ci::Vendor::get_name())?;
    writeln!(
        out,
        "   AI agent: {}",
        turborepo_ai_agents::get_agent().unwrap_or("None")
    )?;
    writeln!(
        out,
        "   Terminal (TERM): {}",
        env::var("TERM").unwrap_or_else(|_| "unknown".to_owned())
    )?;

    writeln!(
        out,
        "   Terminal program (TERM_PROGRAM): {}",
        env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_owned())
    )?;
    writeln!(
        out,
        "   Terminal program version (TERM_PROGRAM_VERSION): {}",
        env::var("TERM_PROGRAM_VERSION").unwrap_or_else(|_| "unknown".to_owned())
    )?;
    writeln!(
        out,
        "   Shell (SHELL): {}",
        env::var("SHELL").unwrap_or_else(|_| "unknown".to_owned())
    )?;
    writeln!(out, "   stdin: {}", turborepo_ci::is_ci())?;
    writeln!(out, "   Node.js version: {node_version}")?;
    writeln!(out)
}
