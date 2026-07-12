use std::collections::{HashMap, HashSet};

use miette::{NamedSource, SourceSpan};
use turborepo_errors::Spanned;
use turborepo_repository::{
    package_graph::{PackageGraph, PackageName, PackageNode},
    toolchain::ToolchainId,
};
use turborepo_task_id::{TaskId, TaskName};
use turborepo_turbo_json::{
    HasConfigBeyondExtends, ProcessedCommand, ProcessedTaskDefinition, RawTaskDefinition, TurboJson,
};
use turborepo_types::{TaskCommandOverride, TaskDefinition};

use super::EngineBuilder;
use crate::{
    BuilderError, CyclicExtends, MissingPackageTaskError, MissingRootTaskInTurboJsonError,
    MissingTurboJsonExtends, TaskDefinitionFromProcessed, TaskDefinitionResult, TurboJsonLoader,
};

/// Memo key for resolved task definitions: the turbo.json chain (by
/// address; loader-owned for the duration of a build), the task name, and
/// the two package-dependent inputs that survive resolution — path to the
/// repo root and whether the package's toolchain defines a command for the
/// task. See `task_definition_cached`.
#[derive(PartialEq, Eq, Hash)]
pub(super) struct TaskDefMemoKey {
    chain: Vec<usize>,
    task_name: TaskName<'static>,
    path_to_root: turbopath::RelativeUnixPathBuf,
    defines_task: bool,
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
        def_memo: &mut HashMap<TaskDefMemoKey, TaskDefinition>,
    ) -> Result<TaskDefinition, BuilderError> {
        let package_name = PackageName::from(task_id.package());
        let turbo_json_chain = match chain_cache.get(&package_name) {
            Some(cached) => cached.clone(),
            None => {
                let chain = self.turbo_json_chain(turbo_json_loader, &package_name)?;
                chain_cache.insert(package_name, chain.clone());
                chain
            }
        };

        let path_to_root = self.path_to_root(task_id.as_inner())?;
        let package_info = self
            .package_graph
            .package_info(&PackageName::from(task_id.as_inner().package()));
        let toolchain =
            package_info.and_then(|info| self.package_graph.toolchains().get(&info.toolchain));
        // Whether the package's toolchain defines a command for this task.
        // Tasks without one are phantom/transit tasks (they exist solely for
        // dependency ordering via `dependsOn: ["^task"]`) and must not hash
        // global input files — they don't execute, and including the files
        // would cause their hash to change and cascade into downstream
        // tasks that depend on them.
        let defines_task = package_info
            .zip(toolchain)
            .is_some_and(|(info, toolchain)| toolchain.defines_task(info, task_id.task()));

        // Most tasks resolve to an identical definition: the same turbo.json
        // chain and task name, differing only by the package's depth (for
        // `$TURBO_ROOT$`/global-input anchoring) and whether the package's
        // toolchain defines a command for the task. Memoize on exactly
        // those inputs. Two exceptions must skip the memo: a package-scoped
        // task key (`web#build`) in the chain, which `TurboJson::task`
        // consults first, and packages whose toolchain derives per-package
        // hash wiring (e.g. Cargo crate closures differ per crate).
        let memo_key = {
            let package_scoped = turbo_json_chain.iter().any(|turbo_json| {
                turbo_json
                    .tasks
                    .get(&task_id.as_inner().as_task_name())
                    .is_some()
            });
            let javascript =
                package_info.is_none_or(|info| info.toolchain == ToolchainId::JAVASCRIPT);
            (!package_scoped && javascript).then(|| TaskDefMemoKey {
                chain: turbo_json_chain
                    .iter()
                    .map(|turbo_json| *turbo_json as *const TurboJson as usize)
                    .collect(),
                task_name: task_name.clone().into_owned(),
                path_to_root: path_to_root.clone(),
                defines_task,
            })
        };
        if let Some(key) = &memo_key
            && let Some(cached) = def_memo.get(key)
        {
            return Ok(cached.clone());
        }

        let chain_definitions = Self::resolve_task_definitions_from_chain(
            turbo_json_chain,
            task_id,
            task_name,
            self.is_single,
            self.should_validate_engine,
        )?;

        // Resolve the task's `command` override across all five precedence
        // levels. Scoped commands (root `pkg#task` keys, Package
        // Configurations) always win; a package-authored definition (a
        // package.json script) shadows unscoped defaults; unscoped defaults
        // fan out per toolchain; and `None` leaves the toolchain's own
        // resolution (level 5) in charge.
        let mut scoped_command = None;
        let mut unscoped_command = None;
        for (definition, scoped) in &chain_definitions {
            if let Some(command) = &definition.command {
                if *scoped {
                    scoped_command = Some(command.clone());
                } else {
                    unscoped_command = Some(command.clone());
                }
            }
        }
        let command_override = resolve_command_override(
            scoped_command,
            unscoped_command,
            package_info.zip(toolchain),
            task_id.as_inner().task(),
        );

        let mut processed_task_definition = ProcessedTaskDefinition::from_iter(
            chain_definitions
                .into_iter()
                .map(|(definition, _)| definition),
        );
        // Toolchain defaults describe the command the toolchain synthesizes.
        // An override owns its behavior, including the generic cache default.
        if should_apply_toolchain_defaults(command_override.as_ref())
            && let Some((info, toolchain)) = package_info.zip(toolchain)
        {
            let defaults = toolchain.task_defaults(info, task_id.as_inner().task());
            if processed_task_definition.cache.is_none() {
                processed_task_definition.cache = defaults.cache.map(Spanned::new);
            }
        }
        let had_explicit_inputs = processed_task_definition.inputs.is_some();
        let mut task_def =
            TaskDefinition::from_processed(processed_task_definition, &path_to_root)?;
        task_def.command = command_override;

        if !self.future_flags.incremental_tasks {
            task_def.incremental = None;
        }

        // Whether this task will actually execute. A command override is
        // authoritative in both directions: an argv executes even where the
        // toolchain defines nothing, and an opt-out never executes even
        // where it does.
        let executes = match &task_def.command {
            Some(turborepo_types::TaskCommandOverride::Argv(_)) => true,
            Some(turborepo_types::TaskCommandOverride::OptOut) => false,
            None => defines_task,
        };

        if !self.global_deps.is_empty() && executes {
            crate::task_definition::prepend_global_inputs(
                &mut task_def.inputs,
                had_explicit_inputs,
                &self.global_deps,
                &path_to_root,
            );
        }

        // Apply toolchain-derived hash wiring: extra input globs and env
        // vars, cacheable outputs, and default-hashing behavior. For
        // JavaScript this is nothing (turbo.json is the whole story); for
        // Cargo it is the workspace files, crate closures, and deliverable
        // artifacts. `$TURBO_DEFAULT$` on a derived task means "everything
        // the toolchain derives automatically", so explicit `inputs` can
        // append without forfeiting automatic invalidation; explicit inputs
        // without `$TURBO_DEFAULT$` take full control.
        if inherits_toolchain_task_io(task_def.command.as_ref())
            && let Some((info, toolchain)) =
                package_info.zip(toolchain).filter(|(info, toolchain)| {
                    toolchain.derives_task_io(info, task_id.as_inner().task())
                })
        {
            let wants_automatic_inputs = !had_explicit_inputs || task_def.inputs.default;
            // Only assembled when the toolchain will actually use it:
            // `dependencies` walks the package's full transitive closure,
            // which is far too expensive to compute per task just to hand
            // to a toolchain that derives nothing (JavaScript).
            let dependencies: Vec<_> = self
                .package_graph
                .dependencies(&PackageNode::Workspace(PackageName::from(
                    task_id.as_inner().package(),
                )))
                .into_iter()
                .filter_map(|dep| match dep {
                    PackageNode::Workspace(name) => self.package_graph.package_info(name),
                    _ => None,
                })
                .collect();
            if let Some(derived) = toolchain.derived_task_io(
                info,
                task_id.as_inner().task(),
                path_to_root.as_str(),
                &dependencies,
                wants_automatic_inputs,
            ) {
                task_def.inputs.globs.extend(derived.input_globs);
                if let Some(default) = derived.package_default_inputs {
                    task_def.inputs.default = default;
                }
                for var in derived.env {
                    if !task_def.env.contains(&var) {
                        task_def.env.push(var);
                    }
                }
                task_def.env.sort();
                task_def.outputs.inclusions.extend(derived.output_globs);
            }
        }

        if let Some(key) = memo_key {
            def_memo.insert(key, task_def.clone());
        }

        Ok(task_def)
    }

    pub fn task_definition_chain(
        &self,
        turbo_json_loader: &L,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
    ) -> Result<Vec<ProcessedTaskDefinition>, BuilderError> {
        let package_name = PackageName::from(task_id.package());
        let turbo_json_chain = self.turbo_json_chain(turbo_json_loader, &package_name)?;
        Ok(Self::resolve_task_definitions_from_chain(
            turbo_json_chain,
            task_id,
            task_name,
            self.is_single,
            self.should_validate_engine,
        )?
        .into_iter()
        .map(|(definition, _)| definition)
        .collect())
    }

    /// Given a resolved turbo.json chain for a package, extract the task
    /// definitions for a specific task by walking the chain and handling
    /// `extends: false`.
    /// Resolve the chain into per-file processed definitions, each tagged
    /// with whether it came from a package-scoped position: a `pkg#task`
    /// key in the root turbo.json, or any entry in a Package Configuration
    /// (the file scopes it). The tag drives `command` precedence (see
    /// [`resolve_command_override`]).
    fn resolve_task_definitions_from_chain(
        turbo_json_chain: Vec<&TurboJson>,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
        is_single: bool,
        should_validate_engine: bool,
    ) -> Result<Vec<(ProcessedTaskDefinition, bool)>, BuilderError> {
        let root_used_scoped_key = |turbo_json: &TurboJson| {
            turbo_json
                .tasks
                .get(&task_id.as_inner().as_task_name())
                .is_some()
        };
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
                let scoped = index > 0 || root_used_scoped_key(turbo_json);
                task_definitions.push((local_def, scoped));
            }
            // Process any packages after this one (towards the leaf)
            for turbo_json in turbo_json_chain.iter().skip(index + 1) {
                if let Some(workspace_def) = turbo_json.task(task_id, task_name)? {
                    task_definitions.push((workspace_def, true));
                }
            }
            return Ok(task_definitions);
        }

        // Normal inheritance path
        let mut turbo_json_chain = turbo_json_chain.into_iter();

        if let Some(root_turbo_json) = turbo_json_chain.next()
            && let Some(root_definition) = root_turbo_json.task(task_id, task_name)?
        {
            let scoped = root_used_scoped_key(root_turbo_json);
            task_definitions.push((root_definition, scoped))
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
                task_definitions.push((workspace_def, true));
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

/// Resolve a task's `command` override across the five precedence levels
/// (highest to lowest):
///
/// 1. `command` in a Package Configuration
/// 2. `command` on a package-scoped root key (`web#test`)
/// 3. a package-authored native definition — a package.json script
/// 4. `command` on an unscoped root task (argv, or per-toolchain map fanned out
///    to this package's toolchain)
/// 5. the toolchain's synthesized command (Cargo verb tables)
///
/// Levels 1–2 arrive merged as `scoped_command` (most specific already
/// won); level 4 as `unscoped_command`. `None` means levels 3/5 are in
/// charge: the toolchain resolves the command as it always has.
fn resolve_command_override(
    scoped_command: Option<ProcessedCommand>,
    unscoped_command: Option<ProcessedCommand>,
    package_toolchain: Option<(
        &turborepo_repository::package_graph::PackageInfo,
        &dyn turborepo_repository::toolchain::Toolchain,
    )>,
    task: &str,
) -> Option<TaskCommandOverride> {
    // Levels 1–2: an explicit per-package command beats everything,
    // including the package's own script — the user targeted this package
    // by name.
    if let Some(command) = scoped_command {
        return match command {
            ProcessedCommand::OptOut(_) => Some(TaskCommandOverride::OptOut),
            ProcessedCommand::Argv(argv) => Some(TaskCommandOverride::Argv(argv.into_inner())),
            // The validator rejects the map form in scoped positions.
            ProcessedCommand::PerToolchain(_) => None,
        };
    }

    // Level 3: a package-authored definition shadows unscoped defaults —
    // lean into what the toolchain does natively. Toolchain-synthesized
    // fallbacks (Cargo verb tables) are authored by nobody and sit below
    // the defaults instead.
    if package_toolchain.is_some_and(|(info, toolchain)| toolchain.authors_task(info, task)) {
        return None;
    }

    // Level 4: unscoped defaults. The map form grants the task only to
    // packages of the listed toolchains.
    match unscoped_command? {
        ProcessedCommand::Argv(argv) => Some(TaskCommandOverride::Argv(argv.into_inner())),
        ProcessedCommand::PerToolchain(entries) => {
            let (info, _) = package_toolchain?;
            entries
                .into_inner()
                .into_iter()
                .find(|(toolchain_id, _)| info.toolchain.as_str() == toolchain_id.as_str())
                .map(|(_, argv)| TaskCommandOverride::Argv(argv))
        }
        // The validator rejects unscoped opt-outs.
        ProcessedCommand::OptOut(_) => None,
    }
}

fn should_apply_toolchain_defaults(command: Option<&TaskCommandOverride>) -> bool {
    command.is_none()
}

/// Native hash wiring describes a toolchain-synthesized command. An argv
/// override executes arbitrary user-selected work, so only turbo.json can
/// soundly describe its inputs, outputs, and environment.
fn inherits_toolchain_task_io(command: Option<&TaskCommandOverride>) -> bool {
    !matches!(command, Some(TaskCommandOverride::Argv(_)))
}

#[cfg(test)]
mod command_override_tests {
    use turborepo_errors::Spanned;
    use turborepo_repository::{
        package_graph::PackageInfo,
        toolchain::{Toolchain, ToolchainId},
    };
    use turborepo_types::TaskCommandOverride;

    use super::{ProcessedCommand, inherits_toolchain_task_io, resolve_command_override};

    /// A toolchain stub whose only knob is whether the package authors the
    /// task — the single toolchain property the resolver consults.
    struct Stub {
        id: ToolchainId,
        authors: bool,
    }

    impl Toolchain for Stub {
        fn id(&self) -> ToolchainId {
            self.id.clone()
        }

        fn discover_packages(&self) -> turborepo_repository::toolchain::DiscoverPackagesFuture<'_> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn authors_task(&self, _package: &PackageInfo, _task: &str) -> bool {
            self.authors
        }
    }

    fn argv(items: &[&str]) -> ProcessedCommand {
        ProcessedCommand::Argv(Spanned::new(items.iter().map(|s| s.to_string()).collect()))
    }

    fn per_toolchain(entries: &[(&str, &[&str])]) -> ProcessedCommand {
        ProcessedCommand::PerToolchain(Spanned::new(
            entries
                .iter()
                .map(|(id, items)| {
                    (
                        id.to_string(),
                        items.iter().map(|s| s.to_string()).collect(),
                    )
                })
                .collect(),
        ))
    }

    fn package(toolchain: &ToolchainId) -> PackageInfo {
        PackageInfo {
            toolchain: toolchain.clone(),
            ..Default::default()
        }
    }

    #[test]
    fn test_precedence_levels() {
        let rust = Stub {
            id: ToolchainId::RUST,
            authors: false,
        };
        let js_with_script = Stub {
            id: ToolchainId::JAVASCRIPT,
            authors: true,
        };
        let js_without_script = Stub {
            id: ToolchainId::JAVASCRIPT,
            authors: false,
        };
        let rust_pkg = package(&ToolchainId::RUST);
        let js_pkg = package(&ToolchainId::JAVASCRIPT);

        // Levels 1–2: a scoped command beats everything, including an
        // authored script and any unscoped default.
        assert_eq!(
            resolve_command_override(
                Some(argv(&["vitest"])),
                Some(argv(&["ignored"])),
                Some((&js_pkg, &js_with_script)),
                "test",
            ),
            Some(TaskCommandOverride::Argv(vec!["vitest".to_string()])),
        );
        // A scoped opt-out silences even an authored script.
        assert_eq!(
            resolve_command_override(
                Some(ProcessedCommand::OptOut(Spanned::new(()))),
                None,
                Some((&js_pkg, &js_with_script)),
                "test",
            ),
            Some(TaskCommandOverride::OptOut),
        );

        // Level 3: a package-authored definition shadows the unscoped
        // default…
        assert_eq!(
            resolve_command_override(
                None,
                Some(argv(&["from-default"])),
                Some((&js_pkg, &js_with_script)),
                "test",
            ),
            None,
        );
        // …but a script-less package takes the default (level 4).
        assert_eq!(
            resolve_command_override(
                None,
                Some(argv(&["from-default"])),
                Some((&js_pkg, &js_without_script)),
                "test",
            ),
            Some(TaskCommandOverride::Argv(vec!["from-default".to_string()])),
        );

        // Level 4 map form: fans out to the matching toolchain only. The
        // Cargo verb table (level 5) never shadows it — verb tables are
        // authored by nobody.
        let map = per_toolchain(&[("rust", &["cargo", "nextest", "run"])]);
        assert_eq!(
            resolve_command_override(None, Some(map.clone()), Some((&rust_pkg, &rust)), "test"),
            Some(TaskCommandOverride::Argv(vec![
                "cargo".to_string(),
                "nextest".to_string(),
                "run".to_string(),
            ])),
        );
        assert_eq!(
            resolve_command_override(None, Some(map), Some((&js_pkg, &js_without_script)), "test"),
            None,
            "a toolchain without a map key is untouched",
        );

        // Level 5: nothing configured → the toolchain resolves as usual.
        assert_eq!(
            resolve_command_override(None, None, Some((&rust_pkg, &rust)), "test"),
            None,
        );
    }

    #[test]
    fn only_native_commands_inherit_toolchain_defaults() {
        assert!(super::should_apply_toolchain_defaults(None));
        assert!(!super::should_apply_toolchain_defaults(Some(
            &TaskCommandOverride::Argv(vec!["node".to_string()])
        )));
        assert!(!super::should_apply_toolchain_defaults(Some(
            &TaskCommandOverride::OptOut
        )));
    }

    #[test]
    fn synthesized_commands_inherit_toolchain_task_io() {
        assert!(inherits_toolchain_task_io(None));
    }

    #[test]
    fn command_opt_out_preserves_toolchain_task_io() {
        assert!(inherits_toolchain_task_io(Some(
            &TaskCommandOverride::OptOut
        )));
    }

    #[test]
    fn argv_override_does_not_inherit_toolchain_task_io() {
        assert!(!inherits_toolchain_task_io(Some(
            &TaskCommandOverride::Argv(vec!["node".to_string(), "build.js".to_string(),])
        )));
    }
}
