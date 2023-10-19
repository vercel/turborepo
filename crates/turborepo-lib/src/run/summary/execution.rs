use std::{fmt, fmt::Formatter};

use chrono::{DateTime, Duration, Local, SubsecRound};
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::log::warn;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_ui::{color, BOLD, BOLD_GREEN, BOLD_RED, MAGENTA, UI};

use crate::run::{summary::task::TaskSummary, task_id::TaskId};

// Just used to make changing the type that gets passed to the state management
// thread easy
type Message = Event;

// Should *not* be exposed outside of run summary module
/// Spawns task trackers and records the final state of all tasks
#[derive(Debug)]
pub struct ExecutionTracker {
    // this thread handles the state management
    state_thread: tokio::task::JoinHandle<SummaryState>,
    sender: mpsc::Sender<Message>,
    pub command: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSummary<'a> {
    // a synthesized turbo command to produce this invocation
    command: String,
    // number of tasks that exited successfully (does not include cache hits)
    success: usize,
    // number of tasks that exited with failure
    failed: usize,
    // number of tasks that had a cache hit
    cached: usize,
    // number of tasks that started
    attempted: usize,
    // the (possibly empty) path from the turborepo root to where the command was run
    #[serde(rename = "repoPath", skip_serializing_if = "Option::is_none")]
    package_inference_root: Option<&'a AnchoredSystemPath>,
    pub(crate) start_time: i64,
    pub(crate) end_time: i64,
    #[serde(skip)]
    duration: TurboDuration,
    pub(crate) exit_code: i32,
}

#[derive(Debug)]
struct TurboDuration(Duration);

impl TurboDuration {
    pub fn new(start_time: &DateTime<Local>, end_time: &DateTime<Local>) -> Self {
        TurboDuration(
            end_time
                .trunc_subsecs(3)
                .signed_duration_since(start_time.trunc_subsecs(3)),
        )
    }
}

impl From<Duration> for TurboDuration {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl fmt::Display for TurboDuration {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let duration = &self.0;

        if duration.num_hours() > 0 {
            write!(
                f,
                "{}h{}m{}s",
                duration.num_hours(),
                duration.num_minutes(),
                duration.num_seconds()
            )
        } else if duration.num_minutes() > 0 {
            write!(f, "{}m{}s", duration.num_minutes(), duration.num_seconds())
        } else if duration.num_seconds() > 0 {
            write!(f, "{}s", duration.num_seconds())
        } else {
            write!(f, "{}ms", duration.num_milliseconds())
        }
    }
}

impl ExecutionSummary<'_> {
    /// We implement this on `ExecutionSummary` and not `RunSummary` because
    /// the `execution` field is nullable (due to normalize).
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
                    color!(ui, BOLD_GREEN, "{} successful", self.successful()),
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
                    color!(ui, BOLD, "{}", self.duration),
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

    fn successful(&self) -> usize {
        self.success + self.attempted
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
pub struct TaskTracker<T> {
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
    Built { exit_code: i32 },
    Cached,
    BuildFailed { exit_code: i32, err: String },
    SpawnFailed { err: String },
}

#[derive(Debug, Serialize)]
pub struct TaskExecutionSummary {
    started_at: i64,
    ended_at: i64,
    #[serde(skip)]
    duration: TurboDuration,
    pub(crate) state: ExecutionState,
}

impl TaskExecutionSummary {
    pub fn exit_code(&self) -> Option<i32> {
        match self.state {
            ExecutionState::BuildFailed { exit_code, .. } | ExecutionState::Built { exit_code } => {
                Some(exit_code)
            }
            _ => None,
        }
    }
}

impl ExecutionTracker {
    pub fn new(command: &str) -> Self {
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
            command: command.to_string(),
            state_thread,
            sender,
        }
    }

    // Produce a tracker for the task
    pub fn task_tracker(&self, task_id: TaskId<'static>) -> TaskTracker<()> {
        TaskTracker {
            sender: self.sender.clone(),
            task_id,
            started_at: (),
        }
    }

    pub async fn finish(
        self,
        package_inference_root: Option<&AnchoredSystemPath>,
        exit_code: i32,
        start_time: DateTime<Local>,
        end_time: DateTime<Local>,
    ) -> Result<ExecutionSummary<'_>, tokio::task::JoinError> {
        let Self {
            state_thread,
            sender,
            ..
        } = self;
        // We drop the sender so the channel closes once all trackers have finished.
        // We don't explicitly close as that would cause running trackers to be unable
        // to send their execution summary.
        drop(sender);

        let summary_state = state_thread.await?;

        let duration = TurboDuration::new(&start_time, &end_time);

        Ok(ExecutionSummary {
            command: self.command,
            success: summary_state.success,
            failed: summary_state.failed,
            cached: summary_state.cached,
            attempted: summary_state.attempted,
            package_inference_root,
            start_time: start_time.timestamp_millis(),
            end_time: end_time.timestamp_millis(),
            duration,
            exit_code,
        })
    }
}

