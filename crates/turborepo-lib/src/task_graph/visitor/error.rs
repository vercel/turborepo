// Warning that comes from the execution of the task
#[derive(Debug, Clone)]
pub struct TaskWarning {
    task_id: String,
    missing_platform_env: Vec<String>,
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

impl TaskWarning {
    /// Construct a new warning for a given task with the
    /// Returns `None` if there are no missing platform environment variables
    pub fn new(task_id: &str, missing_platform_env: Vec<String>) -> Option<Self> {
        if missing_platform_env.is_empty() {
            return None;
        }
        Some(Self {
            task_id: task_id.to_owned(),
            missing_platform_env,
        })
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// All missing platform environment variables.
    /// Guaranteed to have at least length 1 due to constructor validation.
    pub fn missing_platform_env(&self) -> &[String] {
        &self.missing_platform_env
    }
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_warning_no_vars() {
        let no_warning = TaskWarning::new("a-task", vec![]);
        assert!(no_warning.is_none());
    }

    #[test]
    fn test_warning_some_var() {
        let warning = TaskWarning::new("a-task", vec!["MY_VAR".into()]);
        assert!(warning.is_some());
        let warning = warning.unwrap();
        assert_eq!(warning.task_id(), "a-task");
        assert_eq!(warning.missing_platform_env(), &["MY_VAR".to_owned()]);
    }
}
