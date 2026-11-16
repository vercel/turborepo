//! Deciding which cache hits a run restores.
//!
//! A filtered or `--affected` run also visits tasks that were not selected,
//! because selected tasks depend on them. When such a dependency is a cache
//! hit, its outputs are only read by tasks that execute, and a dependent that
//! is itself a cache hit hashes the dependency's task hash, not its outputs.
//! So once every task's cache status is known up front, a dependency hit that
//! no executing task depends on needs no fetch, no restore, and no log
//! replay: it is reported as a hit and left alone.
//!
//! Planning can be wrong when a task expected to hit misses after all (for
//! example, because the entry was evicted). [`SkippedOutputs`] restores the
//! skipped dependencies of such a task before it executes.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use turborepo_cache::CacheHitMetadata;
use turborepo_engine::TaskNode;
use turborepo_run_cache::RunCache;
use turborepo_task_executor::DependencyOutputs;
use turborepo_task_hash::TaskHashTracker;
use turborepo_task_id::TaskId;

use crate::Engine;

/// What planning expects of a task, before any task has run.
#[derive(Debug, Clone)]
pub(super) enum Expectation {
    /// The task has no command, so it neither runs nor reads the cache.
    NoCommand,
    /// The cache holds outputs for the task's hash.
    Hit(CacheHitMetadata),
    /// The task will execute: a cache miss, an uncacheable task, or a task
    /// whose hash is only known once its dependencies have run.
    Executes,
}

