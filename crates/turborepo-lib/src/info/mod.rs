use std::io::Write;

use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_repository::{
    package_graph::{PackageGraph, PackageName, PackageNode},
    package_manager::PackageManager,
};
use turborepo_ui::GREY;

use crate::config::ConfigurationOptions;

mod scip;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("cannot emit scip for a single package")]
    ScipForPackage,
    #[error("scip output requires `--out` flag")]
    ScipOutputRequired,
    #[error(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(transparent)]
    PackageGraph(#[from] turborepo_repository::package_graph::Error),
    #[error("failed to serialize to json")]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    Scip(#[from] scip::Error),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoConfig {
    api_url: Option<String>,
    login_url: Option<String>,
    team_slug: Option<String>,
    team_id: Option<String>,
    token: Option<String>,
    signature: Option<bool>,
    preflight: Option<bool>,
    timeout: Option<u64>,
    enabled: Option<bool>,
    spaces_id: Option<String>,
}

impl<'a> From<&'a ConfigurationOptions> for InfoConfig {
    fn from(config: &'a ConfigurationOptions) -> Self {
        Self {
            api_url: config.api_url.clone(),
            login_url: config.login_url.clone(),
            team_slug: config.team_slug.clone(),
            team_id: config.team_id.clone(),
            token: config.token.clone(),
            signature: config.signature,
            preflight: config.preflight,
            timeout: config.timeout,
            enabled: config.enabled,
            spaces_id: config.spaces_id.clone(),
        }
    }
}

pub struct RepositoryState {
    pkg_dep_graph: PackageGraph,
    config: InfoConfig,
    repo_root: AbsoluteSystemPathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDetails<'a> {
    config: &'a InfoConfig,
    package_manager: &'a PackageManager,
    workspaces: Vec<(&'a PackageName, RepositoryWorkspaceDetails<'a>)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryWorkspaceDetails<'a> {
    path: &'a AnchoredSystemPath,
}

#[derive(Serialize)]
pub struct PackageDetails<'a> {
    name: &'a str,
    dependencies: Vec<&'a str>,
}

impl RepositoryState {
    pub fn new(
        package_graph: PackageGraph,
        config: &ConfigurationOptions,
        repo_root: AbsoluteSystemPathBuf,
    ) -> Self {
        Self {
            config: config.into(),
            pkg_dep_graph: package_graph,
            repo_root,
        }
    }

    pub fn as_details(&self) -> RepositoryDetails {
        let mut workspaces: Vec<_> = self
            .pkg_dep_graph
            .packages()
            .map(|(workspace_name, workspace_info)| {
                let workspace_details = RepositoryWorkspaceDetails {
                    path: workspace_info.package_path(),
                };

                (workspace_name, workspace_details)
            })
            .collect();
        workspaces.sort_by(|a, b| a.0.cmp(b.0));

        RepositoryDetails {
            config: &self.config,
            package_manager: self.pkg_dep_graph.package_manager(),
            workspaces,
        }
    }

    pub fn as_package_details<'a>(&'a self, package_name: &'a str) -> PackageDetails<'a> {
        PackageDetails::new(&self.pkg_dep_graph, package_name)
    }
}

impl<'a> RepositoryDetails<'a> {
    pub fn print_to(&self, writer: &mut dyn Write) -> Result<(), Error> {
        let is_logged_in = self.config.token.is_some();
        let is_linked = self.config.team_id.is_some();
        let team_slug = self.config.team_slug.as_deref();

        match (is_logged_in, is_linked, team_slug) {
            (true, true, Some(slug)) => {
                writeln!(writer, "You are logged in and linked to {}", slug)?
            }
            (true, true, None) => writeln!(writer, "You are logged in and linked")?,
            (true, false, _) => writeln!(writer, "You are logged in but not linked")?,
            (false, _, _) => writeln!(writer, "You are not logged in")?,
        }

        // We subtract 1 for the root workspace
        writeln!(
            writer,
            "{} packages found in workspace\n",
            self.workspaces.len() - 1
        )?;

        for (workspace_name, entry) in &self.workspaces {
            if matches!(workspace_name, PackageName::Root) {
                continue;
            }
            writeln!(writer, "- {} {}", workspace_name, GREY.apply_to(entry.path))?;
        }

        Ok(())
    }
}

impl<'a> PackageDetails<'a> {
    pub fn new(package_graph: &'a PackageGraph, workspace_name: &'a str) -> Self {
        let workspace_node = match workspace_name {
            "//" => PackageNode::Root,
            name => PackageNode::Workspace(PackageName::Other(name.to_string())),
        };

        let transitive_dependencies = package_graph.transitive_closure(Some(&workspace_node));

        let mut workspace_dep_names: Vec<&str> = transitive_dependencies
            .into_iter()
            .filter_map(|dependency| match dependency {
                PackageNode::Root | PackageNode::Workspace(PackageName::Root) => Some("root"),
                PackageNode::Workspace(PackageName::Other(dep_name))
                    if dep_name == workspace_name =>
                {
                    None
                }
                PackageNode::Workspace(PackageName::Other(dep_name)) => Some(dep_name.as_str()),
            })
            .collect();
        workspace_dep_names.sort();

        Self {
            name: workspace_name,
            dependencies: workspace_dep_names,
        }
    }

    pub fn print_to(&self, writer: &mut dyn Write) -> Result<(), Error> {
        writeln!(writer, "{} depends on:", self.name)?;
        for dep_name in &self.dependencies {
            writeln!(writer, "- {}", dep_name)?;
        }

        Ok(())
    }
}
