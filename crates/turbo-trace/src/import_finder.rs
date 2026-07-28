use oxc_ast::ast::{Argument, Expression, Statement};
use oxc_span::Span;
use oxc_syntax::module_record::ModuleRecord;

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

pub struct ImportResult {
    pub specifier: String,
    /// Span of the specifier string (for error reporting).
    pub span: Span,
    /// Span of the entire import/require statement (for comment detection).
    #[allow(dead_code)]
    pub statement_span: Span,
    #[allow(dead_code)]
    pub import_type: ImportType,
}

/// Extract imports from a parsed oxc module.
///
/// Uses `ModuleRecord` for ES import/export declarations (which covers
/// `import`, `export { } from`, and `export * from`), and manually walks
/// statements for CommonJS `require()` calls.
pub fn find_imports(
    module_record: &ModuleRecord,
    statements: &[Statement],
    import_trace_type: ImportTraceType,
) -> Vec<ImportResult> {
    let mut results = Vec::new();

    for (specifier, requested_modules) in &module_record.requested_modules {
        let specifier_str: String = specifier.to_string();
        for requested in requested_modules {
            let import_type = if requested.is_type {
                ImportType::Type
            } else {
                ImportType::Value
            };

            let should_include = match import_trace_type {
                ImportTraceType::All => true,
                ImportTraceType::Types => requested.is_type,
                ImportTraceType::Values => !requested.is_type,
            };

            if should_include {
                results.push(ImportResult {
                    specifier: specifier_str.clone(),
                    span: requested.span,
                    statement_span: requested.statement_span,
                    import_type,
                });
            }
        }
    }

    find_require_calls(statements, &mut results);

    results
}

/// Recursively walk statements looking for `const foo = require("./bar")`
/// patterns.
fn find_require_calls(statements: &[Statement], results: &mut Vec<ImportResult>) {
    for stmt in statements {
        match stmt {
            Statement::VariableDeclaration(var_decl) => {
                for decl in &var_decl.declarations {
                    let Some(init) = &decl.init else {
                        continue;
                    };
                    let Expression::CallExpression(call_expr) = init else {
                        continue;
                    };
                    let Expression::Identifier(callee) = &call_expr.callee else {
                        continue;
                    };
                    if callee.name != "require" {
                        continue;
                    }
                    let Some(first_arg) = call_expr.arguments.first() else {
                        continue;
                    };
                    let Argument::StringLiteral(lit) = first_arg else {
                        continue;
                    };
                    results.push(ImportResult {
                        specifier: lit.value.to_string(),
                        span: callee.span,
                        statement_span: var_decl.span,
                        import_type: ImportType::Value,
                    });
                }
            }
            Statement::BlockStatement(block) => {
                find_require_calls(&block.body, results);
            }
            Statement::IfStatement(if_stmt) => {
                find_require_calls(std::slice::from_ref(&if_stmt.consequent), results);
                if let Some(alt) = &if_stmt.alternate {
                    find_require_calls(std::slice::from_ref(alt), results);
                }
            }
            Statement::TryStatement(try_stmt) => {
                find_require_calls(&try_stmt.block.body, results);
                if let Some(handler) = &try_stmt.handler {
                    find_require_calls(&handler.body.body, results);
                }
                if let Some(finalizer) = &try_stmt.finalizer {
                    find_require_calls(&finalizer.body, results);
                }
            }
            Statement::ForStatement(for_stmt) => {
                find_require_calls(std::slice::from_ref(&for_stmt.body), results);
            }
            Statement::ForInStatement(for_in) => {
                find_require_calls(std::slice::from_ref(&for_in.body), results);
            }
            Statement::ForOfStatement(for_of) => {
                find_require_calls(std::slice::from_ref(&for_of.body), results);
            }
            Statement::WhileStatement(while_stmt) => {
                find_require_calls(std::slice::from_ref(&while_stmt.body), results);
            }
            Statement::DoWhileStatement(do_while) => {
                find_require_calls(std::slice::from_ref(&do_while.body), results);
            }
            Statement::SwitchStatement(switch) => {
                for case in &switch.cases {
                    find_require_calls(&case.consequent, results);
                }
            }
            Statement::LabeledStatement(labeled) => {
                find_require_calls(std::slice::from_ref(&labeled.body), results);
            }
            _ => {}
        }
    }
}
