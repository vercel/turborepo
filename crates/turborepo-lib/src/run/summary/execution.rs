use std::fmt;

use chrono::{DateTime, Duration, Local};
use serde::{ser::SerializeStruct, Serialize};
use tokio::sync::mpsc;
use turbopath::AnchoredSystemPath;

use crate::run::task_id::TaskId;

// Just used to make changing the type that gets passed to the state management
// thread easy
type Message = Event;

// Should *not* be exposed outside of run summary module
/// The execution summary
#[derive(Debug, Serialize)]
pub struct ExecutionSummary<'a> {
    // this thread handles the state management
    #[serde(skip)]
    state_thread: tokio::task::JoinHandle<SummaryState>,
    #[serde(skip)]
    sender: mpsc::Sender<Message>,
    command: String,
    package_inference_path: Option<&'a AnchoredSystemPath>,
    started_at: DateTime<Local>,
}

/// The final states of all task executions
#[derive(Debug, Default, Clone, Copy)]
pub struct SummaryState {
    pub attempted: usize,
    pub failed: usize,
    pub cached: usize,
    pub success: usize,
}

impl SummaryState {
    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Building => self.attempted += 1,
            Event::BuildFailed => self.failed += 1,
            Event::Cached => self.cached += 1,
            Event::Built => self.success += 1,
        }
    }
}

/// A tracker constructed for each task and used to communicate task events back
/// to the execution summary.
pub struct Tracker<T> {
    sender: mpsc::Sender<Message>,
    started_at: T,
    // task_id is only used as a name for the event in the chrometracing profile
    #[allow(dead_code)]
    task_id: TaskId<'static>,
}

#[derive(Debug, Clone, Copy, Serialize)]
enum Event {
    Building,
    BuildFailed,
    Cached,
    Built,
}

#[derive(Debug, Serialize)]
enum ExecutionState {
    Canceled,
    Built { exit_code: u32 },
    Cached,
    BuildFailed { exit_code: u32, err: String },
}

#[derive(Debug)]
pub struct TaskExecutionSummary {
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    state: ExecutionState,
}

impl Serialize for TaskExecutionSummary {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("TaskExecutionSummary", 4)?;
        state.serialize_field("startedAt", &self.started_at.timestamp_millis())?;
        state.serialize_field("endedAt", &self.ended_at.timestamp_millis())?;
        state.serialize_field("state", &self.state)?;

        state.end()
    }
}

impl TaskExecutionSummary {
    pub fn exit_code(&self) -> Option<u32> {
        match self.state {
            ExecutionState::BuildFailed { exit_code, .. } | ExecutionState::Built { exit_code } => {
                Some(exit_code)
            }
            _ => None,
        }
    }

    pub fn duration(&self) -> Duration {
        self.ended_at.signed_duration_since(self.started_at)
    }
}

impl<'a> ExecutionSummary<'a> {
    pub fn new(
        command: String,
        package_inference_path: Option<&'a AnchoredSystemPath>,
        started_at: DateTime<Local>,
    ) -> Self {
        // This buffer size is probably overkill, but since messages are only a byte
        // it's worth the extra memory to avoid the channel filling up.
        let (sender, mut receiver) = mpsc::channel(128);
        let state_thread = tokio::spawn(async move {
            let mut state = SummaryState::default();
            while let Some(event) = receiver.recv().await {
                state.handle_event(event);
            }
            state
        });

        Self {
            state_thread,
            sender,
            command,
            package_inference_path,
            started_at,
        }
    }

    // Produce a tracker for the task
    pub fn tracker(&self, task_id: TaskId<'static>) -> Tracker<()> {
        Tracker {
            sender: self.sender.clone(),
            task_id,
            started_at: (),
        }
    }

    pub async fn finish(self) -> Result<SummaryState, tokio::task::JoinError> {
        let Self {
            state_thread,
            sender,
            ..
        } = self;
        // We drop the sender so the channel closes once all trackers have finished.
        // We don't explicitly close as that would cause running trackers to be unable
        // to send their execution summary.
        drop(sender);

        state_thread.await
    }
}

impl Tracker<()> {
    // Start the tracker
    pub async fn start(self) -> Tracker<DateTime<Local>> {
        let Tracker {
            sender, task_id, ..
        } = self;
        let started_at = Local::now();
        sender
            .send(Event::Building)
            .await
            .expect("execution summary state thread finished");
        Tracker {
            sender,
            started_at,
            task_id,
        }
    }
}

impl Tracker<chrono::DateTime<Local>> {
    pub fn cancel(self) -> TaskExecutionSummary {
        let Self { started_at, .. } = self;
        TaskExecutionSummary {
            started_at,
            ended_at: Local::now(),
            state: ExecutionState::Canceled,
        }
    }

    pub async fn cached(self) -> TaskExecutionSummary {
        let Self {
            sender, started_at, ..
        } = self;
        sender
            .send(Event::Cached)
            .await
            .expect("summary state thread finished");
        TaskExecutionSummary {
            started_at,
            ended_at: Local::now(),
            state: ExecutionState::Cached,
        }
    }

    pub async fn build_succeeded(self, exit_code: u32) -> TaskExecutionSummary {
        let Self {
            sender, started_at, ..
        } = self;
        sender
            .send(Event::Built)
            .await
            .expect("summary state thread finished");
        TaskExecutionSummary {
            started_at,
            ended_at: Local::now(),
            state: ExecutionState::Built { exit_code },
        }
    }

    pub async fn build_failed(
        self,
        exit_code: u32,
        error: impl fmt::Display,
    ) -> TaskExecutionSummary {
        let Self {
            sender, started_at, ..
        } = self;
        sender
            .send(Event::BuildFailed)
            .await
            .expect("summary state thread finished");
        TaskExecutionSummary {
            started_at,
            ended_at: Local::now(),
            state: ExecutionState::BuildFailed {
                exit_code,
                err: error.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_multiple_tasks() {
        let summary = ExecutionSummary::new();
        let mut tasks = Vec::new();
        {
            let tracker = summary.tracker(TaskId::new("foo", "build"));
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                let summary = tracker.build_succeeded(0).await;
                assert_eq!(summary.exit_code(), Some(0));
            }));
        }
        {
            let tracker = summary.tracker(TaskId::new("bar", "build"));
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                let summary = tracker.cached().await;
                assert_eq!(summary.exit_code(), None);
            }));
        }
        {
            let tracker = summary.tracker(TaskId::new("baz", "build"));
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                let summary = tracker.build_failed(1, "big bad error").await;
                assert_eq!(summary.exit_code(), Some(1));
            }));
        }
        {
            let tracker = summary.tracker(TaskId::new("boo", "build"));
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                let summary = tracker.cancel();
                assert_eq!(summary.exit_code(), None);
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let state = summary.finish().await.unwrap();
        assert_eq!(state.attempted, 4);
        assert_eq!(state.cached, 1);
        assert_eq!(state.failed, 1);
        assert_eq!(state.success, 1);
    }

    #[tokio::test]
    async fn test_timing() {
        let summary = ExecutionSummary::new();
        let tracker = summary.tracker(TaskId::new("foo", "build"));
        let post_construction_time = Local::now();
        let tracker = tracker.start().await;
        let sleep_duration = Duration::from_millis(5);
        tokio::time::sleep(sleep_duration).await;
        let summary = tracker.build_succeeded(0).await;
        assert!(
            post_construction_time < summary.started_at,
            "tracker start time should start when start is called"
        );
        assert!(
            sleep_duration <= summary.duration(),
            "tracker duration should be at least as long as the time between calls"
        );
    }
}
