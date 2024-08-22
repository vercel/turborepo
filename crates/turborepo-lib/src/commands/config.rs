use camino::Utf8Path;
use serde::Serialize;
use turborepo_repository::{
    package_graph::PackageGraph, package_json::PackageJson, package_manager::PackageManager,
};

use crate::{cli, cli::EnvMode, commands::CommandBase, turbo_json::UIMode};

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
    spaces_id: Option<&'a str>,
    ui: UIMode,
    package_manager: PackageManager,
    daemon: Option<bool>,
    env_mode: EnvMode,
    scm_base: Option<&'a str>,
    scm_head: &'a str,
    cache_dir: &'a Utf8Path,
}

pub async fn run(base: CommandBase) -> Result<(), cli::Error> {
    let config = base.config()?;
    let root_package_json = PackageJson::load(&base.repo_root.join_component("package.json"))?;

    let package_graph = PackageGraph::builder(&base.repo_root, root_package_json)
        .build()
        .await?;

    let package_manager = package_graph.package_manager();

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
            spaces_id: config.spaces_id(),
            ui: config.ui(),
            package_manager: *package_manager,
            daemon: config.daemon,
            env_mode: config.env_mode(),
            scm_base: config.scm_base(),
            scm_head: config.scm_head(),
            cache_dir: config.cache_dir(),
        })?
    );
    Ok(())
}
