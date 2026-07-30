use std::{io, io::ErrorKind};

use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_microfrontends::TurborepoMfeConfig;
use turborepo_repository::{
    cargo::CARGO_TOML,
    package_graph::{PackageGraph, PackageGraphNodeKind, PackageName, PackageNode},
    package_json::PackageJson,
};

use crate::{
    commands::CommandBase, microfrontends::MicrofrontendsConfigs, run::builder::cargo_enabled,
};

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
    let cwd = AbsoluteSystemPathBuf::cwd()?;
    get_port_for_current_package_at(base, &cwd).await
}

async fn build_package_graph(base: &CommandBase) -> Result<PackageGraph, Error> {
    let repo_root = &base.repo_root;
    let root_package_json_path = repo_root.join_component("package.json");
    let cargo_enabled = cargo_enabled(&base.opts().future_flags);
    let root_package_json = match PackageJson::load(&root_package_json_path) {
        Ok(package_json) => Some(package_json),
        Err(turborepo_repository::package_json::Error::Io(io))
            if io.kind() == ErrorKind::NotFound
                && cargo_enabled
                && repo_root.join_component(CARGO_TOML).exists() =>
        {
            None
        }
        Err(error) => return Err(error.into()),
    };

    let mut builder = PackageGraph::builder_optional(repo_root, root_package_json)
        .with_single_package_mode(base.opts().run_opts.single_package)
        .with_allow_no_package_manager(base.opts().repo_opts.allow_no_package_manager);
    if cargo_enabled {
        builder = builder.with_cargo();
    }

    Ok(builder.build().await?)
}

/// Resolve directory ownership from authoritative repository knowledge. The
/// deepest package directory wins, so a nested package owns its descendants
/// instead of inheriting the enclosing package's identity.
fn package_for_directory(
    package_graph: &PackageGraph,
    repo_root: &AbsoluteSystemPath,
    cwd: &AbsoluteSystemPath,
) -> Result<String, Error> {
    let cwd = repo_root.anchor(cwd).map_err(|_| Error::NoPackageJson)?;
    let owner = package_graph
        .node_views()
        .filter_map(|(node, view)| {
            let directory = view.directory()?;
            if view.kind() == PackageGraphNodeKind::RootJavaScript && !cwd.as_str().is_empty() {
                return None;
            }
            let component_count = directory.components().count();
            let is_package_json_scope = view.is_package_json_scope();
            cwd.strip_prefix(directory)
                .map(|_| (component_count, is_package_json_scope, node, view))
        })
        .max_by_key(|(component_count, is_package_json_scope, _, _)| {
            (*component_count, *is_package_json_scope)
        })
        .ok_or(Error::NoPackageJson)?;

    match owner {
        (_, _, PackageNode::Workspace(PackageName::Other(name)), view)
            if view.is_package_json_scope() =>
        {
            Ok(name)
        }
        (_, _, PackageNode::Workspace(PackageName::Root), view) if view.is_package_json_scope() => {
            package_graph
                .root_javascript_scope_name()
                .flatten()
                .map(str::to_owned)
                .ok_or(Error::NoPackageName)
        }
        _ => Err(Error::NoPackageJson),
    }
}

fn check_exact_cwd_manifest(
    repo_root: &AbsoluteSystemPath,
    cwd: &AbsoluteSystemPath,
) -> Result<(), Error> {
    let Ok(relative) = repo_root.anchor(cwd) else {
        return Err(Error::NoPackageJson);
    };
    let manifest = cwd.join_component("package.json");
    if manifest.exists() || relative.as_str().is_empty() {
        let package_json = PackageJson::load(&manifest).map_err(|_| Error::NoPackageJson)?;
        if package_json.name.is_none() {
            return Err(Error::NoPackageName);
        }
    }
    Ok(())
}

async fn get_port_for_current_package_at(
    base: &CommandBase,
    cwd: &AbsoluteSystemPath,
) -> Result<u16, Error> {
    check_exact_cwd_manifest(&base.repo_root, cwd)?;
    let package_graph = build_package_graph(base).await?;
    let package_name = package_for_directory(&package_graph, &base.repo_root, cwd)?;
    get_port_from_graph(base, &package_graph, &package_name)
}

async fn get_port_for_package(base: &CommandBase, package_name: &str) -> Result<u16, Error> {
    let package_graph = build_package_graph(base).await?;
    get_port_from_graph(base, &package_graph, package_name)
}

