#![allow(dead_code)]
use std::{collections::HashSet, mem, time::Instant};

use super::{event::TaskResult, Error};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct Planned;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct Running {
    start: Instant,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct Finished {
    start: Instant,
    end: Instant,
    result: TaskResult,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct Task<S> {
    name: String,
    state: S,
}

pub enum TaskType {
    Planned,
    Running,
    Finished,
}

impl<S> Task<S> {
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Task<Planned> {
    pub fn new(name: String) -> Task<Planned> {
        Task {
            name,
            state: Planned,
        }
    }

    pub fn start(self) -> Task<Running> {
        Task {
            name: self.name,
            state: Running {
                start: Instant::now(),
            },
        }
    }
}

impl Task<Running> {
    pub fn finish(self, result: TaskResult) -> Task<Finished> {
        let Task {
            name,
            state: Running { start },
        } = self;
        Task {
            name,
            state: Finished {
                start,
                result,
                end: Instant::now(),
            },
        }
    }

    pub fn start(&self) -> Instant {
        self.state.start
    }

    pub fn restart(self) -> Task<Planned> {
        Task {
            name: self.name,
            state: Planned,
        }
    }
}

impl Task<Finished> {
    pub fn start(&self) -> Instant {
        self.state.start
    }

    pub fn end(&self) -> Instant {
        self.state.end
    }

    pub fn result(&self) -> TaskResult {
        self.state.result
    }

    pub fn restart(self) -> Task<Planned> {
        Task {
            name: self.name,
            state: Planned,
        }
    }
}

pub struct TaskNamesByStatus {
    pub running: Vec<String>,
    pub planned: Vec<String>,
    pub finished: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TasksByStatus {
    pub running: Vec<Task<Running>>,
    pub planned: Vec<Task<Planned>>,
    pub finished: Vec<Task<Finished>>,
}

impl TasksByStatus {
    pub fn all_empty(&self) -> bool {
        self.planned.is_empty() && self.finished.is_empty() && self.running.is_empty()
    }

    pub fn count_all(&self) -> usize {
        self.task_names_in_displayed_order().count()
    }

    pub fn task_names_in_displayed_order(&self) -> impl DoubleEndedIterator<Item = &str> + '_ {
        let running_names = self.running.iter().map(|task| task.name());
        let planned_names = self.planned.iter().map(|task| task.name());
        let finished_names = self.finished.iter().map(|task| task.name());

        running_names.chain(planned_names).chain(finished_names)
    }

    pub fn task_name(&self, index: usize) -> Result<&str, Error> {
        self.task_names_in_displayed_order()
            .nth(index)
            .ok_or_else(|| Error::TaskNotFoundIndex {
                index,
                len: self.count_all(),
            })
    }

    pub fn tasks_started(&self) -> Vec<String> {
        let (errors, success): (Vec<_>, Vec<_>) = self
            .finished
            .iter()
            .partition(|task| matches!(task.result(), TaskResult::Failure));

        // We return errors last as they most likely have information users want to see
        success
            .into_iter()
            .map(|task| task.name())
            .chain(self.running.iter().map(|task| task.name()))
            .chain(errors.into_iter().map(|task| task.name()))
            .map(|task| task.to_string())
            .collect()
    }

    pub fn restart_tasks<'a>(&mut self, tasks: impl Iterator<Item = &'a str>) {
        let mut tasks_to_restart = tasks.collect::<HashSet<_>>();

        let (restarted_running, keep_running): (Vec<_>, Vec<_>) = mem::take(&mut self.running)
            .into_iter()
            .partition(|task| tasks_to_restart.contains(task.name()));
        self.running = keep_running;

        let (restarted_finished, keep_finished): (Vec<_>, Vec<_>) = mem::take(&mut self.finished)
            .into_iter()
            .partition(|task| tasks_to_restart.contains(task.name()));
        self.finished = keep_finished;
        self.planned.extend(
            restarted_running
                .into_iter()
                .map(|task| task.restart())
                .chain(restarted_finished.into_iter().map(|task| task.restart())),
        );
        // There is a chance that watch might attempt to restart a task that did not
        // exist before.
        for task in &self.planned {
            tasks_to_restart.remove(task.name());
        }
        self.planned.extend(
            tasks_to_restart
                .into_iter()
                .map(ToOwned::to_owned)
                .map(Task::new),
        );
        self.planned.sort_unstable();
    }
}
