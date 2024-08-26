//! Web UI for Turborepo. Creates a WebSocket server that can be subscribed to
//! by a web client to display the status of tasks.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    io::Write,
    sync::Arc,
};

use async_graphql::{
    futures_util::Stream, http::GraphiQLSource, EmptyMutation, Object, Schema, SimpleObject,
    Subscription, Union,
};
use async_graphql_axum::{GraphQL, GraphQLSubscription};
use async_stream::stream;
use axum::{http::Method, response, response::IntoResponse, routing::get, Router};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{net::TcpListener, sync::Mutex};
use tower_http::cors::{Any, CorsLayer};
use tracing::log::warn;

use crate::{
    sender::{TaskSender, UISender},
    tui::event::{CacheResult, OutputLogs, TaskResult},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to start server")]
    Server(#[from] std::io::Error),
    #[error("failed to start websocket server: {0}")]
    WebSocket(#[source] axum::Error),
    #[error("failed to serialize message: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("failed to send message")]
    Send(#[from] axum::Error),
    #[error("failed to send message through channel")]
    Broadcast(#[from] tokio::sync::broadcast::error::SendError<WebUIEvent>),
}

#[derive(Debug, Clone)]
pub struct WebUISender {
    pub tx: tokio::sync::broadcast::Sender<WebUIEvent>,
}

impl WebUISender {
    pub fn start_task(&self, task: String, output_logs: OutputLogs) {
        self.tx
            .send(WebUIEvent::StartTask(StartTask { task, output_logs }))
            .ok();
    }

    pub fn restart_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::RestartTasks(RestartTasks { tasks }))
            .map_err(Error::Broadcast)?;
        Ok(())
    }

    pub fn end_task(&self, task: String, result: TaskResult) {
        self.tx
            .send(WebUIEvent::EndTask(EndTask { task, result }))
            .ok();
    }

    pub fn status(&self, task: String, status: String, result: CacheResult) {
        self.tx
            .send(WebUIEvent::Status(Status {
                task,
                status,
                result,
            }))
            .ok();
    }

    pub fn set_stdin(&self, _: String, _: Box<dyn Write + Send>) {
        warn!("stdin is not supported (yet) in web ui");
    }

    pub fn task(&self, task: String) -> TaskSender {
        TaskSender {
            name: task,
            handle: UISender::Wui(self.clone()),
            logs: Default::default(),
        }
    }

    pub fn stop(&self) {
        self.tx.send(WebUIEvent::Stop(Stop { _stop: true })).ok();
    }

    pub fn update_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::UpdateTasks(UpdateTasks { tasks }))
            .map_err(Error::Broadcast)?;

        Ok(())
    }

    pub fn output(&self, task: String, output: Vec<u8>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::TaskOutput(TaskOutput { task, output }))
            .map_err(Error::Broadcast)?;

        Ok(())
    }
}

#[derive(Debug, Clone, SimpleObject, Serialize)]
pub struct StartTask {
    task: String,
    output_logs: OutputLogs,
}

#[derive(Debug, Clone, SimpleObject, Serialize)]
pub struct TaskOutput {
    task: String,
    output: Vec<u8>,
}

#[derive(Debug, Clone, SimpleObject, Serialize)]
pub struct EndTask {
    task: String,
    result: TaskResult,
}

#[derive(Debug, Clone, SimpleObject, Serialize)]
pub struct Status {
    task: String,
    status: String,
    result: CacheResult,
}

#[derive(Debug, Clone, SimpleObject, Serialize)]
pub struct UpdateTasks {
    tasks: Vec<String>,
}

#[derive(Debug, Clone, SimpleObject, Serialize)]
pub struct RestartTasks {
    tasks: Vec<String>,
}

#[derive(Debug, Clone, SimpleObject, Serialize)]
pub struct Stop {
    // This doesn't actually do anything, but we need a field for GraphQL
    _stop: bool,
}

#[derive(Debug, Clone, Serialize, Union)]
enum SubscriptionMessage {
    InitialState(WebUIState),
    #[graphql(flatten)]
    Event(WebUIEvent),
}

// Specific events that the GraphQL server can send to the client,
// not all the `Event` types from the TUI.
//
// We have to put each variant in a new struct because async graphql doesn't
// allow enums with fields for union types
#[derive(Debug, Clone, Serialize, Union)]
#[serde(tag = "type", content = "payload")]
pub enum WebUIEvent {
    StartTask(StartTask),
    TaskOutput(TaskOutput),
    EndTask(EndTask),
    Status(Status),
    UpdateTasks(UpdateTasks),
    RestartTasks(RestartTasks),
    Stop(Stop),
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerMessage<'a> {
    pub id: u32,
    #[serde(flatten)]
    pub payload: &'a WebUIEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ClientMessage {
    /// Acknowledges the receipt of a message.
    /// If we don't receive an ack, we will resend the message
    Ack { id: u32 },
    /// Asks for all messages from the given id onwards
    CatchUp { start_id: u32 },
}

