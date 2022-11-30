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

#[turbo_tasks::value(shared)]
#[derive(Hash, Debug)]
pub struct ImportMetaRef {
    path: FileSystemPathVc,
    initialize: bool,
    ast_path: AstPathVc,
}

#[turbo_tasks::value_impl]
impl ImportMetaRefVc {
    #[turbo_tasks::function]
    pub fn new(path: FileSystemPathVc, initialize: bool, ast_path: AstPathVc) -> Self {
        ImportMetaRef {
            path,
            initialize,
            ast_path,
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for ImportMetaRef {
    #[turbo_tasks::function]
    async fn code_generation(&self, _context: ChunkingContextVc) -> Result<CodeGenerationVc> {
        // TODO: should only be done in ESM
        let ast_path = &self.ast_path.await?;
        let mut visitors = vec![create_visitor!(ast_path, visit_mut_expr(expr: &mut Expr) {
            let id = Ident::new(magic_identifier::encode("import.meta").into(), DUMMY_SP);
            *expr = Expr::Ident(id);
        })];

        // There can be many references to import.meta, and they appear at nesting in
        // the file. The first reference is responsible for injecting the
        // module-level variable to hold the mutable "meta object".
        if self.initialize {
            let path = as_abs_path(self.path).await?.as_str().map_or_else(
                || {
                    quote!(
                        "(() => { throw new Error('could not convert import.meta.url to filepath') \
                         })()" as Expr
                    )
                },
                |path| format!("file://{path}").into(),
            );

            visitors.push(create_visitor!(visit_mut_program(program: &mut Program) {
                let name = Ident::new(magic_identifier::encode("import.meta").into(), DUMMY_SP);
                let meta = quote!(
                    "const $name = { url: $path };" as Stmt,
                    name = name,
                    path: Expr = path.clone(),
                );
                insert_hoisted_stmt(program, meta);
            }));
        }

        Ok(CodeGeneration { visitors }.into())
    }
}
