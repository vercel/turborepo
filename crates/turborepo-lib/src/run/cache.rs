use std::rc::Rc;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_cache::AsyncCache;
use turborepo_ui::ColorSelector;

use crate::{
    cli::OutputLogsMode, opts::RunCacheOpts, package_task::PackageTask, task_graph::TaskOutputs,
};

struct RunCache {
    task_output_mode: Option<OutputLogsMode>,
    cache: AsyncCache,
    reads_disabled: bool,
    writes_disabled: bool,
    repo_root: AbsoluteSystemPathBuf,
    color_selector: ColorSelector,
}

impl RunCache {
    pub fn new(
        cache: AsyncCache,
        repo_root: &AbsoluteSystemPath,
        opts: RunCacheOpts,
        color_selector: ColorSelector,
    ) -> Self {
        RunCache {
            task_output_mode: opts.task_output_mode_override,
            cache,
            reads_disabled: opts.skip_reads,
            writes_disabled: opts.skip_writes,
            repo_root: repo_root.to_owned(),
            color_selector,
        }
    }

    pub fn task_cache(self: Rc<Self>, package_task: PackageTask, hash: &str) -> TaskCache {
        let log_file_name = self.repo_root.join_component(&package_task.log_file);
        let hashable_outputs = package_task.hashable_outputs();
        let mut repo_relative_globs = TaskOutputs {
            inclusions: Vec::with_capacity(hashable_outputs.inclusions.len()),
            exclusions: Vec::with_capacity(hashable_outputs.exclusions.len()),
        };

        for output in hashable_outputs.inclusions {
            let inclusion_glob = package_task.dir.join_component(&output);
            repo_relative_globs
                .inclusions
                .push(inclusion_glob.to_string());
        }

        for output in hashable_outputs.exclusions {
            let exclusion_glob = package_task.dir.join_component(&output);
            repo_relative_globs
                .exclusions
                .push(exclusion_glob.to_string());
        }

        let mut task_output_mode = package_task.task_definition.output_mode;
        if let Some(task_output_mode_override) = self.task_output_mode {
            task_output_mode = task_output_mode_override;
        }

        let caching_disabled = !package_task.task_definition.cache;

        TaskCache {
            expanded_outputs: Vec::new(),
            run_cache: self.clone(),
            repo_relative_globs,
            hash: hash.to_owned(),
            package_task,
            task_output_mode,
            caching_disabled,
            log_file_name,
        }
    }
}

struct TaskCache {
    expanded_outputs: Vec<AnchoredSystemPathBuf>,
    run_cache: Rc<RunCache>,
    repo_relative_globs: TaskOutputs,
    hash: String,
    package_task: PackageTask,
    task_output_mode: OutputLogsMode,
    caching_disabled: bool,
    log_file_name: AbsoluteSystemPathBuf,
}
