use thiserror::Error;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::package_graph::{
    PackageGraph, PackageGraphNodeKind, PackageName, PackageNode,
};

use crate::MicrofrontendsConfigs;

#[derive(Debug, Error)]
pub enum PortResolutionError {
    #[error("Current directory does not belong to a named JavaScript package")]
    NoPackageJson,
    #[error("package.json is missing the 'name' field")]
    NoPackageName,
    #[error("Package '{0}' not found in microfrontends configuration")]
    PackageNotInConfig(String),
}

/// Resolves a directory to its owning JavaScript package and returns that
/// package's configured microfrontend development port.
pub fn resolve_port_for_directory(
    package_graph: &PackageGraph,
    repo_root: &AbsoluteSystemPath,
    cwd: &AbsoluteSystemPath,
    configs: &MicrofrontendsConfigs,
) -> Result<u16, PortResolutionError> {
    let package_name = package_for_directory(package_graph, repo_root, cwd)?;
    resolve_port_for_package(configs, &package_name)
}

/// Returns the configured microfrontend development port for a package.
pub fn resolve_port_for_package(
    configs: &MicrofrontendsConfigs,
    package_name: &str,
) -> Result<u16, PortResolutionError> {
    configs
        .port_for_package(package_name)
        .ok_or_else(|| PortResolutionError::PackageNotInConfig(package_name.to_owned()))
}

