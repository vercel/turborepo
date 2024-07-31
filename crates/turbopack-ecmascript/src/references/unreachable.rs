use anyhow::Result;
use swc_core::{
    common::{util::take::Take, Spanned},
    ecma::{
        ast::{
            ArrowExpr, BindingIdent, BlockStmt, ClassDecl, Decl, FnDecl, FnExpr, Pat, Stmt,
            VarDecl, VarDeclKind, VarDeclarator,
        },
        visit::{
            fields::{BlockStmtField, SwitchCaseField},
            AstParentKind, VisitMut,
        },
    },
    quote,
};
use turbo_tasks::Vc;
use turbopack_core::chunk::ChunkingContext;

use crate::{
    code_gen::{CodeGenerateable, CodeGeneration},
    create_visitor,
    utils::AstPathRange,
};

#[turbo_tasks::value]
pub struct Unreachable {
    range: Vc<AstPathRange>,
}

#[turbo_tasks::value_impl]
impl Unreachable {
    #[turbo_tasks::function]
    pub fn new(range: Vc<AstPathRange>) -> Vc<Self> {
        Self::cell(Unreachable { range })
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for Unreachable {
    #[turbo_tasks::function]
    async fn code_generation(
        &self,
        _context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<CodeGeneration>> {
        let range = self.range.await?;
        let visitors = match &*range {
            AstPathRange::Exact(path) => {
                [
                    // Unreachable might be used on Stmt or Expr
                    create_visitor!(exact path, visit_mut_expr(expr: &mut Expr) {
                        *expr = quote!("(\"TURBOPACK unreachable\", undefined)" as Expr);
                    }),
                    create_visitor!(exact path, visit_mut_stmt(stmt: &mut Stmt) {
                        // TODO(WEB-553) walk ast to find all `var` declarations and keep them
                        // since they hoist out of the scope
                        let mut replacement = Vec::new();
                        replacement.push(quote!("\"TURBOPACK unreachable\";" as Stmt));
                        ExtractDeclarations {
                            stmts: &mut replacement,
                        }.visit_mut_stmt(stmt);
                        *stmt = Stmt::Block(BlockStmt {
                            span: stmt.span(),
                            stmts: replacement,
                        });
                    }),
                ]
                .into()
            }
            AstPathRange::StartAfter(path) => {
                let mut parent = &path[..];
                while !parent.is_empty()
                    && !matches!(parent.last().unwrap(), AstParentKind::Stmt(_))
                {
                    parent = &parent[0..parent.len() - 1];
                }
                if !parent.is_empty() {
                    parent = &parent[0..parent.len() - 1];
                    fn replace(stmts: &mut Vec<Stmt>, start_index: usize) {
                        if stmts.len() > start_index + 1 {
                            let unreachable = stmts
                                .splice(
                                    start_index + 1..,
                                    [quote!("\"TURBOPACK unreachable\";" as Stmt)].into_iter(),
                                )
                                .collect::<Vec<_>>();
                            for mut stmt in unreachable {
                                ExtractDeclarations { stmts }.visit_mut_stmt(&mut stmt);
                            }
                        }
                    }
                    let (parent, [last]) = parent.split_at(parent.len() - 1) else {
                        unreachable!();
                    };
                    if let &AstParentKind::BlockStmt(BlockStmtField::Stmts(start_index)) = last {
                        [
                            create_visitor!(exact parent, visit_mut_block_stmt(block: &mut BlockStmt) {
                                replace(&mut block.stmts, start_index);
                            }),
                        ]
                        .into()
                    } else if let &AstParentKind::SwitchCase(SwitchCaseField::Cons(start_index)) =
                        last
                    {
                        [
                            create_visitor!(exact parent, visit_mut_switch_case(case: &mut SwitchCase) {
                                replace(&mut case.cons, start_index);
                            }),
                        ]
                        .into()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            }
        };

        Ok(CodeGeneration { visitors }.cell())
    }
}

struct ExtractDeclarations<'a> {
    stmts: &'a mut Vec<Stmt>,
}

impl<'a> VisitMut for ExtractDeclarations<'a> {
    fn visit_mut_var_decl(&mut self, decl: &mut VarDecl) {
        let VarDecl {
            span,
            kind,
            declare,
            decls,
        } = decl;
        let mut decls = decls.take();
        for decl in decls.iter_mut() {
            if matches!(kind, VarDeclKind::Const) {
                decl.init = Some(quote!("undefined" as Box<Expr>));
            } else {
                decl.init = None;
            }
        }
        self.stmts.push(Stmt::Decl(Decl::Var(Box::new(VarDecl {
            span: *span,
            kind: *kind,
            declare: *declare,
            decls,
        }))));
    }

    fn visit_mut_fn_decl(&mut self, decl: &mut FnDecl) {
        let FnDecl {
            declare,
            ident,
            function,
        } = decl;
        self.stmts.push(Stmt::Decl(Decl::Fn(FnDecl {
            declare: *declare,
            ident: ident.take(),
            function: function.take(),
        })));
    }

    fn visit_mut_fn_expr(&mut self, _: &mut FnExpr) {
        // Do not walk into function expressions
    }

    fn visit_mut_arrow_expr(&mut self, _: &mut ArrowExpr) {
        // Do not walk into arrow expressions
    }

    fn visit_mut_class_decl(&mut self, decl: &mut ClassDecl) {
        let ClassDecl { declare, ident, .. } = decl;
        self.stmts.push(Stmt::Decl(Decl::Var(Box::new(VarDecl {
            span: ident.span,
            declare: *declare,
            decls: vec![VarDeclarator {
                span: ident.span,
                name: Pat::Ident(BindingIdent {
                    type_ann: None,
                    id: ident.clone(),
                }),
                init: None,
                definite: false,
            }],
            kind: VarDeclKind::Let,
        }))));
    }
}
