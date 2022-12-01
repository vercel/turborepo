use anyhow::Result;
use swc_core::{
    common::DUMMY_SP,
    ecma::ast::{Expr, Ident},
    quote,
};
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::chunk::ChunkingContextVc;

use crate::{
    code_gen::{CodeGenerateable, CodeGenerateableVc, CodeGeneration, CodeGenerationVc},
    create_visitor, magic_identifier,
    references::{as_abs_path, esm::base::insert_hoisted_stmt, AstPathVc},
};

/// Responsible for initializing the `import.meta` object binding, so that it
/// may be referenced in th the file.
///
/// There can be many references to import.meta, and they appear at any nesting
/// in the file. But we must only initialize the binding a single time.
#[turbo_tasks::value(shared)]
#[derive(Hash, Debug)]
pub struct ImportMetaBinding {
    path: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl ImportMetaBindingVc {
    #[turbo_tasks::function]
    pub fn new(path: FileSystemPathVc) -> Self {
        ImportMetaBinding { path }.cell()
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for ImportMetaBinding {
    #[turbo_tasks::function]
    async fn code_generation(&self, _context: ChunkingContextVc) -> Result<CodeGenerationVc> {
        let path = as_abs_path(self.path).await?.as_str().map_or_else(
            || {
                quote!(
                    "(() => { throw new Error('could not convert import.meta.url to filepath') })()"
                        as Expr
                )
            },
            |path| format!("file://{path}").into(),
        );

        let visitor = create_visitor!(visit_mut_program(program: &mut Program) {
            let meta = quote!(
                "const $name = { url: $path };" as Stmt,
                name = meta_ident(),
                path: Expr = path.clone(),
            );
            insert_hoisted_stmt(program, meta);
        });

        Ok(CodeGeneration {
            visitors: vec![visitor],
        }
        .into())
    }
}

/// Handles rewriting `import.meta` references into the injected binding created
/// by ImportMetaBindi ImportMetaBinding.
///
/// There can be many references to import.meta, and they appear at any nesting
/// in the file. But all references refer to the same mutable object.
#[turbo_tasks::value(shared)]
#[derive(Hash, Debug)]
pub struct ImportMetaRef {
    ast_path: AstPathVc,
}

#[turbo_tasks::value_impl]
impl ImportMetaRefVc {
    #[turbo_tasks::function]
    pub fn new(ast_path: AstPathVc) -> Self {
        ImportMetaRef { ast_path }.cell()
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for ImportMetaRef {
    #[turbo_tasks::function]
    async fn code_generation(&self, _context: ChunkingContextVc) -> Result<CodeGenerationVc> {
        let ast_path = &self.ast_path.await?;
        let visitor = create_visitor!(ast_path, visit_mut_expr(expr: &mut Expr) {
            *expr = Expr::Ident(meta_ident());
        });

        Ok(CodeGeneration {
            visitors: vec![visitor],
        }
        .into())
    }
}

fn meta_ident() -> Ident {
    Ident::new(magic_identifier::encode("import.meta").into(), DUMMY_SP)
}
