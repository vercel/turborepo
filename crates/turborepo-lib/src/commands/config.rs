use camino::Utf8Path;
use serde::Serialize;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};
use turborepo_turbo_json::RawTurboJson;
use turborepo_types::{EnvMode, UIMode};

use crate::{cli, commands::CommandBase, Args};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigOutput<'a> {
    api_url: &'a str,
    login_url: &'a str,
    team_slug: Option<&'a str>,
    team_id: Option<&'a str>,
    signature: bool,
    preflight: bool,
    timeout: u64,
    upload_timeout: u64,
    enabled: bool,
    ui: UIMode,
    workspace_providers: Vec<String>,
    package_manager: String,
    daemon: Option<bool>,
    env_mode: EnvMode,
    scm_base: Option<&'a str>,
    scm_head: Option<&'a str>,
    cache_dir: &'a Utf8Path,
    concurrency: Option<&'a str>,
}

pub async fn run(repo_root: AbsoluteSystemPathBuf, args: Args) -> Result<(), cli::Error> {
    let config = CommandBase::load_config(&repo_root, &args)?;
    let root_turbo_json = repo_root.join_component("turbo.json");
    let workspace_providers = RawTurboJson::read(&repo_root, &root_turbo_json, true)
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
        .unwrap_or_else(|| vec!["node".to_string()]);

    let package_manager = if workspace_providers
        .iter()
        .any(|provider| provider == "node")
    {
        PackageJson::load(&repo_root.join_component("package.json"))
            .ok()
            .and_then(|package_json| {
                PackageManager::read_or_detect_package_manager(&package_json, &repo_root).ok()
            })
            .map_or_else(
                || "not-found".to_string(),
                |package_manager| package_manager.name().to_string(),
            )
    } else {
        "not-applicable".to_string()
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&ConfigOutput {
            api_url: config.api_url(),
            login_url: config.login_url(),
            team_slug: config.team_slug(),
            team_id: config.team_id(),
            signature: config.signature(),
            preflight: config.preflight(),
            timeout: config.timeout(),
            upload_timeout: config.upload_timeout(),
            enabled: config.enabled(),
            ui: config.ui(),
            workspace_providers,
            package_manager,
            daemon: config.daemon,
            env_mode: config.env_mode(),
            scm_base: config.scm_base(),
            scm_head: config.scm_head(),
            cache_dir: config.cache_dir(),
            concurrency: config.concurrency.as_deref()
        })?
    );
    Ok(())
}
