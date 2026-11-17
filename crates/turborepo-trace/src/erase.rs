//! `affected`: erased-equal changes (`turborepo_trace_graph::barrel::erase`).
//!
//! [`erased_equal`] decides whether the base and head versions of a modified JS
//! or TS file produce the same runtime JavaScript, so that a test run by a
//! per-file transpiler cannot observe the change by loading the file. Both
//! versions are reduced to a [`Canonical`] form and compared; anything that
//! cannot be reduced counts as a change.
//!
//! # The canonical form
//!
//! 1. **Parse** with oxc (parser plus semantic syntax checks). A version with
//!    syntax or early errors, or Flow syntax, has no canonical form: a syntax
//!    error breaks every test that loads the file.
//! 2. **Erase** ([`Mode::Erased`]): run oxc's TypeScript transform, configured
//!    to do nothing but erase, and print the result without comments:
//!    - `only_remove_type_imports`: imports are kept unless they are `import
//!      type` (the `verbatimModuleSyntax` reading). Whether another transpiler
//!      also drops an import whose bindings are unused or only used as types
//!      depends on the value references to those bindings, and the printed code
//!      contains every value reference, so equal prints imply equal decisions
//!      under either setting;
//!    - class fields without initializer are kept (`x: number;` prints as
//!      `x;`): equal prints mean equal field lists, whatever
//!      `useDefineForClassFields` says;
//!    - no JSX transform, no downleveling, no enum inlining: JSX, enums (as
//!      their runtime object), namespaces with values and parameter properties
//!      all stay in the print.
//! 3. **Marks**: facts the print does not carry are compared next to it:
//!    - how each import or re-export without value specifiers was written
//!      (`import 'x'`, `import {} from 'x'`, `import {type A} from 'x'`):
//!      transpilers disagree on which of these load `x`;
//!    - the names bound by ambient declarations (`declare const x`, `declare
//!      module`, `declare global`): they emit nothing, but can shadow an import
//!      and so change which imports count as used;
//!    - `const` and `declare` on enums (inlined or not by some transpilers);
//!    - directive comments: JSX pragmas, `@jest-environment` and friends, Flow
//!      pragmas, coverage and bundler annotations, `sourceMappingURL`,
//!      triple-slash references. All other comments are trivia.
//! 4. **No erasure** ([`Mode::Raw`]) for files where types have runtime meaning
//!    that depends on configuration that is not read: any decorator
//!    (`emitDecoratorMetadata` / SWC `decoratorMetadata` emit the *types* of
//!    decorated members), `import x = require()`, `export =` and `export as
//!    namespace`. Such a file is only equal when both versions have the same
//!    tokens and the same line breaks between them (comments and blank lines
//!    may differ).
//! 5. **Declaration files** ([`Mode::Declaration`]): a `.d.ts` file has no
//!    runtime code at all; two versions that parse are equal.
//!
//! # What this assumes
//!
//! - The runner transpiles each file on its own
//!   (`turborepo_trace_graph::barrel::Runner`), and its transform does not
//!   derive runtime code from type annotations other than decorator metadata.
//! - Tests don't observe source positions or comments of other modules (line
//!   numbers in stack traces, `Function.prototype.toString()` of a function
//!   with comments).

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingIdentifier, Class, Decorator, ExportFromDeclaration, Function, ImportDeclaration,
    ImportDeclarationSpecifier, ImportOrExportKind, Program, TSEnumDeclaration, TSExportAssignment,
    TSExternalModuleDeclaration, TSGlobalDeclaration, TSImportEqualsDeclaration,
    TSInterfaceDeclaration, TSNamespaceDeclaration, TSNamespaceExportDeclaration,
    TSTypeAliasDeclaration, VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc_parser::Parser;
use oxc_semantic::{ScopeFlags, SemanticBuilder};
use oxc_span::SourceType;
use oxc_transformer::{JsxOptions, TransformOptions, Transformer, TypeScriptOptions};
use turborepo_trace_lang_js::{
    Dialect,
    lexer::{Lexer, TokKind},
};

/// How a version was reduced.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Types erased with the TypeScript transform.
    Erased,
    /// Tokens and line breaks (types have runtime meaning here).
    Raw,
    /// A declaration file: no runtime code.
    Declaration,
}

/// The runtime-relevant content of one version of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canonical {
    /// How it was reduced.
    pub mode: Mode,
    /// The canonical code.
    pub code: String,
    /// Facts the code does not carry (see the module documentation), in source
    /// order.
    pub marks: Vec<String>,
    /// Directive comments, in source order.
    pub directives: Vec<String>,
}

