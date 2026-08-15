//! Estimate of the uncached wall-clock duration of a run.
//!
//! `estimatedUncachedDuration` answers: "how long would this run have taken
//! if the cache had not existed?" It walks the task dependency graph and
//! computes the critical path (longest dependency chain) using per-task
//! uncached durations, so parallel branches are not summed.
//!
//! Per-task uncached duration (milliseconds):
//! - Cache hit: `cache.timeSaved` from the cache artifact metadata, i.e. the
//!   duration of the execution that originally produced the artifact. When that
//!   metadata is missing or zero, the task contributes 0.
//! - Executed cache miss (real runs): the task's measured wall-clock execution
//!   duration (`endTime - startTime`).
//! - Anything else (dry-run cache misses, tasks that never ran): 0, because the
//!   task has never been executed by this repository and Turbo has no timing
//!   information for it.
//!
//! Dry runs therefore produce a *partial* estimate: it accounts only for the
//! cache-hit portion of the graph, since misses have no observed timing.
//!
//! The estimate ignores scheduling constraints (concurrency limits,
//! scheduling overhead) and models an ideal machine with unlimited
//! parallelism: it is the wall-clock time of the longest dependency chain,
//! which is the lower bound of any uncached execution.

use std::collections::HashMap;

use turborepo_task_id::TaskId;

use crate::task::{CacheStatus, TaskSummary};

/// Computes the estimated uncached wall-clock duration, in milliseconds, of
/// the critical path through the task DAG.
///
/// `tasks` must contain every task in the run; `dependencies` on each
/// `TaskSummary` are used as the graph edges. Dependencies that point outside
/// `tasks` are ignored (they are not part of this run).
pub fn estimated_uncached_duration(tasks: &[TaskSummary]) -> u64 {
    let by_id: HashMap<&TaskId<'static>, usize> = tasks
        .iter()
        .enumerate()
        .map(|(index, task)| (&task.task_id, index))
        .collect();

    let mut memo: HashMap<usize, u64> = HashMap::with_capacity(tasks.len());
    let mut max_finish = 0u64;
    for index in 0..tasks.len() {
        // Cycle guard: task graphs are validated acyclic at engine
        // construction, but guard anyway so a malformed input yields a
        // (slightly wrong) number instead of unbounded recursion.
        let mut visiting = vec![false; tasks.len()];
        let finish = earliest_finish(index, tasks, &by_id, &mut memo, &mut visiting);
        max_finish = max_finish.max(finish);
    }
    max_finish
}

/// Earliest finish time (ms from run start, assuming unlimited parallelism)
/// for a task: max over dependencies of their earliest finish, plus this
/// task's uncached duration.
fn earliest_finish(
    index: usize,
    tasks: &[TaskSummary],
    by_id: &HashMap<&TaskId<'static>, usize>,
    memo: &mut HashMap<usize, u64>,
    visiting: &mut [bool],
) -> u64 {
    if let Some(&finish) = memo.get(&index) {
        return finish;
    }
    if visiting[index] {
        // Cycle: stop expanding and contribute nothing further.
        return 0;
    }
    visiting[index] = true;

    let task = &tasks[index];
    let mut dependency_finish = 0u64;
    for dependency in &task.shared.dependencies {
        let finish = match by_id.get(dependency) {
            Some(&dependency_index) => {
                earliest_finish(dependency_index, tasks, by_id, memo, visiting)
            }
            // Dependency is not part of this run (e.g. filtered out).
            None => 0,
        };
        dependency_finish = dependency_finish.max(finish);
    }

    visiting[index] = false;

    let finish = dependency_finish.saturating_add(task_uncached_duration_ms(task));
    memo.insert(index, finish);
    finish
}

