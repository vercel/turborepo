use std::collections::HashSet;

use turborepo_repository::package_graph::PackageName;
use turborepo_task_id::TaskName;
use turborepo_turbo_json::{HasConfigBeyondExtends, RawTaskDefinition, TurboJson};

use crate::{BuilderError, TurboJsonLoader};

/// Controls whether validation is performed during task inheritance resolution.
///
/// This enum replaces a boolean flag to make the code's intent clearer at call
/// sites. Validation checks that tasks referenced with `extends: false`
/// actually exist in the extends chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// Validate that `extends: false` references existing tasks.
    /// Used at the entry point of resolution.
    Validate,
    /// Skip validation. Used in recursive calls where validation
    /// has already been performed at the entry point.
    Skip,
}

/// Resolves task inheritance through the extends chain.
///
/// This struct encapsulates the logic for collecting tasks from a turbo.json
/// and its extends chain, handling task-level `extends: false` which can:
/// - Exclude a task entirely (when no other config is provided)
/// - Create a fresh task definition (when other config is provided)
///
/// Task exclusions propagate through the extends chain. If package B
/// excludes a task from package C, and package A extends B, then A will
/// not see that task from C (unless A explicitly re-adds it).
pub struct TaskInheritanceResolver<'a, L: TurboJsonLoader> {
    loader: &'a L,
    /// Controls validation of `extends: false` usage.
    /// Set to `Validate` at entry point, `Skip` in recursive calls.
    validation_mode: ValidationMode,
    implicit_tasks: HashSet<TaskName<'static>>,
}

/// Internal state for recursive resolution.
/// Separated from TaskInheritanceResolver to allow sharing the visited set
/// across the entire resolution without cloning.
struct ResolutionState {
    /// Tasks collected from the inheritance chain
    tasks: HashSet<TaskName<'static>>,
    /// Tasks that have been excluded via `extends: false`
    excluded_tasks: HashSet<TaskName<'static>>,
    /// Packages that have been visited to prevent infinite loops.
    /// This is shared across all recursive calls to avoid O(n²) cloning.
    visited: HashSet<PackageName>,
}

impl<'a, L: TurboJsonLoader> TaskInheritanceResolver<'a, L> {
    /// Creates a new resolver for collecting tasks from a workspace.
    pub fn new(loader: &'a L) -> Self {
        Self {
            loader,
            validation_mode: ValidationMode::Validate,
            implicit_tasks: HashSet::new(),
        }
    }

    pub fn with_implicit_tasks(
        mut self,
        tasks: impl IntoIterator<Item = TaskName<'static>>,
    ) -> Self {
        self.implicit_tasks.extend(tasks);
        self
    }

