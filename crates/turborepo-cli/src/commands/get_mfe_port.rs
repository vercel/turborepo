use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_microfrontends_config::{
    MicrofrontendsConfigs,
    port::{PortResolutionError, resolve_port_for_directory},
};
use turborepo_repository::package_graph::PackageGraph;

use crate::commands::CommandBase;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get current working directory: {0}")]
    Cwd(#[from] turbopath::PathError),
    #[error("Failed to read package.json: {0}")]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error("Failed to build package graph: {0}")]
    PackageGraph(#[from] turborepo_repository::package_graph::Error),
    #[error("Failed to load microfrontends configuration: {0}")]
    MicrofrontendsConfig(#[from] turborepo_microfrontends::Error),
    #[error("No microfrontends configuration found")]
    NoMicrofrontendsConfig,
    #[error(transparent)]
    PortResolution(#[from] PortResolutionError),
}

pub async fn run(base: &CommandBase) -> Result<(), Error> {
    let port = get_port_for_current_package(base).await?;

    // Output just the port number
    println!("{}", port);

    Ok(())
}

async fn get_port_for_current_package(base: &CommandBase) -> Result<u16, Error> {
    let cwd = AbsoluteSystemPathBuf::cwd()?;
    get_port_for_current_package_at(base, &cwd).await
}

async fn build_package_graph(base: &CommandBase) -> Result<PackageGraph, Error> {
    let repo_root = &base.repo_root;
    let features = turborepo_package_watcher::repository_graph::RepositoryGraphFeatures::new(
        &base.opts().future_flags,
    );
    let root_package_json = features.load_root_package_json(repo_root)?;

    let builder = PackageGraph::builder_optional(repo_root, root_package_json)
        .with_single_package_mode(base.opts().run_opts.single_package)
        .with_allow_no_package_manager(base.opts().repo_opts.allow_no_package_manager);

    Ok(features.configure(builder).build().await?)
}

fn load_microfrontends_configs(
    base: &CommandBase,
    package_graph: &PackageGraph,
) -> Result<MicrofrontendsConfigs, Error> {
    MicrofrontendsConfigs::from_disk(&base.repo_root, package_graph)?
        .ok_or(Error::NoMicrofrontendsConfig)
}

async fn get_port_for_current_package_at(
    base: &CommandBase,
    cwd: &AbsoluteSystemPath,
) -> Result<u16, Error> {
    let package_graph = build_package_graph(base).await?;
    let mfe_configs = load_microfrontends_configs(base, &package_graph)?;

    Ok(resolve_port_for_directory(
        &package_graph,
        &base.repo_root,
        cwd,
        &mfe_configs,
    )?)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use turborepo_run_opts::Opts;
    use turborepo_ui::ColorConfig;

    use super::*;
    use crate::config::TurborepoConfigBuilder;

    fn setup_test_repo(tmp: &TempDir) -> AbsoluteSystemPathBuf {
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path().to_path_buf()).unwrap();

        repo_root
            .join_component("package.json")
            .create_with_contents(
                r#"{
                "name": "root",
                "packageManager": "pnpm@9.0.0",
                "workspaces": ["apps/*", "apps/*/packages/*"]
            }"#,
            )
            .unwrap();
        repo_root
            .join_component("pnpm-workspace.yaml")
            .create_with_contents("packages:\n  - 'apps/*'\n  - 'apps/*/packages/*'\n")
            .unwrap();
        repo_root
            .join_component("turbo.json")
            .create_with_contents(r#"{"$schema": "https://turbo.build/schema.json"}"#)
            .unwrap();

        repo_root
    }

    fn create_command_base(repo_root: AbsoluteSystemPathBuf) -> CommandBase {
        let config = TurborepoConfigBuilder::new(&repo_root).build().unwrap();
        let opts = Opts::new(&repo_root, &Default::default(), &Default::default(), config).unwrap();

        CommandBase::from_opts(opts, repo_root, "test-version", ColorConfig::new(false))
    }

    #[tokio::test]
    async fn cli_wires_current_directory_to_the_configured_port() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);
        let app_dir = repo_root.join_components(&["apps", "web"]);
        app_dir.join_component("src").create_dir_all().unwrap();
        app_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name":"web"}"#)
            .unwrap();
        app_dir
            .join_component("microfrontends.json")
            .create_with_contents(
                r#"{"version":"1","applications":{"web":{"development":{"local":3001}}}}"#,
            )
            .unwrap();
        let base = create_command_base(repo_root);

        assert_eq!(
            get_port_for_current_package_at(&base, &app_dir.join_component("src"))
                .await
                .unwrap(),
            3001
        );
    }
}
