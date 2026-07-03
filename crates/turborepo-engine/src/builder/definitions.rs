use std::collections::{HashMap, HashSet};

use miette::{NamedSource, SourceSpan};
use turborepo_errors::Spanned;
use turborepo_repository::{
    cargo,
    package_graph::{PackageGraph, PackageName, PackageNode, PackageToolchain},
};
use turborepo_task_id::{TaskId, TaskName};
use turborepo_turbo_json::{
    HasConfigBeyondExtends, ProcessedTaskDefinition, RawTaskDefinition, TurboJson,
};
use turborepo_types::TaskDefinition;

use super::EngineBuilder;
use crate::{
    BuilderError, CyclicExtends, MissingPackageTaskError, MissingRootTaskInTurboJsonError,
    MissingTurboJsonExtends, TaskDefinitionFromProcessed, TaskDefinitionResult, TurboJsonLoader,
};

/// Input globs covering a Cargo crate's sources, with Turborepo's own task
/// log directory excluded. Explicit input globs hash the filesystem (unlike
/// default hashing, which is git-index based), so without the exclusion the
/// `.turbo/turbo-<task>.log` written by each run would invalidate the next
/// run's hash.
fn crate_source_globs(prefix: &str, crate_path: &str) -> [String; 2] {
    let base = if prefix.is_empty() {
        crate_path.to_string()
    } else {
        format!("{prefix}/{crate_path}")
    };
    [format!("{base}/**"), format!("!{base}/.turbo/**")]
}

