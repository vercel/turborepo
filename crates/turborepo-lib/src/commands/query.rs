use std::{fs, sync::Arc};

use async_graphql::{EmptyMutation, EmptySubscription, Request, Schema, ServerError, Variables};
use camino::Utf8Path;
use miette::{Diagnostic, Report, SourceSpan};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    commands::CommandBase,
    query,
    query::{Error, RepositoryQuery},
    run::{builder::RunBuilder, Run},
};

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
    graph: bool,
) -> Result<i32, Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run_builder = RunBuilder::new(base)?
        .add_all_tasks()
        .do_not_validate_engine();
    let run = run_builder.build(&handler, telemetry).await?;

    if graph {
        return handle_graph_mode(run).await;
    }

    let query = query.as_deref().or(include_schema.then_some(SCHEMA_QUERY));
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
            &fs::read_to_string(AbsoluteSystemPathBuf::from_unknown(run.repo_root(), query))?
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

        let request = Request::new(query).variables(variables);

        let result = schema.execute(request).await;
        println!("{}", serde_json::to_string_pretty(&result)?);
        if !result.errors.is_empty() {
            for error in result.errors {
                let error = QueryError::new(error, query.to_string());
                eprintln!("{:?}", Report::new(error));
            }
        }
    } else {
        query::run_query_server(run, handler).await?;
    }

    Ok(0)
}

async fn handle_graph_mode(run: Run) -> Result<i32, Error> {
    use std::process::Command;

    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Create the GraphQL schema
    let schema = Schema::new(
        RepositoryQuery::new(Arc::new(run)),
        EmptyMutation,
        EmptySubscription,
    );

    // Query to get package graph data
    let graph_query = r#"
        query {
            packageGraph {
                nodes {
                    items {
                        name
                        path
                    }
                }
                edges {
                    items {
                        source
                        target
                    }
                }
            }
        }
    "#;

    let request = Request::new(graph_query);
    let result = schema.execute(request).await;

    if !result.errors.is_empty() {
        for error in result.errors {
            eprintln!("GraphQL error: {}", error.message);
        }
        return Ok(1);
    }

    // Serialize and base64 encode the graph data
    let graph_data = serde_json::to_string(&result.data)?;
    let encoded_data = STANDARD.encode(graph_data.as_bytes());

    // Determine the base URL based on environment
    let is_dev = std::env::var("NODE_ENV")
        .map(|v| v == "development")
        .unwrap_or(false)
        || std::env::var("CARGO_PROFILE")
            .map(|v| v == "dev")
            .unwrap_or(false);
    let base_url = if is_dev {
        "http://localhost:3000"
    } else {
        "https://turborepo.com"
    };

    let graph_url = format!("{}/graph?data={}", base_url, encoded_data);

    // Open the URL in the default browser
    let status = if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(&["/C", "start", &graph_url])
            .status()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(&graph_url).status()
    } else {
        Command::new("xdg-open").arg(&graph_url).status()
    };

    match status {
        Ok(_) => {
            println!("Opening {} with your package graph data...", graph_url);
            Ok(0)
        }
        Err(e) => {
            eprintln!("Failed to open browser: {}", e);
            println!("Please manually open: {}", graph_url);
            Ok(0)
        }
    }
}
