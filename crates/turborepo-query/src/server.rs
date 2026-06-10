use std::sync::Arc;

use async_graphql::{EmptyMutation, EmptySubscription, Schema};
use async_graphql_axum::GraphQL;
use axum::{http::Method, routing::get, Router};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};

use crate::{graphiql, QueryRun, RepositoryQuery};

pub async fn run_server(run: Arc<dyn QueryRun>) -> std::io::Result<()> {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
        .allow_origin(Any);

    let turbo_query = RepositoryQuery::new(run);

    let schema = Schema::new(turbo_query, EmptyMutation, EmptySubscription);
    let app = Router::new()
        .route("/", get(graphiql).post_service(GraphQL::new(schema)))
        .layer(cors);

    axum::serve(TcpListener::bind("127.0.0.1:8000").await?, app).await?;

    Ok(())
}
