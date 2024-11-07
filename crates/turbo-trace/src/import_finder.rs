use swc_common::{Span, Spanned};
use swc_ecma_ast::{Decl, ModuleDecl, Stmt};
use swc_ecma_visit::{Visit, VisitWith};

use crate::tracer::ImportType;

pub struct ImportFinder {
    import_type: ImportType,
    imports: Vec<(String, Span)>,
}

impl ImportFinder {
    pub fn new(import_type: ImportType) -> Self {
        Self {
            import_type,
            imports: Vec::new(),
        }
    }

    pub fn imports(&self) -> &[(String, Span)] {
        &self.imports
    }
}

impl Visit for ImportFinder {
    fn visit_module_decl(&mut self, decl: &ModuleDecl) {
        if let ModuleDecl::Import(import) = decl {
            match self.import_type {
                ImportType::All => {
                    self.imports
                        .push((import.src.value.to_string(), import.span));
                }
                ImportType::Types if import.type_only => {
                    self.imports
                        .push((import.src.value.to_string(), import.span));
                }
                ImportType::Values if !import.type_only => {
                    self.imports
                        .push((import.src.value.to_string(), import.span));
                }
                _ => {}
            }
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        if let Stmt::Decl(Decl::Var(var_decl)) = stmt {
            for decl in &var_decl.decls {
                if let Some(init) = &decl.init {
                    if let swc_ecma_ast::Expr::Call(call_expr) = &**init {
                        if let swc_ecma_ast::Callee::Expr(expr) = &call_expr.callee {
                            if let swc_ecma_ast::Expr::Ident(ident) = &**expr {
                                if ident.sym == *"require" {
                                    if let Some(arg) = call_expr.args.first() {
                                        if let swc_ecma_ast::Expr::Lit(swc_ecma_ast::Lit::Str(
                                            lit_str,
                                        )) = &*arg.expr
                                        {
                                            self.imports
                                                .push((lit_str.value.to_string(), expr.span()));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        stmt.visit_children_with(self);
    }
}
