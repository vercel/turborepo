use std::{fs, sync::Arc};

use camino::Utf8Path;
use miette::{Diagnostic, Report, SourceSpan};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_query::QueryRun;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{cli, commands::CommandBase, run::builder::RunBuilder};

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

    fn from_query_error(error: turborepo_query::QueryErrorLocation, query: String) -> Self {
        let idx = Self::get_index_from_row_column(&query, error.line, error.column);
        QueryError {
            message: error.message,
            query,
            span: Some((idx, idx + 1).into()),
            span2: None,
            span3: None,
        }
    }
}

pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    query: Option<String>,
    variables_path: Option<&Utf8Path>,
    include_schema: bool,
) -> Result<i32, cli::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run_builder = RunBuilder::new(base, None)?
        .add_all_tasks()
        .do_not_validate_engine();
    let (run, _analytics) = run_builder.build(&handler, telemetry).await?;
    let run: Arc<dyn QueryRun> = Arc::new(run);

    let query = query
        .as_deref()
        .or(include_schema.then_some(turborepo_query::SCHEMA_QUERY));
    if let Some(query) = query {
        let trimmed_query = query.trim();
        let query = if (trimmed_query.starts_with("query")
            || trimmed_query.starts_with("mutation")
            || trimmed_query.starts_with('{'))
            && trimmed_query.ends_with('}')
        {
            query
        } else {
            &fs::read_to_string(AbsoluteSystemPathBuf::from_unknown(run.repo_root(), query))
                .map_err(turborepo_query::Error::Server)?
        };

        let variables_json = variables_path
            .map(AbsoluteSystemPathBuf::from_cwd)
            .transpose()
            .map_err(turborepo_query::Error::Path)?
            .map(|path| path.read_to_string())
            .transpose()
            .map_err(turborepo_query::Error::Server)?;

        let result = turborepo_query::execute_query(run, query, variables_json.as_deref()).await?;

        println!("{}", result.result_json);
        if !result.errors.is_empty() {
            for error in result.errors {
                let error = QueryError::from_query_error(error, query.to_string());
                eprintln!("{:?}", Report::new(error));
            }
        }
    } else {
        turborepo_query::run_query_server(run, handler).await?;
    }

    Ok(0)
}
