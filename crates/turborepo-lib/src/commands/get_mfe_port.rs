use std::io;

use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_microfrontends::TurborepoMfeConfig;
use turborepo_repository::{package_graph::PackageGraphBuilder, package_json::PackageJson};

use crate::{commands::CommandBase, microfrontends::MicrofrontendsConfigs};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get current working directory: {0}")]
    Cwd(#[from] turbopath::PathError),
    #[error("No package.json found in current directory")]
    NoPackageJson,
    #[error("package.json is missing the 'name' field")]
    NoPackageName,
    #[error("Failed to read package.json: {0}")]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error("Failed to build package graph: {0}")]
    PackageGraph(#[from] turborepo_repository::package_graph::Error),
    #[error("Failed to load microfrontends configuration: {0}")]
    MicrofrontendsConfig(#[from] turborepo_microfrontends::Error),
    #[error("No microfrontends configuration found")]
    NoMicrofrontendsConfig,
    #[error("Failed to read microfrontends configuration file: {0}")]
    ConfigFileRead(#[from] io::Error),
    #[error("Package '{0}' not found in microfrontends configuration")]
    PackageNotInConfig(String),
}

pub async fn run(base: &CommandBase) -> Result<(), Error> {
    let port = get_port_for_current_package(base).await?;

    // Output just the port number
    println!("{}", port);

    Ok(())
}

// Extracted logic for testing
async fn get_port_for_current_package(base: &CommandBase) -> Result<u16, Error> {
    // Get the current working directory
    let cwd = AbsoluteSystemPathBuf::cwd()?;

    // Read package.json from current directory to get package name
    let package_json_path = cwd.join_component("package.json");
    let package_json = PackageJson::load(&package_json_path).map_err(|_| Error::NoPackageJson)?;
    let package_name = package_json.name.as_deref().ok_or(Error::NoPackageName)?;

    get_port_for_package(base, package_name).await
}

async fn get_port_for_package(base: &CommandBase, package_name: &str) -> Result<u16, Error> {
    // Build package graph to find microfrontends config
    let repo_root = &base.repo_root;
    let root_package_json_path = repo_root.join_component("package.json");
    let root_package_json = PackageJson::load(&root_package_json_path)?;

    let package_graph = PackageGraphBuilder::new(repo_root, root_package_json)
        .with_single_package_mode(false)
        .build()
        .await?;

    // Load microfrontends configuration to find the config file
    let mfe_configs = MicrofrontendsConfigs::from_disk(repo_root, &package_graph)?
        .ok_or(Error::NoMicrofrontendsConfig)?;

    // Find the config file path
    let config_path = mfe_configs
        .configs()
        .find_map(|(pkg, _)| mfe_configs.config_filename(pkg))
        .ok_or(Error::NoMicrofrontendsConfig)?;

    // Load the actual TurborepoMfeConfig
    let full_path = repo_root.join_unix_path(config_path);
    let contents = std::fs::read_to_string(&full_path)?;
    let config = TurborepoMfeConfig::from_str(&contents, full_path.as_str())?;

    // Get port for the current package
    let port = config
        .port(package_name)
        .ok_or_else(|| Error::PackageNotInConfig(package_name.to_string()))?;

    Ok(port)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use turborepo_ui::ColorConfig;

    use super::*;
    use crate::{config::TurborepoConfigBuilder, opts::Opts, Args};

    fn setup_test_repo(tmp: &TempDir) -> AbsoluteSystemPathBuf {
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path().to_path_buf()).unwrap();

        // Create root package.json
        repo_root
            .join_component("package.json")
            .create_with_contents(
                r#"{
                "name": "root",
                "packageManager": "pnpm@9.0.0",
                "workspaces": ["apps/*"]
            }"#,
            )
            .unwrap();

        // Create pnpm-workspace.yaml
        repo_root
            .join_component("pnpm-workspace.yaml")
            .create_with_contents("packages:\n  - 'apps/*'\n")
            .unwrap();

        // Create turbo.json
        repo_root
            .join_component("turbo.json")
            .create_with_contents(r#"{"$schema": "https://turbo.build/schema.json"}"#)
            .unwrap();

        repo_root
    }

    fn create_command_base(repo_root: AbsoluteSystemPathBuf) -> CommandBase {
        let args = Args::default();
        let config = TurborepoConfigBuilder::new(&repo_root).build().unwrap();
        let opts = Opts::new(&repo_root, &args, config).unwrap();

        CommandBase::from_opts(opts, repo_root, "test-version", ColorConfig::new(false))
    }

    #[tokio::test]
    async fn test_get_port_with_explicit_port() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);

        // Create app with explicit port
        let app_dir = repo_root.join_components(&["apps", "web"]);
        app_dir.create_dir_all().unwrap();

        app_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "web"}"#)
            .unwrap();

        app_dir
            .join_component("microfrontends.json")
            .create_with_contents(
                r#"{
                "version": "1",
                "applications": {
                    "web": {
                        "development": {
                            "local": {
                                "port": 3001
                            }
                        }
                    }
                }
            }"#,
            )
            .unwrap();

        let base = create_command_base(repo_root);
        let port = get_port_for_package(&base, "web").await.unwrap();

        assert_eq!(port, 3001);
    }

    #[tokio::test]
    async fn test_get_port_with_auto_generated_port() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);

        // Create app without explicit port
        let app_dir = repo_root.join_components(&["apps", "web"]);
        app_dir.create_dir_all().unwrap();

        app_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "web"}"#)
            .unwrap();

        app_dir
            .join_component("microfrontends.json")
            .create_with_contents(
                r#"{
                "version": "1",
                "applications": {
                    "web": {
                        "development": {
                            "local": {}
                        }
                    }
                }
            }"#,
            )
            .unwrap();

        let base = create_command_base(repo_root);
        let port = get_port_for_package(&base, "web").await.unwrap();

        // Port should be deterministically generated from "web"
        // Based on the hash function in the microfrontends crate
        assert!((3000..=8000).contains(&port));

        // Verify it's deterministic - calling again should return same port
        let port2 = get_port_for_package(&base, "web").await.unwrap();
        assert_eq!(port, port2);
    }

    #[tokio::test]
    async fn test_get_port_multiple_apps() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);

        // Create multiple apps
        let web_dir = repo_root.join_components(&["apps", "web"]);
        web_dir.create_dir_all().unwrap();
        web_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "web"}"#)
            .unwrap();

        let docs_dir = repo_root.join_components(&["apps", "docs"]);
        docs_dir.create_dir_all().unwrap();
        docs_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "docs"}"#)
            .unwrap();

        // Config in web app with routing
        web_dir
            .join_component("microfrontends.json")
            .create_with_contents(
                r#"{
                "version": "1",
                "applications": {
                    "web": {
                        "development": {
                            "local": {
                                "port": 3001
                            }
                        }
                    },
                    "docs": {
                        "packageName": "docs",
                        "development": {
                            "local": {
                                "port": 4000
                            }
                        },
                        "routing": [{"paths": ["/docs"]}]
                    }
                }
            }"#,
            )
            .unwrap();

        let base = create_command_base(repo_root);

        let web_port = get_port_for_package(&base, "web").await.unwrap();
        assert_eq!(web_port, 3001);

        let docs_port = get_port_for_package(&base, "docs").await.unwrap();
        assert_eq!(docs_port, 4000);
    }

    #[tokio::test]
    async fn test_error_no_microfrontends_config() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);

        // Create app without microfrontends.json
        let app_dir = repo_root.join_components(&["apps", "web"]);
        app_dir.create_dir_all().unwrap();
        app_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "web"}"#)
            .unwrap();

        let base = create_command_base(repo_root);
        let result = get_port_for_package(&base, "web").await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::NoMicrofrontendsConfig));
    }

    #[tokio::test]
    async fn test_error_package_not_in_config() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);

        // Create web app with config
        let web_dir = repo_root.join_components(&["apps", "web"]);
        web_dir.create_dir_all().unwrap();
        web_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "web"}"#)
            .unwrap();
        web_dir
            .join_component("microfrontends.json")
            .create_with_contents(
                r#"{
                "version": "1",
                "applications": {
                    "web": {
                        "development": {
                            "local": {
                                "port": 3001
                            }
                        }
                    }
                }
            }"#,
            )
            .unwrap();

        // Create docs app without it in config
        let docs_dir = repo_root.join_components(&["apps", "docs"]);
        docs_dir.create_dir_all().unwrap();
        docs_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "docs"}"#)
            .unwrap();

        let base = create_command_base(repo_root);
        let result = get_port_for_package(&base, "docs").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::PackageNotInConfig(pkg) => assert_eq!(pkg, "docs"),
            _ => panic!("Expected PackageNotInConfig error"),
        }
    }

    #[test]
    fn test_error_display() {
        let err = Error::NoPackageJson;
        assert_eq!(
            err.to_string(),
            "No package.json found in current directory"
        );

        let err = Error::NoPackageName;
        assert_eq!(err.to_string(), "package.json is missing the 'name' field");

        let err = Error::NoMicrofrontendsConfig;
        assert_eq!(err.to_string(), "No microfrontends configuration found");

        let err = Error::PackageNotInConfig("my-app".to_string());
        assert_eq!(
            err.to_string(),
            "Package 'my-app' not found in microfrontends configuration"
        );
    }
}
