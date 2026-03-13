use std::{fmt::Write, fs, sync::Arc};

use camino::Utf8Path;
use miette::{Diagnostic, Report, SourceSpan};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_query_api::{QueryRun, QueryServer};
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    cli::{self, AffectedArgs, QuerySubcommand},
    commands::CommandBase,
    run::builder::RunBuilder,
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

async fn execute_query_and_print(
    run: Arc<dyn QueryRun>,
    query_server: &dyn QueryServer,
    query: &str,
    variables_json: Option<&str>,
) -> Result<i32, cli::Error> {
    let result = query_server
        .execute_query(run, query, variables_json)
        .await?;

    println!("{}", result.result_json);
    if !result.errors.is_empty() {
        for error in result.errors {
            let error = QueryError::from_query_error(error, query.to_string());
            eprintln!("{:?}", Report::new(error));
        }
        return Ok(1);
    }
    Ok(0)
}

fn escape_graphql_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if c.is_control() => {
                write!(out, "\\u{:04X}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out
}

impl AffectedArgs {
    fn to_graphql_query(&self) -> String {
        // --packages alone → affectedPackages
        // Everything else (default, --tasks, --tasks + --packages) → affectedTasks
        if self.packages.is_some() && self.tasks.is_none() {
            self.build_affected_packages_query()
        } else {
            self.build_affected_tasks_query()
        }
    }

    fn build_affected_packages_query(&self) -> String {
        let mut query = String::from("{ affectedPackages");
        let mut args = self.build_ref_args();
        self.push_package_filter(&mut args);
        if !args.is_empty() {
            let joined = args.join(", ");
            write!(query, "({joined})").unwrap();
        }
        query.push_str(" { items { name path reason { __typename } } length } }");
        query
    }

    fn build_affected_tasks_query(&self) -> String {
        let mut query = String::from("{ affectedTasks");
        let mut args = self.build_ref_args();
        let tasks = self.tasks.as_deref().unwrap_or_default();
        if !tasks.is_empty() {
            let task_values: Vec<String> = tasks
                .iter()
                .map(|t| format!("\"{}\"", escape_graphql_string(t)))
                .collect();
            args.push(format!("tasks: [{}]", task_values.join(", ")));
        }
        self.push_package_filter(&mut args);
        if !args.is_empty() {
            let joined = args.join(", ");
            write!(query, "({joined})").unwrap();
        }
        query.push_str(
            " { items { name fullName package { name } reason { __typename } } length } }",
        );
        query
    }

    fn build_ref_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(ref base) = self.base {
            args.push(format!("base: \"{}\"", escape_graphql_string(base)));
        }
        if let Some(ref head) = self.head {
            args.push(format!("head: \"{}\"", escape_graphql_string(head)));
        }
        args
    }

    fn push_package_filter(&self, args: &mut Vec<String>) {
        let packages = self.packages.as_deref().unwrap_or_default();
        if packages.is_empty() {
            return;
        }
        let filter = if packages.len() == 1 {
            format!(
                "{{ equal: {{ field: NAME, value: \"{}\" }} }}",
                escape_graphql_string(&packages[0])
            )
        } else {
            let predicates: Vec<String> = packages
                .iter()
                .map(|p| {
                    format!(
                        "{{ equal: {{ field: NAME, value: \"{}\" }} }}",
                        escape_graphql_string(p)
                    )
                })
                .collect();
            format!("{{ or: [{}] }}", predicates.join(", "))
        };
        args.push(format!("filter: {filter}"));
    }
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
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run_builder = RunBuilder::new(base, None)?
        .add_all_tasks()
        .do_not_validate_engine();
    let (run, _analytics) = run_builder.build(&handler, telemetry).await?;
    let run: Arc<dyn QueryRun> = Arc::new(run);

    if let Some(subcommand) = subcommand {
        let query = match &subcommand {
            QuerySubcommand::Affected(args) => args.to_graphql_query(),
        };
        return execute_query_and_print(run, query_server, &query, None).await;
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
            &fs::read_to_string(AbsoluteSystemPathBuf::from_unknown(run.repo_root(), query))
                .map_err(turborepo_query_api::Error::Server)?
        };

        let variables_json = variables_path
            .map(AbsoluteSystemPathBuf::from_cwd)
            .transpose()
            .map_err(turborepo_query_api::Error::Path)?
            .map(|path| path.read_to_string())
            .transpose()
            .map_err(turborepo_query_api::Error::Server)?;

        execute_query_and_print(run, query_server, query, variables_json.as_deref()).await
    } else {
        query_server.run_query_server(run, handler).await?;
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::escape_graphql_string;
    use crate::cli::AffectedArgs;

    fn affected(
        packages: Option<Vec<&str>>,
        tasks: Option<Vec<&str>>,
        base: Option<&str>,
        head: Option<&str>,
    ) -> AffectedArgs {
        AffectedArgs {
            packages: packages.map(|v| v.into_iter().map(String::from).collect()),
            tasks: tasks.map(|v| v.into_iter().map(String::from).collect()),
            base: base.map(String::from),
            head: head.map(String::from),
        }
    }

    // -- escape tests --

    #[test]
    fn escape_noop_for_plain_strings() {
        assert_eq!(escape_graphql_string("main"), "main");
        assert_eq!(escape_graphql_string("my-app"), "my-app");
    }

    #[test]
    fn escape_double_quotes() {
        assert_eq!(escape_graphql_string(r#"a"b"#), r#"a\"b"#);
    }

    #[test]
    fn escape_backslashes() {
        assert_eq!(escape_graphql_string(r"a\b"), r"a\\b");
    }

    #[test]
    fn escape_combined() {
        assert_eq!(escape_graphql_string(r#"a\"b"#), r#"a\\\"b"#);
    }

    #[test]
    fn escape_newline() {
        assert_eq!(escape_graphql_string("a\nb"), "a\\nb");
    }

    #[test]
    fn escape_carriage_return() {
        assert_eq!(escape_graphql_string("a\rb"), "a\\rb");
    }

    #[test]
    fn escape_tab() {
        assert_eq!(escape_graphql_string("a\tb"), "a\\tb");
    }

    #[test]
    fn escape_null_byte() {
        assert_eq!(escape_graphql_string("a\x00b"), "a\\u0000b");
    }

    #[test]
    fn escape_unicode_passthrough() {
        assert_eq!(escape_graphql_string("日本語"), "日本語");
    }

    #[test]
    fn escape_empty() {
        assert_eq!(escape_graphql_string(""), "");
    }

    // -- default behavior: affected tasks --

    #[test]
    fn no_flags_defaults_to_affected_tasks() {
        let q = affected(None, None, None, None).to_graphql_query();
        assert_eq!(
            q,
            "{ affectedTasks { items { name fullName package { name } reason { __typename } } \
             length } }"
        );
    }

    #[test]
    fn bare_tasks_flag_returns_all_affected_tasks() {
        let q = affected(None, Some(vec![]), None, None).to_graphql_query();
        assert_eq!(
            q,
            "{ affectedTasks { items { name fullName package { name } reason { __typename } } \
             length } }"
        );
    }

    #[test]
    fn tasks_with_values_filters() {
        let q = affected(None, Some(vec!["build"]), None, None).to_graphql_query();
        assert!(q.starts_with("{ affectedTasks"), "{q}");
        assert!(q.contains(r#"tasks: ["build"]"#), "{q}");
    }

    #[test]
    fn multiple_tasks_all_appear() {
        let q = affected(None, Some(vec!["build", "test"]), None, None).to_graphql_query();
        assert!(q.contains(r#"tasks: ["build", "test"]"#), "{q}");
    }

    // -- --packages routes to affected packages --

    #[test]
    fn bare_packages_flag_returns_all_affected_packages() {
        let q = affected(Some(vec![]), None, None, None).to_graphql_query();
        assert_eq!(
            q,
            "{ affectedPackages { items { name path reason { __typename } } length } }"
        );
    }

    #[test]
    fn single_package_uses_equal_filter() {
        let q = affected(Some(vec!["web"]), None, None, None).to_graphql_query();
        assert!(q.starts_with("{ affectedPackages"), "{q}");
        assert!(q.contains(r#"equal: { field: NAME, value: "web" }"#), "{q}");
        assert!(!q.contains("or:"), "single package should not use or: {q}");
    }

    #[test]
    fn multiple_packages_use_or_filter() {
        let q = affected(Some(vec!["web", "docs"]), None, None, None).to_graphql_query();
        assert!(q.contains("or: ["), "{q}");
        assert!(q.contains(r#"value: "web""#), "{q}");
        assert!(q.contains(r#"value: "docs""#), "{q}");
    }

    // -- ref args --

    #[test]
    fn base_and_head_appear_in_tasks_query() {
        let q = affected(None, None, Some("main"), Some("HEAD")).to_graphql_query();
        assert!(q.starts_with("{ affectedTasks"), "{q}");
        assert!(q.contains(r#"base: "main""#), "{q}");
        assert!(q.contains(r#"head: "HEAD""#), "{q}");
    }

    #[test]
    fn base_and_head_appear_in_packages_query() {
        let q = affected(Some(vec![]), None, Some("main"), Some("HEAD")).to_graphql_query();
        assert!(q.starts_with("{ affectedPackages"), "{q}");
        assert!(q.contains(r#"base: "main""#), "{q}");
        assert!(q.contains(r#"head: "HEAD""#), "{q}");
    }

    // -- escaping in context --

    #[test]
    fn base_with_quotes_is_escaped() {
        let q = affected(None, None, Some(r#"feat/"branch"#), None).to_graphql_query();
        assert!(
            q.contains(r#"base: "feat/\"branch""#),
            "quotes should be escaped: {q}"
        );
    }

    #[test]
    fn package_with_quotes_is_escaped() {
        let q = affected(Some(vec![r#"@scope/"pkg""#]), None, None, None).to_graphql_query();
        assert!(
            q.contains(r#"value: "@scope/\"pkg\""#),
            "package quotes should be escaped: {q}"
        );
    }

    #[test]
    fn task_with_quotes_is_escaped() {
        let q = affected(None, Some(vec![r#"build"inject"#]), None, None).to_graphql_query();
        assert!(
            q.contains(r#""build\"inject""#),
            "task quotes should be escaped: {q}"
        );
    }

    #[test]
    fn head_with_backslash_is_escaped() {
        let q = affected(None, None, None, Some(r"ref\path")).to_graphql_query();
        assert!(
            q.contains(r#"head: "ref\\path""#),
            "backslash should be escaped: {q}"
        );
    }

    // -- combined --packages + --tasks → affectedTasks with both filters
    // (intersection) --

    #[test]
    fn combined_packages_and_tasks_routes_to_affected_tasks() {
        let q = affected(Some(vec!["web"]), Some(vec!["build"]), None, None).to_graphql_query();
        assert!(q.starts_with("{ affectedTasks"), "{q}");
        assert!(q.contains(r#"tasks: ["build"]"#), "{q}");
        assert!(
            q.contains(r#"filter: { equal: { field: NAME, value: "web" } }"#),
            "{q}"
        );
    }

    #[test]
    fn combined_bare_tasks_with_packages_filters_by_package_only() {
        // --tasks (bare) + --packages web → affectedTasks with only package filter
        let q = affected(Some(vec!["web"]), Some(vec![]), None, None).to_graphql_query();
        assert!(q.starts_with("{ affectedTasks"), "{q}");
        assert!(
            !q.contains("tasks:"),
            "bare --tasks should not add tasks arg: {q}"
        );
        assert!(q.contains("filter:"), "{q}");
    }

    #[test]
    fn combined_tasks_with_bare_packages_filters_by_task_only() {
        // --tasks build + --packages (bare) → affectedTasks with only task filter
        let q = affected(Some(vec![]), Some(vec!["build"]), None, None).to_graphql_query();
        assert!(q.starts_with("{ affectedTasks"), "{q}");
        assert!(q.contains(r#"tasks: ["build"]"#), "{q}");
        assert!(
            !q.contains("filter:"),
            "bare --packages should not add filter: {q}"
        );
    }
}
