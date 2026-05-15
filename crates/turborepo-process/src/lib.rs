//! `process`
//!
//! This module contains the code that is responsible for running the commands
//! that are queued by run. It consists of a set of child processes that are
//! spawned and managed by the manager. The manager is responsible for
//! running these processes to completion, forwarding signals, and closing
//! them when the manager is closed.
//!
//! As of now, the manager will execute futures in a random order, and
//! must be either `wait`ed on or `stop`ped to drive state.

#![deny(clippy::all)]

mod child;
mod command;
#[cfg(windows)]
mod job_object;

use std::{
    collections::HashMap,
    io::{self, IsTerminal},
    sync::{Arc, Mutex},
    time::Duration,
};

pub use command::Command;
use tokio::task::JoinSet;
use tracing::{debug, trace};
use turborepo_task_id::TaskId;

pub use self::child::{Child, ChildExit};

/// A process manager that is responsible for spawning and managing child
/// processes. When the manager is Open, new child processes can be spawned
/// using `spawn`. When the manager is Closed, all currently-running children
/// will be closed, and no new children can be spawned.
#[derive(Debug, Clone)]
pub struct ProcessManager {
    state: Arc<Mutex<ProcessManagerInner>>,
    use_pty: bool,
}

#[derive(Debug)]
struct ProcessManagerInner {
    is_closing: bool,
    children: HashMap<TaskId<'static>, Vec<child::Child>>,
    size: Option<PtySize>,
}

#[derive(Debug, Clone, Copy)]
enum CloseMode {
    Stop,
    Shutdown(Option<Duration>),
    Kill,
    Wait,
}

#[derive(Debug, Clone, Copy)]
pub struct PtySize {
    rows: u16,
    cols: u16,
}

impl ProcessManager {
    pub fn new(use_pty: bool) -> Self {
        debug!("spawning children with pty: {use_pty}");
        Self {
            state: Arc::new(Mutex::new(ProcessManagerInner {
                is_closing: false,
                children: HashMap::new(),
                size: None,
            })),
            use_pty,
        }
    }

    /// Construct a process manager and infer if pty should be used
    pub fn infer() -> Self {
        // Only use PTY if we're not on windows and we're currently hooked up to a
        // in a TTY
        let use_pty = !cfg!(windows) && std::io::stdout().is_terminal();
        Self::new(use_pty)
    }

    pub fn running_task_ids(&self) -> Vec<String> {
        let lock = self.state.lock().expect("lock poisoned");
        let mut task_ids = lock
            .children
            .iter()
            .filter(|(_, children)| children.iter().any(|child| !child.has_exited()))
            .map(|(task_id, _)| task_id.to_string())
            .collect::<Vec<_>>();
        task_ids.sort();
        task_ids
    }

    /// Returns whether children will be spawned attached to a pseudoterminal
    pub fn use_pty(&self) -> bool {
        self.use_pty
    }

    /// Returns whether or not closing a child's stdin will result in it
    /// immediately exiting.
    pub fn closing_stdin_ends_process(&self) -> bool {
        // Processes spawned hooked up to ConPTY on Windows will immediately exit
        // if their stdin is closed. We avoid closing stdin in this case.
        cfg!(windows) && self.use_pty
    }
}

impl ProcessManager {
    /// Spawn a new child process to run the given command.
    ///
    /// The handle of the child can be either waited or stopped by the caller,
    /// as well as the entire process manager.
    ///
    /// If spawn returns None, the process manager is closed and the child
    /// process was not spawned. If spawn returns Some(Err), the process
    /// manager is open, but the child process failed to spawn.
    ///
    /// The task_id is used to track which processes belong to which tasks,
    /// enabling selective stopping of processes by task.
    pub fn spawn(
        &self,
        command: Command,
        stop_timeout: Duration,
        task_id: TaskId<'static>,
    ) -> Option<io::Result<child::Child>> {
        let label = tracing::enabled!(tracing::Level::TRACE)
            .then(|| command.label())
            .unwrap_or_default();
        trace!("acquiring lock for spawning {label}");
        let mut lock = self.state.lock().unwrap();
        trace!("acquired lock for spawning {label}");
        if lock.is_closing {
            debug!("process manager closing");
            return None;
        }
        let pty_size = self.use_pty.then(|| lock.pty_size()).flatten();
        let child = child::Child::spawn(
            command,
            child::ShutdownStyle::Graceful(Some(stop_timeout)),
            pty_size,
        );
        if let Ok(child) = &child {
            lock.children
                .entry(task_id)
                .or_default()
                .push(child.clone());
        }
        trace!("releasing lock for spawning {label}");
        Some(child)
    }