/// Returns the expected cache hits whose outputs nothing in the run reads:
/// tasks that were not selected and that no executing task transitively
/// depends on. Outputs of selected tasks are always restored, so that what
/// the user asked for is on disk when the run ends.
pub(super) fn unread_hits(
    expectations: &HashMap<TaskId<'static>, Expectation>,
    is_selected: impl Fn(&TaskId<'static>) -> bool,
    dependencies: impl Fn(&TaskId<'static>) -> Vec<TaskId<'static>>,
) -> HashMap<TaskId<'static>, CacheHitMetadata> {
    // Everything an executing task transitively depends on can be read by it,
    // whether or not intermediate tasks have commands.
    let mut read = HashSet::new();
    let mut stack: Vec<TaskId<'static>> = expectations
        .iter()
        .filter(|(_, expectation)| matches!(expectation, Expectation::Executes))
        .map(|(task_id, _)| task_id.clone())
        .collect();
    while let Some(task_id) = stack.pop() {
        for dependency in dependencies(&task_id) {
            if read.insert(dependency.clone()) {
                stack.push(dependency);
            }
        }
    }

    expectations
        .iter()
        .filter_map(|(task_id, expectation)| match expectation {
            Expectation::Hit(status) if !is_selected(task_id) && !read.contains(task_id) => {
                Some((task_id.clone(), status.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The direct task dependencies of `task_id` in `engine`.
pub(super) fn task_dependencies(
    engine: &Engine,
    task_id: &TaskId<'static>,
) -> Vec<TaskId<'static>> {
    engine
        .dependencies(task_id)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|node| match node {
            TaskNode::Task(dependency) => Some(dependency.clone()),
            TaskNode::Root => None,
        })
        .collect()
}

struct SkippedRestore {
    hash: String,
    restored: tokio::sync::OnceCell<Result<(), String>>,
}

/// Restores outputs whose restore was skipped during planning, for a task
/// that unexpectedly has to execute.
pub(super) struct SkippedOutputs {
    engine: Arc<Engine>,
    run_cache: Arc<RunCache>,
    hash_tracker: TaskHashTracker,
    skipped: HashMap<TaskId<'static>, SkippedRestore>,
}

impl SkippedOutputs {
    pub(super) fn new(
        engine: Arc<Engine>,
        run_cache: Arc<RunCache>,
        hash_tracker: TaskHashTracker,
        skipped: impl IntoIterator<Item = (TaskId<'static>, String)>,
    ) -> Self {
        let skipped = skipped
            .into_iter()
            .map(|(task_id, hash)| {
                (
                    task_id,
                    SkippedRestore {
                        hash,
                        restored: tokio::sync::OnceCell::new(),
                    },
                )
            })
            .collect();
        Self {
            engine,
            run_cache,
            hash_tracker,
            skipped,
        }
    }

    /// Skipped transitive dependencies of `task_id`, deepest first.
    fn skipped_dependencies(&self, task_id: &TaskId<'static>) -> Vec<TaskId<'static>> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut queue = std::collections::VecDeque::from([task_id.clone()]);
        while let Some(current) = queue.pop_front() {
            for dependency in task_dependencies(&self.engine, &current) {
                if seen.insert(dependency.clone()) {
                    if self.skipped.contains_key(&dependency) {
                        order.push(dependency.clone());
                    }
                    queue.push_back(dependency);
                }
            }
        }
        order.reverse();
        order
    }

    async fn restore(&self, task_id: &TaskId<'static>, hash: &str) -> Result<(), String> {
        match self.run_cache.restore_outputs_for_hash(hash).await {
            Ok(Some(files)) => {
                tracing::debug!("restored skipped outputs of {task_id} for a dependent");
                self.hash_tracker
                    .insert_expanded_outputs(task_id.clone(), files);
                self.hash_tracker
                    .set_outputs_restored(task_id.clone(), true);
                Ok(())
            }
            Ok(None) => Err(format!(
                "outputs of {task_id} are needed but are no longer in the cache"
            )),
            Err(error) => Err(format!("failed to restore outputs of {task_id}: {error}")),
        }
    }
}

impl DependencyOutputs for SkippedOutputs {
    fn restore_for<'a>(
        &'a self,
        task_id: &'a TaskId<'static>,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            for dependency in self.skipped_dependencies(task_id) {
                let Some(skipped) = self.skipped.get(&dependency) else {
                    continue;
                };
                skipped
                    .restored
                    .get_or_init(|| self.restore(&dependency, &skipped.hash))
                    .await
                    .clone()?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use turborepo_cache::CacheSource;

    use super::*;

    fn hit() -> Expectation {
        Expectation::Hit(CacheHitMetadata {
            source: CacheSource::Remote,
            time_saved: 1,
            sha: None,
            dirty_hash: None,
        })
    }

    fn id(name: &str) -> TaskId<'static> {
        TaskId::try_from(name).unwrap().into_owned()
    }

    /// Plans over `edges` (dependent, dependency) with the given expectations
    /// and selected tasks, returning the skipped tasks sorted.
    fn plan(
        expectations: &[(&str, Expectation)],
        edges: &[(&str, &str)],
        selected: &[&str],
    ) -> Vec<String> {
        let expectations = expectations
            .iter()
            .map(|(task, expectation)| (id(task), expectation.clone()))
            .collect();
        let selected: HashSet<_> = selected.iter().map(|task| id(task)).collect();
        let mut skipped: Vec<_> = unread_hits(
            &expectations,
            |task| selected.contains(task),
            |task| {
                edges
                    .iter()
                    .filter(|(dependent, _)| id(dependent) == *task)
                    .map(|(_, dependency)| id(dependency))
                    .collect()
            },
        )
        .into_keys()
        .map(|task| task.to_string())
        .collect();
        skipped.sort();
        skipped
    }

    #[test]
    fn dependency_hits_under_only_hits_are_skipped() {
        // app#test (selected hit) -> lib#build (hit) -> util#build (hit)
        let skipped = plan(
            &[
                ("app#test", hit()),
                ("lib#build", hit()),
                ("util#build", hit()),
            ],
            &[("app#test", "lib#build"), ("lib#build", "util#build")],
            &["app#test"],
        );
        assert_eq!(skipped, ["lib#build", "util#build"]);
    }

    #[test]
    fn transitive_dependencies_of_an_executing_task_are_restored() {
        // app#test misses, so everything it can read must be on disk.
        let skipped = plan(
            &[
                ("app#test", Expectation::Executes),
                ("lib#build", hit()),
                ("util#build", hit()),
                ("other#build", hit()),
            ],
            &[("app#test", "lib#build"), ("lib#build", "util#build")],
            &["app#test"],
        );
        assert_eq!(skipped, ["other#build"]);
    }

    #[test]
    fn selected_hits_are_always_restored() {
        let skipped = plan(
            &[("app#build", hit()), ("lib#build", hit())],
            &[("app#build", "lib#build")],
            &["app#build", "lib#build"],
        );
        assert!(skipped.is_empty());
    }

    #[test]
    fn restores_reach_through_tasks_without_commands() {
        // A transit task between an executing task and a hit still connects
        // them: the executing task can read the hit's outputs.
        let skipped = plan(
            &[
                ("app#typecheck", Expectation::Executes),
                ("app#topo", Expectation::NoCommand),
                ("lib#topo", Expectation::NoCommand),
                ("lib#build", hit()),
            ],
            &[
                ("app#typecheck", "app#topo"),
                ("app#topo", "lib#topo"),
                ("lib#topo", "lib#build"),
            ],
            &["app#typecheck"],
        );
        assert!(skipped.is_empty());
    }

    #[test]
    fn an_executing_dependency_needs_its_own_dependencies() {
        // lib#build misses as a dependency of a selected hit: it executes, so
        // its dependency is restored, but the selected hit's other
        // dependency is not.
        let skipped = plan(
            &[
                ("app#test", hit()),
                ("lib#build", Expectation::Executes),
                ("util#build", hit()),
                ("config#build", hit()),
            ],
            &[
                ("app#test", "lib#build"),
                ("app#test", "config#build"),
                ("lib#build", "util#build"),
            ],
            &["app#test"],
        );
        assert_eq!(skipped, ["config#build"]);
    }
}