#[derive(Debug, Clone, Serialize, SimpleObject)]
struct TaskState {
    output: Vec<u8>,
    status: Option<String>,
    result: Option<TaskResult>,
    cache_result: Option<CacheResult>,
}

#[derive(Debug, Default, Clone, Serialize, SimpleObject)]
struct WebUIState {
    tasks: BTreeMap<String, TaskState>,
}

struct Subscriber {
    rx: tokio::sync::broadcast::Receiver<WebUIEvent>,
    // We use a tokio::sync::Mutex here because we want this future to be Send.
    #[allow(clippy::type_complexity)]
    state: Arc<Mutex<RefCell<WebUIState>>>,
}

impl Subscriber {
    fn new(rx: tokio::sync::broadcast::Receiver<WebUIEvent>) -> Self {
        Self {
            rx,
            state: Default::default(),
        }
    }

    fn watch(&self) {
        let mut rx = self.rx.resubscribe();
        let state = self.state.clone();
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                Self::add_message(&state, event).await;
            }
        });
    }

    async fn add_message(state: &Arc<Mutex<RefCell<WebUIState>>>, event: WebUIEvent) {
        let state = state.lock().await;

        match event {
            WebUIEvent::StartTask(StartTask {
                task,
                output_logs: _,
            }) => {
                state.borrow_mut().tasks.insert(
                    task,
                    TaskState {
                        output: Vec::new(),
                        status: None,
                        result: None,
                        cache_result: None,
                    },
                );
            }
            WebUIEvent::TaskOutput(TaskOutput { task, output }) => {
                state
                    .borrow_mut()
                    .tasks
                    .get_mut(&task)
                    .unwrap()
                    .output
                    .extend(output);
            }
            WebUIEvent::EndTask(EndTask { task, result }) => {
                state.borrow_mut().tasks.get_mut(&task).unwrap().result = Some(result);
            }
            WebUIEvent::Status(Status { task, result, .. }) => {
                state
                    .borrow_mut()
                    .tasks
                    .get_mut(&task)
                    .unwrap()
                    .cache_result = Some(result);
            }
            WebUIEvent::Stop(_) => {
                // TODO: stop watching
            }
            WebUIEvent::UpdateTasks(UpdateTasks { tasks }) => {
                state.borrow_mut().tasks = tasks
                    .into_iter()
                    .map(|task| {
                        (
                            task,
                            TaskState {
                                output: Vec::new(),
                                status: None,
                                result: None,
                                cache_result: None,
                            },
                        )
                    })
                    .collect();
            }
            WebUIEvent::RestartTasks(RestartTasks { tasks }) => {
                state.borrow_mut().tasks = tasks
                    .into_iter()
                    .map(|task| {
                        (
                            task,
                            TaskState {
                                output: Vec::new(),
                                status: None,
                                result: None,
                                cache_result: None,
                            },
                        )
                    })
                    .collect();
            }
        }
    }
}

impl Clone for Subscriber {
    fn clone(&self) -> Self {
        Self {
            rx: self.rx.resubscribe(),
            state: self.state.clone(),
        }
    }
}

#[Subscription]
impl Subscriber {
    async fn events<'a>(&'a self) -> impl Stream<Item = SubscriptionMessage> + 'a {
        let mut rx = self.rx.resubscribe();
        let state = self.state.clone();

        stream! {
            // There's a race condition where the channel receiver can be out of sync with the state.
            {
                let message = SubscriptionMessage::InitialState(state.lock().await.borrow().clone());
                yield message;
            }

            while let Ok(event) = rx.recv().await {
                yield SubscriptionMessage::Event(event);
            }
        }
    }
}

struct Query {
    app_state: Subscriber,
}

#[Object]
impl Query {
    async fn tasks(&self) -> HashMap<String, TaskState> {
        self.app_state
            .state
            .lock()
            .await
            .borrow()
            .tasks
            .iter()
            .map(|(task, state)| (task.clone(), state.clone()))
            .collect()
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
    rx: tokio::sync::broadcast::Receiver<WebUIEvent>,
) -> Result<(), crate::Error> {
    let subscriber = Subscriber::new(rx);
    subscriber.watch();

    let cors = CorsLayer::new()
        // allow `GET` and `POST` when accessing the resource
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
        // allow requests from any origin
        .allow_origin(Any);

    let schema = Schema::new(
        Query {
            app_state: subscriber.clone(),
        },
        EmptyMutation,
        subscriber,
    );
    let app = Router::new()
        .route(
            "/",
            get(graphiql).post_service(GraphQL::new(schema.clone())),
        )
        .route_service("/subscriptions", GraphQLSubscription::new(schema))
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
