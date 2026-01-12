use swc_common::{Span, Spanned};
use swc_ecma_ast::{Decl, ModuleDecl, Stmt};
use swc_ecma_visit::{Visit, VisitWith};

use crate::tracer::ImportTraceType;

/// The type of import that we find.
///
/// Either an import with a `type` keyword (indicating that it is importing only
/// types) or an import without the `type` keyword (indicating that it is
/// importing values and possibly types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportType {
    Type,
    Value,
}

pub struct ImportFinder {
    import_type: ImportTraceType,
    imports: Vec<(String, Span, ImportType)>,
}

impl Default for ImportFinder {
    fn default() -> Self {
        Self::new(ImportTraceType::All)
    }
}

impl ImportFinder {
    pub fn new(import_type: ImportTraceType) -> Self {
        Self {
            import_type,
            imports: Vec::new(),
        }
    }

    pub fn imports(&self) -> &[(String, Span, ImportType)] {
        &self.imports
    }
}

impl Visit for ImportFinder {
    fn visit_module_decl(&mut self, decl: &ModuleDecl) {
        match decl {
            ModuleDecl::Import(import) => {
                let import_type = if import.type_only {
                    ImportType::Type
                } else {
                    ImportType::Value
                };
                match self.import_type {
                    ImportTraceType::All => {
                        self.imports
                            .push((import.src.value.to_string(), import.span, import_type));
                    }
                    ImportTraceType::Types if import.type_only => {
                        self.imports
                            .push((import.src.value.to_string(), import.span, import_type));
                    }
                    ImportTraceType::Values if !import.type_only => {
                        self.imports
                            .push((import.src.value.to_string(), import.span, import_type));
                    }
                    _ => {}
                }
            }
            ModuleDecl::ExportNamed(named_export) => {
                if let Some(decl) = &named_export.src {
                    self.imports
                        .push((decl.value.to_string(), decl.span, ImportType::Value));
                }
            }
            ModuleDecl::ExportAll(export_all) => {
                self.imports.push((
                    export_all.src.value.to_string(),
                    export_all.span,
                    ImportType::Value,
                ));
            }
            _ => {}
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        if let Stmt::Decl(Decl::Var(var_decl)) = stmt {
            for decl in &var_decl.decls {
                if let Some(init) = &decl.init
                    && let swc_ecma_ast::Expr::Call(call_expr) = &**init
                    && let swc_ecma_ast::Callee::Expr(expr) = &call_expr.callee
                    && let swc_ecma_ast::Expr::Ident(ident) = &**expr
                    && ident.sym == *"require"
                    && let Some(arg) = call_expr.args.first()
                    && let swc_ecma_ast::Expr::Lit(swc_ecma_ast::Lit::Str(lit_str)) = &*arg.expr
                {
                    self.imports
                        .push((lit_str.value.to_string(), expr.span(), ImportType::Value));
                }
            }
        }
        stmt.visit_children_with(self);
    }
}