    /// Stop the process manager, closing all child processes. On posix
    /// systems this will send a SIGINT, and on windows it will just kill
    /// the process immediately.
    pub async fn stop(&self) {
        self.close(CloseMode::Stop).await
    }

    /// Gracefully shut down all child processes, optionally escalating to a
    /// force kill after `stop_timeout` elapses.
    pub async fn shutdown(&self, stop_timeout: Option<Duration>) {
        self.close(CloseMode::Shutdown(stop_timeout)).await
    }

    /// Force kill all child processes immediately.
    pub async fn kill_all(&self) {
        self.close(CloseMode::Kill).await
    }

    /// Stop all processes associated with the given task IDs.
    /// This is used to selectively restart tasks without shutting down
    /// the entire process manager.
    pub async fn stop_tasks(&self, task_ids: &[TaskId<'static>]) {
        let children_to_stop: Vec<_> = {
            let mut lock = self.state.lock().expect("not poisoned");

            // If we're closing, return early - close() will stop all children and they
            // should report Shutdown. By checking is_closing under the lock, we ensure
            // mutual exclusion: either close() runs first (we return early here), or we
            // run first and remove children before close() can see them.
            if lock.is_closing {
                return;
            }

            task_ids
                .iter()
                .filter_map(|task_id| lock.children.remove(task_id))
                .flatten()
                .collect()
        };

        debug!(
            "stopping {} processes for {} tasks",
            children_to_stop.len(),
            task_ids.len()
        );

        let mut set = JoinSet::new();
        for child in children_to_stop {
            set.spawn(async move {
                let mut child = child;
                child.stop().await
            });
        }

        while let Some(out) = set.join_next().await {
            trace!("process exited: {:?}", out);
        }
    }

    /// Stop the process manager, waiting for all child processes to exit.
    ///
    /// If you want to set a timeout, use `tokio::time::timeout` and
    /// `Self::stop` if the timeout elapses.
    pub async fn wait(&self) {
        self.close(CloseMode::Wait).await
    }

    /// Close the process manager, running the given callback on each child
    ///
    /// note: this is designed to be called multiple times, ie calling close
    /// with two different strategies will propagate both signals to the child
    /// processes. clearing the task queue and re-enabling spawning are both
    /// idempotent operations
    async fn close(&self, close_mode: CloseMode) {
        let mut set = JoinSet::new();

        {
            let mut lock = self.state.lock().expect("not poisoned");
            lock.is_closing = true;

            for child in lock.children.values().flatten() {
                // Mark each child as closing so it knows this is a shutdown, not a restart.
                // This is done under the lock to ensure mutual exclusion with stop_tasks().
                child.set_closing();
                let child = child.clone();
                set.spawn(async move {
                    let mut child = child;
                    match close_mode {
                        CloseMode::Stop => child.stop().await,
                        CloseMode::Shutdown(stop_timeout) => {
                            child
                                .shutdown(child::ShutdownStyle::Graceful(stop_timeout))
                                .await
                        }
                        CloseMode::Kill => child.kill().await,
                        CloseMode::Wait => child.wait().await,
                    }
                });
            }
        }

        debug!("waiting for {} processes to exit", set.len());

        while let Some(out) = set.join_next().await {
            trace!("process exited: {:?}", out);
        }

        {
            let mut lock = self.state.lock().expect("not poisoned");
            lock.children.clear();
        }
    }

    pub fn set_pty_size(&self, rows: u16, cols: u16) {
        self.state.lock().expect("not poisoned").size = Some(PtySize { rows, cols });
    }

    pub fn is_closing(&self) -> bool {
        self.state.lock().expect("not poisoned").is_closing
    }
}

impl ProcessManagerInner {
    fn pty_size(&mut self) -> Option<PtySize> {
        if self.size.is_none() {
            self.size = PtySize::from_tty();
        }
        self.size
    }
}

impl PtySize {
    fn from_tty() -> Option<Self> {
        console::Term::stdout()
            .size_checked()
            .map(|(rows, cols)| Self { rows, cols })
    }
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

#[cfg(test)]
mod test {
    use std::{fs, time::Instant};