/// Per-task uncached duration in milliseconds. See module docs for semantics.
fn task_uncached_duration_ms(task: &TaskSummary) -> u64 {
    let cache = &task.shared.cache;
    match cache.status() {
        // A cache hit did not execute; the best estimate of its uncached
        // duration is the recorded duration of the run that produced the
        // artifact. Missing/zero metadata means we cannot estimate, so the
        // task contributes 0.
        CacheStatus::Hit => cache.time_saved(),
        CacheStatus::Miss => {
            // A real run that executed this task measured its wall-clock
            // duration directly.
            task.shared
                .execution
                .as_ref()
                .map(|execution| {
                    execution
                        .end_time
                        .saturating_sub(execution.start_time)
                        .max(0) as u64
                })
                // Dry runs (and tasks that never started) have no execution
                // timing; there is nothing to estimate from.
                .unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod test {
    use std::{collections::BTreeMap, sync::Arc};

    use turborepo_types::EnvMode;

    use super::*;
    use crate::task::{
        SharedTaskSummary, TaskCacheSummary, TaskEnvConfiguration, TaskEnvVarSummary,
        TaskExecutionSummary, TaskSummaryTaskDefinition,
    };

    fn task(task_id: &str, deps: &[&str]) -> TaskSummary {
        let (package, name) = task_id.split_once('#').unwrap();
        TaskSummary {
            task_id: TaskId::new(package, name).into_owned(),
            task: name.to_string(),
            package: package.to_string(),
            shared: SharedTaskSummary {
                hash: Some(Arc::from("hash")),
                hash_reason: None,
                inputs: BTreeMap::new(),
                hash_of_external_dependencies: String::new(),
                cache: TaskCacheSummary::cache_miss(),
                command: String::new(),
                cli_arguments: Vec::new(),
                outputs: None,
                excluded_outputs: None,
                log_file: None,
                directory: None,
                dependencies: deps
                    .iter()
                    .map(|dep| {
                        let (package, name) = dep.split_once('#').unwrap();
                        TaskId::new(package, name).into_owned()
                    })
                    .collect(),
                dependents: Vec::new(),
                with: Vec::new(),
                resolved_task_definition: TaskSummaryTaskDefinition::default(),
                expanded_outputs: Vec::new(),
                framework: String::new(),
                env_mode: EnvMode::Strict,
                environment_variables: TaskEnvVarSummary {
                    specified: TaskEnvConfiguration {
                        env: Vec::new(),
                        pass_through_env: None,
                    },
                    configured: Vec::new(),
                    inferred: Vec::new(),
                    pass_through: None,
                },
                execution: None,
            },
        }
    }

    fn executed(mut task: TaskSummary, start: i64, end: i64) -> TaskSummary {
        task.shared.execution = Some(TaskExecutionSummary {
            start_time: start,
            end_time: end,
            error: None,
            exit_code: Some(0),
        });
        task
    }

    fn cache_hit(mut task: TaskSummary, time_saved: u64) -> TaskSummary {
        task.shared.cache = TaskCacheSummary::from(Some(turborepo_cache::CacheHitMetadata {
            source: turborepo_cache::CacheSource::Local,
            time_saved,
            sha: None,
            dirty_hash: None,
        }));
        task
    }

    #[test]
    fn empty_run_is_zero() {
        assert_eq!(estimated_uncached_duration(&[]), 0);
    }

    #[test]
    fn single_executed_miss() {
        let tasks = vec![executed(task("app#build", &[]), 1000, 3000)];
        assert_eq!(estimated_uncached_duration(&tasks), 2000);
    }

    #[test]
    fn dependency_chain_is_summed() {
        // app#build -> lib#build -> util#build, each 1s: 3s total.
        let tasks = vec![
            executed(task("app#build", &["lib#build"]), 0, 1000),
            executed(task("lib#build", &["util#build"]), 0, 1000),
            executed(task("util#build", &[]), 0, 1000),
        ];
        assert_eq!(estimated_uncached_duration(&tasks), 3000);
    }

    #[test]
    fn parallel_tasks_take_the_max_not_the_sum() {
        // app#build depends on both libs; libs run in parallel.
        let tasks = vec![
            executed(task("app#build", &["lib-a#build", "lib-b#build"]), 0, 500),
            executed(task("lib-a#build", &[]), 0, 2000),
            executed(task("lib-b#build", &[]), 0, 3000),
        ];
        // max(2000, 3000) + 500
        assert_eq!(estimated_uncached_duration(&tasks), 3500);
    }

    #[test]
    fn diamond_dependencies_are_not_double_counted() {
        let tasks = vec![
            executed(task("app#build", &["a#build", "b#build"]), 0, 100),
            executed(task("a#build", &["base#build"]), 0, 100),
            executed(task("b#build", &["base#build"]), 0, 100),
            executed(task("base#build", &[]), 0, 100),
        ];
        assert_eq!(estimated_uncached_duration(&tasks), 300);
    }

    #[test]
    fn cache_hit_uses_time_saved() {
        let tasks = vec![cache_hit(task("app#build", &[]), 42_000)];
        assert_eq!(estimated_uncached_duration(&tasks), 42_000);
    }

    #[test]
    fn mixed_hits_and_misses_compose_along_the_chain() {
        // app (miss, ran 1s) depends on lib (hit, saved 10s).
        let tasks = vec![
            executed(task("app#build", &["lib#build"]), 10_000, 11_000),
            cache_hit(task("lib#build", &[]), 10_000),
        ];
        assert_eq!(estimated_uncached_duration(&tasks), 11_000);
    }

    #[test]
    fn cache_hit_with_zero_time_saved_contributes_nothing() {
        let tasks = vec![cache_hit(task("app#build", &[]), 0)];
        assert_eq!(estimated_uncached_duration(&tasks), 0);
    }

    #[test]
    fn dry_run_misses_contribute_nothing() {
        // Dry run: no execution timing at all, one miss and one hit.
        let tasks = vec![
            task("app#build", &["lib#build"]),
            cache_hit(task("lib#build", &[]), 5_000),
        ];
        assert_eq!(estimated_uncached_duration(&tasks), 5_000);
    }

    #[test]
    fn cache_hit_beats_execution_when_both_present() {
        // A real-run cache hit has a near-zero execution (restore time);
        // the uncached estimate must come from timeSaved.
        let tasks = vec![cache_hit(executed(task("app#build", &[]), 0, 3), 7_000)];
        assert_eq!(estimated_uncached_duration(&tasks), 7_000);
    }

    #[test]
    fn dependencies_outside_the_run_are_ignored() {
        let mut t = task("app#build", &["filtered-out#build"]);
        t = executed(t, 0, 1000);
        assert_eq!(estimated_uncached_duration(&[t]), 1000);
    }

    #[test]
    fn cycles_terminate() {
        let tasks = vec![
            executed(task("a#build", &["b#build"]), 0, 1000),
            executed(task("b#build", &["a#build"]), 0, 1000),
        ];
        // Must terminate; exact value is unspecified for malformed input.
        assert!(estimated_uncached_duration(&tasks) <= 2000);
    }
}