impl<'a, L: TurboJsonLoader> EngineBuilder<'a, L> {
    // Helper methods used when building the engine
    /// Checks if there's a task definition somewhere in the repository
    pub fn has_task_definition_in_repo(
        loader: &L,
        package_graph: &PackageGraph,
        task_name: &TaskName<'static>,
    ) -> Result<bool, BuilderError> {
        for (package, _) in package_graph.packages() {
            let task_id = task_name
                .task_id()
                .unwrap_or_else(|| TaskId::new(package.as_str(), task_name.task()));
            if Self::has_task_definition_in_run(loader, package, task_name, &task_id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Checks if there's a task definition in the current run
    pub fn has_task_definition_in_run(
        loader: &L,
        workspace: &PackageName,
        task_name: &TaskName<'static>,
        task_id: &TaskId,
    ) -> Result<bool, BuilderError> {
        let result = Self::has_task_definition_in_run_inner(
            loader,
            workspace,
            task_name,
            task_id,
            &mut HashSet::new(),
        )?;
        Ok(result.has_definition())
    }

    fn has_task_definition_in_run_inner(
        loader: &L,
        workspace: &PackageName,
        task_name: &TaskName<'static>,
        task_id: &TaskId,
        visited: &mut HashSet<PackageName>,
    ) -> Result<TaskDefinitionResult, BuilderError> {
        // Avoid infinite loops from cyclic extends
        if visited.contains(workspace) {
            return Ok(TaskDefinitionResult::not_found());
        }
        visited.insert(workspace.clone());

        let turbo_json = loader.load(workspace).map_or_else(
            |err| {
                if err.is_no_turbo_json() && !matches!(workspace, PackageName::Root) {
                    Ok(None)
                } else {
                    Err(err)
                }
            },
            |turbo_json| Ok(Some(turbo_json)),
        )?;

        let Some(turbo_json) = turbo_json else {
            // If there was no turbo.json in the workspace, fallback to the root turbo.json
            return Self::has_task_definition_in_run_inner(
                loader,
                &PackageName::Root,
                task_name,
                task_id,
                visited,
            );
        };

        let task_id_as_name = task_id.as_task_name();

        // Helper to check task definition status based on extends configuration
        let check_task_def = |task_def: &RawTaskDefinition| -> TaskDefinitionResult {
            let has_extends_false = task_def
                .extends
                .as_ref()
                .map(|e| !*e.as_inner())
                .unwrap_or(false);

            if has_extends_false && !task_def.has_config_beyond_extends() {
                // Task is explicitly excluded via `extends: false` with no config
                TaskDefinitionResult::excluded()
            } else {
                // Task exists (either with `extends: false` + config, or normal definition)
                TaskDefinitionResult::found()
            }
        };

        // Check if this package's turbo.json has the task defined under various key
        // formats
        let base_task_name = TaskName::from(task_name.task());
        let check_base_task = matches!(workspace, PackageName::Root)
            || workspace == &PackageName::from(task_id.package());

        // Try task keys in order of specificity: task_id, task_name, base_task_name
        let task_def = turbo_json
            .tasks
            .get(&task_id_as_name)
            .or_else(|| turbo_json.tasks.get(task_name))
            .or_else(|| {
                if check_base_task {
                    turbo_json.tasks.get(&base_task_name)
                } else {
                    None
                }
            });

        if let Some(task_def) = task_def {
            return Ok(check_task_def(task_def));
        }

        // Check the extends chain for the task definition
        // Track if any package in the chain excluded this task
        for extend in turbo_json.extends.as_inner().iter() {
            let extend_package = PackageName::from(extend.as_str());
            let result = Self::has_task_definition_in_run_inner(
                loader,
                &extend_package,
                task_name,
                task_id,
                visited,
            )?;
            // If any package in the chain excluded this task, propagate that exclusion
            if result.is_excluded() {
                return Ok(TaskDefinitionResult::excluded());
            }
            if result.has_definition() {
                return Ok(TaskDefinitionResult::found());
            }
        }

        // This fallback only applies when there's no explicit `extends` field.
        // If `extends` is present (even if it only contains non-root packages),
        // we don't implicitly fall back to root since the validator ensures
        // the extends chain will eventually reach root.
        if turbo_json.extends.is_empty() && !matches!(workspace, PackageName::Root) {
            return Self::has_task_definition_in_run_inner(
                loader,
                &PackageName::Root,
                task_name,
                task_id,
                visited,
            );
        }

        Ok(TaskDefinitionResult::not_found())
    }

    /// Resolves the merged `TaskDefinition` for a task, caching the turbo.json
    /// chain per package. The chain only depends on the package name (not the
    /// task), so multiple tasks in the same package share the cached chain.
    pub(super) fn task_definition_cached<'b>(
        &self,
        turbo_json_loader: &'b L,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
        chain_cache: &mut HashMap<PackageName, Vec<&'b TurboJson>>,
    ) -> Result<TaskDefinition, BuilderError> {
        let processed_task_definition = ProcessedTaskDefinition::from_iter(
            self.task_definition_chain_cached(turbo_json_loader, task_id, task_name, chain_cache)?,
        );
        let had_explicit_inputs = processed_task_definition.inputs.is_some();
        let path_to_root = self.path_to_root(task_id.as_inner())?;
        let mut task_def =
            TaskDefinition::from_processed(processed_task_definition, &path_to_root)?;

        if !self.future_flags.incremental_tasks {
            task_def.incremental = None;
        }

        let task_id_inner = task_id.as_inner();
        let package_info = self
            .package_graph
            .package_info(&PackageName::from(task_id_inner.package()));

        let cargo_details = package_info
            .filter(|info| info.toolchain == PackageToolchain::Cargo)
            .and_then(|info| info.cargo.as_ref());
        // A Cargo task "has a script" when its task name maps to a Cargo
        // subcommand for the package's kind: `build`/`run` for entrypoint
        // crates, verification verbs for the workspace package, and nothing
        // for library crates (Cargo builds those implicitly as part of an
        // entrypoint's closure).
        let cargo_subcommand = cargo_details
            .and_then(|details| cargo::task_subcommand(details.kind, task_id_inner.task()));

        // Only prepend global inputs to tasks that actually execute (a
        // matching package.json script, or a Cargo verb for this package).
        // Phantom/transit tasks (packages without a matching script that
        // exist solely for dependency ordering via `dependsOn: ["^task"]`)
        // should not hash global input files — they don't execute, and
        // including the files would cause their hash to change and cascade
        // into downstream tasks that depend on them.
        let package_has_script = package_info
            .and_then(|info| info.package_json.scripts.get(task_id_inner.task()))
            .is_some_and(|script| !script.is_empty());

        if !self.global_deps.is_empty() && (package_has_script || cargo_subcommand.is_some()) {
            crate::task_definition::prepend_global_inputs(
                &mut task_def.inputs,
                had_explicit_inputs,
                &self.global_deps,
                &path_to_root,
            );
        }

        // Cargo tasks that will execute get hashing wiring: the workspace
        // lockfile/manifest, Cargo config, and pinned rust-toolchain files
        // are hashed (dependency, profile, or toolchain changes invalidate
        // the cache), along with the env vars that change what Cargo builds.
        // Explicit user `inputs` are respected.
        if let Some(details) = cargo_details
            && let Some(subcommand) = cargo_subcommand
        {
            let prefix = path_to_root.as_str();
            task_def
                .inputs
                .globs
                .extend(cargo::hash_input_globs(prefix));
            for var in cargo::HASHED_ENV_VARS {
                if !task_def.env.iter().any(|existing| existing == var) {
                    task_def.env.push(var.to_string());
                }
            }
            task_def.env.sort();

            // For Cargo packages, `$TURBO_DEFAULT$` means "everything turbo
            // hashes automatically" (own sources plus the flattened crate
            // closure), so users can append extra inputs — e.g. a file
            // embedded via `include_str!` from outside any crate directory —
            // without forfeiting automatic invalidation. Explicit inputs
            // without `$TURBO_DEFAULT$` take full control.
            let wants_automatic_inputs = !had_explicit_inputs || task_def.inputs.default;
            match details.kind {
                // An entrypoint build compiles its whole dependency closure
                // in one cargo process, so the closure's sources are
                // flattened into this task's inputs — invalidation must not
                // depend on users wiring up `dependsOn` between crates. The
                // crate's bin/cdylib/staticlib artifacts are the
                // deliverables and the only target/ contents worth caching;
                // Cargo's internal target/ state is its own incremental
                // cache and is left alone.
                cargo::CargoPackageKind::Entrypoint => {
                    if wants_automatic_inputs {
                        task_def.inputs.default = true;
                        task_def
                            .inputs
                            .globs
                            .extend(self.cargo_dependency_globs(task_id_inner.package(), prefix));
                    }
                    if subcommand == "build" {
                        task_def
                            .outputs
                            .inclusions
                            .extend(cargo::deliverable_output_globs(
                                prefix,
                                &details.deliverables,
                            ));
                    }
                }
                // The workspace package's directory is the repo root, so
                // default hashing would pull in the entire repository
                // (including JS packages). Hash the crate directories
                // instead — `$TURBO_DEFAULT$` resolves to those, not to the
                // repo root's default hash.
                cargo::CargoPackageKind::Workspace => {
                    if wants_automatic_inputs {
                        task_def.inputs.default = false;
                        task_def.inputs.globs.extend(self.cargo_crate_globs());
                    }
                }
                // Library tasks never map to a subcommand, so this branch is
                // unreachable while `cargo_subcommand` is `Some`.
                cargo::CargoPackageKind::Library => {}
            }
        }

        Ok(task_def)
    }

    /// Source globs for a Cargo entrypoint crate's transitive internal
    /// dependencies, relative to the entrypoint's directory via `prefix`.
    /// Cargo compiles these sources as part of the entrypoint's task, so
    /// they participate in its hash.
    fn cargo_dependency_globs(&self, package: &str, prefix: &str) -> Vec<String> {
        let node = PackageNode::Workspace(PackageName::from(package));
        let mut globs: Vec<String> =
            self.package_graph
                .dependencies(&node)
                .into_iter()
                .filter_map(|dep| match dep {
                    PackageNode::Workspace(name) => self.package_graph.package_info(name),
                    _ => None,
                })
                .filter(|info| {
                    info.toolchain == PackageToolchain::Cargo
                        && info.cargo.as_ref().is_some_and(|details| {
                            details.kind != cargo::CargoPackageKind::Workspace
                        })
                })
                .flat_map(|info| crate_source_globs(prefix, info.package_path().to_unix().as_str()))
                .collect();
        globs.sort();
        globs
    }

    /// Source globs for every Cargo crate, relative to the repo root (the
    /// synthetic workspace package's directory). Used to hash the workspace
    /// package's tasks without pulling the whole repository into the hash.
    fn cargo_crate_globs(&self) -> Vec<String> {
        let mut globs: Vec<String> = self
            .package_graph
            .packages()
            .filter(|(_, info)| {
                info.toolchain == PackageToolchain::Cargo
                    && info
                        .cargo
                        .as_ref()
                        .is_some_and(|details| details.kind != cargo::CargoPackageKind::Workspace)
            })
            .flat_map(|(_, info)| crate_source_globs("", info.package_path().to_unix().as_str()))
            .collect();
        globs.sort();
        globs
    }

    /// Like `task_definition_chain` but caches the turbo.json chain per
    /// package.
    fn task_definition_chain_cached<'b>(
        &self,
        turbo_json_loader: &'b L,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
        chain_cache: &mut HashMap<PackageName, Vec<&'b TurboJson>>,
    ) -> Result<Vec<ProcessedTaskDefinition>, BuilderError> {
        let package_name = PackageName::from(task_id.package());
        let turbo_json_chain = match chain_cache.get(&package_name) {
            Some(cached) => cached.clone(),
            None => {
                let chain = self.turbo_json_chain(turbo_json_loader, &package_name)?;
                chain_cache.insert(package_name, chain.clone());
                chain
            }
        };

        Self::resolve_task_definitions_from_chain(
            turbo_json_chain,
            task_id,
            task_name,
            self.is_single,
            self.should_validate_engine,
        )
    }

    pub fn task_definition_chain(
        &self,
        turbo_json_loader: &L,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
    ) -> Result<Vec<ProcessedTaskDefinition>, BuilderError> {
        let package_name = PackageName::from(task_id.package());
        let turbo_json_chain = self.turbo_json_chain(turbo_json_loader, &package_name)?;
        Self::resolve_task_definitions_from_chain(
            turbo_json_chain,
            task_id,
            task_name,
            self.is_single,
            self.should_validate_engine,
        )
    }

    /// Given a resolved turbo.json chain for a package, extract the task
    /// definitions for a specific task by walking the chain and handling
    /// `extends: false`.
    fn resolve_task_definitions_from_chain(
        turbo_json_chain: Vec<&TurboJson>,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
        is_single: bool,
        should_validate_engine: bool,
    ) -> Result<Vec<ProcessedTaskDefinition>, BuilderError> {
        let mut task_definitions = Vec::new();

        // Find the first package in the chain (iterating in reverse from leaf to root)
        // that has `extends: false` for this task. This stops inheritance from earlier
        // packages.
        let mut extends_false_index: Option<usize> = None;
        for (index, turbo_json) in turbo_json_chain.iter().enumerate().rev() {
            if let Some(task_def) = turbo_json.tasks.get(task_name)
                && task_def
                    .extends
                    .as_ref()
                    .map(|e| !*e.as_inner())
                    .unwrap_or(false)
            {
                // Found `extends: false` for this task in this package
                extends_false_index = Some(index);
                break;
            }
        }

        // If we found extends: false, only process from that point onwards
        if let Some(index) = extends_false_index {
            if let Some(turbo_json) = turbo_json_chain.get(index)
                && let Some(local_def) = turbo_json.task(task_id, task_name)?
                && local_def.has_config_beyond_extends()
            {
                task_definitions.push(local_def);
            }
            // Process any packages after this one (towards the leaf)
            for turbo_json in turbo_json_chain.iter().skip(index + 1) {
                if let Some(workspace_def) = turbo_json.task(task_id, task_name)? {
                    task_definitions.push(workspace_def);
                }
            }
            return Ok(task_definitions);
        }

        // Normal inheritance path
        let mut turbo_json_chain = turbo_json_chain.into_iter();

        if let Some(root_turbo_json) = turbo_json_chain.next()
            && let Some(root_definition) = root_turbo_json.task(task_id, task_name)?
        {
            task_definitions.push(root_definition)
        }

        if is_single {
            return match task_definitions.is_empty() {
                true => {
                    let (span, text) = task_id.span_and_text("turbo.json");
                    Err(BuilderError::MissingRootTaskInTurboJson(Box::new(
                        MissingRootTaskInTurboJsonError {
                            span,
                            text,
                            task_id: task_id.to_string(),
                        },
                    )))
                }
                false => Ok(task_definitions),
            };
        }

        for turbo_json in turbo_json_chain {
            if let Some(workspace_def) = turbo_json.task(task_id, task_name)? {
                task_definitions.push(workspace_def);
            }
        }

        if task_definitions.is_empty() && should_validate_engine {
            let (span, text) = task_id.span_and_text("turbo.json");
            return Err(BuilderError::MissingPackageTask(Box::new(
                MissingPackageTaskError {
                    span,
                    text,
                    task_id: task_id.to_string(),
                    task_name: task_name.to_string(),
                },
            )));
        }

        Ok(task_definitions)
    }

    // Provide the chain of turbo.json's to load to fully resolve all extends for a
    // package turbo.json.
    fn turbo_json_chain<'b>(
        &self,
        turbo_json_loader: &'b L,
        package_name: &PackageName,
    ) -> Result<Vec<&'b TurboJson>, BuilderError> {
        let validator = &self.validator;
        let mut turbo_jsons = Vec::with_capacity(2);