    use futures::{StreamExt, stream::FuturesUnordered};
    use test_case::test_case;
    use tokio::{join, time::sleep};
    use tracing_test::traced_test;

    use super::*;

    fn get_command() -> Command {
        get_script_command("sleep_5_interruptable.js")
    }

    fn get_script_command(script_name: &str) -> Command {
        let mut cmd = Command::new("node");
        cmd.args([format!("./test/scripts/{script_name}")]);
        cmd
    }

    fn test_task_id() -> TaskId<'static> {
        TaskId::new("test-pkg", "test-task")
    }

    #[cfg(unix)]
    fn process_exists(pid: libc::pid_t) -> bool {
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(unix)]
    async fn wait_for_process_to_exit(pid: libc::pid_t) {
        for _ in 0..50 {
            if !process_exists(pid) {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }

        panic!("process {pid} should have exited");
    }

    const STOPPED_EXIT: Option<ChildExit> = Some(if cfg!(windows) {
        ChildExit::Killed
    } else {
        ChildExit::Interrupted
    });

    #[tokio::test]
    async fn test_basic() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(
                get_script_command("hello_world.js"),
                Duration::from_secs(2),
                test_task_id(),
            )
            .unwrap()
            .unwrap();
        let mut out = Vec::new();
        let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();
        assert_eq!(exit, Some(ChildExit::Finished(Some(0))));
        assert_eq!(out, b"hello world\n");
    }

    #[tokio::test]
    async fn test_multiple() {
        let manager = ProcessManager::new(false);

        let children = (0..2)
            .map(|i| {
                manager
                    .spawn(
                        get_command(),
                        Duration::from_secs(2),
                        TaskId::new("test-pkg", &format!("task-{i}")).into_owned(),
                    )
                    .unwrap()
                    .unwrap()
            })
            .collect::<Vec<_>>();

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;

        for mut child in children {
            let exit = child.wait().await;
            assert_eq!(exit, STOPPED_EXIT,);
        }
    }

    #[tokio::test]
    async fn test_closed() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(get_command(), Duration::from_secs(2), test_task_id())
            .unwrap()
            .unwrap();
        let mut out = Vec::new();
        let (exit, _) = join! {
            child.wait_with_piped_outputs(&mut out),
            manager.stop(),
        };
        let exit = exit.unwrap();
        assert_eq!(exit, STOPPED_EXIT,);
        assert_eq!(
            out, b"",
            "child process should exit before output is printed"
        );

        // Verify that we can't start new child processes
        assert!(
            manager
                .spawn(get_command(), Duration::from_secs(2), test_task_id())
                .is_none()
        );

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_exit_code() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(
                get_script_command("hello_world.js"),
                Duration::from_secs(2),
                test_task_id(),
            )
            .unwrap()
            .unwrap();

        let code = child.wait().await;
        assert_eq!(code, Some(ChildExit::Finished(Some(0))));

        // TODO: maybe we should do some assertion that there was nothing to shut down
        // and this is a noop?
        manager.stop().await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_message_after_stop() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(
                get_script_command("hello_world.js"),
                Duration::from_secs(2),
                test_task_id(),
            )
            .unwrap()
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        let exit = child.wait().await;
        assert_eq!(exit, Some(ChildExit::Finished(Some(0))));

        manager.stop().await;

        // this is idempotent, so calling it after the manager is stopped is ok
        let kill_code = child.kill().await;
        assert_eq!(kill_code, Some(ChildExit::Finished(Some(0))));

        let code = child.wait().await;
        assert_eq!(code, Some(ChildExit::Finished(Some(0))));
    }

