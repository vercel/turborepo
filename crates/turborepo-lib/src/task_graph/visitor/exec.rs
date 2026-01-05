//! Task execution context creation for Turborepo.
//!
//! This module provides the factory for creating task executors. The actual
//! execution logic is in `turborepo-task-executor`.

use std::sync::{Arc, Mutex};

use console::StyledObject;
use turborepo_engine::{TaskError, TaskErrorCollectorWrapper, TaskWarningCollectorWrapper};
use turborepo_env::{platform::PlatformEnv, EnvironmentVariableMap};
use turborepo_process::ProcessManager;
use turborepo_task_executor::{DryRunExecutor, TaskExecutor};
use turborepo_task_id::TaskId;

use super::{
    command::{CommandFactory, MicroFrontendProxyProvider, PackageGraphCommandProvider},
    Visitor,
};
use crate::{
    engine::Engine,
    run::{task_access::TaskAccess, TaskCache},
    task_hash::TaskHashTracker,
};

/// Type alias for the concrete TaskExecutor used in turborepo-lib.
pub type ExecContext = TaskExecutor<
    TaskHashTracker,
    TaskErrorCollectorWrapper,
    TaskWarningCollectorWrapper,
    TaskAccess,
>;

/// Type alias for the concrete DryRunExecutor used in turborepo-lib.
pub type DryRunExecContext = DryRunExecutor<TaskHashTracker>;

/// Factory for creating task execution contexts.
///
/// This struct wraps the visitor and provides methods to create TaskExecutor
/// and DryRunExecutor instances.
pub struct ExecContextFactory<'a> {
    visitor: &'a Visitor<'a>,
    errors: Arc<Mutex<Vec<TaskError>>>,
    manager: ProcessManager,
    #[allow(dead_code)]
    engine: &'a Arc<Engine>,
    command_factory: CommandFactory<'a>,
}

impl<'a> ExecContextFactory<'a> {
    pub fn new(
        visitor: &'a Visitor<'a>,
        errors: Arc<Mutex<Vec<TaskError>>>,
        manager: ProcessManager,
        engine: &'a Arc<Engine>,
    ) -> Result<Self, super::Error> {
        let pkg_graph_provider = PackageGraphCommandProvider::new(
            visitor.repo_root,
            &visitor.package_graph,
            visitor.run_opts.task_args(),
            visitor.micro_frontends_configs,
        );
        let mut command_factory = CommandFactory::new();
        if let Some(micro_frontends_configs) = visitor.micro_frontends_configs {
            command_factory.add_provider(MicroFrontendProxyProvider::new(
                visitor.repo_root,
                visitor.package_graph.as_ref(),
                engine.task_ids(),
                micro_frontends_configs,
            ));
        }
        command_factory.add_provider(pkg_graph_provider);

        Ok(Self {
            visitor,
            errors,
            manager,
            engine,
            command_factory,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn exec_context(
        &self,
        task_id: TaskId<'static>,
        task_hash: String,
        task_cache: TaskCache,
        mut execution_env: EnvironmentVariableMap,
        takes_input: bool,
        task_access: TaskAccess,
    ) -> Result<Option<ExecContext>, super::Error> {
        let task_id_for_display = self.visitor.display_task_id(&task_id);
        self.populate_env(&mut execution_env, &task_hash, &task_access);

        let Some(cmd) = self
            .command_factory
            .command(&task_id, execution_env.clone())?
        else {
            return Ok(None);
        };

        let pretty_prefix = self.prefix_with_color(&task_id);

        Ok(Some(TaskExecutor {
            task_id,
            task_id_for_display,
            task_hash,
            cmd,
            execution_env,
            manager: self.manager.clone(),
            takes_input,
            continue_on_error: self.visitor.run_opts.continue_on_error,
            ui_mode: self.visitor.run_opts.ui_mode,
            color_config: self.visitor.color_config,
            is_github_actions: self.visitor.run_opts.is_github_actions,
            pretty_prefix,
            task_cache,
            hash_tracker: self.visitor.task_hasher.task_hash_tracker(),
            errors: TaskErrorCollectorWrapper::from_arc(self.errors.clone()),
            warnings: TaskWarningCollectorWrapper::from_arc(self.visitor.warnings.clone()),
            task_access,
            platform_env: PlatformEnv::new(),
        }))
    }

    pub fn dry_run_exec_context(
        &self,
        task_id: TaskId<'static>,
        task_cache: TaskCache,
    ) -> DryRunExecContext {
        DryRunExecutor {
            task_id,
            task_cache,
            hash_tracker: self.visitor.task_hasher.task_hash_tracker(),
        }
    }

    /// Get a colored prefix for the task.
    fn prefix_with_color(&self, task_id: &TaskId) -> StyledObject<String> {
        let task_id_string = &task_id.to_string();
        self.visitor
            .color_cache
            .prefix_with_color(task_id_string, &self.visitor.prefix(task_id))
    }

    /// Add turbo-specific env vars to the task environment.
    fn populate_env(
        &self,
        execution_env: &mut EnvironmentVariableMap,
        task_hash: &str,
        task_access: &TaskAccess,
    ) {
        // Always last to make sure it overwrites any user configured env var.
        execution_env.insert("TURBO_HASH".to_owned(), task_hash.to_owned());

        // Allow downstream tools to detect if the task is being ran with TUI
        if self.visitor.run_opts.ui_mode.use_tui() {
            execution_env.insert("TURBO_IS_TUI".to_owned(), "true".to_owned());
        }

        // enable task access tracing
        // set the trace file env var - frameworks that support this can use it to
        // write out a trace file that we will use to automatically cache the task
        if task_access.is_enabled() {
            let (task_access_trace_key, trace_file) = task_access.get_env_var(task_hash);
            execution_env.insert(task_access_trace_key, trace_file.to_string());
        }
    }
}
