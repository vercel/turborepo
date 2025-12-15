//! Task inheritance resolution for turbo.json extends chains.
//!
//! This module handles the logic for collecting tasks from a turbo.json
//! and its extends chain, including support for task-level `extends: false`
//! which can exclude tasks from inheritance.

use std::collections::HashSet;

use turborepo_repository::package_graph::PackageName;
use turborepo_task_id::TaskName;

use crate::{
    config,
    turbo_json::{HasConfigBeyondExtends, RawTaskDefinition, TurboJson, TurboJsonLoader},
};

/// Error type for task inheritance resolution.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] config::Error),
}

impl From<Error> for super::builder::Error {
    fn from(err: Error) -> Self {
        match err {
            Error::Config(e) => super::builder::Error::Config(e),
        }
    }
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
pub struct TaskInheritanceResolver<'a> {
    loader: &'a TurboJsonLoader,
    /// Tasks collected from the inheritance chain
    tasks: HashSet<TaskName<'static>>,
    /// Tasks that have been excluded via `extends: false`
    excluded_tasks: HashSet<TaskName<'static>>,
    /// Packages that have been visited to prevent infinite loops
    visited: HashSet<PackageName>,
    /// Whether to validate `extends: false` usage (only at entry point)
    validate: bool,
}

impl<'a> TaskInheritanceResolver<'a> {
    /// Creates a new resolver for collecting tasks from a workspace.
    pub fn new(loader: &'a TurboJsonLoader) -> Self {
        Self {
            loader,
            tasks: HashSet::new(),
            excluded_tasks: HashSet::new(),
            visited: HashSet::new(),
            validate: true,
        }
    }

