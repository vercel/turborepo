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

#[derive(Default)]
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
    pub fn new() -> Self {
        Self::default()
    }

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

    /// Insert a finished task into the correct place in the finished section.
    /// The order of `finished` is expected to be: failure, success, cached
    /// with each subsection being sorted by finish time.
    /// Returns the index task was inserted at
    pub fn insert_finished_task(&mut self, task: Task<Finished>) -> usize {
        let index = match task.result() {
            TaskResult::Failure => self
                .finished
                .iter()
                .enumerate()
                .skip_while(|(_, task)| task.result() == TaskResult::Failure)
                .map(|(idx, _)| idx)
                .next(),
            TaskResult::Success => self
                .finished
                .iter()
                .enumerate()
                .skip_while(|(_, task)| {
                    task.result() == TaskResult::Failure || task.result() == TaskResult::Success
                })
                .map(|(idx, _)| idx)
                .next(),
            TaskResult::CacheHit => None,
        }
        .unwrap_or(self.finished.len());
        self.finished.insert(index, task);
        index
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    struct TestCase {
        failed: &'static [&'static str],
        passed: &'static [&'static str],
        cached: &'static [&'static str],
        result: TaskResult,
        expected_index: usize,
    }

    impl TestCase {
        pub const fn new(result: TaskResult, expected_index: usize) -> Self {
            Self {
                failed: &[],
                passed: &[],
                cached: &[],
                result,
                expected_index,
            }
        }

        pub const fn failed<const N: usize>(mut self, failed: &'static [&'static str; N]) -> Self {
            self.failed = failed.as_slice();
            self
        }

        pub const fn passed<const N: usize>(mut self, passed: &'static [&'static str; N]) -> Self {
            self.passed = passed.as_slice();
            self
        }

        pub const fn cached<const N: usize>(mut self, cached: &'static [&'static str; N]) -> Self {
            self.cached = cached.as_slice();
            self
        }

        pub fn tasks(&self) -> TasksByStatus {
            let failed = self.failed.iter().map(|name| {
                Task::new(name.to_string())
                    .start()
                    .finish(TaskResult::Failure)
            });
            let passed = self.passed.iter().map(|name| {
                Task::new(name.to_string())
                    .start()
                    .finish(TaskResult::Success)
            });
            let cached = self.passed.iter().map(|name| {
                Task::new(name.to_string())
                    .start()
                    .finish(TaskResult::CacheHit)
            });
            TasksByStatus {
                running: Vec::new(),
                planned: Vec::new(),
                finished: failed.chain(passed).chain(cached).collect(),
            }
        }
    }

    const EMPTY_FAIL: TestCase = TestCase::new(TaskResult::Failure, 0);
    const EMPTY_PASS: TestCase = TestCase::new(TaskResult::Success, 0);
    const EMPTY_CACHE: TestCase = TestCase::new(TaskResult::CacheHit, 0);
    const BASIC_FAIL: TestCase = TestCase::new(TaskResult::Failure, 1)
        .failed(&["fail"])
        .passed(&["passed"])
        .cached(&["cached"]);
    const BASIC_PASS: TestCase = TestCase::new(TaskResult::Success, 2)
        .failed(&["fail"])
        .passed(&["passed"])
        .cached(&["cached"]);
    const BASIC_CACHE: TestCase = TestCase::new(TaskResult::CacheHit, 3)
        .failed(&["fail"])
        .passed(&["passed"])
        .cached(&["cached"]);

    #[test_case(EMPTY_FAIL)]
    #[test_case(EMPTY_PASS)]
    #[test_case(EMPTY_CACHE)]
    #[test_case(BASIC_FAIL)]
    #[test_case(BASIC_PASS)]
    #[test_case(BASIC_CACHE)]
    fn test_finished_task(test_case: TestCase) {
        let mut tasks = test_case.tasks();
        let actual = tasks.insert_finished_task(
            Task::new("inserted".into())
                .start()
                .finish(test_case.result),
        );
        assert_eq!(actual, test_case.expected_index);
    }
}