    #[tokio::test]
    async fn test_reuse_manager() {
        let manager = ProcessManager::new(false);
        manager.spawn(get_command(), Duration::from_secs(2), test_task_id());

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;

        {
            let lock = manager.state.lock().unwrap();
            assert!(lock.children.is_empty());
        }

        // TODO: actually do some check that this is idempotent
        // idempotent
        manager.stop().await;
    }

    #[test_case("stop", "sleep_5_interruptable.js", STOPPED_EXIT)]
    #[test_case("wait", "hello_world.js", Some(ChildExit::Finished(Some(0))))]
    #[tokio::test]
    async fn test_stop_multiple_tasks_shared(
        strategy: &str,
        script: &str,
        expected: Option<ChildExit>,
    ) {
        let manager = ProcessManager::new(false);
        let tasks = FuturesUnordered::new();

        for i in 0..10 {
            let manager = manager.clone();
            let command = get_script_command(script);
            let task_id = TaskId::new("test-pkg", &format!("task-{i}")).into_owned();
            tasks.push(tokio::spawn(async move {
                manager
                    .spawn(command, Duration::from_secs(1), task_id)
                    .unwrap()
                    .unwrap()
                    .wait()
                    .await
            }));
        }

        // wait for tasks to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        match strategy {
            "stop" => manager.stop().await,
            "wait" => manager.wait().await,
            _ => panic!("unknown strategy"),
        }

