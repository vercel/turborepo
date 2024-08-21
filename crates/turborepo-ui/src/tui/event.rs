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
        result: CacheResult,
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
    Mouse(crossterm::event::MouseEvent),
    CopySelection,
    RestartTasks {
        tasks: Vec<String>,
    },
    Resize {
        rows: u16,
        cols: u16,
    },
    SearchEnter,
    SearchExit {
        restore_scroll: bool,
    },
    SearchScroll {
        direction: Direction,
    },
    SearchEnterChar(char),
    SearchBackspace,
}

pub enum Direction {
    Up,
    Down,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum TaskResult {
    Success,
    Failure,
    CacheHit,
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
