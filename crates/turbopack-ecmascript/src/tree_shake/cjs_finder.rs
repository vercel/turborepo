use swc_core::ecma::{
    ast::*,
    visit::{noop_visit_type, Visit, VisitWith},
};

use crate::TURBOPACK_HELPER;

pub fn should_skip_tree_shaking(m: &Program) -> bool {
    if let Program::Module(m) = m {
        for item in m.body.iter() {
            match item {
                // Skip turbopack helpers.
                ModuleItem::ModuleDecl(ModuleDecl::Import(ImportDecl {
                    with: Some(with), ..
                })) => {
                    let with = with.as_import_with();
                    if let Some(with) = with {
                        for item in with.values.iter() {
                            if item.key.sym == *TURBOPACK_HELPER {
                                // Skip tree shaking if the import is from turbopack-helper
                                return true;
                            }
                        }
                    }
                }

                // We don't have logic to tree shake export * from
                ModuleItem::ModuleDecl(ModuleDecl::ExportAll(..)) => return true,

                // Tree shaking has a bug related to ModuleExportName::Str
                ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                    for s in import.specifiers.iter() {
                        if let ImportSpecifier::Named(is) = s {
                            if matches!(is.imported, Some(ModuleExportName::Str(..))) {
                                return true;
                            }
                        }
                    }
                }

                // Tree shaking has a bug related to ModuleExportName::Str
                ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(NamedExport {
                    src: Some(..),
                    specifiers,
                    ..
                })) => {
                    for s in specifiers {
                        if let ExportSpecifier::Named(es) = s {
                            if matches!(es.orig, ModuleExportName::Str(..))
                                || matches!(es.exported, Some(ModuleExportName::Str(..)))
                            {
                                return true;
                            }
                        }
                    }
                }

                // Turbopack has a bug related to top-level `let` declarations.
                // Tree shaking result is correct, but it seems like some steps after tree shaking
                // are not working correctly.
                ModuleItem::Stmt(Stmt::Decl(Decl::Var(box VarDecl {
                    kind: VarDeclKind::Let,
                    ..
                }))) => return true,

                // Skip sever actions
                ModuleItem::Stmt(Stmt::Expr(ExprStmt {
                    expr: box Expr::Lit(Lit::Str(Str { value, .. })),
                    ..
                })) => {
                    if value == "use server" {
                        return true;
                    }
                }

                _ => {}
            }
        }

        {
            let mut visitor = AbortFinder::default();
            m.visit_with(&mut visitor);
            if visitor.abort {
                return true;
            }
        }

        for item in m.body.iter() {
            if item.is_module_decl() {
                return false;
            }
        }
    }

    true
}

#[derive(Default)]
struct AbortFinder {
    abort: bool,
}

impl Visit for AbortFinder {
    noop_visit_type!();

    fn visit_expr_stmt(&mut self, e: &ExprStmt) {
        e.visit_children_with(self);

        if let Expr::Lit(Lit::Str(Str { value, .. })) = &*e.expr {
            if value == "use server" {
                self.abort = true;
            }
        }
    }
}
