//! A command for outputting information about a turborepo.
//! Currently just for internal use (not a public command)
//! Can output in either text or JSON
//! Different than run summary or dry run because it can include
//! sensitive data like your auth token
use serde::Serialize;
use turbopath::AnchoredSystemPath;
use turborepo_repository::{
    package_graph::{PackageGraph, WorkspaceName, WorkspaceNode},
    package_json::PackageJson,
    package_manager::PackageManager,
};
use turborepo_ui::GREY;

use crate::{cli, commands::CommandBase, config::ConfigurationOptions};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryDetails<'a> {
    config: &'a ConfigurationOptions,
    package_manager: &'a PackageManager,
    workspaces: Vec<(&'a WorkspaceName, RepositoryWorkspaceDetails<'a>)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryWorkspaceDetails<'a> {
    path: &'a AnchoredSystemPath,
}

#[derive(Serialize)]
struct WorkspaceDetails<'a> {
    name: &'a str,
    dependencies: Vec<&'a str>,
}

pub async fn run(
    base: &mut CommandBase,
    workspace: Option<&str>,
    json: bool,
) -> Result<(), cli::Error> {
    let root_package_json = PackageJson::load(&base.repo_root.join_component("package.json"))?;

    let package_graph = PackageGraph::builder(&base.repo_root, root_package_json)
        .build()
        .await?;

    let config = base.config()?;

    if let Some(workspace) = workspace {
        let workspace_details = WorkspaceDetails::new(&package_graph, workspace);
        if json {
            println!("{}", serde_json::to_string_pretty(&workspace_details)?);
        } else {
            workspace_details.print();
        }
    } else {
        let repo_details = RepositoryDetails::new(&package_graph, config);
        if json {
            println!("{}", serde_json::to_string_pretty(&repo_details)?);
        } else {
            repo_details.print()?;
        }
    }

    Ok(())
}

impl<'a> RepositoryDetails<'a> {
    fn new(package_graph: &'a PackageGraph, config: &'a ConfigurationOptions) -> Self {
        let mut workspaces: Vec<_> = package_graph
            .workspaces()
            .map(|(workspace_name, workspace_info)| {
                let workspace_details = RepositoryWorkspaceDetails {
                    path: workspace_info.package_path(),
                };

                (workspace_name, workspace_details)
            })
            .collect();
        workspaces.sort_by(|a, b| a.0.cmp(b.0));

        Self {
            config,
            package_manager: package_graph.package_manager(),
            workspaces,
        }
    }
    fn print(&self) -> Result<(), cli::Error> {
        let is_logged_in = self.config.token.is_some();
        let is_linked = self.config.team_id.is_some();
        let team_slug = self.config.team_slug.as_deref();

        match (is_logged_in, is_linked, team_slug) {
            (true, true, Some(slug)) => println!("You are logged in and linked to {}", slug),
            (true, true, None) => println!("You are logged in and linked"),
            (true, false, _) => println!("You are logged in but not linked"),
            (false, _, _) => println!("You are not logged in"),
        }

        // We subtract 1 for the root workspace
        println!(
            "{} packages found in workspace\n",
            self.workspaces.len() - 1
        );

        for (workspace_name, entry) in &self.workspaces {
            if matches!(workspace_name, WorkspaceName::Root) {
                continue;
            }
            println!("- {} {}", workspace_name, GREY.apply_to(entry.path));
        }

        Ok(())
    }
}

impl<'a> WorkspaceDetails<'a> {
    fn new(package_graph: &'a PackageGraph, workspace_name: &'a str) -> Self {
        let workspace_node = match workspace_name {
            "//" => WorkspaceNode::Root,
            name => WorkspaceNode::Workspace(WorkspaceName::Other(name.to_string())),
        };

        let transitive_dependencies = package_graph.transitive_closure(Some(&workspace_node));

        let mut workspace_dep_names: Vec<&str> = transitive_dependencies
            .into_iter()
            .filter_map(|dependency| match dependency {
                WorkspaceNode::Root | WorkspaceNode::Workspace(WorkspaceName::Root) => Some("root"),
                WorkspaceNode::Workspace(WorkspaceName::Other(dep_name))
                    if dep_name == workspace_name =>
                {
                    None
                }
                WorkspaceNode::Workspace(WorkspaceName::Other(dep_name)) => Some(dep_name.as_str()),
            })
            .collect();
        workspace_dep_names.sort();

        Self {
            name: workspace_name,
            dependencies: workspace_dep_names,
        }
    }

    fn print(&self) {
        println!("{} depends on:", self.name);
        for dep_name in &self.dependencies {
            println!("- {}", dep_name);
        }
    }
}
