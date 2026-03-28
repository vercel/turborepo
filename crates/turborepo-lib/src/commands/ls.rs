//! A command for outputting info about packages and tasks in a turborepo.
//!
//! Both `turbo ls` and `turbo query ls` are backed by this module. Data
//! retrieval is done through the query server (GraphQL execution), keeping
//! the ls command in sync with `turbo query` semantics.

use std::sync::Arc;

use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;
use turborepo_query_api::{QueryRun, QueryServer};
use turborepo_repository::package_graph::PackageName;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{color, cprint, cprintln, ColorConfig, BOLD, BOLD_GREEN, GREY};

use crate::{cli, cli::OutputFormat, commands::CommandBase, run::builder::RunBuilder};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Package `{package}` not found.")]
    PackageNotFound { package: String },
    #[error("Query returned errors")]
    QueryError,
}

// GraphQL query: list all packages with name and path
const PACKAGES_QUERY: &str = "{ packages { items { name path } length } }";

fn package_detail_query(name: &str) -> String {
    let escaped = super::query::escape_graphql_string(name);
    format!(
        r#"{{ package(name: "{escaped}") {{ name path tasks {{ items {{ name script }} length }} allDependencies {{ items {{ name }} length }} allDependents {{ items {{ name }} length }} }} }}"#
    )
}

#[derive(Serialize)]
struct ItemsWithCount<T> {
    count: usize,
    items: Vec<T>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryDetailsDisplay {
    package_manager: String,
    packages: ItemsWithCount<PackageDetailDisplay>,
}

#[derive(Serialize)]
struct PackageDetailDisplay {
    name: String,
    path: String,
}

#[derive(Clone, Serialize)]
struct PackageTask {
    name: String,
    command: String,
}

#[derive(Serialize)]
struct PackageDetailsDisplay {
    name: String,
    path: String,
    tasks: ItemsWithCount<PackageTask>,
    dependencies: Vec<String>,
    dependents: Vec<String>,
}

#[derive(Serialize)]
struct PackageDetailsList {
    packages: Vec<PackageDetailsDisplay>,
}

pub async fn run(
    base: CommandBase,
    packages: Vec<String>,
    telemetry: CommandEventBuilder,
    output: Option<OutputFormat>,
    query_server: &dyn QueryServer,
) -> Result<(), cli::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let color_config = base.color_config;

    let run_builder = RunBuilder::new(base, None)?;
    let (run, _analytics) = run_builder.build(&handler, telemetry).await?;

    let package_manager_name = run.pkg_dep_graph().package_manager().name().to_string();
    let filtered_pkgs = run.filtered_pkgs().clone();
    let run: Arc<dyn QueryRun> = Arc::new(run);

    if packages.is_empty() {
        let repo = query_packages(run, query_server, &filtered_pkgs, &package_manager_name).await?;
        print_repo_details(&repo, color_config, output)?;
    } else {
        match output {
            Some(OutputFormat::Json) => {
                let mut details_list = Vec::new();
                for package in &packages {
                    let detail = query_package_detail(run.clone(), query_server, package).await?;
                    details_list.push(detail);
                }
                let list = PackageDetailsList {
                    packages: details_list,
                };
                println!("{}", serde_json::to_string_pretty(&list)?);
            }
            Some(OutputFormat::Pretty) | None => {
                for package in &packages {
                    let detail = query_package_detail(run.clone(), query_server, package).await?;
                    print_package_detail(&detail, color_config);
                }
            }
        }
    }

    Ok(())
}

async fn query_packages(
    run: Arc<dyn QueryRun>,
    query_server: &dyn QueryServer,
    filtered_pkgs: &std::collections::HashSet<PackageName>,
    package_manager_name: &str,
) -> Result<RepositoryDetailsDisplay, cli::Error> {
    let result = query_server
        .execute_query(run, PACKAGES_QUERY, None)
        .await?;

    if !result.errors.is_empty() {
        return Err(Error::QueryError.into());
    }

    let value: serde_json::Value = serde_json::from_str(&result.result_json)?;
    let items = value
        .pointer("/data/packages/items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut packages: Vec<PackageDetailDisplay> = items
        .into_iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.to_string();
            let path = item.get("path")?.as_str()?.to_string();
            let pkg_name = PackageName::from(name.as_str());
            if pkg_name == PackageName::Root {
                return None;
            }
            if !filtered_pkgs.contains(&pkg_name) {
                return None;
            }
            Some(PackageDetailDisplay { name, path })
        })
        .collect();
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(RepositoryDetailsDisplay {
        package_manager: package_manager_name.to_string(),
        packages: ItemsWithCount {
            count: packages.len(),
            items: packages,
        },
    })
}

