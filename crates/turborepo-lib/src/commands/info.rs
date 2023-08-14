use anyhow::Result;
use turborepo_ui::GREY;

use crate::{
    commands::CommandBase,
    package_graph::{PackageGraph, WorkspaceName, WorkspaceNode},
    package_json::PackageJson,
    package_manager::PackageManager,
};

pub fn run(base: &mut CommandBase, workspace: Option<&str>) -> Result<()> {
    let root_package_json = PackageJson::load(&base.repo_root.join_component("package.json"))?;

    let package_manager =
        PackageManager::get_package_manager(&base.repo_root, Some(&root_package_json))?;

    let package_graph = PackageGraph::builder(&base.repo_root, root_package_json)
        .with_package_manger(Some(package_manager))
        .build()?;

    if let Some(workspace) = workspace {
        print_workspace_details(&package_graph, workspace)
    } else {
        print_repo_details(&package_graph)
    }
}

fn print_repo_details(package_graph: &PackageGraph) -> Result<()> {
    // We subtract 1 for the root workspace
    println!("{} packages found in workspace\n", package_graph.len() - 1);

    let mut workspaces: Vec<_> = package_graph.workspaces().collect();
    workspaces.sort_by(|a, b| a.0.cmp(b.0));

    for (workspace_name, entry) in workspaces {
        if matches!(workspace_name, WorkspaceName::Root) {
            continue;
        }
        println!(
            "- {} {}",
            workspace_name,
            GREY.apply_to(entry.package_json_path())
        );
    }

    Ok(())
}

fn print_workspace_details(package_graph: &PackageGraph, workspace_name: &str) -> Result<()> {
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

    println!("{} depends on:", workspace_name);
    for dep_name in workspace_dep_names {
        println!("- {}", dep_name);
    }

    Ok(())
}
