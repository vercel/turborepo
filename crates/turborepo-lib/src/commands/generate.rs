use std::{
    io,
    process::{Command, Stdio},
};

use thiserror::Error;
use tracing::debug;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};
use turborepo_telemetry::events::command::CommandEventBuilder;
use which::which;

use crate::{
    child::spawn_child,
    cli::{GenerateCommand, GeneratorCustomArgs},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unable to run generate - missing requirements ({command}): {source}")]
    PackageManagerNotFound {
        command: &'static str,
        #[source]
        source: which::Error,
    },
    #[error("Failed to run package manager command: {0}")]
    PackageManagerCommandFailed(#[source] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, PartialEq, Eq)]
struct PackageManagerCommand {
    executable: &'static str,
    args: Vec<String>,
}

fn package_manager_for_generate(repo_root: &AbsoluteSystemPath) -> PackageManager {
    let package_json_path = repo_root.join_component("package.json");
    let detected = PackageJson::load(&package_json_path)
        .ok()
        .and_then(|package_json| {
            PackageManager::read_or_detect_package_manager(&package_json, repo_root).ok()
        })
        .or_else(|| PackageManager::detect_package_manager(repo_root).ok());

    detected.unwrap_or(PackageManager::Npm)
}

fn turbo_gen_command(package_manager: &PackageManager, tag: &str) -> PackageManagerCommand {
    let package = format!("@turbo/gen@{tag}");
    match package_manager {
        PackageManager::Npm | PackageManager::Yarn => PackageManagerCommand {
            executable: "npx",
            args: vec!["--yes".to_string(), package],
        },
        PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => {
            PackageManagerCommand {
                executable: "pnpm",
                args: vec!["dlx".to_string(), package],
            }
        }
        PackageManager::Berry => PackageManagerCommand {
            executable: "yarn",
            args: vec!["dlx".to_string(), package],
        },
        PackageManager::Bun => PackageManagerCommand {
            executable: "bun",
            args: vec!["x".to_string(), package],
        },
        PackageManager::Nub { .. } => PackageManagerCommand {
            executable: "nubx",
            args: vec![package],
        },
        PackageManager::Aube { .. } => PackageManagerCommand {
            executable: "aubx",
            args: vec![package],
        },
    }
}

fn call_turbo_gen(
    repo_root: &AbsoluteSystemPath,
    command: &str,
    tag: &String,
    raw_args: &str,
) -> Result<i32, Error> {
    let package_manager = package_manager_for_generate(repo_root);
    let package_manager_command = turbo_gen_command(&package_manager, tag);

    debug!(
        "Running @turbo/gen@{} with package manager `{}` command `{}` and args {:?}",
        tag,
        package_manager.name(),
        command,
        raw_args
    );
    let command_path = which(package_manager_command.executable).map_err(|source| {
        Error::PackageManagerNotFound {
            command: package_manager_command.executable,
            source,
        }
    })?;
    let mut package_manager_process = Command::new(command_path);
    package_manager_process
        .args(package_manager_command.args)
        .arg("raw")
        .arg(command)
        .args(["--json", raw_args])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let child = spawn_child(package_manager_process).map_err(Error::PackageManagerCommandFailed)?;
    let exit_code = child
        .wait()
        .map_err(Error::PackageManagerCommandFailed)?
        .code()
        .unwrap_or(2);
    Ok(exit_code)
}

pub fn run(
    repo_root: &AbsoluteSystemPath,
    tag: &String,
    command: &Option<Box<GenerateCommand>>,
    args: &GeneratorCustomArgs,
    telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    telemetry.track_generator_tag(tag);
    // check if a subcommand was passed
    if let Some(box GenerateCommand::Workspace(workspace_args)) = command {
        let raw_args = serde_json::to_string(&workspace_args)?;
        telemetry.track_generator_option("workspace");
        call_turbo_gen(repo_root, "workspace", tag, &raw_args)?;
    } else {
        // if no subcommand was passed, run the generate command as default
        let raw_args = serde_json::to_string(&args)?;
        telemetry.track_generator_option("run");
        call_turbo_gen(repo_root, "run", tag, &raw_args)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_pnpm_dlx_for_pnpm() {
        assert_eq!(
            turbo_gen_command(&PackageManager::Pnpm9, "1.2.3"),
            PackageManagerCommand {
                executable: "pnpm",
                args: vec!["dlx".to_string(), "@turbo/gen@1.2.3".to_string()],
            }
        );
    }

    #[test]
    fn uses_npx_for_npm() {
        assert_eq!(
            turbo_gen_command(&PackageManager::Npm, "1.2.3"),
            PackageManagerCommand {
                executable: "npx",
                args: vec!["--yes".to_string(), "@turbo/gen@1.2.3".to_string()],
            }
        );
    }

    #[test]
    fn detects_pnpm_from_dev_engines_package_manager() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::write(
            tempdir.path().join("package.json"),
            r#"{"devEngines":{"packageManager":{"name":"pnpm","version":"11.7.0"}}}"#,
        )
        .unwrap();
        std::fs::write(tempdir.path().join("pnpm-lock.yaml"), "").unwrap();
        let repo_root = turbopath::AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();

        assert_eq!(
            package_manager_for_generate(&repo_root),
            PackageManager::Pnpm9
        );
    }
}
