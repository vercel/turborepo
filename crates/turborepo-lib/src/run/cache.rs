use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_cache::AsyncCache;
use turborepo_ui::ColorSelector;

use crate::{cli::OutputLogsMode, opts::RunCacheOpts};

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
}
