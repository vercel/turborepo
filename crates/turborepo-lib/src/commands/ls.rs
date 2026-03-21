//! A command for outputting info about packages and tasks in a turborepo.

use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;
use turbopath::AnchoredSystemPath;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageNode};
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{color, cprint, cprintln, ColorConfig, BOLD, BOLD_GREEN, GREY};

use crate::{
    cli,
    cli::OutputFormat,
    commands::CommandBase,
    run::{builder::RunBuilder, Run},
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Package `{package}` not found.")]
    PackageNotFound { package: String },
}

#[derive(Serialize)]
struct ItemsWithCount<T> {
    count: usize,
    items: Vec<T>,
}

#[derive(Clone, Serialize)]
#[serde(into = "RepositoryDetailsDisplay")]
struct RepositoryDetails<'a> {
    color_config: ColorConfig,
    package_manager: String,
    workspace_providers: Vec<String>,
    packages: Vec<(&'a PackageName, &'a AnchoredSystemPath)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryDetailsDisplay {
    package_manager: String,
    workspace_providers: Vec<String>,
    packages: ItemsWithCount<PackageDetailDisplay>,
}

#[derive(Serialize)]
struct PackageDetailDisplay {
    name: String,
    path: String,
}

impl<'a> From<RepositoryDetails<'a>> for RepositoryDetailsDisplay {
    fn from(val: RepositoryDetails) -> Self {
        RepositoryDetailsDisplay {
            package_manager: val.package_manager,
            workspace_providers: val.workspace_providers,
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
    path: &'a AnchoredSystemPath,
    name: &'a str,
    tasks: Vec<PackageTask<'a>>,
    dependencies: Vec<&'a str>,
    dependents: Vec<&'a str>,
}

#[derive(Clone, Serialize)]
struct PackageDetailsList<'a> {
    packages: Vec<PackageDetails<'a>>,
}

#[derive(Serialize)]
struct PackageDetailsDisplay<'a> {
    name: &'a str,
    path: &'a AnchoredSystemPath,
    tasks: ItemsWithCount<PackageTask<'a>>,
    dependencies: Vec<&'a str>,
    dependents: Vec<&'a str>,
}

impl<'a> From<PackageDetails<'a>> for PackageDetailsDisplay<'a> {
    fn from(val: PackageDetails<'a>) -> Self {
        PackageDetailsDisplay {
            name: val.name,
            path: val.path,
            dependencies: val.dependencies,
            dependents: val.dependents,
            tasks: ItemsWithCount {
                count: val.tasks.len(),
                items: val.tasks,
            },
        }
    }
}

pub async fn run(
    base: CommandBase,
    packages: Vec<String>,
    telemetry: CommandEventBuilder,
    output: Option<OutputFormat>,
) -> Result<(), cli::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run_builder = RunBuilder::new(base, None)?;
    let (run, _analytics) = run_builder.build(&handler, telemetry).await?;

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

fn infer_workspace_providers(package_graph: &PackageGraph) -> Vec<String> {
    let mut workspace_providers = package_graph
        .packages()
        .filter_map(|(_, package_info)| package_info.package_json_path().as_path().file_name())
        .filter_map(|manifest_name| {
            if manifest_name.eq_ignore_ascii_case("Cargo.toml") {
                Some("cargo".to_string())
            } else if manifest_name.eq_ignore_ascii_case("pyproject.toml") {
                Some("uv".to_string())
            } else if manifest_name.eq_ignore_ascii_case("package.json") {
                Some("node".to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    workspace_providers.sort();
    workspace_providers.dedup();
    workspace_providers
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

        let workspace_providers = infer_workspace_providers(package_graph);

        Self {
            color_config,
            package_manager: package_graph.package_manager().name().to_string(),
            workspace_providers,
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
        cprintln!(
            self.color_config,
            GREY,
            "({}; providers: {})\n",
            self.package_manager,
            self.workspace_providers.join(", ")
        );

        for (package_name, entry) in &self.packages {
            println!("  {package_name} {}", GREY.apply_to(entry));
        }
    }

    fn json_print(&self) -> Result<(), cli::Error> {
        let as_json = serde_json::to_string_pretty(&self)?;
        println!("{as_json}");
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{package_graph::PackageGraph, package_json::PackageJson};

    use super::infer_workspace_providers;

    async fn make_graph_with_manifests(manifests: &[(&str, serde_json::Value)]) -> PackageGraph {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        repo_root
            .join_component("package.json")
            .create_with_contents(
                r#"{
  "name": "root",
  "private": true,
  "packageManager": "npm@10.0.0"
}"#,
            )
            .unwrap();

        let mut package_jsons = HashMap::new();
        for (manifest_path, manifest_json) in manifests {
            let manifest = manifest_path
                .split('/')
                .fold(repo_root.to_owned(), |path, segment| {
                    path.join_component(segment)
                });
            if let Some(parent) = manifest.parent() {
                parent.create_dir_all().unwrap();
            }
            manifest
                .create_with_contents(&manifest_json.to_string())
                .unwrap();
            package_jsons.insert(
                manifest.clone(),
                PackageJson::from_value(manifest_json.clone()).unwrap(),
            );
        }

        let root_package_json =
            PackageJson::load(&repo_root.join_component("package.json")).unwrap();
        PackageGraph::builder(&repo_root, root_package_json)
            .with_allow_no_package_manager(true)
            .with_package_jsons(Some(package_jsons))
            .build()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn infers_workspace_providers_from_manifest_file_names() {
        let package_graph = make_graph_with_manifests(&[
            ("crates/a/Cargo.toml", json!({"name":"crate-a"})),
            (
                "apps/py/pyproject.toml",
                json!({"name":"py-app","version":"0.1.0"}),
            ),
            (
                "apps/web/package.json",
                json!({"name":"web","version":"0.0.0"}),
            ),
        ])
        .await;

        assert_eq!(
            infer_workspace_providers(&package_graph),
            vec!["cargo".to_string(), "node".to_string(), "uv".to_string()]
        );
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
        let package_path = package_graph
            .package_info(package_node.as_package_name())
            .ok_or_else(|| Error::PackageNotFound {
                package: package.to_string(),
            })?
            .package_path();

        let mut package_dep_names: Vec<&str> = transitive_dependencies
            .into_iter()
            .filter_map(|dependency| match dependency {
                PackageNode::Root | PackageNode::Workspace(PackageName::Root) => None,
                PackageNode::Workspace(PackageName::Other(dep_name)) if dep_name == package => None,
                PackageNode::Workspace(PackageName::Other(dep_name)) => Some(dep_name.as_str()),
            })
            .collect();
        package_dep_names.sort();

        let mut package_rdep_names: Vec<&str> = package_graph
            .ancestors(&package_node)
            .into_iter()
            .filter_map(|dependent| match dependent {
                PackageNode::Root | PackageNode::Workspace(PackageName::Root) => None,
                PackageNode::Workspace(PackageName::Other(dep_name)) if dep_name == package => None,
                PackageNode::Workspace(PackageName::Other(dep_name)) => Some(dep_name.as_str()),
            })
            .collect();
        package_rdep_names.sort();

        Ok(Self {
            color_config,
            path: package_path,
            name: package,
            dependencies: package_dep_names,
            dependents: package_rdep_names,
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

        cprintln!(self.color_config, GREY, "{} ", self.path);
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
