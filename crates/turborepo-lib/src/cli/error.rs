use std::backtrace;

use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;
use turborepo_repository::package_graph;
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{color, BOLD, GREY};

use crate::{
    commands::{bin, generate, prune, run::get_signal, CommandBase},
    daemon::DaemonError,
    rewrite_json::RewriteError,
    run,
    run::{builder::RunBuilder, watch},
    signal::SignalHandler,
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("No command specified")]
    NoCommand(#[backtrace] backtrace::Backtrace),
    #[error("{0}")]
    Bin(#[from] bin::Error, #[backtrace] backtrace::Backtrace),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    ChromeTracing(#[from] crate::tracing::Error),
    #[error(transparent)]
    BuildPackageGraph(#[from] package_graph::builder::Error),
    #[error(transparent)]
    Rewrite(#[from] RewriteError),
    #[error(transparent)]
    Auth(#[from] turborepo_auth::Error),
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    #[error(transparent)]
    Generate(#[from] generate::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Prune(#[from] prune::Error),
    #[error(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(transparent)]
    PackageManager(#[from] turborepo_repository::package_manager::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Run(#[from] run::Error),
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Watch(#[from] watch::Error),
}

const MAX_CHARS_PER_TASK_LINE: usize = 100;

pub async fn print_potential_tasks(base: CommandBase, telemetry: CommandEventBuilder) {
    let output: Result<_, Error> = try {
        let signal = get_signal()?;
        let handler = SignalHandler::new(signal);
        let ui = base.ui;

        let run_builder = RunBuilder::new(base)?;
        let run = run_builder.build(&handler, telemetry).await?;
        let potential_tasks = run.get_potential_tasks()?;

        potential_tasks
            .into_iter()
            .sorted_by(|(a, _), (b, _)| a.cmp(b))
            .map(|(task, packages)| {
                let task = color!(ui, BOLD, "{}", task);
                let mut line_length = 0;

                let mut packages_str = String::with_capacity(80);
                for (idx, package) in packages.iter().enumerate() {
                    if line_length > MAX_CHARS_PER_TASK_LINE {
                        if idx != packages.len() {
                            packages_str.push_str(&format!(" and {} more", packages.len() - idx));
                        }

                        break;
                    }

                    line_length += package.len() + 2;
                    if idx != 0 {
                        packages_str.push_str(", ");
                    }
                    packages_str.push_str(&format!("{}", package));
                }

                let packages = color!(ui, GREY, "> {}", packages_str);

                format!("{}\n  {}", task, packages)
            })
            .join("\n")
    };

    // We don't want to show a random error if someone is running `turbo run`
    // without any tasks. Instead, we'll just show the no tasks error and exit.
    match output {
        Ok(output) => {
            println!(
                "No tasks provided, here are some potential ones to run\n\n{}",
                output
            );
        }
        Err(_) => {
            println!("No tasks provided");
        }
    }
}
