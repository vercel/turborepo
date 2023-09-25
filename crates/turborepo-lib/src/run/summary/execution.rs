use std::fmt;

use chrono::{DateTime, Duration, Local, SubsecRound};
use serde::{ser::SerializeStruct, Serialize, Serializer};
use tokio::sync::mpsc;
use tracing::log::warn;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_ui::{color, BOLD, BOLD_GREEN, BOLD_RED, MAGENTA, UI};

use crate::run::{summary::task::TaskSummary, task_id::TaskId};

// Just used to make changing the type that gets passed to the state management
// thread easy
type Message = Event;

fn serialize_datetime<S: Serializer>(
    date_time: &DateTime<Local>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_i64(date_time.timestamp_millis())
}

fn serialize_optional_datetime<S: Serializer>(
    date_time: &Option<DateTime<Local>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let millis = date_time
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_default();
    serializer.serialize_i64(millis)
}

// Should *not* be exposed outside of run summary module
/// The execution summary
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSummary<'a> {
    // this thread handles the state management
    #[serde(skip)]
    state_thread: tokio::task::JoinHandle<SummaryState>,
    #[serde(skip)]
    sender: mpsc::Sender<Message>,
    command: String,
    success: usize,
    failed: usize,
    cached: usize,
    attempted: usize,
    #[serde(rename = "repoPath", skip_serializing_if = "Option::is_none")]
    package_inference_root: Option<&'a AnchoredSystemPath>,
    #[serde(serialize_with = "serialize_datetime")]
    pub(crate) start_time: DateTime<Local>,
    #[serde(serialize_with = "serialize_optional_datetime")]
    pub(crate) end_time: Option<DateTime<Local>>,
    pub(crate) exit_code: Option<u32>,
}

impl ExecutionSummary<'_> {
    fn duration(&self) -> String {
        let duration = self
            .end_time
            .unwrap_or_else(Local::now)
            .trunc_subsecs(3)
            .signed_duration_since(self.start_time.trunc_subsecs(3));

        if duration.num_hours() > 0 {
            format!(
                "{}h{}m{}s",
                duration.num_hours(),
                duration.num_minutes(),
                duration.num_seconds()
            )
        } else if duration.num_minutes() > 0 {
            format!("{}m{}s", duration.num_minutes(), duration.num_seconds())
        } else if duration.num_seconds() > 0 {
            format!("{}s", duration.num_seconds())
        } else {
            format!("{}ms", duration.num_milliseconds())
        }
    }

    /// We implement this on `ExecutionSummary` and not `RunSummary` because
    /// the `execution_summary` field is nullable (due to normalize).
    pub fn print(&self, ui: UI, path: AbsoluteSystemPathBuf, failed_tasks: Vec<&TaskSummary>) {
        let maybe_full_turbo = if self.cached == self.attempted && self.attempted > 0 {
            match std::env::var("TERM_PROGRAM").as_deref() {
                Ok("Apple_Terminal") => color!(ui, MAGENTA, ">>> FULL TURBO").to_string(),
                _ => ui.rainbow(">>> FULL TURBO").to_string(),
            }
        } else {
            String::new()
        };

        let mut line_data = vec![
            (
                "Tasks",
                format!(
                    "{}, {} total",
                    color!(ui, BOLD_GREEN, "{} successful", self.success),
                    self.attempted
                ),
            ),
            (
                "Cached",
                format!(
                    "{}, {} total",
                    color!(ui, BOLD, "{} cached", self.cached),
                    self.attempted
                )
                .to_string(),
            ),
            (
                "Time",
                format!(
                    "{} {}",
                    color!(ui, BOLD, "{}", self.duration()),
                    maybe_full_turbo
                ),
            ),
        ];

        if path.exists() {
            line_data.push(("Summary", path.to_string()));
        }

        if !failed_tasks.is_empty() {
            let mut formatted: Vec<_> = failed_tasks
                .iter()
                .map(|task| color!(ui, BOLD_RED, "{}", task.task_id).to_string())
                .collect();
            formatted.sort();
            line_data.push(("Failed", formatted.join(", ")));
        }

        let max_length = line_data
            .iter()
            .map(|(header, _)| header.len())
            .max()
            .unwrap_or_default();

        let lines: Vec<_> = line_data
            .into_iter()
            .map(|(header, trailer)| {
                color!(
                    ui,
                    BOLD,
                    "{}{}:    {}",
                    " ".repeat(max_length - header.len()),
                    header,
                    trailer
                )
            })
            .collect();

        if self.attempted == 0 {
            println!();
            warn!("No tasks were executed as a part of this run.");
        }

        println!();
        for line in lines {
            println!("{}", line);
        }

        println!();
    }
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
pub enum ExecutionState {
    Canceled,
    Built { exit_code: u32 },
    Cached,
    BuildFailed { exit_code: u32, err: String },
}

#[derive(Debug)]
pub struct TaskExecutionSummary {
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    pub(crate) state: ExecutionState,
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
        package_inference_root: Option<&'a AnchoredSystemPath>,
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

            package_inference_root,
            start_time: started_at,
            success: 0,
            failed: 0,
            cached: 0,
            attempted: 0,
            end_time: None,
            exit_code: None,
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
        let summary = ExecutionSummary::new("turbo run build".to_string(), None, Local::now());
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
        let summary = ExecutionSummary::new("turbo run build".to_string(), None, Local::now());
        let tracker = summary.tracker(TaskId::new("foo", "build"));
        let post_construction_time = Local::now();
        let sleep_duration = Duration::milliseconds(5);
        tokio::time::sleep(sleep_duration.to_std().unwrap()).await;

        let tracker = tracker.start().await;

        tokio::time::sleep(sleep_duration.to_std().unwrap()).await;
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