impl TaskTracker<()> {
    // Start the tracker
    pub async fn start(self) -> TaskTracker<DateTime<Local>> {
        let TaskTracker {
            sender, task_id, ..
        } = self;
        let started_at = Local::now();
        sender
            .send(Event::Building)
            .await
            .expect("execution summary state thread finished");
        TaskTracker {
            sender,
            started_at,
            task_id,
        }
    }
}

impl TaskTracker<chrono::DateTime<Local>> {
    pub fn cancel(self) -> TaskExecutionSummary {
        let Self { started_at, .. } = self;
        let ended_at = Local::now();
        let duration = TurboDuration::new(&started_at, &Local::now());
        TaskExecutionSummary {
            started_at: started_at.timestamp_millis(),
            ended_at: ended_at.timestamp_millis(),
            duration,
            state: ExecutionState::Canceled,
        }
    }

    pub async fn cached(self) -> TaskExecutionSummary {
        let Self {
            sender, started_at, ..
        } = self;

        let ended_at = Local::now();
        let duration = TurboDuration::new(&started_at, &Local::now());

        sender
            .send(Event::Cached)
            .await
            .expect("summary state thread finished");

        TaskExecutionSummary {
            started_at: started_at.timestamp_millis(),
            ended_at: ended_at.timestamp_millis(),
            duration,
            state: ExecutionState::Cached,
        }
    }

    pub async fn build_succeeded(self, exit_code: i32) -> TaskExecutionSummary {
        let Self {
            sender, started_at, ..
        } = self;

        let ended_at = Local::now();
        let duration = TurboDuration::new(&started_at, &Local::now());

        sender
            .send(Event::Built)
            .await
            .expect("summary state thread finished");

        TaskExecutionSummary {
            started_at: started_at.timestamp_millis(),
            ended_at: ended_at.timestamp_millis(),
            duration,
            state: ExecutionState::Built { exit_code },
        }
    }

    pub async fn build_failed(
        self,
        exit_code: i32,
        error: impl fmt::Display,
    ) -> TaskExecutionSummary {
        let Self {
            sender, started_at, ..
        } = self;

        let ended_at = Local::now();
        let duration = TurboDuration::new(&started_at, &Local::now());

        sender
            .send(Event::BuildFailed)
            .await
            .expect("summary state thread finished");
        TaskExecutionSummary {
            started_at: started_at.timestamp_millis(),
            ended_at: ended_at.timestamp_millis(),
            duration,
            state: ExecutionState::BuildFailed {
                exit_code,
                err: error.to_string(),
            },
        }
    }

    pub async fn spawn_failed(self, error: impl fmt::Display) -> TaskExecutionSummary {
        let Self {
            sender, started_at, ..
        } = self;

        let ended_at = Local::now();
        let duration = TurboDuration::new(&started_at, &Local::now());

        sender
            .send(Event::BuildFailed)
            .await
            .expect("summary state thread finished");
        TaskExecutionSummary {
            started_at: started_at.timestamp_millis(),
            ended_at: ended_at.timestamp_millis(),
            duration,
            state: ExecutionState::SpawnFailed {
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
        let started_at = Local::now();
        let summary = ExecutionTracker::new("turbo build");
        let mut tasks = Vec::new();
        {
            let tracker = summary.task_tracker(TaskId::new("foo", "build"));
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                let summary = tracker.build_succeeded(0).await;
                assert_eq!(summary.exit_code(), Some(0));
            }));
        }
        {
            let tracker = summary.task_tracker(TaskId::new("bar", "build"));
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                let summary = tracker.cached().await;
                assert_eq!(summary.exit_code(), None);
            }));
        }
        {
            let tracker = summary.task_tracker(TaskId::new("baz", "build"));
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                let summary = tracker.build_failed(1, "big bad error").await;
                assert_eq!(summary.exit_code(), Some(1));
            }));
        }
        {
            let tracker = summary.task_tracker(TaskId::new("boo", "build"));
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                let summary = tracker.cancel();
                assert_eq!(summary.exit_code(), None);
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        let ended_at = Local::now();
        let state = summary.finish(None, 0, started_at, ended_at).await.unwrap();
        assert_eq!(state.attempted, 4);
        assert_eq!(state.cached, 1);
        assert_eq!(state.failed, 1);
        assert_eq!(state.success, 1);
    }

    #[tokio::test]
    async fn test_timing() {
        let summary = ExecutionTracker::new("turbo build");
        let tracker = summary.task_tracker(TaskId::new("foo", "build"));
        let post_construction_time = Local::now().timestamp_millis();
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
            sleep_duration <= summary.duration.0,
            "tracker duration should be at least as long as the time between calls"
        );
    }
}
