//! The opt-in task-aware closure for package-scoped prune.

use std::{cell::RefCell, collections::HashSet};

use turborepo_engine::{BuilderError, EngineBuilder, TurboJsonLoader};
use turborepo_microfrontends_config::{TurboJsonReader, UnifiedTurboJsonLoader};
use turborepo_repository::package_graph::{PackageGraph, PackageName, PruneDependencyMode};
use turborepo_turbo_json::TurboJson;

use super::{CommandBase, Error};
use crate::engine::EngineTurboJsonLoader;

/// Remember configuration owners consulted by the engine as well as task
/// owners: a package configuration can extend another package without depending
/// on any of its tasks. That configuration must still exist in the pruned
/// repository.
struct PruneLoader {
    loader: UnifiedTurboJsonLoader,
    config_owners: RefCell<HashSet<PackageName>>,
}

impl TurboJsonLoader for PruneLoader {
    fn load(&self, package: &PackageName) -> Result<&TurboJson, BuilderError> {
        let engine_loader = EngineTurboJsonLoader::new(&self.loader);
        let config = engine_loader.load(package)?;
        self.config_owners.borrow_mut().insert(package.clone());
        Ok(config)
    }
}

pub(super) fn retain_task_dependencies(
    base: &CommandBase,
    package_graph: &PackageGraph,
    mut retained: Vec<PackageName>,
    production: bool,
) -> Result<Vec<PackageName>, Error> {
    let future_flags = base.opts().future_flags;
    let loader = PruneLoader {
        loader: UnifiedTurboJsonLoader::workspace(
            TurboJsonReader::new(base.repo_root.clone()).with_future_flags(future_flags),
            base.opts().repo_opts.root_turbo_json_path.clone(),
            package_graph.package_scope_directories(),
        ),
        config_owners: RefCell::new(HashSet::new()),
    };
    let mode = if production {
        PruneDependencyMode::ProductionOnly
    } else {
        PruneDependencyMode::IncludeDevDependencies
    };

    loop {
        // Positional scopes are packages, not task selectors: conservatively
        // discover every task in each retained package, including inherited and
        // toolchain-registered tasks. Do not load unrelated package configs.
        // Root stays an implicit install seed, not a task entrypoint; root tasks
        // are traversed only when a retained package's tasks depend on them.
        let workspaces = retained
            .iter()
            .filter(|package| !matches!(package, PackageName::Root))
            .cloned()
            .collect();
        let engine = EngineBuilder::new(&base.repo_root, package_graph, &loader, false)
            .with_future_flags(future_flags)
            .with_workspaces(workspaces)
            .add_all_tasks()
            .build()?;

        let mut seeds: HashSet<_> = retained.iter().cloned().collect();
        seeds.extend(
            engine
                .task_ids()
                .map(|task| PackageName::from(task.package())),
        );
        seeds.extend(loader.config_owners.borrow().iter().cloned());
        let next = package_graph
            .prune_relationships()
            .package_closure(&seeds.into_iter().collect::<Vec<_>>(), mode)?;
        // The set only grows. Newly retained task/config owners need their
        // install dependencies (including required peers), whose tasks may in
        // turn require further packages. Keep task edges out of PackageGraph:
        // an acyclic task graph can project to a cyclic package graph.
        if next.len() == retained.len() {
            return Ok(next);
        }
        retained = next;
    }
}
