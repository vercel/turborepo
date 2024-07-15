use anyhow::Result;
use turbo_tasks::Vc;

use crate::parse::ParseResult;

/// Designed after the renamer of esbuild.
///
/// This renamer renames non-top-level identifiers in parallel, and top-level
/// identifiers in series.

struct Renamer {}

async fn rename_module(module: Vc<ParseResult>) -> Result<Vc<ParseResult>> {
    match &*module.await? {
        ParseResult::Ok {
            program,
            comments,
            eval_context,
            globals,
            source_map,
        } => {}
        _ => module,
    }
}
