use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

static ENABLE_FULL_STATS: AtomicBool = AtomicBool::new(false);

/// Enables full stats collection.
///
/// This is useful for debugging, but it has a slight memory and performance
/// impact.
pub fn enable_full_stats() {
    ENABLE_FULL_STATS.store(true, Ordering::Release);
}

/// Returns `true` if full stats collection is enabled.
pub fn full_stats() -> bool {
    ENABLE_FULL_STATS.load(Ordering::Acquire)
}

/// Keeps track of the number of times a task has been executed, and its
/// duration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TaskStats {
    Small(TaskStatsSmall),
    Full(Box<TaskStatsFull>),
}

impl TaskStats {
    /// Creates a new [`TaskStats`].
    pub fn new() -> Self {
        if full_stats() {
            Self::Full(Box::default())
        } else {
            Self::Small(TaskStatsSmall::default())
        }
    }

    /// Resets the number of executions to 1 only if it was greater than 1.
    pub fn reset_executions(&mut self) {
        if let Self::Full(stats) = self {
            if stats.executions > 1 {
                stats.executions = 1;
            }
        }
    }

    /// Increments the number of executions by 1.
    pub fn increment_executions(&mut self) {
        if let Self::Full(stats) = self {
            stats.executions += 1;
        }
    }

    /// Registers a task duration.
    pub fn register_duration(&mut self, duration: Duration) {
        match self {
            Self::Full(stats) => {
                stats.total_duration += duration;
                stats.last_duration = duration;
            }
            Self::Small(stats) => {
                stats.last_duration = duration.as_nanos().try_into().unwrap_or(u64::MAX);
            }
        }
    }

    /// Resets stats to their default, zero-value.
    pub fn reset(&mut self) {
        match self {
            Self::Full(stats) => {
                stats.executions = 0;
                stats.total_duration = Duration::ZERO;
                stats.last_duration = Duration::ZERO;
            }
            Self::Small(stats) => {
                stats.last_duration = 0;
            }
        }
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct TaskStatsSmall {
    /// The last duration of the task in nanoseconds.
    last_duration: u64,
}

impl TaskStatsSmall {
    /// Returns the last duration of the task.
    pub fn last_duration(&self) -> Duration {
        Duration::from_nanos(self.last_duration)
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct TaskStatsFull {
    // While a u64 might be optimistic for executions, `TaskStatsFull` is aligned to 8 bytes
    // anyway, so it makes no difference to the size of this struct.
    /// The number of times the task has been executed.
    executions: u64,
    /// The last duration of the task.
    last_duration: Duration,
    /// The total duration of the task.
    total_duration: Duration,
}

impl TaskStatsFull {
    /// Returns the number of times the task has been executed.
    pub fn executions(&self) -> u64 {
        self.executions
    }

    /// Returns the last duration of the task.
    pub fn last_duration(&self) -> Duration {
        self.last_duration
    }

    /// Returns the total duration of the task.
    pub fn total_duration(&self) -> Duration {
        self.total_duration
    }
}
