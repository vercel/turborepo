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
                                            self.imports.push((
                                                lit_str.value.to_string(),
                                                expr.span(),
                                                ImportType::Value,
                                            ));
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

#[cfg(test)]
mod tests {
    use swc_ecma_ast::Module;
    use swc_ecma_parser::{Syntax, TsSyntax};
    use swc_ecma_visit::VisitWith;

    use super::*;

    fn parse_module(code: &str) -> Module {
        let syntax = Syntax::Typescript(TsSyntax {
            tsx: false,
            decorators: true,
            ..Default::default()
        });

        let source_map = swc_common::SourceMap::default();
        let source_file = source_map.new_source_file(
            swc_common::FileName::Custom("test.ts".to_string()).into(),
            code.to_string(),
        );

        let mut parser = swc_ecma_parser::Parser::new_from(swc_ecma_parser::lexer::Lexer::new(
            syntax,
            swc_ecma_ast::EsVersion::EsNext,
            swc_common::input::StringInput::from(&*source_file),
            None,
        ));

        parser.parse_module().unwrap()
    }

    #[test]
    fn test_import_finder_types_only() {
        let code = r#"
            import type { Foo } from './foo';
            import { Bar } from './bar';
            import type { Baz } from './baz';
        "#;

        let module = parse_module(code);
        let mut finder = ImportFinder::new(ImportTraceType::Types);
        module.visit_with(&mut finder);

        let imports = finder.imports();
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].0, "./foo");
        assert_eq!(imports[0].2, ImportType::Type);
        assert_eq!(imports[1].0, "./baz");
        assert_eq!(imports[1].2, ImportType::Type);
    }

    #[test]
    fn test_import_finder_values_only() {
        let code = r#"
            import type { Foo } from './foo';
            import { Bar } from './bar';
            import type { Baz } from './baz';
        "#;

        let module = parse_module(code);
        let mut finder = ImportFinder::new(ImportTraceType::Values);
        module.visit_with(&mut finder);

        let imports = finder.imports();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].0, "./bar");
        assert_eq!(imports[0].2, ImportType::Value);
    }

    #[test]
    fn test_import_finder_export_named() {
        let code = r#"
            export { Foo } from './foo';
            export { Bar, Baz } from './bar';
        "#;

        let module = parse_module(code);
        let mut finder = ImportFinder::new(ImportTraceType::All);
        module.visit_with(&mut finder);

        let imports = finder.imports();
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].0, "./foo");
        assert_eq!(imports[0].2, ImportType::Value);
        assert_eq!(imports[1].0, "./bar");
        assert_eq!(imports[1].2, ImportType::Value);
    }

    #[test]
    fn test_import_finder_export_all() {
        let code = r#"
            export * from './foo';
            export * as namespace from './bar';
        "#;

        let module = parse_module(code);
        let mut finder = ImportFinder::new(ImportTraceType::All);
        module.visit_with(&mut finder);

        let imports = finder.imports();
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].0, "./foo");
        assert_eq!(imports[0].2, ImportType::Value);
        assert_eq!(imports[1].0, "./bar");
        assert_eq!(imports[1].2, ImportType::Value);
    }

    #[test]
    fn test_import_finder_require_statement() {
        let code = r#"
            const foo = require('./foo');
            const bar = require('./bar');
            const baz = require('./baz');
        "#;

        let module = parse_module(code);
        let mut finder = ImportFinder::new(ImportTraceType::All);
        module.visit_with(&mut finder);

        let imports = finder.imports();
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].0, "./foo");
        assert_eq!(imports[0].2, ImportType::Value);
        assert_eq!(imports[1].0, "./bar");
        assert_eq!(imports[1].2, ImportType::Value);
        assert_eq!(imports[2].0, "./baz");
        assert_eq!(imports[2].2, ImportType::Value);
    }
}
