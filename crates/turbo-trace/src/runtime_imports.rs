use std::collections::BTreeSet;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    CallExpression, Expression, IdentifierName, IdentifierReference, ImportExpression, JSXElement,
    JSXFragment, NewExpression, TSImportEqualsDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use turbopath::AbsoluteSystemPath;

use crate::{ImportTraceType, find_imports};

/// Written imports and syntax that can introduce additional runtime imports.
pub struct RuntimeImports {
    pub specifiers: BTreeSet<String>,
    pub uses_jsx: bool,
}

/// Collect runtime ESM imports without materializing a JSON copy of the AST.
/// An opaque loader makes the graph incomplete even if other imports are known.
pub fn parse_runtime_imports(
    file: &AbsoluteSystemPath,
    text: &str,
) -> Result<RuntimeImports, String> {
    parse_imports(file, text, ImportTraceType::Values)
}

/// Runtime-neutral tracing retains type imports conservatively and rejects
/// JSX, whose implicit runtime imports require a runtime-specific resolver.
pub fn parse_explicit_imports(
    file: &AbsoluteSystemPath,
    text: &str,
) -> Result<BTreeSet<String>, String> {
    let imports = parse_imports(file, text, ImportTraceType::All)?;
    if imports.uses_jsx {
        return Err("JSX requires resolution:bun or explicit inputs with trace:false".into());
    }
    Ok(imports.specifiers)
}

fn parse_imports(
    file: &AbsoluteSystemPath,
    text: &str,
    import_type: ImportTraceType,
) -> Result<RuntimeImports, String> {
    let source_type = SourceType::from_path(file.as_std_path()).map_err(|e| e.to_string())?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, text, source_type).parse();
    if parsed.panicked || !parsed.errors.is_empty() {
        return Err(parsed
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "));
    }
    if text.contains("@jsxImportSource") {
        return Err("custom JSX import sources require explicit inputs".into());
    }
    let imports = find_imports(&parsed.module_record, &parsed.program.body, import_type)
        .into_iter()
        .map(|import| import.specifier)
        .collect();
    let mut collector = RuntimeImportCollector {
        imports,
        opaque: false,
        uses_jsx: false,
    };
    collector.visit_program(&parsed.program);
    if collector.opaque {
        return Err(
            "dynamic import/require or eval is not statically traceable; declare complete \
             fallbackInputs or use trace:false"
                .into(),
        );
    }
    Ok(RuntimeImports {
        specifiers: collector.imports,
        uses_jsx: collector.uses_jsx,
    })
}

struct RuntimeImportCollector {
    imports: BTreeSet<String>,
    opaque: bool,
    uses_jsx: bool,
}

impl<'a> Visit<'a> for RuntimeImportCollector {
    fn visit_jsx_element(&mut self, element: &JSXElement<'a>) {
        self.uses_jsx = true;
        walk::walk_jsx_element(self, element);
    }

    fn visit_jsx_fragment(&mut self, fragment: &JSXFragment<'a>) {
        self.uses_jsx = true;
        walk::walk_jsx_fragment(self, fragment);
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        if let Expression::StringLiteral(literal) = &expression.source {
            self.imports.insert(literal.value.to_string());
        } else {
            self.opaque = true;
        }
        walk::walk_import_expression(self, expression);
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if matches!(identifier.name.as_str(), "require" | "eval") {
            self.opaque = true;
        }
    }

    fn visit_identifier_name(&mut self, identifier: &IdentifierName<'a>) {
        if matches!(identifier.name.as_str(), "require" | "createRequire") {
            self.opaque = true;
        }
    }

    fn visit_call_expression(&mut self, expression: &CallExpression<'a>) {
        if matches!(&expression.callee, Expression::Identifier(identifier) if identifier.name == "Function")
        {
            self.opaque = true;
        }
        walk::walk_call_expression(self, expression);
    }

    fn visit_new_expression(&mut self, expression: &NewExpression<'a>) {
        if matches!(&expression.callee, Expression::Identifier(identifier) if identifier.name == "Function")
        {
            self.opaque = true;
        }
        walk::walk_new_expression(self, expression);
    }

    fn visit_ts_import_equals_declaration(&mut self, _: &TSImportEqualsDeclaration<'a>) {
        self.opaque = true;
    }
}