        enum ReadReq {
            // An inferred check we perform for each package to see if there is a package specific
            // turbo.json
            Infer(PackageName),
            // A specifically requested read from a package name being present in `extends`
            Request(Spanned<PackageName>),
        }

        impl ReadReq {
            fn package_name(&self) -> &PackageName {
                match self {
                    ReadReq::Infer(package_name) => package_name,
                    ReadReq::Request(package_name) => package_name.as_inner(),
                }
            }

            fn required(&self) -> Option<(Option<SourceSpan>, NamedSource<String>)> {
                match self {
                    ReadReq::Infer(_) => None,
                    ReadReq::Request(spanned) => Some(spanned.span_and_text("turbo.json")),
                }
            }
        }

        let mut read_stack = vec![(ReadReq::Infer(package_name.clone()), vec![])];
        let mut visited = std::collections::HashSet::new();

        while let Some((read_req, mut path)) = read_stack.pop() {
            let package_name = read_req.package_name();

            // Check for cycle by seeing if this package is already in the current path
            if let Some(cycle_index) = path.iter().position(|p: &PackageName| p == package_name) {
                // Found a cycle - build the cycle portion for error
                let mut cycle = path[cycle_index..]
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>();
                cycle.push(package_name.to_string());

                let (span, text) = read_req
                    .required()
                    .unwrap_or_else(|| (None, NamedSource::new("turbo.json", String::new())));

                return Err(BuilderError::CyclicExtends(Box::new(CyclicExtends {
                    cycle,
                    span,
                    text,
                })));
            }

            // Skip if we've already fully processed this package
            if visited.contains(package_name) {
                continue;
            }

            let turbo_json = turbo_json_loader
                .load(package_name)
                .map(Some)
                .or_else(|err| {
                    if let Some((span, text)) = read_req.required() {
                        if err.is_no_turbo_json() {
                            Err(BuilderError::MissingTurboJsonExtends(Box::new(
                                MissingTurboJsonExtends {
                                    package_name: read_req.package_name().to_string(),
                                    span,
                                    text,
                                },
                            )))
                        } else {
                            Err(err)
                        }
                    } else if err.is_no_turbo_json() {
                        Ok(None)
                    } else {
                        Err(err)
                    }
                })?;
            if let Some(turbo_json) = turbo_json {
                BuilderError::from_validation(
                    validator
                        .validate_turbo_json(package_name, turbo_json)
                        .into_iter()
                        .map(turborepo_config::Error::from)
                        .collect(),
                )?;
                turbo_jsons.push(turbo_json);
                visited.insert(package_name.clone());

                // Add current package to path for cycle detection
                path.push(package_name.clone());

                // Add the new turbo.json we are extending from
                let (extends, span) = turbo_json.extends.clone().split();
                for extend_package in extends {
                    let extend_package_name = PackageName::from(extend_package);
                    read_stack.push((
                        ReadReq::Request(span.clone().to(extend_package_name)),
                        path.clone(),
                    ));
                }
            } else if turbo_jsons.is_empty() {
                // If there is no package turbo.json extend from root by default
                read_stack.push((ReadReq::Infer(PackageName::Root), path));
            }
        }

        Ok(turbo_jsons.into_iter().rev().collect())
    }
}
