pub enum Event {
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
    },
    Stop(std::sync::mpsc::SyncSender<()>),
    // Stop initiated by the TUI itself
    InternalStop,
    Tick,
    Up,
    Down,
    ScrollUp,
    ScrollDown,
    SetStdin {
        task: String,
        stdin: Box<dyn std::io::Write + Send>,
    },
    EnterInteractive,
    ExitInteractive,
    Input {
        bytes: Vec<u8>,
    },
    UpdateTasks {
        tasks: Vec<String>,
    },
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum TaskResult {
    Success(CacheResult),
    Failure,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum CacheResult {
    Hit,
    Miss,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum OutputLogs {
    // Entire task output is persisted after run
    Full,
    // None of a task output is persisted after run
    None,
    // Only the status line of a task is persisted
    HashOnly,
    // Output is only persisted if it is a cache miss
    NewOnly,
    // Output is only persisted if the task failed
    ErrorsOnly,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn assert_event_send() {
        fn send_sync<T: Send>() {}
        send_sync::<Event>();
    }
}