    /// Resolves all tasks from the given workspace and its extends chain.
    pub fn resolve(
        self,
        workspace: &PackageName,
    ) -> Result<HashSet<TaskName<'static>>, BuilderError> {
        let mut state = ResolutionState {
            tasks: self.implicit_tasks.clone(),
            excluded_tasks: HashSet::new(),
            visited: HashSet::new(),
        };
        self.collect_from_workspace(workspace, &mut state)?;
        Ok(state.tasks)
    }

    /// Internal recursive collection that tracks exclusions.
    /// Uses a shared mutable state to avoid cloning the visited set on each
    /// iteration.
    fn collect_from_workspace(
        &self,
        workspace: &PackageName,
        state: &mut ResolutionState,
    ) -> Result<(), BuilderError> {
        // Avoid infinite loops from cyclic extends
        if state.visited.contains(workspace) {
            return Ok(());
        }
        state.visited.insert(workspace.clone());

        let turbo_json = match self.loader.load(workspace) {
            Ok(json) => json,
            Err(err) if err.is_no_turbo_json() && !matches!(workspace, PackageName::Root) => {
                // If no turbo.json for this workspace, check root
                return self.collect_from_workspace(&PackageName::Root, state);
            }
            Err(err) => return Err(err),
        };

        // Collect inherited tasks from the extends chain
        let (inherited_tasks, chain_exclusions) =
            self.collect_from_extends_chain(turbo_json, state)?;

        // Process tasks from this turbo.json
        self.process_local_tasks(turbo_json, &inherited_tasks, state)?;

        // Add inherited tasks that aren't excluded
        Self::merge_inherited_tasks(inherited_tasks, &chain_exclusions, state);

        // Merge chain exclusions into our exclusions (they propagate up)
        state.excluded_tasks.extend(chain_exclusions);

        Ok(())
    }

    /// Collects tasks from the extends chain of a turbo.json.
    /// Uses the shared visited set from state to avoid O(n²) cloning for deep
    /// chains.
    fn collect_from_extends_chain(
        &self,
        turbo_json: &TurboJson,
        state: &mut ResolutionState,
    ) -> Result<(HashSet<TaskName<'static>>, HashSet<TaskName<'static>>), BuilderError> {
        let mut inherited_tasks = HashSet::new();
        let mut chain_exclusions = HashSet::new();

        for extend in turbo_json.extends.as_inner().iter() {
            let extend_package = PackageName::from(extend.as_str());

            // Skip if already visited (cycle detection without cloning)
            if state.visited.contains(&extend_package) {
                continue;
            }

            // Create a child resolver that skips validation (only validate at entry point)
            let child_resolver = TaskInheritanceResolver {
                loader: self.loader,
                validation_mode: ValidationMode::Skip,
                implicit_tasks: HashSet::new(),
            };

            // Use separate state for child to collect its tasks/exclusions,
            // but share the visited set to avoid cloning
            let mut child_state = ResolutionState {
                tasks: HashSet::new(),
                excluded_tasks: HashSet::new(),
                // Take ownership of visited temporarily to avoid cloning
                visited: std::mem::take(&mut state.visited),
            };

            child_resolver.collect_from_workspace(&extend_package, &mut child_state)?;

            // Restore visited set (now includes all packages visited by child)
            state.visited = child_state.visited;

            inherited_tasks.extend(child_state.tasks);
            chain_exclusions.extend(child_state.excluded_tasks);
        }

        // Fallback to root if no explicit extends and not already at root
        if turbo_json.extends.is_empty() && !state.visited.contains(&PackageName::Root) {
            let child_resolver = TaskInheritanceResolver {
                loader: self.loader,
                validation_mode: ValidationMode::Skip,
                implicit_tasks: HashSet::new(),
            };

            // Use separate state for child, sharing visited set
            let mut child_state = ResolutionState {
                tasks: HashSet::new(),
                excluded_tasks: HashSet::new(),
                visited: std::mem::take(&mut state.visited),
            };

            child_resolver.collect_from_workspace(&PackageName::Root, &mut child_state)?;

            // Restore visited set
            state.visited = child_state.visited;

            inherited_tasks.extend(child_state.tasks);
            chain_exclusions.extend(child_state.excluded_tasks);
        }

        Ok((inherited_tasks, chain_exclusions))
    }

    /// Processes tasks defined in the local turbo.json.
    fn process_local_tasks(
        &self,
        turbo_json: &TurboJson,
        inherited_tasks: &HashSet<TaskName<'static>>,
        state: &mut ResolutionState,
    ) -> Result<(), BuilderError> {
        for (task_name, task_def) in turbo_json.tasks.iter() {
            match task_def.extends.as_ref().map(|s| *s.as_inner()) {
                Some(false) => {
                    self.handle_excluded_task(
                        turbo_json,
                        task_name,
                        task_def,
                        inherited_tasks,
                        state,
                    )?;
                }
                _ => {
                    // Normal task or explicit `extends: true` - add it
                    state.tasks.insert(task_name.clone());
                }
            }
        }
        Ok(())
    }

    /// Handles a task with `extends: false`.
    fn handle_excluded_task(
        &self,
        turbo_json: &TurboJson,
        task_name: &TaskName<'static>,
        task_def: &RawTaskDefinition,
        inherited_tasks: &HashSet<TaskName<'static>>,
        state: &mut ResolutionState,
    ) -> Result<(), BuilderError> {
        // Validate that the task exists in the extends chain (only at entry point)
        if self.validation_mode == ValidationMode::Validate
            && !inherited_tasks.contains(task_name)
            && !self.implicit_tasks.contains(task_name)
        {
            let Some(extends) = task_def.extends.as_ref() else {
                return Ok(());
            };
            let (span, text) = extends.span_and_text("turbo.json");
            let extends_chain = Self::format_extends_chain(turbo_json, inherited_tasks);
            return Err(BuilderError::TurboJson(
                turborepo_turbo_json::Error::TaskNotInExtendsChain {
                    task_name: task_name.to_string(),
                    extends_chain,
                    span,
                    text,
                },
            ));
        }

        if task_def.has_config_beyond_extends() {
            // Has other config - this is a fresh definition, add it
            state.tasks.insert(task_name.clone());
        } else {
            state.tasks.remove(task_name);
        }
        // Track as excluded (propagates to parent packages)
        state.excluded_tasks.insert(task_name.clone());
        Ok(())
    }

    /// Merges inherited tasks that aren't excluded.
    fn merge_inherited_tasks(
        inherited_tasks: HashSet<TaskName<'static>>,
        chain_exclusions: &HashSet<TaskName<'static>>,
        state: &mut ResolutionState,
    ) {
        for task in inherited_tasks {
            if !state.excluded_tasks.contains(&task) && !chain_exclusions.contains(&task) {
                state.tasks.insert(task);
            }
        }
    }

    /// Formats the extends chain for error messages.
    fn format_extends_chain(
        turbo_json: &TurboJson,
        available_tasks: &HashSet<TaskName<'static>>,
    ) -> String {
        let mut result = String::new();
        result.push_str("The extends chain includes:\n");

        let extends = turbo_json.extends.as_inner();
        if extends.is_empty() {
            result.push_str("  → // (root)\n");
        } else {
            for extend in extends {
                result.push_str(&format!("  → {}\n", extend));
            }
        }

        result.push_str("\nTasks available from extends chain:\n");
        if available_tasks.is_empty() {
            result.push_str("  (none)\n");
        } else {
            let mut sorted_tasks: Vec<_> = available_tasks.iter().collect();
            sorted_tasks.sort();
            for task in sorted_tasks {
                result.push_str(&format!("  • {}\n", task));
            }
        }

        result
    }
}
