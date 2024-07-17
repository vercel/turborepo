//! A command for outputting information about a turborepo.
//! Currently just for internal use (not a public command)
//! Can output in either text or JSON
//! Different than run summary or dry run because it can include
//! sensitive data like your auth token

use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;
use turbopath::AnchoredSystemPath;
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_manager::PackageManager,
};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{color, cprintln, BOLD, BOLD_GREEN, GREY, UI};

use crate::{
    cli,
    cli::{Command, ExecutionArgs},
    commands::{run::get_signal, CommandBase},
    run::{builder::RunBuilder, Run},
    signal::SignalHandler,
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("package `{package}` not found")]
    PackageNotFound { package: String },
}

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
struct PackageDetails<'a> {
    #[serde(skip)]
    ui: UI,
    name: &'a str,
    tasks: Vec<(&'a str, &'a str)>,
    dependencies: Vec<&'a str>,
}

pub async fn run(
    mut base: CommandBase,
    packages: Vec<String>,
    telemetry: CommandEventBuilder,
    filter: Vec<String>,
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

    if packages.is_empty() {
        RepositoryDetails::new(&run).print()?;
    } else {
        for package in packages {
            let package_details = PackageDetails::new(&run, &package)?;
            package_details.print();
        }
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
        if self.workspaces.len() == 1 {
            cprintln!(self.ui, BOLD, "{} package\n", self.workspaces.len());
        } else {
            cprintln!(self.ui, BOLD, "{} packages\n", self.workspaces.len());
        }

        for (workspace_name, entry) in &self.workspaces {
            if matches!(workspace_name, PackageName::Root) {
                continue;
            }
            println!("  {} {}", workspace_name, GREY.apply_to(entry.path));
        }

        Ok(())
    }
}

impl<'a> PackageDetails<'a> {
    fn new(run: &'a Run, package: &'a str) -> Result<Self, Error> {
        let ui = run.ui();
        let package_graph = run.pkg_dep_graph();
        let package_node = match package {
            "//" => PackageNode::Root,
            name => PackageNode::Workspace(PackageName::Other(name.to_string())),
        };

        let package_json = package_graph
            .package_json(package_node.as_package_name())
            .ok_or_else(|| Error::PackageNotFound {
                package: package.to_string(),
            })?;

        let transitive_dependencies = package_graph.transitive_closure(Some(&package_node));

        let mut workspace_dep_names: Vec<&str> = transitive_dependencies
            .into_iter()
            .filter_map(|dependency| match dependency {
                PackageNode::Root | PackageNode::Workspace(PackageName::Root) => None,
                PackageNode::Workspace(PackageName::Other(dep_name)) if dep_name == package => None,
                PackageNode::Workspace(PackageName::Other(dep_name)) => Some(dep_name.as_str()),
            })
            .collect();
        workspace_dep_names.sort();

        Ok(Self {
            ui,
            name: package,
            dependencies: workspace_dep_names,
            tasks: package_json
                .scripts
                .iter()
                .map(|(name, command)| (name.as_str(), command.as_str()))
                .collect(),
        })
    }

    fn print(&self) {
        let name = color!(self.ui, BOLD_GREEN, "{}", self.name);
        let depends_on = color!(self.ui, BOLD, "depends on");
        let dependencies = if self.dependencies.is_empty() {
            "<no packages>".to_string()
        } else {
            self.dependencies.join(", ")
        };
        println!(
            "{} {}: {}",
            name,
            depends_on,
            color!(self.ui, GREY, "{}", dependencies)
        );
        println!();

        cprintln!(self.ui, BOLD, "tasks:");
        for (name, command) in &self.tasks {
            println!("  {}: {}", name, color!(self.ui, GREY, "{}", command));
        }
        println!();
    }
}
