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
                    with, specifiers, ..
                })) => {
                    if let Some(with) = with.as_deref().and_then(|v| v.as_import_with()) {
                        for item in with.values.iter() {
                            if item.key.sym == *TURBOPACK_HELPER {
                                // Skip tree shaking if the import is from turbopack-helper
                                return true;
                            }
                        }
                    }

                    // Tree shaking has a bug related to ModuleExportName::Str
                    for s in specifiers.iter() {
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

                // Skip sever actions
                ModuleItem::Stmt(Stmt::Expr(ExprStmt {
                    expr: box Expr::Lit(Lit::Str(Str { value, .. })),
                    ..
                })) => {
                    if value == "use server" {
                        return true;
                    }
                }

                // Skip special reexports that are recognized by next.js
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl {
                    decl: Decl::Var(box VarDecl { decls, .. }),
                    ..
                })) => {
                    for decl in decls {
                        if let Pat::Ident(name) = &decl.name {
                            match &*name.sym {
                                "runtime" | "revalidate" => return true,

                                _ => {}
                            }
                        }
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
