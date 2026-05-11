// Bump all rust changes
#![deny(clippy::all)]

use std::{future::Future, pin::Pin, process, sync::Arc};

use anyhow::Result;
use miette::Report;

const INTERNAL_LSP_COMMAND: &str = "__internal_lsp";

#[derive(Debug, PartialEq)]
enum InternalLspCommand {
    Probe,
    Server,
}

/// Concrete [`turborepo_query_api::QueryServer`] that delegates to
/// `turborepo_query`.
///
/// Lives in the binary crate because it's the only place that depends on both
/// `turborepo-lib` and `turborepo-query`, enabling the dependency inversion
/// that allows them to compile in parallel.
struct TurboQueryServer;

impl turborepo_query_api::QueryServer for TurboQueryServer {
    fn execute_query<'a>(
        &'a self,
        run: Arc<dyn turborepo_query_api::QueryRun>,
        query: &'a str,
        variables_json: Option<&'a str>,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<turborepo_query_api::QueryResult, turborepo_query_api::Error>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            turborepo_query::execute_query(run, query, variables_json)
                .await
                .map_err(Into::into)
        })
    }

    fn run_query_server(
        &self,
        run: Arc<dyn turborepo_query_api::QueryRun>,
        signal: turborepo_signals::SignalHandler,
    ) -> Pin<Box<dyn Future<Output = Result<(), turborepo_query_api::Error>> + Send + '_>> {
        Box::pin(async move {
            turborepo_query::run_query_server(run, signal)
                .await
                .map_err(Into::into)
        })
    }

    fn run_web_ui_server(
        &self,
        state: turborepo_ui::wui::query::SharedState,
        run: Arc<dyn turborepo_query_api::QueryRun>,
    ) -> Pin<Box<dyn Future<Output = Result<(), turborepo_query_api::Error>> + Send + '_>> {
        Box::pin(async move {
            turborepo_query::run_server(Some(state), run)
                .await
                .map_err(Into::into)
        })
    }
}

// This function should not expanded. Please add any logic to
// `turborepo_lib::main` instead
fn main() -> Result<()> {
    if let Some(command) = internal_lsp_command(std::env::args()) {
        if command == InternalLspCommand::Probe {
            println!("turbo-lsp");
            return Ok(());
        }

        turborepo_lsp::run_lsp_server();
        return Ok(());
    }

    std::panic::set_hook(Box::new(turborepo_lib::panic_handler));

    let query_server: Arc<dyn turborepo_lib::QueryServer> = Arc::new(TurboQueryServer);
    let exit_code = turborepo_lib::main(Some(query_server)).unwrap_or_else(|err| {
        eprintln!("{:?}", Report::new(err));
        1
    });

    process::exit(exit_code)
}

fn internal_lsp_command(args: impl IntoIterator<Item = String>) -> Option<InternalLspCommand> {
    let mut args = args.into_iter().skip(1);
    let first_arg = args.next()?;
    let command_arg = if first_arg == "--skip-infer" {
        args.next()?
    } else {
        first_arg
    };

    if command_arg != INTERNAL_LSP_COMMAND {
        return None;
    }

    if args.next().as_deref() == Some("--probe") {
        Some(InternalLspCommand::Probe)
    } else {
        Some(InternalLspCommand::Server)
    }
}

#[cfg(test)]
mod tests {
    use super::{InternalLspCommand, internal_lsp_command};

    fn args(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn detects_internal_lsp_probe() {
        assert_eq!(
            internal_lsp_command(args(&["turbo", "__internal_lsp", "--probe"])),
            Some(InternalLspCommand::Probe)
        );
    }

    #[test]
    fn detects_shimmed_internal_lsp_probe() {
        assert_eq!(
            internal_lsp_command(args(&[
                "turbo",
                "--skip-infer",
                "__internal_lsp",
                "--probe",
                "--",
            ])),
            Some(InternalLspCommand::Probe)
        );
    }

    #[test]
    fn detects_internal_lsp_server() {
        assert_eq!(
            internal_lsp_command(args(&["turbo", "--skip-infer", "__internal_lsp", "--"])),
            Some(InternalLspCommand::Server)
        );
    }

    #[test]
    fn ignores_regular_turbo_command() {
        assert_eq!(internal_lsp_command(args(&["turbo", "run", "build"])), None);
    }
}
