use std::fs;

use camino::Utf8PathBuf;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::package_graph::PackageNode;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    cli,
    commands::CommandBase,
    run::{builder::RunBuilder, Run},
};

pub async fn run_typescript(
    config: Option<Utf8PathBuf>,
    base: &CommandBase,
    telemetry: CommandEventBuilder,
) -> Result<(), cli::Error> {
    // Create a Run instance to access the package graph
    let run_builder = RunBuilder::new(base.clone())?;
    let signal_handler = SignalHandler::new(get_signal()?);
    let run = run_builder.build(&signal_handler, telemetry).await?;

    // Iterate through all packages in the workspace
    for node in run.pkg_dep_graph().node_indices() {
        if let Some(package_node) = run.pkg_dep_graph().get_package_by_index(node) {
            // Skip the root package
            if matches!(package_node, PackageNode::Root) {
                continue;
            }

            // Get the package path
            if let PackageNode::Workspace(pkg_name) = package_node {
                if let Some(pkg_info) = run.pkg_dep_graph().package_info(pkg_name) {
                    let package_json_path = base.repo_root.resolve(pkg_info.package_json_path());
                    let package_dir = package_json_path.parent().unwrap();
                    let tsconfig_path = package_dir.join_component("tsconfig.json");

                    // Read and print the package.json contents
                    if let Ok(contents) = fs::read_to_string(package_json_path) {
                        println!("Package: {}", pkg_name);
                        println!("package.json:");
                        println!("{}", contents);
                    }

                    // Read and print the tsconfig.json contents if it exists
                    if let Ok(contents) = fs::read_to_string(tsconfig_path) {
                        println!("tsconfig.json:");
                        println!("{}", contents);
                    } else {
                        println!("No tsconfig.json found for package {}", pkg_name);
                    }
                    println!("---");
                }
            }
        }
    }

    Ok(())
}
