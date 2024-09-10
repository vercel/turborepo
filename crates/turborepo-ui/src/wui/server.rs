use std::{cell::RefCell, sync::Arc};

use async_graphql::{
    http::GraphiQLSource, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject,
};
use async_graphql_axum::GraphQL;
use axum::{http::Method, response, response::IntoResponse, routing::get, Router};
use serde::Serialize;
use tokio::{net::TcpListener, sync::Mutex};
use tower_http::cors::{Any, CorsLayer};

use crate::wui::{
    event::WebUIEvent,
    subscriber::{Subscriber, TaskState, WebUIState},
    Error,
};

#[derive(Debug, Clone, Serialize, SimpleObject)]
struct Task {
    name: String,
    state: TaskState,
}

struct CurrentRun<'a> {
    state: &'a SharedState,
}

#[Object]
impl<'a> CurrentRun<'a> {
    async fn tasks(&self) -> Vec<Task> {
        self.state
            .lock()
            .await
            .borrow()
            .tasks()
            .iter()
            .map(|(task, state)| Task {
                name: task.clone(),
                state: state.clone(),
            })
            .collect()
    }
}

/// We keep the state in a `Arc<Mutex<RefCell<T>>>` so both `Subscriber` and
/// `Query` can access it, with `Subscriber` mutating it and `Query` only
/// reading it.
type SharedState = Arc<Mutex<RefCell<WebUIState>>>;

pub struct Query {
    state: SharedState,
}

impl Query {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[Object]
impl Query {
    async fn current_run(&self) -> CurrentRun {
        CurrentRun { state: &self.state }
    }
}

async fn graphiql() -> impl IntoResponse {
    response::Html(
        GraphiQLSource::build()
            .endpoint("/")
            .subscription_endpoint("/subscriptions")
            .finish(),
    )
}

pub async fn start_server(
    rx: tokio::sync::mpsc::UnboundedReceiver<WebUIEvent>,
) -> Result<(), crate::Error> {
    let state = Arc::new(Mutex::new(RefCell::new(WebUIState::default())));
    let subscriber = Subscriber::new(rx);
    tokio::spawn(subscriber.watch(state.clone()));

    run_server(state.clone()).await?;

    Ok(())
}

pub(crate) async fn run_server(state: SharedState) -> Result<(), crate::Error> {
    let cors = CorsLayer::new()
        // allow `GET` and `POST` when accessing the resource
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
        // allow requests from any origin
        .allow_origin(Any);

    let schema = Schema::new(Query { state }, EmptyMutation, EmptySubscription);
    let app = Router::new()
        .route("/", get(graphiql).post_service(GraphQL::new(schema)))
        .layer(cors);

    axum::serve(
        TcpListener::bind("127.0.0.1:8000")
            .await
            .map_err(Error::Server)?,
        app,
    )
    .await
    .map_err(Error::Server)?;

    Ok(())
}