fn get_port_from_graph(
    base: &CommandBase,
    package_graph: &PackageGraph,
    package_name: &str,
) -> Result<u16, Error> {
    let repo_root = &base.repo_root;

    // Load microfrontends configuration to find the config file
    let mfe_configs = MicrofrontendsConfigs::from_disk(repo_root, package_graph)?
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
                "workspaces": ["apps/*", "apps/*/packages/*"]
            }"#,
            )
            .unwrap();

        // Create pnpm-workspace.yaml
        repo_root
            .join_component("pnpm-workspace.yaml")
            .create_with_contents("packages:\n  - 'apps/*'\n  - 'apps/*/packages/*'\n")
            .unwrap();

        // Create turbo.json
        repo_root
            .join_component("turbo.json")
            .create_with_contents(r#"{"$schema": "https://turbo.build/schema.json"}"#)
            .unwrap();

        repo_root
    }

    fn create_command_base(repo_root: AbsoluteSystemPathBuf) -> CommandBase {
        create_command_base_with_cargo(repo_root, false)
    }

    fn create_command_base_with_cargo(
        repo_root: AbsoluteSystemPathBuf,
        cargo_enabled: bool,
    ) -> CommandBase {
        let args = Args::default();
        let config = TurborepoConfigBuilder::new(&repo_root).build().unwrap();
        let mut opts = Opts::new(&repo_root, &args, config).unwrap();
        opts.future_flags.experimental_cargo_workspaces = cargo_enabled;

        CommandBase::from_opts(opts, repo_root, "test-version", ColorConfig::new(false))
    }

    fn add_cargo_workspace(repo_root: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
        repo_root
            .join_component("Cargo.toml")
            .create_with_contents(
                "[workspace]\nmembers = [\"rust/member\"]\nresolver = \
                 \"2\"\n\n[workspace.metadata]\nname = \"cargo-workspace\"\n",
            )
            .unwrap();
        let member = repo_root.join_components(&["rust", "member"]);
        member.join_component("src").create_dir_all().unwrap();
        member
            .join_component("Cargo.toml")
            .create_with_contents(
                "[package]\nname = \"cargo-member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .unwrap();
        member
            .join_components(&["src", "lib.rs"])
            .create_with_contents("")
            .unwrap();
        repo_root
            .join_component("Cargo.lock")
            .create_with_contents(
                "version = 4\n\n[[package]]\nname = \"cargo-member\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
        member
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
    async fn test_get_port_with_package_name_different_from_app_key() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);

        let app_dir = repo_root.join_components(&["apps", "my-app"]);
        app_dir.create_dir_all().unwrap();

        app_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name": "my-app"}"#)
            .unwrap();

        app_dir
            .join_component("microfrontends.json")
            .create_with_contents(
                r#"{
                "version": "1",
                "applications": {
                    "my-vercel-project": {
                        "packageName": "my-app",
                        "development": {
                            "local": 3005
                        }
                    }
                }
            }"#,
            )
            .unwrap();

        let base = create_command_base(repo_root);

        let port = get_port_for_package(&base, "my-app").await.unwrap();
        assert_eq!(port, 3005);
    }

    #[tokio::test]
    async fn test_current_directory_inside_package_uses_directory_owner() {
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
                r#"{"version":"1","applications":{"web":{"development":{"local":3010}}}}"#,
            )
            .unwrap();
        let base = create_command_base(repo_root);

        let port = get_port_for_current_package_at(&base, &app_dir.join_component("src"))
            .await
            .unwrap();

        assert_eq!(port, 3010);
    }

    #[tokio::test]
    async fn test_nested_package_wins_longest_directory_prefix() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);
        let shell_dir = repo_root.join_components(&["apps", "shell"]);
        let widget_dir = shell_dir.join_components(&["packages", "widget"]);
        widget_dir.join_component("src").create_dir_all().unwrap();
        shell_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name":"shell"}"#)
            .unwrap();
        widget_dir
            .join_component("package.json")
            .create_with_contents(r#"{"name":"widget"}"#)
            .unwrap();
        shell_dir
            .join_component("microfrontends.json")
            .create_with_contents(
                r#"{"version":"1","applications":{"shell":{"development":{"local":3010}},"widget":{"packageName":"widget","development":{"local":4020},"routing":[{"paths":["/widget"]}]}}}"#,
            )
            .unwrap();
        let base = create_command_base(repo_root);

        let port = get_port_for_current_package_at(&base, &widget_dir.join_component("src"))
            .await
            .unwrap();

        assert_eq!(port, 4020);
    }

    #[tokio::test]
    async fn test_root_scope_only_owns_exact_repository_root() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);
        repo_root
            .join_component("microfrontends.json")
            .create_with_contents(
                r#"{"version":"1","applications":{"root":{"development":{"local":3007}}}}"#,
            )
            .unwrap();
        let unowned = repo_root.join_component("tools");
        unowned.create_dir_all().unwrap();
        let base = create_command_base(repo_root.clone());

        assert_eq!(
            get_port_for_current_package_at(&base, &repo_root)
                .await
                .unwrap(),
            3007
        );
        assert!(matches!(
            get_port_for_current_package_at(&base, &unowned).await,
            Err(Error::NoPackageJson)
        ));

        let outside = TempDir::new().unwrap();
        let outside = AbsoluteSystemPathBuf::try_from(outside.path().to_path_buf()).unwrap();
        assert!(matches!(
            get_port_for_current_package_at(&base, &outside).await,
            Err(Error::NoPackageJson)
        ));
    }

    #[tokio::test]
    async fn test_exact_unnamed_and_malformed_workspace_errors_precede_graph_identity() {
        for (manifest, expected_unnamed) in [(r#"{}"#, true), ("{", false)] {
            let tmp = TempDir::new().unwrap();
            let repo_root = setup_test_repo(&tmp);
            let app = repo_root.join_components(&["apps", "web"]);
            app.create_dir_all().unwrap();
            app.join_component("package.json")
                .create_with_contents(manifest)
                .unwrap();
            let base = create_command_base(repo_root);

            let error = get_port_for_current_package_at(&base, &app)
                .await
                .unwrap_err();
            assert!(if expected_unnamed {
                matches!(error, Error::NoPackageName)
            } else {
                matches!(error, Error::NoPackageJson)
            });
        }
    }

    #[tokio::test]
    async fn test_cargo_package_is_not_owned_by_javascript_root() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);
        let cargo_member = add_cargo_workspace(&repo_root);
        let base = create_command_base_with_cargo(repo_root, true);

        assert!(matches!(
            get_port_for_current_package_at(&base, &cargo_member).await,
            Err(Error::NoPackageJson)
        ));
    }

    #[tokio::test]
    async fn test_colocated_javascript_package_wins_equal_depth_native_scope() {
        let tmp = TempDir::new().unwrap();
        let repo_root = setup_test_repo(&tmp);
        let app = repo_root.join_components(&["apps", "hybrid"]);
        app.join_component("src").create_dir_all().unwrap();
        app.join_component("package.json")
            .create_with_contents(r#"{"name":"hybrid"}"#)
            .unwrap();
        app.join_component("microfrontends.json")
            .create_with_contents(
                r#"{"version":"1","applications":{"hybrid":{"development":{"local":4555}}}}"#,
            )
            .unwrap();
        app.join_component("Cargo.toml")
            .create_with_contents(
                "[package]\nname = \"rust-hybrid\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .unwrap();
        app.join_components(&["src", "lib.rs"])
            .create_with_contents("")
            .unwrap();
        repo_root
            .join_component("Cargo.toml")
            .create_with_contents(
                "[workspace]\nmembers = [\"apps/hybrid\"]\nresolver = \
                 \"2\"\n\n[workspace.metadata]\nname = \"cargo-workspace\"\n",
            )
            .unwrap();
        repo_root
            .join_component("Cargo.lock")
            .create_with_contents(
                "version = 4\n\n[[package]]\nname = \"rust-hybrid\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
        let base = create_command_base_with_cargo(repo_root, true);

        assert_eq!(
            get_port_for_current_package_at(&base, &app).await.unwrap(),
            4555
        );
    }

    #[tokio::test]
    async fn test_missing_root_manifest_requires_cargo_feature() {
        let tmp = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        add_cargo_workspace(&repo_root);
        repo_root
            .join_component("turbo.json")
            .create_with_contents("{}")
            .unwrap();

        let disabled = create_command_base_with_cargo(repo_root.clone(), false);
        assert!(matches!(
            build_package_graph(&disabled).await,
            Err(Error::PackageJson(_))
        ));
        let enabled = create_command_base_with_cargo(repo_root, true);
        assert!(build_package_graph(&enabled).await.is_ok());
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
