use std::{fs, sync::Arc};

use async_graphql::{EmptyMutation, EmptySubscription, Request, Schema, ServerError, Variables};
use camino::Utf8Path;
use miette::{Diagnostic, Report, SourceSpan};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_query::{QueryRun, RepositoryQuery};
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{cli, commands::CommandBase, run::builder::RunBuilder};

const SCHEMA_QUERY: &str = "query IntrospectionQuery {
  __schema {
    queryType {
      name
    }
    mutationType {
      name
    }
    subscriptionType {
      name
    }
    types {
      ...FullType
    }
    directives {
      name
      description
      locations
      args {
        ...InputValue
      }
    }
  }
}

fragment FullType on __Type {
  kind
  name
  description
  fields(includeDeprecated: true) {
    name
    description
    args {
      ...InputValue
    }
    type {
      ...TypeRef
    }
    isDeprecated
    deprecationReason
  }
  inputFields {
    ...InputValue
  }
  interfaces {
    ...TypeRef
  }
  enumValues(includeDeprecated: true) {
    name
    description
    isDeprecated
    deprecationReason
  }
  possibleTypes {
    ...TypeRef
  }
}

fragment InputValue on __InputValue {
  name
  description
  type {
    ...TypeRef
  }
  defaultValue
}

fragment TypeRef on __Type {
  kind
  name
  ofType {
    kind
    name
    ofType {
      kind
      name
      ofType {
        kind
        name
        ofType {
          kind
          name
          ofType {
            kind
            name
            ofType {
              kind
              name
              ofType {
                kind
                name
              }
            }
          }
        }
      }
    }
  }
}";

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

    let query = query.as_deref().or(include_schema.then_some(SCHEMA_QUERY));
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

        let schema = Schema::new(RepositoryQuery::new(run), EmptyMutation, EmptySubscription);

        let variables: Variables = variables_path
            .map(AbsoluteSystemPathBuf::from_cwd)
            .transpose()
            .map_err(turborepo_query::Error::Path)?
            .map(|path| path.read_to_string())
            .transpose()
            .map_err(turborepo_query::Error::Server)?
            .map(|content| serde_json::from_str(&content))
            .transpose()
            .map_err(turborepo_query::Error::Serde)?
            .unwrap_or_default();

        let request = Request::new(query).variables(variables);

        let result = schema.execute(request).await;
        println!(
            "{}",
            serde_json::to_string_pretty(&result).map_err(turborepo_query::Error::Serde)?
        );
        if !result.errors.is_empty() {
            for error in result.errors {
                let error = QueryError::new(error, query.to_string());
                eprintln!("{:?}", Report::new(error));
            }
        }
    } else {
        turborepo_query::run_query_server(run, handler).await?;
    }

    Ok(0)
}