async fn query_package_detail(
    run: Arc<dyn QueryRun>,
    query_server: &dyn QueryServer,
    package: &str,
) -> Result<PackageDetailsDisplay, cli::Error> {
    let query = package_detail_query(package);
    let result = query_server.execute_query(run, &query, None).await?;

    if !result.errors.is_empty() {
        return Err(Error::PackageNotFound {
            package: package.to_string(),
        }
        .into());
    }

    let value: serde_json::Value = serde_json::from_str(&result.result_json)?;
    let pkg = value
        .pointer("/data/package")
        .ok_or_else(|| Error::PackageNotFound {
            package: package.to_string(),
        })?;

    let name = pkg
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(package)
        .to_string();
    let path = pkg
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let tasks: Vec<PackageTask> = pkg
        .pointer("/tasks/items")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let name = t.get("name")?.as_str()?.to_string();
                    let command = t
                        .get("script")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(PackageTask { name, command })
                })
                .collect()
        })
        .unwrap_or_default();

    let dependencies: Vec<String> = pkg
        .pointer("/allDependencies/items")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let mut deps: Vec<String> = arr
                .iter()
                .filter_map(|d| {
                    let dep_name = d.get("name")?.as_str()?;
                    if dep_name == "//" || dep_name == name {
                        return None;
                    }
                    Some(dep_name.to_string())
                })
                .collect();
            deps.sort();
            deps
        })
        .unwrap_or_default();

    let dependents: Vec<String> = pkg
        .pointer("/allDependents/items")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let mut deps: Vec<String> = arr
                .iter()
                .filter_map(|d| {
                    let dep_name = d.get("name")?.as_str()?;
                    if dep_name == "//" || dep_name == name {
                        return None;
                    }
                    Some(dep_name.to_string())
                })
                .collect();
            deps.sort();
            deps
        })
        .unwrap_or_default();

    Ok(PackageDetailsDisplay {
        name,
        path,
        tasks: ItemsWithCount {
            count: tasks.len(),
            items: tasks,
        },
        dependencies,
        dependents,
    })
}

fn print_repo_details(
    repo: &RepositoryDetailsDisplay,
    color_config: ColorConfig,
    output: Option<OutputFormat>,
) -> Result<(), cli::Error> {
    match output {
        Some(OutputFormat::Json) => {
            println!("{}", serde_json::to_string_pretty(repo)?);
        }
        Some(OutputFormat::Pretty) | None => {
            let package_copy = match repo.packages.count {
                0 => "no packages",
                1 => "package",
                _ => "packages",
            };
            cprint!(
                color_config,
                BOLD,
                "{} {} ",
                repo.packages.count,
                package_copy
            );
            cprintln!(color_config, GREY, "({})\n", repo.package_manager);

            for pkg in &repo.packages.items {
                println!("  {} {}", pkg.name, GREY.apply_to(&pkg.path));
            }
        }
    }
    Ok(())
}

fn print_package_detail(detail: &PackageDetailsDisplay, color_config: ColorConfig) {
    let name = color!(color_config, BOLD_GREEN, "{}", detail.name);
    let depends_on = color!(color_config, BOLD, "depends on");
    let dependencies = if detail.dependencies.is_empty() {
        "<no packages>".to_string()
    } else {
        detail.dependencies.join(", ")
    };

    cprintln!(color_config, GREY, "{} ", detail.path);
    println!(
        "{} {}: {}",
        name,
        depends_on,
        color!(color_config, GREY, "{}", dependencies)
    );
    println!();

    cprint!(color_config, BOLD, "tasks:");
    if detail.tasks.items.is_empty() {
        println!(" <no tasks>");
    } else {
        println!();
    }
    for task in &detail.tasks.items {
        println!(
            "  {}: {}",
            task.name,
            color!(color_config, GREY, "{}", task.command)
        );
    }
    println!();
}
