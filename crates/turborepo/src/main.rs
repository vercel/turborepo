// Bump all rust changes
#![deny(clippy::all)]

use std::{future::Future, pin::Pin, process, sync::Arc};

use anyhow::Result;
use miette::Report;

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
    std::panic::set_hook(Box::new(turborepo_lib::panic_handler));

    let query_server: Arc<dyn turborepo_lib::QueryServer> = Arc::new(TurboQueryServer);
    let exit_code = turborepo_lib::main(Some(query_server)).unwrap_or_else(|err| {
        eprintln!("{:?}", Report::new(err));
        1
    });

    process::exit(exit_code)
}
