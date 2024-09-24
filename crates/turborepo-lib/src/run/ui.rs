use std::sync::Arc;

use async_graphql::{EmptyMutation, EmptySubscription, MergedObject, Schema};
use async_graphql_axum::GraphQL;
use axum::{http::Method, routing::get, Router};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use turborepo_ui::wui::{event::WebUIEvent, server::SharedState};

use crate::{query, query::graphiql, run::Run};

pub async fn start_web_ui_server(
    rx: tokio::sync::mpsc::UnboundedReceiver<WebUIEvent>,
    run: Arc<Run>,
) -> Result<(), turborepo_ui::Error> {
    let state = SharedState::default();
    let subscriber = turborepo_ui::wui::subscriber::Subscriber::new(rx);
    tokio::spawn(subscriber.watch(state.clone()));

    run_server(state.clone(), run).await?;

    Ok(())
}

#[derive(MergedObject)]
struct Query(turborepo_ui::wui::RunQuery, query::RepositoryQuery);

async fn run_server(state: SharedState, run: Arc<Run>) -> Result<(), turborepo_ui::Error> {
    let cors = CorsLayer::new()
        // allow `GET` and `POST` when accessing the resource
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
        // allow requests from any origin
        .allow_origin(Any);

    let web_ui_query = turborepo_ui::wui::RunQuery::new(state.clone());
    let turbo_query = query::RepositoryQuery::new(run);
    let combined_query = Query(web_ui_query, turbo_query);

    let schema = Schema::new(combined_query, EmptyMutation, EmptySubscription);
    let app = Router::new()
        .route("/", get(graphiql).post_service(GraphQL::new(schema)))
        .layer(cors);

    axum::serve(
        TcpListener::bind("127.0.0.1:8000")
            .await
            .map_err(turborepo_ui::wui::Error::Server)?,
        app,
    )
    .await
    .map_err(turborepo_ui::wui::Error::Server)?;

    Ok(())
}
