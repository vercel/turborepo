//! A command for outputting info about packages and tasks in a turborepo.

use miette::Diagnostic;
use thiserror::Error;
use turbopath::AnchoredSystemPath;
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_manager::PackageManager,
};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{color, cprint, cprintln, BOLD, BOLD_GREEN, GREY, UI};

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

struct RepositoryDetails<'a> {
    ui: UI,
    package_manager: &'a PackageManager,
    packages: Vec<(&'a PackageName, &'a AnchoredSystemPath)>,
}

struct PackageDetails<'a> {
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

        let mut packages: Vec<_> = package_graph
            .packages()
            .filter_map(|(package_name, package_info)| {
                if !filtered_pkgs.contains(package_name) {
                    return None;
                }

                Some((package_name, package_info.package_path()))
            })
            .collect();
        packages.sort_by(|a, b| a.0.cmp(b.0));

        Self {
            ui,
            package_manager: package_graph.package_manager(),
            packages,
        }
    }
    fn print(&self) -> Result<(), cli::Error> {
        if self.packages.len() == 1 {
            cprintln!(self.ui, BOLD, "{} package\n", self.packages.len());
        } else {
            cprintln!(self.ui, BOLD, "{} packages\n", self.packages.len());
        }

        for (package_name, entry) in &self.packages {
            if matches!(package_name, PackageName::Root) {
                continue;
            }
            println!("  {} {}", package_name, GREY.apply_to(entry));
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

        let mut package_dep_names: Vec<&str> = transitive_dependencies
            .into_iter()
            .filter_map(|dependency| match dependency {
                PackageNode::Root | PackageNode::Workspace(PackageName::Root) => None,
                PackageNode::Workspace(PackageName::Other(dep_name)) if dep_name == package => None,
                PackageNode::Workspace(PackageName::Other(dep_name)) => Some(dep_name.as_str()),
            })
            .collect();
        package_dep_names.sort();

        Ok(Self {
            ui,
            name: package,
            dependencies: package_dep_names,
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

        cprint!(self.ui, BOLD, "tasks:");
        if self.tasks.is_empty() {
            println!(" <no tasks>");
        } else {
            println!();
        }
        for (name, command) in &self.tasks {
            println!("  {}: {}", name, color!(self.ui, GREY, "{}", command));
        }
        println!();
    }
}