pub(crate) use turborepo_trace_lang_js::symbols::DIRECTIVES;

/// What the pre-transform scan collects.
#[derive(Default)]
struct Scan {
    /// A construct whose runtime meaning depends on types or configuration.
    hard: bool,
    marks: Vec<String>,
    /// Depth of ambient declarations being walked.
    ambient: u32,
}

impl Scan {
    fn in_ambient(&mut self, f: impl FnOnce(&mut Self)) {
        self.ambient += 1;
        f(self);
        self.ambient -= 1;
    }
}

impl<'a> Visit<'a> for Scan {
    fn visit_decorator(&mut self, _it: &Decorator<'a>) {
        self.hard = true;
    }

    fn visit_ts_import_equals_declaration(&mut self, _it: &TSImportEqualsDeclaration<'a>) {
        self.hard = true;
    }

    fn visit_ts_export_assignment(&mut self, _it: &TSExportAssignment<'a>) {
        self.hard = true;
    }

    fn visit_ts_namespace_export_declaration(&mut self, _it: &TSNamespaceExportDeclaration<'a>) {
        self.hard = true;
    }

    // Type declarations bind no values and contain nothing the scan looks for.
    fn visit_ts_interface_declaration(&mut self, _it: &TSInterfaceDeclaration<'a>) {}

    fn visit_ts_type_alias_declaration(&mut self, _it: &TSTypeAliasDeclaration<'a>) {}

    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        if self.ambient > 0 {
            self.marks.push(format!("ambient {}", it.name));
        }
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if it.declare {
            self.in_ambient(|s| walk::walk_variable_declaration(s, it));
        } else {
            walk::walk_variable_declaration(self, it);
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if it.declare {
            self.in_ambient(|s| walk::walk_function(s, it, flags));
        } else {
            walk::walk_function(self, it, flags);
        }
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        if it.declare {
            self.in_ambient(|s| walk::walk_class(s, it));
        } else {
            walk::walk_class(self, it);
        }
    }

    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        self.marks.push(format!(
            "enum{}{} {}",
            if it.r#const { " const" } else { "" },
            if it.declare { " declare" } else { "" },
            it.id.name
        ));
        if it.declare {
            self.in_ambient(|s| walk::walk_ts_enum_declaration(s, it));
        } else {
            walk::walk_ts_enum_declaration(self, it);
        }
    }

    fn visit_ts_namespace_declaration(&mut self, it: &TSNamespaceDeclaration<'a>) {
        if it.declare {
            self.in_ambient(|s| walk::walk_ts_namespace_declaration(s, it));
        } else {
            walk::walk_ts_namespace_declaration(self, it);
        }
    }

    fn visit_ts_external_module_declaration(&mut self, it: &TSExternalModuleDeclaration<'a>) {
        self.marks.push(format!("ambient module {:?}", it.id.value));
        self.in_ambient(|s| walk::walk_ts_external_module_declaration(s, it));
    }

    fn visit_ts_global_declaration(&mut self, it: &TSGlobalDeclaration<'a>) {
        self.in_ambient(|s| walk::walk_ts_global_declaration(s, it));
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        if it.import_kind == ImportOrExportKind::Type {
            return;
        }
        let form = match &it.specifiers {
            None => Some("bare"),
            Some(s) if s.is_empty() => Some("empty"),
            Some(s)
                if s.iter().all(|x| {
                    matches!(x, ImportDeclarationSpecifier::ImportSpecifier(i)
                        if i.import_kind == ImportOrExportKind::Type)
                }) =>
            {
                Some("types")
            }
            Some(_) => None,
        };
        if let Some(form) = form {
            self.marks
                .push(format!("import {form} {:?}", it.source.value));
        }
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        if it.export_kind == ImportOrExportKind::Type {
            return;
        }
        let form = if it.specifiers.is_empty() {
            Some("empty")
        } else if it
            .specifiers
            .iter()
            .all(|x| x.export_kind == ImportOrExportKind::Type)
        {
            Some("types")
        } else {
            None
        };
        if let Some(form) = form {
            self.marks
                .push(format!("export {form} {:?}", it.source.value));
        }
    }
}

/// Prints a program without comments.
fn print(program: &Program<'_>) -> String {
    Codegen::new()
        .with_options(CodegenOptions {
            comments: CommentOptions::disabled(),
            ..CodegenOptions::default()
        })
        .build(program)
        .code
}

/// The transform options that only erase types (see the module documentation).
fn erase_options() -> TransformOptions {
    TransformOptions {
        typescript: TypeScriptOptions {
            only_remove_type_imports: true,
            allow_namespaces: true,
            remove_class_fields_without_initializer: false,
            optimize_const_enums: false,
            optimize_enums: false,
            rewrite_import_extensions: None,
            ..TypeScriptOptions::default()
        },
        jsx: JsxOptions::disable(),
        ..TransformOptions::default()
    }
}

/// Whether `c` ends a line (ECMAScript line terminators).
fn is_line_end(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// The tokens of `src` with the line breaks between them, comments and other
/// whitespace dropped. `comments` are the comment spans (sorted). `None` when
/// the lexer fails or leaves bytes that are neither a token, a comment nor
/// whitespace.
fn raw_tokens(path: &str, src: &str, comments: &[(u32, u32)]) -> Option<String> {
    let bytes = src.as_bytes();
    let mut lx = Lexer::new(bytes, Dialect::from_path(Path::new(path)));
    let mut out = String::with_capacity(src.len());
    let mut pos = 0usize;
    let mut ci = 0usize;
    loop {
        let t = lx.next_token();
        let (start, end) = (t.start as usize, t.end as usize);
        if start < pos || end < start || end > bytes.len() {
            return None;
        }
        // The gap before the token: whitespace and comments only.
        let mut newline = false;
        let mut p = pos;
        while p < start {
            while ci < comments.len() && (comments[ci].1 as usize) <= p {
                ci += 1;
            }
            if let Some(&(cs, ce)) = comments.get(ci)
                && (cs as usize) <= p
            {
                let ce = (ce as usize).min(start);
                newline |= src.get(p..ce)?.chars().any(is_line_end);
                p = ce;
                continue;
            }
            let c = src.get(p..)?.chars().next()?;
            if !c.is_whitespace() && c != '\u{feff}' {
                return None;
            }
            newline |= is_line_end(c);
            p += c.len_utf8();
        }
        if t.kind == TokKind::Eof {
            break;
        }
        // A line break before the first token means nothing.
        out.push(if newline || out.is_empty() { '\n' } else { ' ' });
        out.push_str(src.get(start..end)?);
        pos = end;
    }
    if lx.take_error().is_some() || lx.jsx_failed() {
        return None;
    }
    Some(out)
}

/// The canonical form of one version of `path`, or why it has none.
pub fn canonical(path: &str, src: &str) -> Result<Canonical, &'static str> {
    let st = SourceType::from_path(path).map_err(|_| "not a JS or TS file")?;
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, st).parse();
    if ret.fatal_error || !ret.diagnostics.is_empty() || ret.is_flow_language {
        return Err("parse errors");
    }
    let mut program = ret.program;
    let mut scan = Scan::default();
    scan.visit_program(&program);
    let mut directives = Vec::new();
    let mut comments = Vec::with_capacity(program.comments.len());
    for c in &program.comments {
        comments.push((c.span.start, c.span.end));
        let text = src
            .get(c.span.start as usize..c.span.end as usize)
            .unwrap_or("");
        if DIRECTIVES.iter().any(|d| text.contains(d)) {
            directives.push(text.to_owned());
        }
    }
    let sem = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_excess_capacity(2.0)
        .with_enum_eval(true)
        .build(&program);
    if !sem.diagnostics.is_empty() {
        return Err("parse errors");
    }
    if st.is_typescript_definition() {
        return Ok(Canonical {
            mode: Mode::Declaration,
            code: String::new(),
            marks: Vec::new(),
            directives: Vec::new(),
        });
    }
    if scan.hard {
        let code = raw_tokens(path, src, &comments).ok_or("not tokenizable")?;
        return Ok(Canonical {
            mode: Mode::Raw,
            code,
            marks: Vec::new(),
            directives,
        });
    }
    let scoping = sem.semantic.into_scoping();
    let ret = Transformer::new(&alloc, Path::new(path), &erase_options())
        .build_with_scoping(scoping, &mut program);
    if !ret.diagnostics.is_empty() {
        return Err("not transformable");
    }
    Ok(Canonical {
        mode: Mode::Erased,
        code: print(&program),
        marks: scan.marks,
        directives,
    })
}

