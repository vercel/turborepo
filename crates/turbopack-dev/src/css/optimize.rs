use turbo_tasks::Vc;
use turbopack_css::chunk::CssChunks;

#[turbo_tasks::function]
pub fn optimize_css_chunks(chunks: Vc<CssChunks>) -> Vc<CssChunks> {
    // TODO: We're in the middle of completely refactoring our chunking and
    // optimization logic. Rather than rewrite the algorithm twice (once to
    // handle the new interface using the old logic, and once with the new
    // interface and new logic), we're just not going to optimize for now.
    chunks
}