/// Resolves directory ownership using authoritative package-graph scopes. The
/// deepest package directory wins; Cargo and aggregate scopes are excluded.
fn package_for_directory(
    package_graph: &PackageGraph,
    repo_root: &AbsoluteSystemPath,
    cwd: &AbsoluteSystemPath,
) -> Result<String, PortResolutionError> {
    let cwd = repo_root
        .anchor(cwd)
        .map_err(|_| PortResolutionError::NoPackageJson)?;
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
        .ok_or(PortResolutionError::NoPackageJson)?;

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
                .ok_or(PortResolutionError::NoPackageName)
        }
        _ => Err(PortResolutionError::NoPackageJson),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use serde_json::json;
    use tempfile::TempDir;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
    use turborepo_microfrontends::TurborepoMfeConfig;
    use turborepo_repository::{
        package_graph::PackageGraph, package_json::PackageJson, package_manager::PackageManager,
    };

    use super::{
        MicrofrontendsConfigs, PortResolutionError, resolve_port_for_directory,
        resolve_port_for_package,
    };

    fn root(tmp: &TempDir) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::try_from(tmp.path().to_path_buf()).unwrap()
    }

    fn package(root: &AbsoluteSystemPath, directory: &str, name: &str) {
        let components = directory.split('/').collect::<Vec<_>>();
        let package_dir = root.join_components(&components);
        package_dir.create_dir_all().unwrap();
        package_dir
            .join_component("package.json")
            .create_with_contents(format!(r#"{{"name":"{name}"}}"#))
            .unwrap();
    }

    async fn package_graph(root: &AbsoluteSystemPath) -> PackageGraph {
        let package_json = PackageJson::from_value(json!({
            "name": "root-app",
            "packageManager": "pnpm@9.0.0",
            "workspaces": ["apps/*", "apps/*/packages/*"],
        }))
        .unwrap();
        root.join_component("package.json")
            .create_with_contents(
                r#"{"name":"root-app","packageManager":"pnpm@9.0.0","workspaces":["apps/*","apps/*/packages/*"]}"#,
            )
            .unwrap();
        root.join_component("pnpm-workspace.yaml")
            .create_with_contents("packages:\n  - 'apps/*'\n  - 'apps/*/packages/*'\n")
            .unwrap();
        PackageGraph::builder(root, package_json)
            .with_package_manager(PackageManager::Pnpm)
            .build()
            .await
            .unwrap()
    }

    fn configs(package_names: &[&str], contents: &str) -> MicrofrontendsConfigs {
        let config = TurborepoMfeConfig::from_str(contents, "microfrontends.json").unwrap();
        MicrofrontendsConfigs::from_configs(
            package_names.iter().copied().collect::<HashSet<_>>(),
            [("apps/shell", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap()
        .expect("the fixture contains an MFE config")
    }

    #[tokio::test]
    async fn directory_resolution_uses_the_deepest_package_owner() {
        let tmp = TempDir::new().unwrap();
        let root = root(&tmp);
        package(&root, "apps/shell", "shell");
        package(&root, "apps/shell/packages/widget", "widget");
        let graph = package_graph(&root).await;
        let configs = configs(
            &["shell", "widget"],
            r#"{"applications":{"shell":{"development":{"local":{"port":3010}}},"widget":{"packageName":"widget","development":{"local":{"port":4020}}}}}"#,
        );
        let widget_src = root.join_components(&["apps", "shell", "packages", "widget", "src"]);
        widget_src.create_dir_all().unwrap();

        assert_eq!(
            resolve_port_for_directory(&graph, &root, &widget_src, &configs).unwrap(),
            4020
        );
    }

    #[tokio::test]
    async fn directory_resolution_accepts_the_named_root_only_at_repository_root() {
        let tmp = TempDir::new().unwrap();
        let root = root(&tmp);
        let graph = package_graph(&root).await;
        let configs = configs(
            &["root-app"],
            r#"{"applications":{"root-app":{"development":{"local":{"port":3007}}}}}"#,
        );
        let unowned = root.join_component("tools");
        unowned.create_dir_all().unwrap();

        assert_eq!(
            resolve_port_for_directory(&graph, &root, &root, &configs).unwrap(),
            3007
        );
        assert!(matches!(
            resolve_port_for_directory(&graph, &root, &unowned, &configs),
            Err(PortResolutionError::NoPackageJson)
        ));
        let outside = TempDir::new().unwrap();
        let outside = AbsoluteSystemPathBuf::try_from(outside.path().to_path_buf()).unwrap();
        assert!(matches!(
            resolve_port_for_directory(&graph, &root, &outside, &configs),
            Err(PortResolutionError::NoPackageJson)
        ));
    }

    #[tokio::test]
    async fn unnamed_root_has_no_port_owner() {
        let tmp = TempDir::new().unwrap();
        let root = root(&tmp);
        root.join_component("package.json")
            .create_with_contents(r#"{"packageManager":"pnpm@9.0.0","workspaces":["apps/*"]}"#)
            .unwrap();
        root.join_component("pnpm-workspace.yaml")
            .create_with_contents("packages:\n  - 'apps/*'\n")
            .unwrap();
        let root_manifest = PackageJson::from_value(json!({
            "packageManager": "pnpm@9.0.0",
            "workspaces": ["apps/*"],
        }))
        .unwrap();
        let graph = PackageGraph::builder(&root, root_manifest)
            .with_package_manager(PackageManager::Pnpm)
            .build()
            .await
            .unwrap();
        let configs = configs(
            &["web"],
            r#"{"applications":{"web":{"development":{"local":{"port":3010}}}}}"#,
        );

        assert!(matches!(
            resolve_port_for_directory(&graph, &root, &root, &configs),
            Err(PortResolutionError::NoPackageName)
        ));
    }

    #[test]
    fn package_name_mapping_and_automatic_ports_are_preserved() {
        let mapped = configs(
            &["my-app"],
            r#"{"applications":{"vercel-project":{"packageName":"my-app","development":{"local":{"port":3005}}}}}"#,
        );
        assert_eq!(resolve_port_for_package(&mapped, "my-app").unwrap(), 3005);

        let automatic = configs(
            &["web"],
            r#"{"applications":{"web":{"development":{"local":{}}}}}"#,
        );
        let port = resolve_port_for_package(&automatic, "web").unwrap();
        assert!((3000..=8000).contains(&port));
        assert_eq!(resolve_port_for_package(&automatic, "web").unwrap(), port);
    }

    #[test]
    fn package_not_in_microfrontends_config_is_a_typed_error() {
        let configs = configs(
            &["web"],
            r#"{"applications":{"web":{"development":{"local":{"port":3001}}}}}"#,
        );
        assert!(matches!(
            resolve_port_for_package(&configs, "docs"),
            Err(PortResolutionError::PackageNotInConfig(name)) if name == "docs"
        ));
    }
}
