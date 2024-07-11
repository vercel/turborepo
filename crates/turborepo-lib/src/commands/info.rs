//! A command for outputting information about a turborepo.
//! Currently just for internal use (not a public command)
//! Can output in either text or JSON
//! Different than run summary or dry run because it can include
//! sensitive data like your auth token

use serde::Serialize;
use turbopath::AnchoredSystemPath;
use turborepo_repository::{
    package_graph::{PackageGraph, PackageName, PackageNode},
    package_manager::PackageManager,
};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{cprintln, BOLD, GREY, UI};

use crate::{
    cli,
    cli::{Command, ExecutionArgs},
    commands::{run::get_signal, CommandBase},
    run::{builder::RunBuilder, Run},
    signal::SignalHandler,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryDetails<'a> {
    #[serde(skip)]
    ui: UI,
    package_manager: &'a PackageManager,
    workspaces: Vec<(&'a PackageName, RepositoryWorkspaceDetails<'a>)>,
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
    mut base: CommandBase,
    telemetry: CommandEventBuilder,
    filter: Vec<String>,
    json: bool,
) -> Result<(), cli::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    // We fake a run command, so we can construct a `Run` type
    base.args_mut().command = Some(Command::Run {
        run_args: Box::default(),
        execution_args: Box::new(ExecutionArgs {
            filter,
            ..Default::default()
        }),
    });

    let run_builder = RunBuilder::new(base)?;
    let run = run_builder.build(&handler, telemetry).await?;

    let repo_details = RepositoryDetails::new(&run);
    if json {
        println!("{}", serde_json::to_string_pretty(&repo_details)?);
    } else {
        repo_details.print()?;
    }

    Ok(())
}

impl<'a> RepositoryDetails<'a> {
    fn new(run: &'a Run) -> Self {
        let ui = run.ui();
        let package_graph = run.pkg_dep_graph();
        let filtered_pkgs = run.filtered_pkgs();

        let mut workspaces: Vec<_> = package_graph
            .packages()
            .filter_map(|(workspace_name, workspace_info)| {
                if !filtered_pkgs.contains(workspace_name) {
                    return None;
                }

                let workspace_details = RepositoryWorkspaceDetails {
                    path: workspace_info.package_path(),
                };

                Some((workspace_name, workspace_details))
            })
            .collect();
        workspaces.sort_by(|a, b| a.0.cmp(b.0));

        Self {
            ui,
            package_manager: package_graph.package_manager(),
            workspaces,
        }
    }
    fn print(&self) -> Result<(), cli::Error> {
        // We subtract 1 for the root workspace
        cprintln!(self.ui, BOLD, "{} packages\n", self.workspaces.len() - 1);

        for (workspace_name, entry) in &self.workspaces {
            if matches!(workspace_name, PackageName::Root) {
                continue;
            }
            println!("  {} {}", workspace_name, GREY.apply_to(entry.path));
        }

        Ok(())
    }
}

impl<'a> WorkspaceDetails<'a> {
    fn new(package_graph: &'a PackageGraph, workspace_name: &'a str) -> Self {
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

    fn print(&self) {
        println!("{} depends on:", self.name);
        for dep_name in &self.dependencies {
            println!("- {}", dep_name);
        }
    }
}
