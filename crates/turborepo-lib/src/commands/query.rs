use std::{fs, sync::Arc};

use async_graphql::{EmptyMutation, EmptySubscription, Request, Schema, ServerError, Variables};
use camino::Utf8Path;
use miette::{Diagnostic, Report, SourceSpan};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    cli::Command,
    commands::{run::get_signal, CommandBase},
    query,
    query::{Error, RepositoryQuery},
    run::builder::RunBuilder,
    signal::SignalHandler,
};

#[derive(Debug, Diagnostic, Error)]
#[error("{message}")]
struct QueryError {
    message: String,
    #[source_code]
    query: String,
    #[label]
    span: Option<SourceSpan>,
    #[label]
    span2: Option<SourceSpan>,
    #[label]
    span3: Option<SourceSpan>,
}

impl QueryError {
    fn get_index_from_row_column(query: &str, row: usize, column: usize) -> usize {
        let mut index = 0;
        for line in query.lines().take(row.saturating_sub(1)) {
            index += line.len() + 1;
        }
        index + column - 1
    }
    fn new(server_error: ServerError, query: String) -> Self {
        let span: Option<SourceSpan> = server_error.locations.first().map(|location| {
            let idx =
                Self::get_index_from_row_column(query.as_ref(), location.line, location.column);
            (idx, idx + 1).into()
        });

        QueryError {
            message: server_error.message,
            query,
            span,
            span2: None,
            span3: None,
        }
    }
}

pub async fn run(
    mut base: CommandBase,
    telemetry: CommandEventBuilder,
    query: Option<String>,
    variables_path: Option<&Utf8Path>,
) -> Result<i32, Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    // We fake a run command, so we can construct a `Run` type
    base.args_mut().command = Some(Command::Run {
        run_args: Box::default(),
        execution_args: Box::default(),
    });

    let run_builder = RunBuilder::new(base)?
        .add_all_tasks()
        .do_not_validate_engine();
    let run = run_builder.build(&handler, telemetry).await?;

    if let Some(query) = query {
        let trimmed_query = query.trim();
        // If the arg starts with "query" or "mutation", and ends in a bracket, it's
        // likely a direct query If it doesn't, it's a file path, so we need to
        // read it
        let query = if (trimmed_query.starts_with("query")
            || trimmed_query.starts_with("mutation")
            || trimmed_query.starts_with('{'))
            && trimmed_query.ends_with('}')
        {
            query
        } else {
            fs::read_to_string(AbsoluteSystemPathBuf::from_unknown(run.repo_root(), query))?
        };

        let schema = Schema::new(
            RepositoryQuery::new(Arc::new(run)),
            EmptyMutation,
            EmptySubscription,
        );

        let variables: Variables = variables_path
            .map(AbsoluteSystemPathBuf::from_cwd)
            .transpose()?
            .map(|path| path.read_to_string())
            .transpose()?
            .map(|content| serde_json::from_str(&content))
            .transpose()?
            .unwrap_or_default();

        let request = Request::new(&query).variables(variables);

        let result = schema.execute(request).await;
        println!("{}", serde_json::to_string_pretty(&result)?);
        if !result.errors.is_empty() {
            for error in result.errors {
                let error = QueryError::new(error, query.clone());
                eprintln!("{:?}", Report::new(error));
            }
        }
    } else {
        query::run_query_server(run, handler).await?;
    }

    Ok(0)
}