        // tasks return proper exit code
        assert!(
            tasks.all(|v| async { v.unwrap() == expected }).await,
            "not all tasks returned the correct code: {expected:?}"
        );
    }

    #[tokio::test]
    async fn test_wait_multiple_tasks() {
        let manager = ProcessManager::new(false);

        let mut out = Vec::new();
        let mut child = manager
            .spawn(get_command(), Duration::from_secs(1), test_task_id())
            .unwrap()
            .unwrap();

        // let the task start
        tokio::time::sleep(Duration::from_millis(50)).await;

        let start_time = Instant::now();

        // we support 'close escalation'; someone can call
        // stop even if others are waiting
        let (exit, _, _) = join! {
            child.wait_with_piped_outputs(&mut out),
            manager.wait(),
            manager.stop(),
        };

        assert_eq!(exit.unwrap(), STOPPED_EXIT);
        assert_eq!(
            out, b"",
            "child process was stopped before any output was written"
        );

        let finish_time = Instant::now();

        assert!((finish_time - start_time).lt(&Duration::from_secs(2)));
    }

    #[tokio::test]
    async fn test_stop_tasks_selective() {
        let manager = ProcessManager::new(false);
        let task_id_1 = TaskId::new("pkg-a", "build").into_owned();
        let task_id_2 = TaskId::new("pkg-b", "build").into_owned();

        let mut child1 = manager
            .spawn(get_command(), Duration::from_secs(2), task_id_1.clone())
            .unwrap()
            .unwrap();
        let mut child2 = manager
            .spawn(get_command(), Duration::from_secs(2), task_id_2)
            .unwrap()
            .unwrap();

        sleep(Duration::from_millis(50)).await;
        manager.stop_tasks(&[task_id_1]).await;

        assert_eq!(child1.wait().await, STOPPED_EXIT);
        assert!(!child1.is_closing());

        manager.stop().await;

        assert_eq!(child2.wait().await, STOPPED_EXIT);
        assert!(child2.is_closing());
    }

    #[tokio::test]
    async fn test_stop_tasks_during_close() {
        let manager = ProcessManager::new(false);
        let task_id = TaskId::new("pkg-a", "build").into_owned();

        let mut child = manager
            .spawn(get_command(), Duration::from_secs(2), task_id.clone())
            .unwrap()
            .unwrap();

        sleep(Duration::from_millis(50)).await;

        let task_ids = [task_id];
        let (_, _) = join! {
            manager.stop(),
            manager.stop_tasks(&task_ids),
        };

        assert_eq!(child.wait().await, STOPPED_EXIT);
        assert!(child.is_closing());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_shutdown_can_be_force_killed() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(
                get_script_command("sleep_5_ignore.js"),
                Duration::from_secs(30),
                test_task_id(),
            )
            .unwrap()
            .unwrap();

        // Give the child time to install its SIGINT handler before we begin the
        // graceful shutdown flow.
        sleep(Duration::from_millis(500)).await;

        let shutdown_manager = manager.clone();
        let shutdown = tokio::spawn(async move {
            shutdown_manager.shutdown(None).await;
        });

        sleep(Duration::from_millis(100)).await;
        assert!(
            !shutdown.is_finished(),
            "graceful shutdown should still be waiting before force kill"
        );

        manager.kill_all().await;
        shutdown.await.unwrap();

        assert_eq!(child.wait().await, Some(ChildExit::Killed));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_shutdown_waits_for_process_group_after_child_exit() {
        let manager = ProcessManager::new(false);
        let pid_file =
            std::env::temp_dir().join(format!("turbo-process-grandchild-{}", std::process::id()));
        let _ = fs::remove_file(&pid_file);

        let script = format!(
            "trap 'exit 0' INT; sh -c 'trap \"\" INT TERM; while true; do sleep 0.2; done' & echo \
             $! > {}; while true; do sleep 0.2; done",
            pid_file.display()
        );
        let mut command = Command::new("sh");
        command.args(["-c", &script]);
        let mut child = manager
            .spawn(command, Duration::from_secs(30), test_task_id())
            .unwrap()
            .unwrap();

        let grandchild_pid = loop {
            if let Ok(contents) = fs::read_to_string(&pid_file)
                && let Ok(pid) = contents.trim().parse::<libc::pid_t>()
            {
                break pid;
            }
            sleep(Duration::from_millis(50)).await;
        };

        let shutdown_manager = manager.clone();
        let shutdown = tokio::spawn(async move {
            shutdown_manager.shutdown(None).await;
        });

        sleep(Duration::from_millis(500)).await;
        assert!(
            !shutdown.is_finished(),
            "shutdown should wait for the still-running process group"
        );
        assert!(
            process_exists(grandchild_pid),
            "grandchild should still be running before force kill"
        );

        manager.kill_all().await;
        tokio::time::timeout(Duration::from_secs(5), shutdown)
            .await
            .expect("shutdown should finish after force kill")
            .expect("shutdown task should not panic");
        assert_eq!(child.wait().await, Some(ChildExit::Killed));
        wait_for_process_to_exit(grandchild_pid).await;

        let _ = fs::remove_file(pid_file);
    }

    #[tokio::test]
    async fn test_normal_exit_waits_for_worker_before_completing() {
        let manager = ProcessManager::new(false);
        let marker_file =
            std::env::temp_dir().join(format!("turbo-normal-exit-marker-{}", std::process::id()));
        let pid_file =
            std::env::temp_dir().join(format!("turbo-normal-exit-worker-{}", std::process::id()));
        let _ = fs::remove_file(&marker_file);
        let _ = fs::remove_file(&pid_file);

        let mut command = Command::new("node");
        command.args([
            std::ffi::OsString::from("./test/scripts/normal_exit_worker.js"),
            marker_file.as_os_str().to_os_string(),
            pid_file.as_os_str().to_os_string(),
            std::ffi::OsString::from("800"),
        ]);
        let mut child = manager
            .spawn(command, Duration::from_secs(30), test_task_id())
            .unwrap()
            .unwrap();

        for _ in 0..50 {
            if pid_file.exists() {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        assert!(
            pid_file.exists(),
            "worker pid file should have been written"
        );

        let start = Instant::now();
        let exit = child.wait().await;
        let elapsed = start.elapsed();
        let marker_existed_when_wait_returned = marker_file.exists();

        if !marker_existed_when_wait_returned {
            for _ in 0..100 {
                if marker_file.exists() {
                    break;
                }
                sleep(Duration::from_millis(20)).await;
            }
        }

        let _ = fs::remove_file(&marker_file);
        let _ = fs::remove_file(&pid_file);

        assert_eq!(exit, Some(ChildExit::Finished(Some(0))));
        assert!(
            marker_existed_when_wait_returned,
            "child.wait() returned before the worker wrote its output"
        );
        assert!(
            elapsed >= Duration::from_millis(700),
            "child.wait() should have blocked for the worker; only waited {elapsed:?}"
        );
    }
}
