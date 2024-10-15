// Warning that comes from the execution of the task
#[derive(Debug, Clone)]
pub struct TaskWarning {
    pub task_id: String,
    pub missing_platform_env: Vec<String>,
}

// Error that comes from the execution of the task
#[derive(Debug, thiserror::Error, Clone)]
#[error("{task_id}: {cause}")]
pub struct TaskError {
    task_id: String,
    cause: TaskErrorCause,
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum TaskErrorCause {
    #[error("unable to spawn child process: {msg}")]
    // We eagerly serialize this in order to allow us to implement clone
    Spawn { msg: String },
    #[error("command {command} exited ({exit_code})")]
    Exit { command: String, exit_code: i32 },
    #[error("turbo has internal error processing task")]
    Internal,
}

impl TaskError {
    pub fn new(task_id: String, cause: TaskErrorCause) -> Self {
        Self { task_id, cause }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self.cause {
            TaskErrorCause::Exit { exit_code, .. } => Some(exit_code),
            _ => None,
        }
    }

    pub fn from_spawn(task_id: String, err: std::io::Error) -> Self {
        Self {
            task_id,
            cause: TaskErrorCause::Spawn {
                msg: err.to_string(),
            },
        }
    }

    pub fn from_execution(task_id: String, command: String, exit_code: i32) -> Self {
        Self {
            task_id,
            cause: TaskErrorCause::Exit { command, exit_code },
        }
    }
}

impl TaskErrorCause {
    pub fn from_spawn(err: std::io::Error) -> Self {
        TaskErrorCause::Spawn {
            msg: err.to_string(),
        }
    }

    pub fn from_execution(command: String, exit_code: i32) -> Self {
        TaskErrorCause::Exit { command, exit_code }
    }
}
