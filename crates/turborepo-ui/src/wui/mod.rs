//! Web UI for Turborepo. Creates a WebSocket server that can be subscribed to
//! by a web client to display the status of tasks.

use std::{cell::RefCell, collections::BTreeMap, io::Write, sync::Arc};

use async_graphql::{
    http::GraphiQLSource, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject,
};
use async_graphql_axum::GraphQL;
use axum::{http::Method, response, response::IntoResponse, routing::get, Router};
use serde::Serialize;
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
            .send(WebUIEvent::StartTask { task, output_logs })
            .ok();
    }

    pub fn restart_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::RestartTasks { tasks })
            .map_err(Error::Broadcast)?;
        Ok(())
    }

    pub fn end_task(&self, task: String, result: TaskResult) {
        self.tx.send(WebUIEvent::EndTask { task, result }).ok();
    }

    pub fn status(&self, task: String, status: String, result: CacheResult) {
        self.tx
            .send(WebUIEvent::Status {
                task,
                status,
                result,
            })
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
        self.tx.send(WebUIEvent::Stop).ok();
    }

    pub fn update_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::UpdateTasks { tasks })
            .map_err(Error::Broadcast)?;

        Ok(())
    }

    pub fn output(&self, task: String, output: Vec<u8>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::TaskOutput { task, output })
            .map_err(Error::Broadcast)?;

        Ok(())
    }
}

// Specific events that the GraphQL server can send to the client,
// not all the `Event` types from the TUI.
//
// We have to put each variant in a new struct because async graphql doesn't
// allow enums with fields for union types
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum WebUIEvent {
    StartTask {
        task: String,
        output_logs: OutputLogs,
    },
    TaskOutput {
        task: String,
        output: Vec<u8>,
    },
    EndTask {
        task: String,
        result: TaskResult,
    },
    Status {
        task: String,
        status: String,
        result: CacheResult,
    },
    UpdateTasks {
        tasks: Vec<String>,
    },
    RestartTasks {
        tasks: Vec<String>,
    },
    Stop,
}

#[derive(Debug, Clone, Serialize, SimpleObject)]
struct TaskState {
    output: Vec<u8>,
    status: Option<String>,
    result: Option<TaskResult>,
    cache_result: Option<CacheResult>,
}

#[derive(Debug, Default, Clone, Serialize)]
struct WebUIState {
    tasks: BTreeMap<String, TaskState>,
}

/// Subscribes to the Web UI events and updates the state
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
            WebUIEvent::StartTask {
                task,
                output_logs: _,
            } => {
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
            WebUIEvent::TaskOutput { task, output } => {
                state
                    .borrow_mut()
                    .tasks
                    .get_mut(&task)
                    .unwrap()
                    .output
                    .extend(output);
            }
            WebUIEvent::EndTask { task, result } => {
                state.borrow_mut().tasks.get_mut(&task).unwrap().result = Some(result);
            }
            WebUIEvent::Status { task, result, .. } => {
                state
                    .borrow_mut()
                    .tasks
                    .get_mut(&task)
                    .unwrap()
                    .cache_result = Some(result);
            }
            WebUIEvent::Stop => {
                // TODO: stop watching
            }
            WebUIEvent::UpdateTasks { tasks } => {
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
            WebUIEvent::RestartTasks { tasks } => {
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

struct Query {
    subscriber: Subscriber,
}

#[derive(Debug, Clone, Serialize, SimpleObject)]
struct Task {
    name: String,
    state: TaskState,
}

#[Object]
impl Query {
    async fn tasks(&self) -> Vec<Task> {
        self.subscriber
            .state
            .lock()
            .await
            .borrow()
            .tasks
            .iter()
            .map(|(task, state)| Task {
                name: task.clone(),
                state: state.clone(),
            })
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

    let schema = Schema::new(Query { subscriber }, EmptyMutation, EmptySubscription);
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
