use std::{fmt, fmt::Formatter};

use chrono::{DateTime, Duration, Local, SubsecRound};
use serde::Serialize;
use tokio::sync::mpsc;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_ui::{color, cprintln, BOLD, BOLD_GREEN, BOLD_RED, MAGENTA, UI, YELLOW};

use crate::run::{summary::task::TaskSummary, task_id::TaskId};

// Just used to make changing the type that gets passed to the state management
// thread easy
type Message = TrackerMessage;

// Should *not* be exposed outside of run summary module
/// Spawns task trackers and records the final state of all tasks
#[derive(Debug)]
pub struct ExecutionTracker {
    // this thread handles the state management
    state_thread: tokio::task::JoinHandle<SummaryState>,
    sender: mpsc::Sender<Message>,
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
    #[serde(rename = "repoPath")]
    repo_path: &'a AnchoredSystemPath,
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

impl<'a> ExecutionSummary<'a> {
    pub fn new(
        command: String,
        state: SummaryState,
        package_inference_root: Option<&'a AnchoredSystemPath>,
        exit_code: i32,
        start_time: DateTime<Local>,
        end_time: DateTime<Local>,
    ) -> Self {
        let duration = TurboDuration::new(&start_time, &end_time);
        Self {
            command,
            success: state.success,
            failed: state.failed,
            cached: state.cached,
            attempted: state.attempted,
            // We're either at some path in the repo, or at the root, which is an empty path
            repo_path: package_inference_root.unwrap_or_else(|| AnchoredSystemPath::empty()),
            start_time: start_time.timestamp_millis(),
            end_time: end_time.timestamp_millis(),
            duration,
            exit_code,
        }
    }

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
            cprintln!(ui, YELLOW, "No tasks were executed as part of this run.");
        }

        println!();
        for line in lines {
            println!("{}", line);
        }

        println!();
    }

    fn successful(&self) -> usize {
        self.success + self.cached
    }
}

/// The final states of all task executions
#[derive(Debug, Default, Clone)]
pub struct SummaryState {
    pub attempted: usize,
    pub failed: usize,
    pub cached: usize,
    pub success: usize,
    pub tasks: Vec<TaskState>,
}

#[derive(Debug, Clone)]
pub struct TaskState {
    pub task_id: TaskId<'static>,
    pub execution: Option<TaskExecutionSummary>,
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
    task_id: TaskId<'static>,
}

#[derive(Debug, Clone)]
enum TrackerMessage {
    Starting,
    Finished(TaskState),
}

#[derive(Debug, Clone, Copy, Serialize)]
enum Event {
    Building,
    BuildFailed,
    Cached,
    Built,
}

#[derive(Debug, Serialize, Clone)]
pub enum ExecutionState {
    Canceled,
    Built { exit_code: i32 },
    Cached,
    BuildFailed { exit_code: Option<i32>, err: String },
}

impl ExecutionState {
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            ExecutionState::Built { exit_code } => Some(*exit_code),
            ExecutionState::BuildFailed { exit_code, .. } => *exit_code,
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskExecutionSummary {
    start_time: i64,
    end_time: i64,
    #[serde(skip)]
    state: ExecutionState,
    exit_code: Option<i32>,
}

impl TaskExecutionSummary {
    pub fn is_failure(&self) -> bool {
        matches!(self.state, ExecutionState::BuildFailed { .. })
    }
}

impl ExecutionTracker {
    pub fn new() -> Self {
        // This buffer size is probably overkill, but since messages are only a byte
        // it's worth the extra memory to avoid the channel filling up.
        let (sender, mut receiver) = mpsc::channel::<Message>(128);
        let state_thread = tokio::spawn(async move {
            let mut state = SummaryState::default();
            while let Some(message) = receiver.recv().await {
                if let Some(event) = message.event() {
                    state.handle_event(event);
                }
                if let TrackerMessage::Finished(task_state) = message {
                    state.tasks.push(task_state);
                };
            }
            state
        });

        Self {
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

        let summary_state = state_thread.await?;

        Ok(summary_state)
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
            .send(TrackerMessage::Starting)
            .await
            .expect("execution summary state thread finished");
        TaskTracker {
            sender,
            started_at,
            task_id,
        }
    }

    // Track that the task would be executed
    pub async fn dry_run(self) {
        let Self {
            sender, task_id, ..
        } = self;

        sender
            .send(TrackerMessage::Finished(TaskState {
                task_id,
                execution: None,
            }))
            .await
            .expect("execution summary state thread finished")
    }
}

impl TaskTracker<chrono::DateTime<Local>> {
    // In the case of a task getting canceled we send no information as there was an
    // internal turbo error
    pub fn cancel(self) {}

    pub async fn cached(self) {
        let Self {
            sender,
            started_at,
            task_id,
        } = self;

        let ended_at = Local::now();
        let execution = Some(TaskExecutionSummary {
            start_time: started_at.timestamp_millis(),
            end_time: ended_at.timestamp_millis(),
            state: ExecutionState::Cached,
            exit_code: None,
        });

        sender
            .send(TrackerMessage::Finished(TaskState { task_id, execution }))
            .await
            .expect("summary state thread finished");
    }