    /// Resolves all tasks from the given workspace and its extends chain.
    pub fn resolve(mut self, workspace: &PackageName) -> Result<HashSet<TaskName<'static>>, Error> {
        self.collect_from_workspace(workspace)?;
        Ok(self.tasks)
    }

    /// Internal recursive collection that tracks exclusions.
    fn collect_from_workspace(&mut self, workspace: &PackageName) -> Result<(), Error> {
        // Avoid infinite loops from cyclic extends
        if self.visited.contains(workspace) {
            return Ok(());
        }
        self.visited.insert(workspace.clone());

        let turbo_json = match self.loader.load(workspace) {
            Ok(json) => json,
            Err(config::Error::NoTurboJSON) if !matches!(workspace, PackageName::Root) => {
                // If no turbo.json for this workspace, check root
                return self.collect_from_workspace(&PackageName::Root);
            }
            Err(err) => return Err(err.into()),
        };

        // Collect inherited tasks from the extends chain
        let (inherited_tasks, chain_exclusions) = self.collect_from_extends_chain(&turbo_json)?;

        // Process tasks from this turbo.json
        self.process_local_tasks(&turbo_json, &inherited_tasks)?;

        // Add inherited tasks that aren't excluded
        self.merge_inherited_tasks(inherited_tasks, &chain_exclusions);

        // Merge chain exclusions into our exclusions (they propagate up)
        self.excluded_tasks.extend(chain_exclusions);

        Ok(())
    }

    /// Collects tasks from the extends chain of a turbo.json.
    fn collect_from_extends_chain(
        &mut self,
        turbo_json: &TurboJson,
    ) -> Result<(HashSet<TaskName<'static>>, HashSet<TaskName<'static>>), Error> {
        let mut inherited_tasks = HashSet::new();
        let mut chain_exclusions = HashSet::new();

        for extend in turbo_json.extends.as_inner().iter() {
            let extend_package = PackageName::from(extend.as_str());
            let mut child_resolver = TaskInheritanceResolver {
                loader: self.loader,
                tasks: HashSet::new(),
                excluded_tasks: HashSet::new(),
                visited: self.visited.clone(),
                validate: false, // don't validate in recursive calls
            };
            child_resolver.collect_from_workspace(&extend_package)?;
            inherited_tasks.extend(child_resolver.tasks);
            chain_exclusions.extend(child_resolver.excluded_tasks);
            self.visited = child_resolver.visited;
        }

        // Fallback to root if no explicit extends and not already at root
        if turbo_json.extends.is_empty() {
            // We need to check if we're processing a non-root workspace
            // This is determined by whether we have any extends configured
            let mut child_resolver = TaskInheritanceResolver {
                loader: self.loader,
                tasks: HashSet::new(),
                excluded_tasks: HashSet::new(),
                visited: self.visited.clone(),
                validate: false,
            };
            // The collect_from_workspace will handle the root fallback internally
            // We only need to explicitly extend from root if this turbo.json has no extends
            if !self.visited.contains(&PackageName::Root) {
                child_resolver.collect_from_workspace(&PackageName::Root)?;
                inherited_tasks.extend(child_resolver.tasks);
                chain_exclusions.extend(child_resolver.excluded_tasks);
                self.visited = child_resolver.visited;
            }
        }

        Ok((inherited_tasks, chain_exclusions))
    }

    /// Processes tasks defined in the local turbo.json.
    fn process_local_tasks(
        &mut self,
        turbo_json: &TurboJson,
        inherited_tasks: &HashSet<TaskName<'static>>,
    ) -> Result<(), Error> {
        for (task_name, task_def) in turbo_json.tasks.iter() {
            match task_def.extends.as_ref().map(|s| *s.as_inner()) {
                Some(false) => {
                    self.handle_excluded_task(turbo_json, task_name, task_def, inherited_tasks)?;
                }
                _ => {
                    // Normal task or explicit `extends: true` - add it
                    self.tasks.insert(task_name.clone());
                }
            }
        }
        Ok(())
    }

    /// Handles a task with `extends: false`.
    fn handle_excluded_task(
        &mut self,
        turbo_json: &TurboJson,
        task_name: &TaskName<'static>,
        task_def: &RawTaskDefinition,
        inherited_tasks: &HashSet<TaskName<'static>>,
    ) -> Result<(), Error> {
        // Validate that the task exists in the extends chain (only at entry point)
        if self.validate && !inherited_tasks.contains(task_name) {
            let (span, text) = task_def
                .extends
                .as_ref()
                .unwrap()
                .span_and_text("turbo.json");
            let extends_chain = Self::format_extends_chain(turbo_json, inherited_tasks);
            return Err(Error::Config(config::Error::TaskNotInExtendsChain {
                task_name: task_name.to_string(),
                extends_chain,
                span,
                text,
            }));
        }

        if task_def.has_config_beyond_extends() {
            // Has other config - this is a fresh definition, add it
            self.tasks.insert(task_name.clone());
        }
        // Track as excluded (propagates to parent packages)
        self.excluded_tasks.insert(task_name.clone());
        Ok(())
    }

    /// Merges inherited tasks that aren't excluded.
    fn merge_inherited_tasks(
        &mut self,
        inherited_tasks: HashSet<TaskName<'static>>,
        chain_exclusions: &HashSet<TaskName<'static>>,
    ) {
        for task in inherited_tasks {
            if !self.excluded_tasks.contains(&task) && !chain_exclusions.contains(&task) {
                self.tasks.insert(task);
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use crate::turbo_json::{RawPackageTurboJson, RawRootTurboJson, RawTurboJson, TurboJson};

    fn turbo_json(value: serde_json::Value) -> TurboJson {
        let is_package = value.as_object().unwrap().contains_key("extends");
        let json_text = serde_json::to_string(&value).unwrap();
        let raw: RawTurboJson = if is_package {
            RawPackageTurboJson::parse(&json_text, "").unwrap().into()
        } else {
            RawRootTurboJson::parse(&json_text, "").unwrap().into()
        };
        TurboJson::try_from(raw).unwrap()
    }

    #[test]
    fn test_resolve_root_tasks() {
        let turbo_jsons: HashMap<PackageName, TurboJson> = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "test": {},
                    "lint": {}
                }
            })),
        )]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let tasks = TaskInheritanceResolver::new(&loader)
            .resolve(&PackageName::Root)
            .unwrap();

        assert!(tasks.contains(&TaskName::from("build")));
        assert!(tasks.contains(&TaskName::from("test")));
        assert!(tasks.contains(&TaskName::from("lint")));
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn test_resolve_workspace_inherits_from_root() {
        let turbo_jsons: HashMap<PackageName, TurboJson> = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "test": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "custom": {}
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let tasks = TaskInheritanceResolver::new(&loader)
            .resolve(&PackageName::from("app"))
            .unwrap();

        assert!(tasks.contains(&TaskName::from("build")));
        assert!(tasks.contains(&TaskName::from("test")));
        assert!(tasks.contains(&TaskName::from("custom")));
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn test_task_extends_false_excludes_task() {
        let turbo_jsons: HashMap<PackageName, TurboJson> = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "lint": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let tasks = TaskInheritanceResolver::new(&loader)
            .resolve(&PackageName::from("app"))
            .unwrap();

        assert!(tasks.contains(&TaskName::from("build")));
        assert!(!tasks.contains(&TaskName::from("lint")));
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn test_task_extends_false_with_config_creates_fresh_task() {
        let turbo_jsons: HashMap<PackageName, TurboJson> = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "cache": true }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": {
                            "extends": false,
                            "cache": false
                        }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let tasks = TaskInheritanceResolver::new(&loader)
            .resolve(&PackageName::from("app"))
            .unwrap();

        // build should still be present as a fresh definition
        assert!(tasks.contains(&TaskName::from("build")));
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn test_exclusions_propagate_through_chain() {
        let turbo_jsons: HashMap<PackageName, TurboJson> = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-c"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-b"),
                turbo_json(json!({
                    "extends": ["//", "pkg-c"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
            (
                PackageName::from("pkg-a"),
                turbo_json(json!({
                    "extends": ["//", "pkg-b"],
                    "tasks": {}
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let tasks = TaskInheritanceResolver::new(&loader)
            .resolve(&PackageName::from("pkg-a"))
            .unwrap();

        // lint was excluded by pkg-b, so pkg-a should NOT see it
        assert!(tasks.contains(&TaskName::from("build")));
        assert!(!tasks.contains(&TaskName::from("lint")));
    }

    #[test]
    fn test_workspace_without_turbo_json_falls_back_to_root() {
        let turbo_jsons: HashMap<PackageName, TurboJson> = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "test": {}
                }
            })),
        )]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let tasks = TaskInheritanceResolver::new(&loader)
            .resolve(&PackageName::from("app-without-config"))
            .unwrap();

        // Should fall back to root and get root's tasks
        assert!(tasks.contains(&TaskName::from("build")));
        assert!(tasks.contains(&TaskName::from("test")));
    }

    #[test]
    fn test_error_on_extends_false_for_nonexistent_task() {
        let turbo_jsons: HashMap<PackageName, TurboJson> = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "nonexistent": { "extends": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let result = TaskInheritanceResolver::new(&loader).resolve(&PackageName::from("app"));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }
}
