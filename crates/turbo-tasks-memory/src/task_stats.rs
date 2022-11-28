use std::time::Duration;

use turbo_tasks::StatsType;

/// Keeps track of the number of times a task has been executed, and its
/// duration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TaskStats {
    Essential(TaskStatsEssential),
    Full(Box<TaskStatsFull>),
}

impl TaskStats {
    /// Creates a new [`TaskStats`].
    pub fn new(stats_type: StatsType) -> Self {
        match stats_type {
            turbo_tasks::StatsType::Essential => Self::Essential(TaskStatsEssential::default()),
            turbo_tasks::StatsType::Full => Self::Full(Box::default()),
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
            Self::Essential(stats) => {
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
            Self::Essential(stats) => {
                stats.last_duration = 0;
            }
        }
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct TaskStatsEssential {
    /// The last duration of the task in nanoseconds.
    last_duration: u64,
}

impl TaskStatsEssential {
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
