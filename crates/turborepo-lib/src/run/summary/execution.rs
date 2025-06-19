use std::fmt;

use chrono::{DateTime, Local};
use serde::Serialize;
use tokio::sync::mpsc;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_ui::{color, cprintln, ColorConfig, BOLD, BOLD_GREEN, BOLD_RED, MAGENTA, YELLOW};

use super::TurboDuration;
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
    // the (possibly empty) path from the turborepo root to where the command was run
    #[serde(rename = "repoPath")]
    repo_path: &'a AnchoredSystemPath,
    // number of tasks that exited successfully (does not include cache hits)
    success: usize,
    // number of tasks that exited with failure
    failed: usize,
    // number of tasks that had a cache hit
    cached: usize,
    // number of tasks that started
    attempted: usize,
    pub(crate) start_time: i64,
    pub(crate) end_time: i64,
    #[serde(skip)]
    duration: TurboDuration,
    pub(crate) exit_code: i32,
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
    pub fn print(
        &self,
        ui: ColorConfig,
        path: AbsoluteSystemPathBuf,
        failed_tasks: Vec<&TaskSummary>,
    ) {
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
            Event::Canceled => (),
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
struct TrackerMessage {
    event: Event,
    // Only present if task is finished
    state: Option<TaskState>,
}

#[derive(Debug, Clone, Copy, Serialize)]
enum Event {
    Building,
    BuildFailed,
    Cached,
    Built,
    // Canceled due to external signal or internal failure
    Canceled,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskExecutionSummary {
    pub start_time: i64,
    pub end_time: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub exit_code: Option<i32>,
}

impl TaskExecutionSummary {
    pub fn is_failure(&self) -> bool {
        // We consider None as a failure as it indicates the task failed to start
        // or was killed in a manner where we didn't collect an exit code.
        !matches!(self.exit_code, Some(0))
    }
}

impl ExecutionTracker {
    pub fn new() -> Self {
        // This buffer size is probably overkill, but since messages are only a byte
        // it's worth the extra memory to avoid the channel filling up.
        let (sender, mut receiver) = mpsc::channel::<Message>(128);
        let state_thread = tokio::spawn(async move {
            let mut state = SummaryState::default();
            while let Some(TrackerMessage {
                event,
                state: task_state,
            }) = receiver.recv().await
            {
                state.handle_event(event);
                if let Some(task_state) = task_state {
                    state.tasks.push(task_state);
                }
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
            .send(TrackerMessage {
                event: Event::Building,
                state: None,
            })
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
            .send(TrackerMessage {
                event: Event::Canceled,
                state: Some(TaskState {
                    task_id,
                    execution: None,
                }),
            })
            .await
            .expect("execution summary state thread finished")
    }
}

impl TaskTracker<chrono::DateTime<Local>> {
    // In the case of a task getting canceled we send no information as there was an
    // internal turbo error
    pub fn cancel(self) {}

    pub async fn cached(self) -> TaskExecutionSummary {
        let Self {
            sender,
            started_at,
            task_id,
        } = self;

        let ended_at = Local::now();
        let execution = TaskExecutionSummary {
            start_time: started_at.timestamp_millis(),
            end_time: ended_at.timestamp_millis(),
            // Go synthesizes a zero exit code on cache hits
            exit_code: Some(0),
            error: None,
        };

        let state = TaskState {
            task_id,
            execution: Some(execution.clone()),
        };
        sender
            .send(TrackerMessage {
                event: Event::Cached,
                state: Some(state),
            })
            .await
            .expect("summary state thread finished");
        execution
    }

    pub async fn build_succeeded(self, exit_code: i32) -> TaskExecutionSummary {
        let Self {
            sender,
            started_at,
            task_id,
        } = self;

        let ended_at = Local::now();
        let execution = TaskExecutionSummary {
            start_time: started_at.timestamp_millis(),
            end_time: ended_at.timestamp_millis(),
            exit_code: Some(exit_code),
            error: None,
        };

        let state = TaskState {
            task_id,
            execution: Some(execution.clone()),
        };
        sender
            .send(TrackerMessage {
                event: Event::Built,
                state: Some(state),
            })
            .await
            .expect("summary state thread finished");
        execution
    }

    pub async fn build_failed(
        self,
        exit_code: Option<i32>,
        error: impl fmt::Display,
    ) -> TaskExecutionSummary {
        let Self {
            sender,
            started_at,
            task_id,
        } = self;

        let ended_at = Local::now();
        let execution = TaskExecutionSummary {
            start_time: started_at.timestamp_millis(),
            end_time: ended_at.timestamp_millis(),
            exit_code,
            error: Some(error.to_string()),
        };

        let state = TaskState {
            task_id,
            execution: Some(execution.clone()),
        };
        sender
            .send(TrackerMessage {
                event: Event::BuildFailed,
                state: Some(state),
            })
            .await
            .expect("summary state thread finished");
        execution
    }
}

#[cfg(test)]
mod test {
    use chrono::Duration;
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
        assert_eq!(bar_state.execution.as_ref().unwrap().exit_code, Some(0));
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
            exit_code: Some(0),
            error: None
        },
        json!({ "startTime": 123, "endTime": 234, "exitCode": 0 })
        ; "success"
    )]
    #[test_case(
        TaskExecutionSummary {
            start_time: 123,
            end_time: 234,
            exit_code: Some(1),
            error: Some("cannot find anything".into()),
        },
        json!({ "startTime": 123, "endTime": 234, "exitCode": 1, "error": "cannot find anything" })
        ; "failure"
    )]
    fn test_serialization(value: impl serde::Serialize, expected: serde_json::Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