/// Whether the base and head versions of `path` have the same runtime
/// JavaScript; the error says why not (or why that cannot be shown).
pub fn erased_equal(path: &str, base: &str, head: &str) -> Result<(), String> {
    if base == head {
        return Ok(());
    }
    let h = canonical(path, head).map_err(|e| format!("head: {e}"))?;
    let b = canonical(path, base).map_err(|e| format!("base: {e}"))?;
    if h.mode != b.mode {
        return Err("types with runtime meaning (decorators, import =) at one revision".to_owned());
    }
    if h.directives != b.directives {
        return Err("directive comments differ".to_owned());
    }
    if h.code != b.code {
        return Err(match h.mode {
            Mode::Raw => "code differs (types not erased: decorators or import =)".to_owned(),
            _ => "runtime code differs".to_owned(),
        });
    }
    if h.marks != b.marks {
        return Err("ambient declarations, enum kinds or type-only import forms differ".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(path: &str, base: &str, head: &str) -> bool {
        erased_equal(path, base, head).is_ok()
    }

    fn why(path: &str, base: &str, head: &str) -> String {
        erased_equal(path, base, head).err().unwrap_or_default()
    }

    #[test]
    fn types_comments_and_formatting_are_erased() {
        // Annotations, interfaces, aliases, generics, assertions, modifiers.
        assert!(eq(
            "a.ts",
            "export function f(a: number): number { return a + 1; }\n",
            "export function f(a: string | number): any {\n  return a + 1;\n}\n"
        ));
        assert!(eq(
            "a.ts",
            "export interface A { x: number }\nexport const a = 1;\n",
            "export interface A { x: number; y: string }\nexport interface B {}\nexport type C = \
             A | B;\nexport const a = 1;\n"
        ));
        assert!(eq(
            "a.ts",
            "export const a = f(x);\n",
            "export const a = f<Foo>(x as unknown as Bar)!;\n"
        ));
        assert!(eq(
            "a.ts",
            "export class K { x = 1; m(a) { return a; } }\n",
            "export class K<T> implements I {\n  private readonly x: number = 1;\n  public m(a: \
             T): T { return a; }\n  n?(): void;\n}\n"
        ));
        assert!(eq(
            "a.ts",
            "export function f(a) { return a; }\n",
            "export function f(a: number): number;\nexport function f(a: string): string;\nexport \
             function f(this: void, a?: any) { return a; }\n"
        ));
        // Type-only imports and exports.
        assert!(eq(
            "a.ts",
            "import { b } from './b';\nexport const a = b;\n",
            "import type { T } from './t';\nimport { b, type U } from './b';\nexport type { T } \
             from './t';\nexport const a: T | U = b;\n"
        ));
        // Comments and whitespace.
        assert!(eq(
            "a.ts",
            "export const a = 1; // one\n",
            "/** The number one. */\nexport const a =\n    1;\n\n\n"
        ));
        assert!(eq(
            "a.js",
            "module.exports = { a: 1 }\n",
            "// c\nmodule.exports = {\n  a: 1,\n};\n"
        ));
        assert!(eq(
            "a.tsx",
            "export const C = (p) => <div a={p.a}>x</div>;\n",
            "export const C = (p: Props): JSX.Element => <div a={p.a}>x</div>;\n"
        ));
        // Ambient declarations with the same names.
        assert!(eq(
            "a.ts",
            "declare const x: number;\nexport const a = 1;\n",
            "declare const x: string;\nexport const a = 1;\n"
        ));
    }

    #[test]
    fn runtime_changes_are_changes() {
        assert_eq!(
            why("a.ts", "export const a = 1;\n", "export const a = 2;\n"),
            "runtime code differs"
        );
        assert!(!eq(
            "a.ts",
            "export const a = 1;\n",
            "export const a = 1;\nexport const b = 2;\n"
        ));
        assert!(!eq(
            "a.ts",
            "import { b } from './b';\nexport const a = 1;\n",
            "import { b } from './c';\nexport const a = 1;\n"
        ));
        // A string is not a comment, and JSX text is not whitespace.
        assert!(!eq(
            "a.ts",
            "export const a = 'x';\n",
            "export const a = 'x ';\n"
        ));
        assert!(!eq(
            "a.tsx",
            "export const C = () => <b>a b</b>;\n",
            "export const C = () => <b>a  b</b>;\n"
        ));
        // Statement order.
        assert!(!eq("a.ts", "f();\ng();\n", "g();\nf();\n"));
    }

    #[test]
    fn enums_namespaces_and_parameter_properties_count() {
        assert!(!eq(
            "a.ts",
            "export enum E { A = 1 }\n",
            "export enum E { A = 2 }\n"
        ));
        assert!(!eq(
            "a.ts",
            "export enum E { A }\n",
            "export enum E { A, B }\n"
        ));
        assert!(!eq(
            "a.ts",
            "export enum E { A }\n",
            "export const enum E { A }\n"
        ));
        assert!(!eq(
            "a.ts",
            "export const enum E { A = 1 }\n",
            "export const enum E { A = 2 }\n"
        ));
        assert!(!eq(
            "a.ts",
            "declare enum E { A = 1 }\nexport const a = 1;\n",
            "declare const enum E { A = 1 }\nexport const a = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "export namespace N { export const a = 1; }\n",
            "export namespace N { export const a = 2; }\n"
        ));
        // A namespace with types only has no runtime code.
        assert!(eq(
            "a.ts",
            "export namespace N { export type A = 1; }\nexport const a = 1;\n",
            "export namespace N { export type A = 2; }\nexport const a = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "export class K { constructor(x: number) {} }\n",
            "export class K { constructor(private x: number) {} }\n"
        ));
        // Visibility of a parameter property has no runtime meaning.
        assert!(eq(
            "a.ts",
            "export class K { constructor(public x: number) {} }\n",
            "export class K { constructor(private readonly x: string) {} }\n"
        ));
    }

    #[test]
    fn class_fields_count() {
        // `useDefineForClassFields`: a field without initializer defines a property.
        assert!(!eq(
            "a.ts",
            "export class K { }\n",
            "export class K { x: number; }\n"
        ));
        assert!(eq(
            "a.ts",
            "export class K { x: number; }\n",
            "export class K { x?: string; }\n"
        ));
        // `declare` fields emit nothing.
        assert!(eq(
            "a.ts",
            "export class K { }\n",
            "export class K { declare x: number; }\n"
        ));
        assert!(!eq(
            "a.ts",
            "export class K { x; }\n",
            "export class K { declare x: number; }\n"
        ));
    }

    #[test]
    fn decorators_keep_their_types() {
        // emitDecoratorMetadata: the parameter type is emitted.
        assert!(!eq(
            "a.ts",
            "@Injectable()\nexport class K { constructor(private a: A) {} }\n",
            "@Injectable()\nexport class K { constructor(private a: B) {} }\n"
        ));
        assert!(!eq(
            "a.ts",
            "export class K { @Prop() x: string; }\n",
            "export class K { @Prop() x: number; }\n"
        ));
        // Comments and blank lines are still trivia.
        assert!(eq(
            "a.ts",
            "@Injectable()\nexport class K { constructor(private a: A) {} }\n",
            "// The service.\n@Injectable()\n\nexport class K { constructor(private a: A) {} } // \
             end\n"
        ));
        // Adding a decorator, or a type next to one.
        assert!(!eq(
            "a.ts",
            "export class K { x: string; }\n",
            "export class K { @Prop() x: string; }\n"
        ));
        // Line breaks are kept (automatic semicolon insertion).
        assert!(!eq(
            "a.ts",
            "@D() export class K {}\nfunction f() { return 1 }\n",
            "@D() export class K {}\nfunction f() { return\n1 }\n"
        ));
    }

    #[test]
    fn import_equals_and_export_assignment_are_not_erased() {
        assert!(!eq(
            "a.ts",
            "import x = require('./x');\nexport const a: number = x.a;\n",
            "import x = require('./x');\nexport const a: string = x.a;\n"
        ));
        assert!(eq(
            "a.ts",
            "import x = require('./x');\nexport const a: number = x.a;\n",
            "// c\nimport x = require('./x');\n\nexport const a: number = x.a;\n"
        ));
        assert!(!eq(
            "a.ts",
            "const a: number = 1;\nexport = a;\n",
            "const a: any = 1;\nexport = a;\n"
        ));
    }

    #[test]
    fn imports_that_may_load_a_module_count() {
        // `import {type A}` loads './t' under verbatimModuleSyntax; `import type` never
        // does.
        assert!(!eq(
            "a.ts",
            "import type { A } from './t';\nexport const a = 1;\n",
            "import { type A } from './t';\nexport const a = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "import './t';\nexport const a = 1;\n",
            "import { type A } from './t';\nexport const a = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "import {} from './t';\nexport const a = 1;\n",
            "import './t';\nexport const a = 1;\n"
        ));
        assert!(eq(
            "a.ts",
            "import { type A } from './t';\nexport const a = 1;\n",
            "import { type A, type B } from './t';\nexport const a = 1;\n"
        ));
        // An import whose only use is a type is kept or elided by its value references,
        // which are the same at both revisions.
        assert!(eq(
            "a.ts",
            "import { A } from './t';\nexport const a = 1;\n",
            "import { A } from './t';\nexport const a: A = 1;\n"
        ));
        // A new import is a new module load (unless it is `import type`).
        assert!(!eq(
            "a.ts",
            "export const a = 1;\n",
            "import { A } from './t';\nexport const a: A = 1;\n"
        ));
        assert!(eq(
            "a.ts",
            "export const a = 1;\n",
            "import type { A } from './t';\nexport const a: A = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "export type { A } from './t';\nexport const a = 1;\n",
            "export { type A } from './t';\nexport const a = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "export const a = 1;\n",
            "export * from './t';\nexport const a = 1;\n"
        ));
        assert!(eq(
            "a.ts",
            "export const a = 1;\n",
            "export type * from './t';\nexport const a = 1;\n"
        ));
    }

    #[test]
    fn exported_type_names_count_when_they_disappear() {
        // `export { A }` of a local type is erased; of nothing, it is a syntax error.
        assert!(eq(
            "a.ts",
            "interface A { x: 1 }\nexport { A };\nexport const a = 1;\n",
            "type A = { x: 2 };\nexport { A };\nexport const a = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "interface A { x: 1 }\nexport { A };\nexport const a = 1;\n",
            "export { A };\nexport const a = 1;\n"
        ));
    }

    #[test]
    fn ambient_names_count() {
        assert!(!eq(
            "a.ts",
            "export const a = 1;\n",
            "declare const x: number;\nexport const a = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "declare module 'm' { export const x: number; }\nexport const a = 1;\n",
            "declare module 'm' { export const y: number; }\nexport const a = 1;\n"
        ));
        assert!(eq(
            "a.ts",
            "declare module 'm' { interface I { a: 1 } }\nexport const a = 1;\n",
            "declare module 'm' { interface I { a: 1; b: 2 } interface J {} }\nexport const a = \
             1;\n"
        ));
        assert!(eq(
            "a.ts",
            "declare global { interface Window { a: 1 } }\nexport const a = 1;\n",
            "declare global { interface Window { a: 1; b: 2 } }\nexport const a = 1;\n"
        ));
    }

    #[test]
    fn directive_comments_count() {
        assert!(!eq(
            "a.tsx",
            "export const C = () => <div/>;\n",
            "/** @jsx h */\nexport const C = () => <div/>;\n"
        ));
        assert!(!eq(
            "a.ts",
            "/* istanbul ignore next */\nexport const a = 1;\n",
            "export const a = 1;\n"
        ));
        assert!(!eq(
            "a.ts",
            "/**\n * @jest-environment node\n */\nexport const a = 1;\n",
            "/**\n * @jest-environment jsdom\n */\nexport const a = 1;\n"
        ));
        assert!(eq(
            "a.ts",
            "// eslint-disable-next-line no-console\nexport const a = 1;\n",
            "// @ts-expect-error because\nexport const a = 1;\n"
        ));
    }

    #[test]
    fn declaration_files_have_no_runtime_code() {
        assert!(eq(
            "a.d.ts",
            "export declare const a: number;\n",
            "export declare const a: string;\nexport declare function f(): void;\n"
        ));
        assert!(!eq(
            "a.d.ts",
            "export declare const a: number;\n",
            "export declare const a: ;\n"
        ));
    }

    #[test]
    fn broken_versions_are_changes() {
        assert_eq!(
            why("a.ts", "export const a = 1;\n", "export const a = ;\n"),
            "head: parse errors"
        );
        assert_eq!(
            why("a.ts", "export const a = ;\n", "export const a = 1;\n"),
            "base: parse errors"
        );
        // Early errors.
        assert!(!eq(
            "a.ts",
            "export function f() { let x = 1; }\n",
            "export function f() { let x = 1; let x: number = 1; }\n"
        ));
        assert!(!eq("a.json", "{}", "{ }"));
    }

    #[test]
    fn raw_tokens_cover_the_source() {
        let src = "a /* c */ b\n// d\n`x ${y} z` + 'q'\n";
        // Comment spans as the parser reports them.
        let comments = [(2, 9), (12, 16)];
        assert_eq!(
            raw_tokens("a.ts", src, &comments).as_deref(),
            Some("\na b\n`x ${ y } z` + 'q'")
        );
        // Bytes that are neither tokens, comments nor whitespace.
        assert_eq!(raw_tokens("a.ts", "a /* c */ b", &[]), None);
    }
}
