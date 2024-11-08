//! A command for outputting info about packages and tasks in a turborepo.

use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;
use turbopath::AnchoredSystemPath;
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_manager::PackageManager,
};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{color, cprint, cprintln, ColorConfig, BOLD, BOLD_GREEN, GREY};

use crate::{
    cli,
    cli::{Command, ExecutionArgs, OutputFormat},
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
struct ItemsWithCount<T> {
    count: usize,
    items: Vec<T>,
}

#[derive(Clone, Serialize)]
#[serde(into = "RepositoryDetailsDisplay<'a>")]
struct RepositoryDetails<'a> {
    color_config: ColorConfig,
    package_manager: &'a PackageManager,
    packages: Vec<(&'a PackageName, &'a AnchoredSystemPath)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryDetailsDisplay<'a> {
    package_manager: &'a PackageManager,
    packages: ItemsWithCount<PackageDetailDisplay>,
}

#[derive(Serialize)]
struct PackageDetailDisplay {
    name: String,
    path: String,
}

impl<'a> From<RepositoryDetails<'a>> for RepositoryDetailsDisplay<'a> {
    fn from(val: RepositoryDetails<'a>) -> Self {
        RepositoryDetailsDisplay {
            package_manager: val.package_manager,
            packages: ItemsWithCount {
                count: val.packages.len(),
                items: val
                    .packages
                    .into_iter()
                    .map(|(name, path)| PackageDetailDisplay {
                        name: name.to_string(),
                        path: path.to_string(),
                    })
                    .collect(),
            },
        }
    }
}

#[derive(Clone, Serialize)]
struct PackageTask<'a> {
    name: &'a str,
    command: &'a str,
}

#[derive(Clone, Serialize)]
#[serde(into = "PackageDetailsDisplay<'a>")]
struct PackageDetails<'a> {
    #[serde(skip)]
    color_config: ColorConfig,
    name: &'a str,
    tasks: Vec<PackageTask<'a>>,
    dependencies: Vec<&'a str>,
}

#[derive(Clone, Serialize)]
struct PackageDetailsList<'a> {
    packages: Vec<PackageDetails<'a>>,
}

#[derive(Serialize)]
struct PackageDetailsDisplay<'a> {
    name: &'a str,
    tasks: ItemsWithCount<PackageTask<'a>>,
    dependencies: Vec<&'a str>,
}

impl<'a> From<PackageDetails<'a>> for PackageDetailsDisplay<'a> {
    fn from(val: PackageDetails<'a>) -> Self {
        PackageDetailsDisplay {
            name: val.name,
            dependencies: val.dependencies,
            tasks: ItemsWithCount {
                count: val.tasks.len(),
                items: val.tasks,
            },
        }
    }
}

pub async fn run(
    mut base: CommandBase,
    packages: Vec<String>,
    telemetry: CommandEventBuilder,
    filter: Vec<String>,
    affected: bool,
    output: Option<OutputFormat>,
) -> Result<(), cli::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    // We fake a run command, so we can construct a `Run` type
    base.args_mut().command = Some(Command::Run {
        run_args: Box::default(),
        execution_args: Box::new(ExecutionArgs {
            filter,
            affected,
            ..Default::default()
        }),
    });

    let run_builder = RunBuilder::new(base)?;
    let run = run_builder.build(&handler, telemetry).await?;

    if packages.is_empty() {
        RepositoryDetails::new(&run).print(output)?;
    } else {
        match output {
            Some(OutputFormat::Json) => {
                let mut package_details_list = PackageDetailsList { packages: vec![] };
                //  collect all package details
                for package in &packages {
                    let package_details = PackageDetails::new(&run, package)?;
                    package_details_list.packages.push(package_details);
                }

                let as_json = serde_json::to_string_pretty(&package_details_list)?;
                println!("{}", as_json);
            }
            Some(OutputFormat::Pretty) | None => {
                for package in packages {
                    let package_details = PackageDetails::new(&run, &package)?;
                    package_details.print();
                }
            }
        }
    }

    Ok(())
}

impl<'a> RepositoryDetails<'a> {
    fn new(run: &'a Run) -> Self {
        let color_config = run.color_config();
        let package_graph = run.pkg_dep_graph();
        let filtered_pkgs = run.filtered_pkgs();

        let mut packages: Vec<_> = package_graph
            .packages()
            .filter_map(|(package_name, package_info)| {
                if !filtered_pkgs.contains(package_name) {
                    return None;
                }
                if matches!(package_name, PackageName::Root) {
                    return None;
                }

                Some((package_name, package_info.package_path()))
            })
            .collect();
        packages.sort_by(|a, b| a.0.cmp(b.0));

        Self {
            color_config,
            package_manager: package_graph.package_manager(),
            packages,
        }
    }
    fn pretty_print(&self) {
        let package_copy = match self.packages.len() {
            0 => "no packages",
            1 => "package",
            _ => "packages",
        };

        cprint!(
            self.color_config,
            BOLD,
            "{} {} ",
            self.packages.len(),
            package_copy
        );
        cprintln!(self.color_config, GREY, "({})\n", self.package_manager);

        for (package_name, entry) in &self.packages {
            println!("  {} {}", package_name, GREY.apply_to(entry));
        }
    }

    fn json_print(&self) -> Result<(), cli::Error> {
        let as_json = serde_json::to_string_pretty(&self)?;
        println!("{}", as_json);
        Ok(())
    }

    fn print(&self, output: Option<OutputFormat>) -> Result<(), cli::Error> {
        match output {
            Some(OutputFormat::Json) => {
                self.json_print()?;
            }
            Some(OutputFormat::Pretty) | None => {
                self.pretty_print();
            }
        }

        Ok(())
    }
}

impl<'a> PackageDetails<'a> {
    fn new(run: &'a Run, package: &'a str) -> Result<Self, Error> {
        let color_config = run.color_config();
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
            color_config,
            name: package,
            dependencies: package_dep_names,
            tasks: package_json
                .scripts
                .iter()
                .map(|(name, command)| PackageTask { name, command })
                .collect(),
        })
    }

    fn print(&self) {
        let name = color!(self.color_config, BOLD_GREEN, "{}", self.name);
        let depends_on = color!(self.color_config, BOLD, "depends on");
        let dependencies = if self.dependencies.is_empty() {
            "<no packages>".to_string()
        } else {
            self.dependencies.join(", ")
        };
        println!(
            "{} {}: {}",
            name,
            depends_on,
            color!(self.color_config, GREY, "{}", dependencies)
        );
        println!();

        cprint!(self.color_config, BOLD, "tasks:");
        if self.tasks.is_empty() {
            println!(" <no tasks>");
        } else {
            println!();
        }
        for task in &self.tasks {
            println!(
                "  {}: {}",
                task.name,
                color!(self.color_config, GREY, "{}", task.command)
            );
        }
        println!();
    }
}
