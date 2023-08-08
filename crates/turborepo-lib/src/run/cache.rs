use turbopath::AbsoluteSystemPathBuf;
use turborepo_cache::AsyncCache;
use turborepo_ui::ColorSelector;

use crate::cli::OutputLogsMode;

struct RunCache {
    task_output_mode: OutputLogsMode,
    cache: AsyncCache,
    reads_disabled: bool,
    writes_disabled: bool,
    repo_root: AbsoluteSystemPathBuf,
    color_cache: ColorSelector,
}
