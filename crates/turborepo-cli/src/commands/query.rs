use std::{env, fs, io::Write as IoWrite, sync::Arc};

use camino::Utf8Path;
use miette::{Diagnostic, Report, SourceSpan};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_query::affected_query::{
    AffectedQueryInput, AffectedQuerySelector, affected_query_exit_code, build_affected_query,
};
use turborepo_query_api::{QueryRun, QueryServer};
use turborepo_run::builder::RunBuilder;
use turborepo_signals::{SignalHandler, listeners::get_signal};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    cli::{self, QuerySubcommand},
    commands::{CommandBase, ls},
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

    fn from_query_error(error: turborepo_query_api::QueryErrorLocation, query: String) -> Self {
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

/// Execute a GraphQL query, print results to stdout, and report errors to
/// stderr. Returns `(exit_code, result_json)` so callers can inspect the
/// response for post-processing (e.g. `--exit-code`).
async fn execute_query_and_print(
    run: Arc<dyn QueryRun>,
    query_server: &dyn QueryServer,
    query: &str,
    variables_json: Option<&str>,
    version: &str,
) -> Result<(i32, String), cli::Error> {
    execute_query_and_write(
        run,
        query_server,
        query,
        variables_json,
        version,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
    .await
}

/// Prepend a `version` key to a JSON object so the output stays a single
/// parseable document (the version banner is suppressed for query output).
/// Non-object or unparseable input is returned unchanged.
fn with_version_key(result_json: &str, version: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(result_json) {
        Ok(serde_json::Value::Object(fields)) => {
            let mut output = serde_json::Map::with_capacity(fields.len() + 1);
            output.insert("version".to_string(), version.into());
            output.extend(fields.into_iter().filter(|(key, _)| key != "version"));
            serde_json::to_string_pretty(&serde_json::Value::Object(output))
                .unwrap_or_else(|_| result_json.to_string())
        }
        _ => result_json.to_string(),
    }
}

/// The same query adapter with injectable output streams for in-process tests.
async fn execute_query_and_write(
    run: Arc<dyn QueryRun>,
    query_server: &dyn QueryServer,
    query: &str,
    variables_json: Option<&str>,
    version: &str,
    stdout: &mut (impl IoWrite + Send),
    stderr: &mut (impl IoWrite + Send),
) -> Result<(i32, String), cli::Error> {
    let result = query_server
        .execute_query(run, query, variables_json)
        .await?;

    writeln!(stdout, "{}", with_version_key(&result.result_json, version))?;
    if !result.errors.is_empty() {
        for error in result.errors {
            let error = QueryError::from_query_error(error, query.to_string());
            writeln!(stderr, "{:?}", Report::new(error))?;
        }
        return Ok((2, result.result_json));
    }
    Ok((0, result.result_json))
}

pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    subcommand: Option<QuerySubcommand>,
    query: Option<String>,
    variables_path: Option<&Utf8Path>,
    include_schema: bool,
    query_server: &dyn QueryServer,
) -> Result<i32, cli::Error> {
    // `turbo query ls` builds its own Run with filter/affected opts,
    // so handle it before constructing the general-purpose query Run.
    if let Some(QuerySubcommand::Ls(ls_args)) = subcommand {
        ls::run(
            base,
            ls_args.packages,
            telemetry,
            ls_args.output,
            query_server,
        )
        .await?;
        return Ok(0);
    }

    let version = base.version();
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run_builder = RunBuilder::new(base.run_builder_input()?, None)?
        .add_all_tasks()
        .do_not_validate_engine();
    let (run, _analytics) = run_builder.build(&handler, telemetry).await?;
    let run: Arc<dyn QueryRun> = Arc::new(run);

    if let Some(subcommand) = subcommand {
        match &subcommand {
            QuerySubcommand::Affected(args) => {
                let input = AffectedQueryInput {
                    selector: if args.packages.is_some() && args.tasks.is_none() {
                        AffectedQuerySelector::Packages
                    } else {
                        AffectedQuerySelector::Tasks
                    },
                    base: args.base.clone(),
                    head: args.head.clone(),
                    scm_base: env::var("TURBO_SCM_BASE").ok(),
                    scm_head: env::var("TURBO_SCM_HEAD").ok(),
                    package_filters: args.packages.clone().unwrap_or_default(),
                    task_filters: args.tasks.clone().unwrap_or_default(),
                };
                let query = build_affected_query(&input);
                let (exit_code, result_json) =
                    execute_query_and_print(run, query_server, &query, None, version).await?;

                if exit_code != 0 {
                    return Ok(exit_code);
                }

                if args.exit_code {
                    return match affected_query_exit_code(&result_json) {
                        Some(exit_code) => Ok(exit_code),
                        None => {
                            eprintln!(
                                "error: could not determine affected count from query result"
                            );
                            Ok(2)
                        }
                    };
                }

                return Ok(0);
            }
            QuerySubcommand::Ls(_) => unreachable!("handled above"),
        }
    }

    let query = query
        .as_deref()
        .or(include_schema.then_some(turborepo_query_api::SCHEMA_QUERY));
    if let Some(query) = query {
        let trimmed_query = query.trim();
        let query = if (trimmed_query.starts_with("query")
            || trimmed_query.starts_with("mutation")
            || trimmed_query.starts_with('{'))
            && trimmed_query.ends_with('}')
        {
            query
        } else {
            &fs::read_to_string(AbsoluteSystemPathBuf::from_unknown(
                run.repo_context().repo_root(),
                query,
            ))
            .map_err(turborepo_query_api::Error::Server)?
        };

        let variables_json = variables_path
            .map(AbsoluteSystemPathBuf::from_cwd)
            .transpose()
            .map_err(turborepo_query_api::Error::Path)?
            .map(|path| path.read_to_string())
            .transpose()
            .map_err(turborepo_query_api::Error::Server)?;

        let (exit_code, _) =
            execute_query_and_print(run, query_server, query, variables_json.as_deref(), version)
                .await?;
        Ok(exit_code)
    } else {
        query_server.run_query_server(run, handler).await?;
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    #[derive(Default)]
    struct RecordingRun(std::sync::atomic::AtomicUsize);

    impl turborepo_query_api::QueryRun for RecordingRun {
        fn repo_context(&self) -> &turborepo_run_context::RepoContext {
            unreachable!("the recording query server does not inspect repository data")
        }

        fn task_ids(&self) -> Vec<turborepo_query_api::QueryTaskId> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Vec::new()
        }

        fn task_ids_for_package(&self, _package: &str) -> Vec<turborepo_query_api::QueryTaskId> {
            Vec::new()
        }

        fn task_definition(
            &self,
            _task: &turborepo_query_api::QueryTaskId,
        ) -> Option<&turborepo_types::TaskDefinition> {
            None
        }

        fn task_dependencies(
            &self,
            _task: &turborepo_query_api::QueryTaskId,
        ) -> Vec<turborepo_query_api::QueryTaskId> {
            Vec::new()
        }

        fn task_dependents(
            &self,
            _task: &turborepo_query_api::QueryTaskId,
        ) -> Vec<turborepo_query_api::QueryTaskId> {
            Vec::new()
        }

        fn transitive_task_dependencies(
            &self,
            _task: &turborepo_query_api::QueryTaskId,
        ) -> Vec<turborepo_query_api::QueryTaskId> {
            Vec::new()
        }

        fn transitive_task_dependents(
            &self,
            _task: &turborepo_query_api::QueryTaskId,
        ) -> Vec<turborepo_query_api::QueryTaskId> {
            Vec::new()
        }

        fn collect_task_dependencies(
            &self,
            _tasks: &std::collections::HashSet<turborepo_query_api::QueryTaskId>,
        ) -> std::collections::HashSet<turborepo_query_api::QueryTaskId> {
            Default::default()
        }

        fn calculate_affected_packages(
            &self,
            _base: Option<String>,
            _head: Option<String>,
        ) -> Result<
            std::collections::HashMap<
                turborepo_repository::package_graph::PackageName,
                turborepo_repository::change_mapper::PackageInclusionReason,
            >,
            turborepo_query_api::AffectedPackagesError,
        > {
            Ok(Default::default())
        }

        fn changed_files(
            &self,
            _base: Option<&str>,
            _head: Option<&str>,
        ) -> Result<
            std::collections::HashSet<turbopath::AnchoredSystemPathBuf>,
            turborepo_query_api::AffectedPackagesError,
        > {
            Ok(Default::default())
        }

        fn match_tasks_against_changed_files(
            &self,
            _files: &std::collections::HashSet<turbopath::AnchoredSystemPathBuf>,
        ) -> Result<
            std::collections::HashMap<turborepo_query_api::QueryTaskId, String>,
            turborepo_query_api::AffectedPackagesError,
        > {
            Ok(Default::default())
        }

        fn check_boundaries(
            &self,
            _show_progress: bool,
        ) -> turborepo_query_api::BoundariesFuture<'_> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    #[derive(Default)]
    struct RecordingServer {
        calls: std::sync::Mutex<Vec<(String, Option<String>)>>,
        fail: bool,
    }

    impl turborepo_query_api::QueryServer for RecordingServer {
        fn execute_query<'a>(
            &'a self,
            run: std::sync::Arc<dyn turborepo_query_api::QueryRun>,
            query: &'a str,
            variables_json: Option<&'a str>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            turborepo_query_api::QueryResult,
                            turborepo_query_api::Error,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let _ = run.task_ids();
                self.calls
                    .lock()
                    .unwrap()
                    .push((query.to_string(), variables_json.map(str::to_string)));
                Ok(turborepo_query_api::QueryResult {
                    result_json: r#"{"data":{"version":"fixture"}}"#.to_string(),
                    errors: self
                        .fail
                        .then(|| turborepo_query_api::QueryErrorLocation {
                            message: "fixture error".to_string(),
                            line: 1,
                            column: 1,
                        })
                        .into_iter()
                        .collect(),
                })
            })
        }

        fn run_query_server(
            &self,
            _run: std::sync::Arc<dyn turborepo_query_api::QueryRun>,
            _signal: turborepo_signals::SignalHandler,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<(), turborepo_query_api::Error>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async { unreachable!("no network server is started by this test") })
        }
    }

    #[tokio::test]
    async fn query_adapter_forwards_run_query_variables_and_errors() {
        let run = std::sync::Arc::new(RecordingRun::default());
        for (fail, expected_exit) in [(false, 0), (true, 2)] {
            let server = RecordingServer {
                fail,
                ..Default::default()
            };
            let run_api: std::sync::Arc<dyn turborepo_query_api::QueryRun> = run.clone();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let (exit, json) = super::execute_query_and_write(
                run_api,
                &server,
                "query { version }",
                Some(r#"{"name":"app"}"#),
                "1.2.3",
                &mut stdout,
                &mut stderr,
            )
            .await
            .unwrap();
            assert_eq!(exit, expected_exit);
            assert_eq!(json, r#"{"data":{"version":"fixture"}}"#);
            assert_eq!(
                String::from_utf8(stdout).unwrap(),
                "{\n  \"version\": \"1.2.3\",\n  \"data\": {\n    \"version\": \"fixture\"\n  \
                 }\n}\n"
            );
            let diagnostic = String::from_utf8(stderr).unwrap();
            if fail {
                assert!(diagnostic.contains("fixture error"), "{diagnostic}");
                assert!(diagnostic.contains("query { version }"), "{diagnostic}");
            } else {
                assert!(diagnostic.is_empty(), "{diagnostic}");
            }
            assert_eq!(
                *server.calls.lock().unwrap(),
                [(
                    "query { version }".to_string(),
                    Some(r#"{"name":"app"}"#.to_string())
                )]
            );
        }
        assert_eq!(run.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
