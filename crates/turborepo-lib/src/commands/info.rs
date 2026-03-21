use std::{env, io, path::Path, process};

use sysinfo::{System, SystemExt};
use thiserror::Error;
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};
use turborepo_turbo_json::RawTurboJson;

use super::CommandBase;
use crate::{DaemonConnector, DaemonConnectorError};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Could not get path to `turbo` binary: {0}")]
    NoCurrentExe(#[from] io::Error),
}

// https://superuser.com/questions/1749781/how-can-i-check-if-the-environment-is-wsl-from-a-shell-script/1749811#1749811
fn is_wsl() -> bool {
    Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
}

fn read_workspace_providers(repo_root: &turbopath::AbsoluteSystemPath) -> Vec<String> {
    let root_turbo_json = repo_root.join_component("turbo.json");
    RawTurboJson::read(repo_root, &root_turbo_json, true)
        .ok()
        .flatten()
        .and_then(|raw| {
            raw.workspace_providers.map(|providers| {
                providers
                    .into_iter()
                    .map(|provider| provider.into_inner().into())
                    .collect::<Vec<String>>()
            })
        })
        .filter(|providers| !providers.is_empty())
        .unwrap_or_else(|| vec!["node".to_string()])
}

fn package_manager_display_for_providers(
    repo_root: &turbopath::AbsoluteSystemPath,
    workspace_providers: &[String],
) -> String {
    if workspace_providers
        .iter()
        .any(|provider| provider == "node")
    {
        PackageJson::load(&repo_root.join_component("package.json"))
            .ok()
            .and_then(|package_json| {
                PackageManager::read_or_detect_package_manager(&package_json, repo_root).ok()
            })
            .map_or_else(|| "Not found".to_owned(), |pm| pm.name().to_string())
    } else {
        "Not applicable (non-node workspace providers)".to_owned()
    }
}

pub async fn run(base: CommandBase) {
    let system = System::new_all();
    let connector = DaemonConnector::new(false, false, &base.repo_root, None);
    let daemon_status = match connector.connect().await {
        Ok(_status) => "Running",
        Err(DaemonConnectorError::NotRunning) => "Not running",
        Err(_e) => "Error getting status",
    };
    let workspace_providers = read_workspace_providers(&base.repo_root);
    let package_manager =
        package_manager_display_for_providers(&base.repo_root, &workspace_providers);

    println!("CLI:");
    println!("   Version: {}", base.version);

    let exe_path = std::env::current_exe().map_or_else(
        |e| format!("Cannot determine current binary: {e}").to_owned(),
        |path| path.to_string_lossy().into_owned(),
    );

    println!("   Path to executable: {exe_path}");
    println!("   Daemon status: {daemon_status}");
    println!("   Workspace providers: {}", workspace_providers.join(", "));
    println!("   Package manager: {package_manager}");
    println!();

    println!("Platform:");
    println!("   Architecture: {}", std::env::consts::ARCH);
    println!("   Operating system: {}", std::env::consts::OS);
    println!("   WSL: {}", is_wsl());
    println!(
        "   Available memory (MB): {}",
        system.available_memory() / 1024 / 1024
    );
    println!(
        "   Available CPU cores: {}",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    );
    println!();

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

    println!("Environment:");
    println!("   CI: {:#?}", turborepo_ci::Vendor::get_name());
    println!(
        "   AI agent: {}",
        turborepo_ai_agents::get_agent().unwrap_or("None")
    );
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
    println!("   Node.js version: {node_version}");
    println!();
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::{package_manager_display_for_providers, read_workspace_providers};

    #[test]
    fn reads_workspace_providers_defaulting_to_node() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        repo_root
            .join_component("turbo.json")
            .create_with_contents("{}")
            .unwrap();

        assert_eq!(
            read_workspace_providers(&repo_root),
            vec!["node".to_string()]
        );
    }

    #[test]
    fn package_manager_not_applicable_without_node_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        let providers = vec!["cargo".to_string(), "uv".to_string()];

        assert_eq!(
            package_manager_display_for_providers(&repo_root, &providers),
            "Not applicable (non-node workspace providers)"
        );
    }
}