    pub async fn build_succeeded(self, exit_code: i32) {
        let Self {
            sender,
            started_at,
            task_id,
        } = self;

        let ended_at = Local::now();
        let execution = Some(TaskExecutionSummary {
            start_time: started_at.timestamp_millis(),
            end_time: ended_at.timestamp_millis(),
            state: ExecutionState::Built { exit_code },
            exit_code: Some(exit_code),
        });

        sender
            .send(TrackerMessage::Finished(TaskState { task_id, execution }))
            .await
            .expect("summary state thread finished");
    }

    pub async fn build_failed(self, exit_code: Option<i32>, error: impl fmt::Display) {
        let Self {
            sender,
            started_at,
            task_id,
        } = self;

        let ended_at = Local::now();
        let execution = Some(TaskExecutionSummary {
            start_time: started_at.timestamp_millis(),
            end_time: ended_at.timestamp_millis(),
            state: ExecutionState::BuildFailed {
                exit_code,
                err: error.to_string(),
            },
            exit_code,
        });

        sender
            .send(TrackerMessage::Finished(TaskState { task_id, execution }))
            .await
            .expect("summary state thread finished");
    }
}

impl TrackerMessage {
    fn event(&self) -> Option<Event> {
        match &self {
            TrackerMessage::Starting => Some(Event::Building),
            TrackerMessage::Finished(TaskState {
                execution: Some(TaskExecutionSummary { state, .. }),
                ..
            }) => match state {
                ExecutionState::Built { .. } => Some(Event::Built),
                ExecutionState::Cached => Some(Event::Cached),
                ExecutionState::BuildFailed { .. } => Some(Event::BuildFailed),
                ExecutionState::Canceled => None,
            },
            TrackerMessage::Finished(TaskState {
                execution: None, ..
            }) => None,
        }
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use test_case::test_case;

    use super::*;

    #[tokio::test]
    async fn test_multiple_tasks() {
        let summary = ExecutionTracker::new();
        let foo = TaskId::new("foo", "build");
        let bar = TaskId::new("bar", "build");
        let baz = TaskId::new("baz", "build");
        let boo = TaskId::new("boo", "build");
        let mut tasks = Vec::new();
        {
            let tracker = summary.task_tracker(foo.clone());
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                tracker.build_succeeded(0).await;
            }));
        }
        {
            let tracker = summary.task_tracker(bar.clone());
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                tracker.cached().await;
            }));
        }
        {
            let tracker = summary.task_tracker(baz.clone());
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                tracker.build_failed(Some(1), "big bad error").await;
            }));
        }
        {
            let tracker = summary.task_tracker(boo.clone());
            tasks.push(tokio::spawn(async move {
                let tracker = tracker.start().await;
                tracker.cancel();
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
        let foo_state = state.tasks.iter().find(|task| task.task_id == foo).unwrap();
        assert_eq!(foo_state.execution.as_ref().unwrap().exit_code, Some(0));
        let bar_state = state.tasks.iter().find(|task| task.task_id == bar).unwrap();
        assert_eq!(bar_state.execution.as_ref().unwrap().exit_code, None);
        let baz_state = state.tasks.iter().find(|task| task.task_id == baz).unwrap();
        assert_eq!(baz_state.execution.as_ref().unwrap().exit_code, Some(1));
        let boo_state = state.tasks.iter().find(|task| task.task_id == boo);
        assert!(
            boo_state.is_none(),
            "canceling doesn't produce execution data"
        );
    }

    #[tokio::test]
    async fn test_timing() {
        let summary = ExecutionTracker::new();
        let tracker = summary.task_tracker(TaskId::new("foo", "build"));
        let post_construction_time = Local::now().timestamp_millis();
        let sleep_duration = Duration::milliseconds(5);
        tokio::time::sleep(sleep_duration.to_std().unwrap()).await;

        let tracker = tracker.start().await;

        tokio::time::sleep(sleep_duration.to_std().unwrap()).await;
        tracker.build_succeeded(0).await;
        let mut state = summary.finish().await.unwrap();
        assert_eq!(state.tasks.len(), 1);
        let summary = state.tasks.pop().unwrap().execution.unwrap();
        assert!(
            post_construction_time < summary.start_time,
            "tracker start time should start when start is called"
        );
        assert!(
            summary.start_time + sleep_duration.num_milliseconds() <= summary.end_time,
            "tracker end should be at least as long as the time between calls"
        );
    }

    #[test_case(
        TaskExecutionSummary {
            start_time: 123,
            end_time: 234,
            state: ExecutionState::Built { exit_code: 0 },
            exit_code: Some(0),
        },
        json!({ "startTime": 123, "endTime": 234, "exitCode": 0 })
        ; "success"
    )]
    fn test_serialization(value: impl serde::Serialize, expected: serde_json::Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
