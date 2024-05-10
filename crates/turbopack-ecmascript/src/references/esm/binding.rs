use anyhow::Result;
use swc_core::{
    common::{Span, SyntaxContext, DUMMY_SP},
    ecma::{
        ast::{
            ComputedPropName, Expr, Ident, KeyValueProp, Lit, MemberExpr, MemberProp, Number, Prop,
            PropName, SeqExpr, SimpleAssignTarget, Str,
        },
        utils::private_ident,
        visit::fields::{CalleeField, PropField},
    },
};
use turbo_tasks::Vc;
use turbopack_core::chunk::ChunkingContext;

use super::EsmAssetReference;
use crate::{
    code_gen::{CodeGenerateable, CodeGeneration},
    create_visitor,
    references::AstPath,
};

#[turbo_tasks::value(shared)]
#[derive(Hash, Debug)]
pub struct EsmBinding {
    pub reference: Vc<EsmAssetReference>,
    pub export: Option<String>,
    pub ast_path: Vc<AstPath>,
}

#[turbo_tasks::value_impl]
impl EsmBinding {
    #[turbo_tasks::function]
    pub fn new(
        reference: Vc<EsmAssetReference>,
        export: Option<String>,
        ast_path: Vc<AstPath>,
    ) -> Vc<Self> {
        EsmBinding {
            reference,
            export,
            ast_path,
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for EsmBinding {
    #[turbo_tasks::function]
    async fn code_generation(
        self: Vc<Self>,
        _context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<CodeGeneration>> {
        let this = self.await?;
        let mut visitors = Vec::new();
        let imported_module = this.reference.get_referenced_asset();

        fn make_expr(
            imported_module: &str,
            export: Option<&str>,
            span: Span,
            in_call: bool,
        ) -> Expr {
            let span = span.with_ctxt(SyntaxContext::empty());
            if let Some(export) = export {
                let mut expr = Expr::Member(MemberExpr {
                    span,
                    obj: Box::new(Expr::Ident(Ident::new(imported_module.into(), span))),
                    prop: MemberProp::Computed(ComputedPropName {
                        span,
                        expr: Box::new(Expr::Lit(Lit::Str(Str {
                            span,
                            value: export.into(),
                            raw: None,
                        }))),
                    }),
                });
                if in_call {
                    expr = Expr::Seq(SeqExpr {
                        exprs: vec![
                            Box::new(Expr::Lit(Lit::Num(Number {
                                span,
                                value: 0.0,
                                raw: None,
                            }))),
                            Box::new(expr),
                        ],
                        span,
                    });
                }
                expr
            } else {
                Expr::Ident(Ident::new(imported_module.into(), span))
            }
        }

        let mut ast_path = this.ast_path.await?.clone_value();
        let imported_module = imported_module.await?.get_ident().await?;

        let binding = match imported_module {
            Some(ident) => {
                let id = private_ident!(&*ident);
                let stmt = swc_core::ecma::ast::ModuleItem::Stmt(swc_core::ecma::ast::Stmt::Decl(
                    swc_core::ecma::ast::Decl::Var(Box::new(swc_core::ecma::ast::VarDecl {
                        span: DUMMY_SP,
                        declare: false,
                        kind: swc_core::ecma::ast::VarDeclKind::Const,
                        decls: vec![swc_core::ecma::ast::VarDeclarator {
                            span: DUMMY_SP,
                            name: id.clone().into(),
                            init: Some(Box::new(make_expr(
                                &ident,
                                this.export.as_deref(),
                                DUMMY_SP,
                                false,
                            ))),
                            definite: false,
                        }],
                    })),
                ));

                visitors.push(create_visitor!(visit_mut_program(p: &mut Program) {
                    if let swc_core::ecma::ast::Program::Module(m) = p {
                        m.body.insert(0, stmt.clone());
                    }
                }));

                Some(id)
            }
            _ => None,
        };

        loop {
            match ast_path.last() {
                // Shorthand properties get special treatment because we need to rewrite them to
                // normal key-value pairs.
                Some(swc_core::ecma::visit::AstParentKind::Prop(PropField::Shorthand)) => {
                    ast_path.pop();
                    visitors.push(
                        create_visitor!(exact ast_path, visit_mut_prop(prop: &mut Prop) {
                            if let Prop::Shorthand(ident) = prop {
                                // TODO: Merge with the above condition when https://rust-lang.github.io/rfcs/2497-if-let-chains.html lands.
                                if let Some(binding) = binding.as_ref() {
                                    *prop = Prop::KeyValue(KeyValueProp {
                                        key: PropName::Ident(ident.clone()),
                                        value: binding.clone().into(),
                                    });
                                }
                            }
                        }),
                    );
                    break;
                }
                // Any other expression can be replaced with the import accessor.
                Some(swc_core::ecma::visit::AstParentKind::Expr(_)) => {
                    ast_path.pop();

                    visitors.push(
                        create_visitor!(exact ast_path, visit_mut_expr(expr: &mut Expr) {
                            if let Some(ident) = binding.as_ref() {
                                *expr = Expr::Ident(ident.clone());
                            }
                            // If there's no identifier for the imported module,
                            // resolution failed and will insert code that throws
                            // before this expression is reached. Leave behind the original identifier.
                        }),
                    );
                    break;
                }
                Some(swc_core::ecma::visit::AstParentKind::BindingIdent(
                    swc_core::ecma::visit::fields::BindingIdentField::Id,
                )) => {
                    ast_path.pop();

                    // We need to handle LHS because of code like
                    // (function (RouteKind1){})(RouteKind || RouteKind = {})
                    if let Some(swc_core::ecma::visit::AstParentKind::SimpleAssignTarget(
                        swc_core::ecma::visit::fields::SimpleAssignTargetField::Ident,
                    )) = ast_path.last()
                    {
                        ast_path.pop();

                        dbg!(&ast_path);

                        visitors.push(
                            create_visitor!(exact ast_path, visit_mut_simple_assign_target(l: &mut SimpleAssignTarget) {
                                if let Some(ident) = binding.as_ref() {
                                    *l = SimpleAssignTarget::Ident(ident.clone().into());
                                }
                            }),
                        );
                        break;
                    }
                }
                Some(_) => {
                    ast_path.pop();
                }
                None => break,
            }
        }

        Ok(CodeGeneration { visitors }.into())
    }
}
