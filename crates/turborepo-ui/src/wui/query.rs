use std::sync::Arc;

use async_graphql::{Object, SimpleObject};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::wui::subscriber::{TaskState, WebUIState};

#[derive(Debug, Clone, Serialize, SimpleObject)]
struct RunTask {
    name: String,
    state: TaskState,
}

struct CurrentRun<'a> {
    state: &'a SharedState,
}

#[Object]
impl<'a> CurrentRun<'a> {
    async fn tasks(&self) -> Vec<RunTask> {
        self.state
            .lock()
            .await
            .tasks()
            .iter()
            .map(|(task, state)| RunTask {
                name: task.clone(),
                state: state.clone(),
            })
            .collect()
    }
}

/// We keep the state in a `Arc<Mutex<RefCell<T>>>` so both `Subscriber` and
/// `Query` can access it, with `Subscriber` mutating it and `Query` only
/// reading it.
pub type SharedState = Arc<Mutex<WebUIState>>;

/// The query for actively running tasks.
///
/// (As opposed to the query for general repository state `RepositoryQuery`
/// in `turborepo_lib::query`)
/// This is `None` when we're not actually running a task (e.g. `turbo query`)
pub struct RunQuery {
    state: Option<SharedState>,
}

impl RunQuery {
    pub fn new(state: Option<SharedState>) -> Self {
        Self { state }
    }
}

#[Object]
impl RunQuery {
    async fn current_run(&self) -> Option<CurrentRun> {
        Some(CurrentRun {
            state: self.state.as_ref()?,
        })
    }
}
