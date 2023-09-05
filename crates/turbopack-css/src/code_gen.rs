use lightningcss::stylesheet::StyleSheet;
use turbo_tasks::Vc;
use turbopack_core::chunk::ChunkingContext;

use crate::{chunk::CssImport, references::AstParentKind};

/// impl of code generation inferred from a ModuleReference.
/// This is rust only and can't be implemented by non-rust plugins.
#[turbo_tasks::value(
    shared,
    serialization = "none",
    eq = "manual",
    into = "new",
    cell = "new"
)]
pub struct CodeGeneration {
    #[turbo_tasks(debug_ignore, trace_ignore)]
    pub imports: Vec<CssImport>,
}

#[turbo_tasks::value_trait]
pub trait CodeGenerateable {
    fn code_generation(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Vc<CodeGeneration>;
}

#[turbo_tasks::value(transparent)]
pub struct CodeGenerateables(Vec<Vc<Box<dyn CodeGenerateable>>>);

pub fn path_to(
    path: &[AstParentKind],
    f: impl FnMut(&AstParentKind) -> bool,
) -> Vec<AstParentKind> {
    if let Some(pos) = path.iter().rev().position(f) {
        let index = path.len() - pos - 1;
        path[..index].to_vec()
    } else {
        path.to_vec()
    }
}
