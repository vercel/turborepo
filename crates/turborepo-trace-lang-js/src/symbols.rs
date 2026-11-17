//! Changed exports: definition-level facts of a JS or TS module, for `affected`
//! (`turborepo_trace_graph::barrel`).
//!
//! Graph edges are file to file, so a change anywhere in a module selects every
//! test that loads the module. This module looks inside a file: which top-level
//! statements define each export, which other top-level declarations and
//! imports those statements refer to, and which statements run code when the
//! module is loaded.
//!
//! # The rule
//!
//! **A change inside a module affects only the importers that use what
//! changed.** For a modified file `f` and an entry `T` (a jest or vitest test
//! whose runner transpiles each file on its own and does not type-check; a
//! route file or a Playwright spec that is compiled that way), `T` is not
//! selected through `f` when all of this holds:
//!
//! 1. **`f` is compared.** Both revisions of `f` are ES modules whose export
//!    names are known from the file itself, parse without syntax or early
//!    errors, and have facts (see "When a file counts as a whole"). They are
//!    assumed to transpile.
//! 2. **`f` does the same when it is loaded.** The statements that run code at
//!    load time ("Evaluated statements"), with everything they refer to, are
//!    the same at both revisions, and so are the directives, the modules the
//!    file loads, in order, and what its classes extend. Then the only thing
//!    that differs between the two revisions of `f` is the value of some of its
//!    exports: the *changed exports* ([`diff`]), the exports whose *definition
//!    closure* differs: the statements that declare the export, every top-level
//!    declaration they refer to (transitively, types included), and for every
//!    import they refer to the module and the name it comes from. Statements
//!    are compared as token sequences (comments and white space dropped, line
//!    breaks kept), with the source of their JSX byte for byte.
//! 3. **Nothing that `T` loads reads a changed export when it is loaded, and
//!    `T` reads none.** A file `s` that imports a changed name (directly, or
//!    through re-exports, which forward names) reads it either in a statement
//!    that runs when `s` is loaded: then every entry that loads `s` is
//!    selected; or only in definitions: then exactly the exports of `s` whose
//!    definition closure reads the name are changed in turn
//!    ([`Module::taint_members`], stored per file as [`relation`]), and the
//!    same question is asked of the importers of `s`. An entry is selected when
//!    a statement of it reads a changed name as a value.
//!
//! Why that is enough: a changed value can reach a test in two ways only.
//! Either code the test runs refers to it through a chain of static references
//! (tracked from export to export), or some module used it while it was loaded
//! (an evaluated statement read it: the walk ends there and every entry that
//! loads that module is selected). A binding written from inside a function is
//! covered by the first way: the function that writes has to be called by
//! something that refers to it. State outside the module graph (a
//! global, a registry, a prototype) is written by code that runs, which is
//! load-time code or a function someone refers to.
//!
//! # Conditions, construct by construct
//!
//! When a condition cannot be decided for a file, the file counts as a whole:
//! as a modified file it selects every entry that loads it, as an importer it
//! ends the walk.
//!
//! | Construct | In a modified file | In an importer |
//! |---|---|---|
//! | Load-time code and side effects | compared with its closure: a difference is a whole-file change (`--explain` names the statement) | a load-time statement that reads a changed name ends the walk |
//! | A call that only builds a value (`memo(C)`, `z.object({})`, a first-party closure factory) | not load-time code while the callee and the file that defines it are unchanged ("Calls that only build a value"); never an `async` function (its promise calls `then` of what it returns) | the same; the questions about the callee's file are kept with the file and asked when the walk gets there |
//! | A test-runner mock of the module that defines such a function, or of such a package | the mock sees the call and its arguments: every entry that loads a file with a mock call naming the module is selected | the same |
//! | A read of an import in a statement that is not load-time code (`const table = { a, b }`) | from an ES module of the repository it reads a binding; from a package or a CommonJS module it may run a getter: which of those a module reads when it loads is compared as load-time behaviour | reads the name |
//! | `export *` | the export names are not known from the file: whole | forwards every tracked name but `default` (a name two sources provide reaches both); the order of a module's exports is itself a tracked name that only a reader of the whole module reads |
//! | `export {a as b} from`, `export * as ns from` | a change of where a name comes from is a changed export | forwards `a` as `b`, the whole module as `ns`; a named re-export of a name that was added, removed or redeclared ends the walk (linking can fail) |
//! | Namespace import (`import * as ns`) | | reads the members it accesses (`ns.a`); used any other way (passed on, enumerated, spread, indexed by a computed key, `ns.f()`, JSX in the file) it reads every name, the order of the exports included |
//! | Default export of an object or a class | one definition: any change of a member changes `default` | reading `default` reads all of it |
//! | Object table or enum bound to a name (`export const T = {..}`, `export enum E`) | compared member by member when no member reads the table itself, there is no spread, accessor, computed key or `__proto__` | an import of the name is read where a changed member is read, or where the table is used as a whole (`T.f()` hands the table to `f` as `this`) |
//! | Getters and setters | defining one runs nothing; a member access at load time is load-time code (a getter may run) | the same |
//! | Class static blocks, evaluated static initializers, decorators | load-time code | load-time code |
//! | A class definition | reads what it extends: a change of the base (other than a class or plain function of the module) is a load-time change | a class that extends a changed value ends the walk |
//! | Mutation of an exported binding (`export let x`; a function that assigns) | an assignment at the top level is load-time code; inside a function it is part of that function's definition | the function that writes has to be read by someone: that reader is tracked |
//! | A name that is added, removed, or declared in another way | *rebound*: every import by name reads it at load time (linking fails under ES modules, a hoisted function may now be uninitialized) | ends the walk, also at a named re-export of it (`export { b } from './b'` fails every importer of the barrel once `./b` has no `b`) |
//! | A name the file newly asks of another module (a named import, a named re-export, the same name from another module) | has to be there where modules are linked: the target's own exports, its named re-exports and the modules it stars are read; a package is taken to provide it (not under `--strict`); else whole | |
//! | CommonJS (`module.exports`, `exports.x`, `exports[k]`, `require`) | not an ES module: whole | no facts: ends the walk (`require` with destructuring still says which names it takes, for the walk of the trunk's barrel rule) |
//! | JSX | load-time code at the top level; the markup is part of the statement's text | reads the factory: the names a pragma gives, `React`, and every import nothing else uses |
//! | Decorators | load-time code | with decorator metadata types are emitted as values: in a file with a decorator every reference counts as a value |
//! | `eval(..)` (direct) | no facts: whole | no facts: ends the walk |
//! | `new Function`, indirect `eval`, `globalThis[name]` | run in the global scope: they see no binding of the module; at the top level they are load-time code | the same |
//! | `this` in a plain function that an export hands out (the function, a constant that holds it, what a builder made of it) | reads the namespace it is called on: every export | when some file of the repository has such a function, a namespace import that is used as a whole or called through (`ns.f()`), and a `require`, read every name: the namespace may be a barrel's |
//! | Test-runner mock calls (`jest.mock`, `vi.mock`) | no facts: whole | no facts: ends the walk (mock-aware loading decides what such a test loads) |
//! | An import cycle through the file | a statement that reads an import at load time, and every call, is compared as load-time code (the import may be uninitialized) | |
//! | Types | a change of types only is no change of an export's value | a name used in types only is not read (except in a file with decorators) |
//! | A syntax or early error, Flow, `export =`, `import x = require()`, an import declared again | no facts: whole | no facts: ends the walk |
//!
//! A modified file without facts is not left to barrel pruning either: that
//! rule's own test of "free of side effects" does not see an import that a
//! transpiler drops or a decorator on a parameter. The JSX factory of a
//! configuration (no pragma) is found by name: `React`, `h`, `jsx`,
//! `createElement`, `Fragment` and the like, and every import nothing else in
//! the file uses; a factory of another name that the file also uses by
//! name is not seen as one.
//!
//! How this refines barrel pruning: that rule (`turborepo_trace_graph::barrel`,
//! steps 1 to 4) skips a modified file only when both revisions are free of
//! side effects at the top level, tracks all of its exports, and lets a change
//! through an importer only when the importer is free of side effects too,
//! through all of its exports. Changed exports tracks the exports that changed,
//! of any file whose load-time code did not, and lets an importer pass a change
//! on through the exports that read it, whatever else the importer does at load
//! time.
//!
//! # Assumptions, `--strict` and the switch
//!
//! - **Both revisions of a modified file transpile.** Syntax errors, early
//!   errors and an import that is declared again are found; what only one
//!   transpiler version rejects is not. Barrel pruning and erased-equal changes
//!   assume the same.
//! - **A package provides the names that are newly imported from it.** For a
//!   module of the repository this is read from its exports.
//! - **The builders of third-party packages** (the caller's table: React's
//!   `memo`, `forwardRef`, `lazy`, `createContext`, the schema builders of zod,
//!   `flag()` of the Flags SDK) only build a value from their arguments, as
//!   checked against the versions the table names. `--explain` names the
//!   builder when a load-time call was let through.
//!
//! Under `--strict` the table of package builders is empty (a load-time call
//! into a package is load-time code), and a file that newly imports a name from
//! a package counts as a whole. `TURBO_TRACE_DEBUG_EXPORTS=0` switches the rule
//! off (barrel pruning stays).
//!
//! # The two questions
//!
//! - [`diff`]: between the base and the head version of a modified file,
//!   **which exports changed**, and did **what the module does at load time**
//!   change?
//! - [`Module::taint_members`]: given imports of the head version that carry a
//!   changed value, **which exports of this file can pass the change on**, and
//!   does a statement that runs at load time read it? A statement passes the
//!   change on when it reads (as a value, not as a type) a tainted import or a
//!   top-level binding declared by a tainted statement. [`relation`] holds the
//!   answer per import binding, computed when the file is traced.
//!
//! # Evaluated statements
//!
//! A top-level statement is *evaluated* when loading the module runs code in it
//! that is more than building a value from literals, functions and bindings:
//! any call, `new`, member access (a getter may run), tagged template, `await`,
//! assignment, spread, destructuring, operator on a non-primitive, JSX, a class
//! with static initializers that are themselves evaluated, static blocks or
//! decorators, a namespace with values, and every control-flow statement.
//! Function, class and type declarations, imports, re-exports and `const x =
//! <literal | function | object or array of such | another binding>` are not
//! evaluated: they only declare.
//!
//! A statement that can throw is evaluated as well, because a module that
//! throws while it loads fails every file that loads it, whatever they use: a
//! read of a global that may not exist, a read of a `let`, `const` or class
//! binding declared further down, `new Map` of entries that are not pairs, a
//! class that extends a global that is not a constructor everywhere. Both
//! versions are assumed to transpile; an import that the module declares
//! again is found here ([`import_redeclared`]).
//!
//! ## What an unevaluated statement still does at load time
//!
//! - **A class definition reads what it extends** and throws when that is not a
//!   constructor (`Stmt::extends`). In [`diff`] the bases that are not a class
//!   or a plain function of the module are compared; in [`Module::taint`] a
//!   class that extends a tainted value is a load-time read.
//! - **A transpiler drops an import that no value reads**, and the module is
//!   then not loaded: [`diff`] compares the imports that are surely kept
//!   (`Module::loads`).
//! - **The order of the exports** is the order of the keys of the module object
//!   under a CommonJS transpilation ([`Changed::reordered`]).
//! - **A binding that was added, removed or redeclared** ([`Changed::rebound`])
//!   is read by every import that names it: linking fails under ES module
//!   semantics, and a binding that was a hoisted function may now be
//!   uninitialized.
//! - **On an import cycle** a module can run before what it imports: with
//!   `cyclic`, [`diff`] compares every statement that reads an import at load
//!   time, and every call, as load-time code.
//!
//! Three kinds of reads have no expression that names what is read: `eval` (no
//! facts), JSX (the factory: the bindings a pragma names, `React`, and imports
//! nothing else uses), and `this` in an exported plain function (the namespace
//! the function is called on: every export).
//!
//! ## Calls that only build a value
//!
//! `export const GET = withTrace(handler)`, `export default memo(Component)`,
//! `export const schema = z.object({ id: z.string() }).optional()` and
//! `export const enableX = declareFlag({ key: 'x' })` are calls, but all they
//! do at load time is build a value. A top-level chain of calls and member
//! accesses on a top-level binding, whose arguments are not evaluated
//! themselves, is *conditionally* evaluated ([`Cond`]): it stops counting as
//! load-time code when the binding is
//!
//! - a function of the same file, or an import of a first-party function, whose
//!   body only declares constants from unevaluated expressions and returns one:
//!   a closure factory, a higher-order component, a declarator that hands its
//!   argument to another builder. It may call other builders (the same
//!   condition, one level down) and may spread one of its parameters into an
//!   object literal, provided every call passes it a literal (a spread runs the
//!   getters of an object that has any). Its parameters are plain names;
//! - a constant that holds the result of such a chain (`const adapter =
//!   createAdapter(c)`, then `adapter()`): the chain goes on;
//! - one of the builders the caller knows by package and name
//!   ([`Oracle::builds`]),
//!
//! and neither the binding nor the file that defines it changed. Such a chain
//! cannot call its arguments, touch module or global state or read a property
//! of something it did not make: everything it does is build the value it
//! returns, which is tracked like any other definition. In the same way a
//! template literal or an operator over an imported constant is unevaluated on
//! the condition that the constant holds a primitive ([`Oracle::prim`]).
//! `Symbol()`, `Symbol.for()`, `Object.freeze()` of a literal and `new Map /
//! Set / WeakMap / WeakSet` of nothing or of an array literal are never
//! load-time code.
//!
//! The split is what makes export-level reasoning sound. A changed value can
//! reach a test in two ways only: through a chain of static references from
//! code the test runs (tracked as tainted exports), or because some module used
//! it while loading (an evaluated statement read it; the caller then treats the
//! whole module as changed, as before). Bindings written from inside functions
//! are covered by the first way: the function that writes has to be called by
//! something that refers to it.
//!
//! # Members of enums and object tables
//!
//! `export enum Resource { Team = 'team', ... }` and `export const LIMITS = {
//! free: 1, ... }` are one export with many independent members, and most code
//! reads one member (`Resource.Team`). When the only difference between the two
//! versions of such a statement is in its members, [`diff`] lists the changed
//! members ([`Changed::members`]), and [`Module::taint`] taints a file's import
//! of that export only where the file reads a changed member, or uses the
//! export in any other way than `X.member` (passes it on, enumerates it,
//! indexes it with a computed key, calls a member: the call hands the table
//! to the function as `this`). An enum qualifies when every member has a
//! literal initializer (the value of a member without one depends on its
//! position); an object literal when it is not load-time code, has plain keys,
//! no spread and no `__proto__`, and no member reads the table itself (by name,
//! directly or through a helper).
//!
//! # Types
//!
//! A reference in a type position carries no value: a transpiler erases it. So
//! a change of a type, or of anything an importer names only in types, taints
//! nothing, whatever the transpiler's import-elision settings are (those decide
//! whether the module is *loaded*, which is not what taint is about). The
//! exception is a file with decorators: with `emitDecoratorMetadata` the types
//! of decorated members are emitted as values, so in such a file every
//! reference counts as a value.
//!
//! Anything this module cannot read (syntax or early errors, Flow, CommonJS
//! exports, `export =`, `import x = require()`, test-runner mock calls) has no
//! facts: callers fall back to whole-file granularity.

pub mod relation;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

use oxc_allocator::Allocator;
use oxc_ast::{
    AstKind,
    ast::{
        Argument, ArrayExpressionElement, ArrowFunctionBody, ArrowFunctionExpression,
        BindingPattern, CallExpression, Class, ClassElement, Declaration,
        ExportDefaultDeclarationKind, Expression, FormalParameters, Function,
        ImportDeclarationSpecifier, ImportOrExportKind, ModuleExportName, NewExpression,
        ObjectPropertyKind, PropertyKey, PropertyKind, Statement, StringLiteral, UnaryOperator,
        VariableDeclaration,
    },
};
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder, SymbolId};
use oxc_span::{GetSpan, SourceType, Span};

use crate::{
    Dialect,
    lexer::{Lexer, TokKind},
};

/// What an import binding stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Imported {
    /// `import {name as local}`.
    Name(String),
    /// `import local from`.
    Default,
    /// `import * as local from`.
    Namespace,
}

/// How a top-level binding is declared: what reading it at load time can do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Kind {
    /// A `class` declaration: initialized where it stands, always a
    /// constructor.
    Class,
    /// A `function` declaration that is neither `async` nor a generator:
    /// hoisted, always a constructor.
    Function,
    /// `let` or `const`: initialized where it stands.
    Lexical,
    /// An import.
    Import,
    /// Anything else (`var`, enums, namespaces, `async` and generator
    /// functions, merged declarations, types): hoisted, of any value.
    #[default]
    Other,
}

/// A top-level binding of the module.
#[derive(Clone, Debug)]
struct Binding {
    /// How it is declared.
    kind: Kind,
    /// For an import binding: where it comes from.
    import: Option<Import>,
    /// What using it in a call chain does, when that only builds a value.
    pure: Option<Pure>,
    /// A `const` with a primitive value.
    prim: bool,
}

/// A binding whose calls only build a value (see "Calls that only build a
/// value").
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pure {
    /// A function that is never reassigned.
    Function {
        /// Calls in a row that only build a value: 1 for a function that
        /// returns an unevaluated expression, 2 when that expression is
        /// again such a function (`withX()(Component)`), and so on.
        depth: u8,
        /// It spreads a parameter into an object literal: call sites must pass
        /// literals.
        spreads: bool,
        /// Builders its body calls: each must hold as well.
        needs: Vec<Cond>,
    },
    /// A constant holding the result of a chain on another binding.
    Alias { root: u32, steps: Vec<Step> },
}

/// The origin of an import binding.
#[derive(Clone, Debug)]
struct Import {
    /// Byte offset of the `import` keyword of its declaration.
    start: u32,
    /// The specifier.
    spec: String,
    /// The imported name.
    imported: Imported,
    /// `import type` or an inline `type` specifier.
    type_only: bool,
}

/// One top-level statement.
#[derive(Clone, Debug, Default)]
struct Stmt {
    /// Runs code at load time (see the module documentation).
    evaluated: Option<Evaluated>,
    /// Top-level bindings it declares.
    declares: Vec<u32>,
    /// Top-level bindings it refers to, anywhere inside.
    refs: Vec<Ref>,
    /// Top-level bindings that classes it defines at load time extend. Defining
    /// such a class throws when the binding does not hold a constructor.
    extends: Vec<u32>,
    /// Imports it reads at load time, although it is not load-time code (a name
    /// in an array or object literal, the root or an argument of a chain
    /// that only builds a value). In a module that is part of an import
    /// cycle such a read can find the import uninitialized.
    immediate: Vec<u32>,
    /// For an enum or an object table (only kept for [`diff`]): its members.
    table: Option<Table>,
    /// Its tokens (only kept for [`diff`]).
    text: String,
}

/// A reference of a statement to a top-level binding.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Ref {
    binding: u32,
    /// Read or written as a value somewhere in the statement (not only named in
    /// types).
    value: bool,
    /// Every value use in the statement is a read of a static member
    /// (`X.name`): the names. `None`: some use takes the binding as a
    /// whole.
    members: Option<Vec<String>>,
}

/// The members of an enum or object-table statement, for a member-level
/// comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Table {
    /// The statement's tokens with the members left out.
    shell: String,
    /// Member names with their tokens.
    members: Vec<(String, String)>,
}

/// Why a statement runs code at load time, and when it does not after all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Evaluated {
    /// The reason, for diagnostics.
    why: String,
    /// Not empty: the statement is evaluated only because of these calls, and
    /// is not when every callee only builds a value.
    unless: Vec<Cond>,
}

/// One step of a chain after its root: `z.object(a).optional()` is member,
/// call, member, call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// `(...)`.
    Call,
    /// `.name`.
    Member(String),
}

/// A condition on which a statement is not load-time code.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Cond {
    /// The chain `steps` on the top-level binding `root` only builds a value.
    Builds {
        root: u32,
        steps: Vec<Step>,
        /// Every argument of every call in the chain is a literal: an object
        /// literal without accessors or spreads, an array literal, a
        /// function, a primitive.
        literal_args: bool,
        /// When the last call has one argument and that is an object literal of
        /// plain properties: what each property holds.
        arg: Option<Vec<(String, PropFact)>>,
    },
    /// The top-level binding holds a primitive.
    Prim { binding: u32 },
}

/// What a property of an object-literal argument holds ([`Cond::Builds`]).
#[derive(Clone, Debug, PartialEq, Eq)]
enum PropFact {
    /// A function, an arrow function or a method.
    Function,
    /// A chain of calls and members on a top-level binding.
    Chain { root: u32, steps: Vec<Step> },
    /// A spread of a parameter of the function being judged.
    Spread,
    /// Anything else.
    Other,
}

/// What a property of an object-literal argument holds, for the caller to judge
/// ([`Oracle::builds_with`]).
#[derive(Clone, Debug)]
pub enum Prop<'a> {
    /// A function, an arrow function or a method.
    Function,
    /// A chain of calls and members on an import.
    Chain(Callee<'a>, Vec<Step>),
    /// A spread of a parameter of the function the call stands in (the
    /// property's name is `...`): its properties come before the ones
    /// written after it.
    Spread,
    /// Anything else.
    Other,
}

/// An import of a module, for the caller to judge ([`Oracle`]).
#[derive(Clone, Copy, Debug)]
pub struct Callee<'a> {
    /// The specifier.
    pub spec: &'a str,
    /// The imported name (`default` for a default import, `*` for a namespace
    /// import).
    pub name: &'a str,
}

/// What the caller knows about the imports of a module: other first-party files
/// and builders of packages.
pub trait Oracle {
    /// Whether the chain `steps` on the import only builds a value (see "Calls
    /// that only build a value"). `literal_args`: every argument in the
    /// chain is a literal.
    fn builds(&self, import: Callee<'_>, steps: &[Step], literal_args: bool) -> bool;
    /// Whether the chain `steps` on the value that the chain `made` on the
    /// import returned only builds a value: `made` ran where a constant was
    /// declared, `steps` run where the constant is used (`const adapter =
    /// createAdapter(c)`, then `adapter()`).
    fn continues(&self, import: Callee<'_>, made: &[Step], steps: &[Step]) -> bool {
        let all: Vec<Step> = made.iter().chain(steps).cloned().collect();
        self.builds(import, &all, false)
    }
    /// [`Oracle::builds`] for a chain whose last call has one argument, an
    /// object literal of plain properties `arg`: a builder may only build a
    /// value for some declarations (one that names a function it would
    /// otherwise look for and fail without).
    fn builds_with(
        &self,
        import: Callee<'_>,
        steps: &[Step],
        literal_args: bool,
        _arg: &[(String, Prop<'_>)],
    ) -> bool {
        self.builds(import, steps, literal_args)
    }
    /// Whether the import is a constant with a primitive value.
    fn prim(&self, import: Callee<'_>) -> bool;
    /// Whether the import comes from an ES module of the repository: reading it
    /// is reading a binding. Anything else (a CommonJS module, a package)
    /// may define the name with a getter, which runs when a module that is
    /// transpiled to CommonJS reads the import. The default knows no
    /// module.
    fn esm(&self, _import: Callee<'_>) -> bool {
        false
    }
}

/// An [`Oracle`] that knows nothing about other modules.
pub struct Alone;

impl Oracle for Alone {
    fn builds(&self, _: Callee<'_>, _: &[Step], _: bool) -> bool {
        false
    }
    fn prim(&self, _: Callee<'_>) -> bool {
        false
    }
}

/// How many steps of a chain, calls of a function or hops between modules are
/// followed.
const FUEL: u8 = 8;

/// What defines an export name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Export {
    /// A top-level binding (`export const a`, `export {a as b}`, `export
    /// default function f`).
    Binding(u32),
    /// A statement without a binding (`export default <expression>`).
    Stmt(u32),
    /// Another module (`export {a as b} from 'x'`, `export * as ns from 'x'`):
    /// the fact.
    From(String),
}

/// Definition-level facts of one version of a module. See the module
/// documentation.
#[derive(Clone, Debug, Default)]
pub struct Module {
    stmts: Vec<Stmt>,
    bindings: Vec<Binding>,
    exports: BTreeMap<String, Export>,
    /// The export names in the order of the statements that export them: the
    /// order of the keys of the module object a CommonJS transpilation
    /// builds.
    order: Vec<String>,
    /// `export * from 'x'` specifiers, in order.
    stars: Vec<String>,
    /// The specifiers of the imports a transpiler keeps whatever its settings
    /// are, in order: imports without bindings, and imports of which some
    /// binding is read as a value. An import whose bindings are all unused,
    /// or used in types only, is dropped by most transpilers, and the
    /// module is then not loaded.
    loads: Vec<String>,
    /// [`Module::loads`] without the imports that only JSX may use: in a file
    /// without a `@jsx` pragma the factory is a guess (the usual names, the
    /// imports nothing else uses), and a transpiler that takes another one
    /// drops such an import.
    written_loads: Vec<String>,
    /// Directive comments (`@jsxImportSource`, `@jest-environment`,
    /// triple-slash references), then directives (`'use client'`), each in
    /// order.
    directives: Vec<String>,
    /// An export hands out a function that reads `this`: called as a member of
    /// a namespace (`ns.f()`), it reads whatever that namespace exports,
    /// also the namespace of a barrel that re-exports it.
    reads_this: bool,
}

/// One `import` declaration of a module with changed exports, for
/// [`Module::taint_members`].
#[derive(Clone, Copy, Debug)]
pub struct Asked<'a> {
    /// The byte offset of the `import` keyword.
    pub start: u32,
    /// The changed names (`None`: every name).
    pub names: Option<&'a [String]>,
    /// Of the changed names, the ones whose binding changed
    /// ([`Changed::rebound`]).
    pub rebound: &'a [String],
}

/// What a file derives from tainted imports ([`Module::taint`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Taint {
    /// A statement evaluated at load time reads a tainted value: the reason it
    /// counts as evaluated (`None`: loading the module does not use the
    /// change).
    pub evaluation: Option<String>,
    /// Export names whose definition reads a tainted value, sorted.
    pub exports: Vec<String>,
    /// Statements that read a tainted value in a call that only builds a value.
    pub pure_calls: usize,
    /// Some statement reads a tainted value (`false`: the file names the
    /// changed exports in types only, or reads none of the changed
    /// members).
    pub any: bool,
}

/// What changed between two versions of a module ([`diff`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// What the module does at load time differs (evaluated statements, their
    /// closures or the directives).
    pub evaluation: bool,
    /// With `evaluation`: what differs, in words.
    pub load_time: Option<String>,
    /// Export names whose definition closure differs, or that exist at one
    /// version only; sorted.
    pub exports: Vec<String>,
    /// For changed exports that are an enum or an object table and differ only
    /// in their members: the members that were added, removed or changed.
    pub members: BTreeMap<String, Vec<String>>,
    /// Of `exports`, the names that exist at one version only or are declared
    /// in another way (a function that became a constant, a class that
    /// became a function): for an importer the binding itself changed, not
    /// only its value. Importing such a name can fail when the modules are
    /// linked, or find it uninitialized; sorted.
    pub rebound: Vec<String>,
    /// The exports the two versions share are declared in another order: the
    /// keys of the module object come in another order where modules are
    /// transpiled to CommonJS.
    pub reordered: bool,
}

/// The index of the statement containing `pos`.
fn stmt_at(spans: &[Span], pos: u32) -> Option<u32> {
    let i = spans.partition_point(|s| s.end <= pos);
    spans.get(i).filter(|s| s.start <= pos).map(|_| i as u32)
}

/// The tokens of `src` as `(start, end, line break before)`, comments dropped.
/// `None` when the lexer fails.
fn tokens(path: &str, src: &str) -> Option<Vec<(u32, u32, bool)>> {
    let bytes = src.as_bytes();
    let mut lx = Lexer::new(bytes, Dialect::from_path(Path::new(path)));
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let t = lx.next_token();
        let (start, end) = (t.start as usize, t.end as usize);
        if start < pos || end < start || end > bytes.len() {
            return None;
        }
        if t.kind == TokKind::Eof {
            break;
        }
        let newline = bytes[pos..start].iter().any(|b| matches!(b, b'\n' | b'\r'))
            || src
                .get(pos..start)?
                .chars()
                .any(|c| matches!(c, '\u{2028}' | '\u{2029}'));
        out.push((t.start, t.end, newline));
        pos = end;
    }
    if lx.take_error().is_some() || lx.jsx_failed() {
        return None;
    }
    Some(out)
}

/// The tokens of `span`, separated by a space or (where the source has a line
/// break) a newline.
fn span_text(src: &str, toks: &[(u32, u32, bool)], span: Span) -> String {
    let from = toks.partition_point(|t| t.0 < span.start);
    let mut out = String::new();
    for (i, &(s, e, nl)) in toks[from..].iter().enumerate() {
        if e > span.end {
            break;
        }
        if i > 0 {
            out.push(if nl { '\n' } else { ' ' });
        }
        out.push_str(src.get(s as usize..e as usize).unwrap_or(""));
    }
    out
}

/// Why a statement counts as evaluated.
#[derive(Copy, Clone, Debug)]
enum Why {
    /// A fixed reason.
    Is(&'static str),
    /// A call: the callee.
    Call(Span),
    /// A `new`: the constructor.
    New(Span),
    /// A tagged template: the tag.
    Tag(Span),
}

impl Why {
    /// The reason in words, with the callee as written (shortened).
    fn text(self, src: &str) -> String {
        let name = |span: Span| {
            let t = src
                .get(span.start as usize..span.end as usize)
                .unwrap_or("");
            let mut out: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if out.len() > 48 {
                let cut = (0..=48)
                    .rev()
                    .find(|&i| out.is_char_boundary(i))
                    .unwrap_or(0);
                out.truncate(cut);
                out.push('…');
            }
            out
        };
        match self {
            Why::Is(s) => s.to_owned(),
            Why::Call(c) => format!("a call of {}", name(c)),
            Why::New(c) => format!("a `new {}`", name(c)),
            Why::Tag(c) => format!("a template tagged {}", name(c)),
        }
    }
}

/// A [`Cond`] being collected, with the span that names it in the reason.
struct Pending {
    cond: Cond,
    span: Span,
}

/// Classifies expressions and statements as evaluated or not.
struct Eval<'s> {
    scoping: &'s Scoping,
    /// Top-level bindings by symbol.
    index: &'s HashMap<SymbolId, u32>,
    /// Per top-level symbol: the statement that declares it (the first one),
    /// and whether the binding is uninitialized before that statement ran
    /// (`let`, `const`, `class`).
    declared: &'s HashMap<SymbolId, (u32, bool)>,
    /// The statement being classified.
    current: Cell<u32>,
    /// Top-level bindings that classes defined by the statement extend.
    extends: RefCell<Vec<u32>>,
    /// Imports the statement reads at load time.
    immediate: RefCell<Vec<u32>>,
    /// Top-level bindings known to hold a primitive (a `const` with a primitive
    /// initializer).
    prims: BTreeSet<SymbolId>,
    /// Conditions on which what was classified so far is unevaluated.
    conds: RefCell<Vec<Pending>>,
    /// The parameters of the function whose body is being judged
    /// ([`Eval::purity`]).
    params: RefCell<Vec<SymbolId>>,
    /// That function spreads one of its parameters into an object literal.
    spreads: Cell<bool>,
}

/// `e` without parentheses and type-level wrappers.
fn strip<'a, 'b>(mut e: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        e = match e {
            Expression::ParenthesizedExpression(p) => &p.expression,
            Expression::TSAsExpression(x) => &x.expression,
            Expression::TSSatisfiesExpression(x) => &x.expression,
            Expression::TSNonNullExpression(x) => &x.expression,
            Expression::TSInstantiationExpression(x) => &x.expression,
            _ => return e,
        };
    }
}

/// Globals of the language and of every JavaScript runtime tests run in:
/// reading one never throws. Any other global may not exist where the module is
/// loaded (`window`, a constant a bundler defines), and reading it at load time
/// then throws.
const SAFE_GLOBALS: &[&str] = &[
    "undefined",
    "NaN",
    "Infinity",
    "globalThis",
    "Object",
    "Array",
    "Function",
    "String",
    "Number",
    "Boolean",
    "Symbol",
    "BigInt",
    "Math",
    "JSON",
    "Reflect",
    "Proxy",
    "Promise",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "WeakRef",
    "Date",
    "RegExp",
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "EvalError",
    "URIError",
    "AggregateError",
    "ArrayBuffer",
    "DataView",
    "Uint8Array",
    "Int8Array",
    "Uint16Array",
    "Int16Array",
    "Uint32Array",
    "Int32Array",
    "Float32Array",
    "Float64Array",
    "BigInt64Array",
    "BigUint64Array",
    "Intl",
    "console",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "decodeURIComponent",
    "encodeURI",
    "decodeURI",
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "queueMicrotask",
    "structuredClone",
    "URL",
    "URLSearchParams",
    "TextEncoder",
    "TextDecoder",
    "AbortController",
    "AbortSignal",
    "process",
    "Buffer",
];

/// Globals that a class can extend: constructors that exist in every runtime.
const GLOBAL_CLASSES: &[&str] = &[
    "Object",
    "Array",
    "Function",
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "EvalError",
    "URIError",
    "AggregateError",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Promise",
    "RegExp",
    "Date",
    "String",
    "Number",
    "Boolean",
    "ArrayBuffer",
    "Uint8Array",
    "URL",
    "URLSearchParams",
];

/// Whether `e` is a string or number literal: converting it to a string runs no
/// code.
fn plain_text(e: &Expression<'_>) -> bool {
    match strip(e) {
        Expression::StringLiteral(_) | Expression::NumericLiteral(_) => true,
        Expression::TemplateLiteral(t) => t.expressions.is_empty(),
        _ => false,
    }
}

/// Whether a call argument is a literal: spreading it, or reading its
/// properties, runs no code.
fn literal_arg(e: &Expression<'_>) -> bool {
    match strip(e) {
        Expression::ObjectExpression(o) => o.properties.iter().all(|p| match p {
            ObjectPropertyKind::ObjectProperty(p) => {
                !matches!(p.kind, PropertyKind::Get | PropertyKind::Set)
            }
            ObjectPropertyKind::SpreadProperty(_) => false,
        }),
        Expression::ArrayExpression(_)
        | Expression::ArrowFunctionExpression(_)
        | Expression::FunctionExpression(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::TemplateLiteral(_) => true,
        _ => false,
    }
}

impl Eval<'_> {
    fn symbol(&self, id: &oxc_ast::ast::IdentifierReference<'_>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id())
    }

    /// The top-level binding or global an identifier refers to: `Some(None)`
    /// for a global, `None` for a binding that is not at the top level (a
    /// parameter, a local).
    fn target(&self, id: &oxc_ast::ast::IdentifierReference<'_>) -> Option<Option<u32>> {
        match self.symbol(id) {
            None => Some(None),
            Some(s) => self.index.get(&s).map(|b| Some(*b)),
        }
    }

    /// Why reading the name `id` at load time is more than taking a value: the
    /// name is a global that may not exist, or a `let`, `const` or `class`
    /// binding of the module that is declared further down (reading it
    /// throws). Records reads of imports.
    fn read(&self, id: &oxc_ast::ast::IdentifierReference<'_>) -> Option<Why> {
        let Some(s) = self.symbol(id) else {
            return (!SAFE_GLOBALS.contains(&id.name.as_str()))
                .then_some(Why::Is("a read of a global that may not exist"));
        };
        if let Some(&(at, lexical)) = self.declared.get(&s)
            && lexical
            && at >= self.current.get()
        {
            return Some(Why::Is("a read of a binding before its declaration"));
        }
        if self.scoping.symbol_flags(s).is_import()
            && let Some(&b) = self.index.get(&s)
        {
            self.immediate.borrow_mut().push(b);
        }
        None
    }

    /// Why evaluating the arguments runs code.
    fn args(&self, args: &[Argument<'_>]) -> Option<Why> {
        args.iter().find_map(|a| match a.as_expression() {
            Some(x) => self.expr(x),
            None => Some(Why::Is("a spread")),
        })
    }

    /// The root of the chain that ends in the call `c`, with its steps appended
    /// to `steps` (`literal`: every argument so far is a literal). The
    /// error: an argument that runs code (the reason), or a shape that is
    /// not a chain of calls and members on a name (`None`).
    fn chain_call<'a, 'b>(
        &self,
        c: &'b CallExpression<'a>,
        steps: &mut Vec<Step>,
        literal: &mut bool,
    ) -> Result<&'b oxc_ast::ast::IdentifierReference<'a>, Option<Why>> {
        if c.optional {
            return Err(None);
        }
        if let Some(why) = self.args(&c.arguments) {
            return Err(Some(why));
        }
        *literal &= c
            .arguments
            .iter()
            .all(|a| a.as_expression().is_some_and(literal_arg));
        let root = self.chain(&c.callee, steps, literal)?;
        steps.push(Step::Call);
        Ok(root)
    }

    fn chain<'a, 'b>(
        &self,
        e: &'b Expression<'a>,
        steps: &mut Vec<Step>,
        literal: &mut bool,
    ) -> Result<&'b oxc_ast::ast::IdentifierReference<'a>, Option<Why>> {
        if steps.len() >= usize::from(FUEL) * 4 {
            return Err(None);
        }
        match strip(e) {
            Expression::Identifier(id) => Ok(id),
            Expression::CallExpression(c) => self.chain_call(c, steps, literal),
            Expression::StaticMemberExpression(m) if !m.optional => {
                let root = self.chain(&m.object, steps, literal)?;
                steps.push(Step::Member(m.property.name.to_string()));
                Ok(root)
            }
            _ => Err(None),
        }
    }

    /// A call: unevaluated on the condition that the chain it ends only builds
    /// a value.
    fn call(&self, c: &CallExpression<'_>) -> Option<Why> {
        let runs = Some(Why::Call(c.callee.span()));
        let (mut steps, mut literal) = (Vec::new(), true);
        let root = match self.chain_call(c, &mut steps, &mut literal) {
            Ok(root) => root,
            Err(None) => return runs,
            Err(why) => return why,
        };
        match self.target(root) {
            // A global that only makes a value.
            Some(None) => {
                let name = root.name.as_str();
                // A symbol's description is converted to a string: only literal text is
                // sure to run no code.
                let text = c
                    .arguments
                    .iter()
                    .all(|a| a.as_expression().is_some_and(plain_text));
                let pure = match steps.as_slice() {
                    [Step::Call] => name == "Symbol" && text,
                    // `Object.freeze` of a literal: freezing a shared object would be an
                    // effect on it.
                    [Step::Member(m), Step::Call] => match (name, m.as_str()) {
                        ("Symbol", "for") => text && c.arguments.len() == 1,
                        ("Object", "freeze") => literal,
                        _ => false,
                    },
                    _ => false,
                };
                if pure { None } else { runs }
            }
            Some(Some(b)) => {
                // The callee has to exist when the statement runs: a binding of the module
                // that is declared further down is not initialized yet, and a function
                // declared further down may read bindings that are not.
                if let Some(s) = self.symbol(root) {
                    if self.scoping.symbol_flags(s).is_import() {
                        self.immediate.borrow_mut().push(b);
                    } else if self
                        .declared
                        .get(&s)
                        .is_none_or(|&(at, _)| at >= self.current.get())
                    {
                        return runs;
                    }
                }
                let arg = self.object_arg(c);
                self.conds.borrow_mut().push(Pending {
                    cond: Cond::Builds {
                        root: b,
                        steps,
                        literal_args: literal,
                        arg,
                    },
                    span: c.callee.span(),
                });
                None
            }
            // A parameter or a local of a function: calling it runs it.
            None => runs,
        }
    }

    /// The properties of the only argument of `c`, when that is an object
    /// literal of plain properties (no spread, no accessor, no computed
    /// key).
    fn object_arg(&self, c: &CallExpression<'_>) -> Option<Vec<(String, PropFact)>> {
        let [a] = c.arguments.as_slice() else {
            return None;
        };
        let Expression::ObjectExpression(o) = strip(a.as_expression()?) else {
            return None;
        };
        let mut out = Vec::new();
        for p in &o.properties {
            let p = match p {
                ObjectPropertyKind::ObjectProperty(p) => p,
                ObjectPropertyKind::SpreadProperty(s) if self.is_param(&s.argument) => {
                    out.push(("...".to_owned(), PropFact::Spread));
                    continue;
                }
                ObjectPropertyKind::SpreadProperty(_) => return None,
            };
            if p.computed || !matches!(p.kind, PropertyKind::Init) {
                return None;
            }
            let name = match &p.key {
                PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                PropertyKey::StringLiteral(s) => lit(s),
                _ => return None,
            };
            let fact = match strip(&p.value) {
                Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
                    PropFact::Function
                }
                v @ Expression::CallExpression(_) => {
                    let (mut steps, mut literal) = (Vec::new(), true);
                    let before = self.conds.borrow().len();
                    let root = self.chain(v, &mut steps, &mut literal).ok();
                    self.conds.borrow_mut().truncate(before);
                    match root.and_then(|r| self.target(r)) {
                        Some(Some(root)) => PropFact::Chain { root, steps },
                        _ => PropFact::Other,
                    }
                }
                _ => PropFact::Other,
            };
            out.push((name, fact));
        }
        Some(out)
    }

    /// `new`: only the empty collections of the language are not load-time
    /// code.
    fn construct(&self, n: &NewExpression<'_>) -> Option<Why> {
        let runs = Some(Why::New(n.callee.span()));
        let Expression::Identifier(id) = strip(&n.callee) else {
            return runs;
        };
        let name = id.name.as_str();
        let collection =
            matches!(name, "Map" | "Set" | "WeakMap" | "WeakSet") && self.target(id) == Some(None);
        match n.arguments.as_slice() {
            [] if collection => None,
            // The entries of a `Map` must be pairs (anything else throws), the members of a
            // weak collection objects: only array literals of array literals, and sets.
            [a] if matches!(name, "Map" | "Set") && collection => {
                match a.as_expression().map(strip) {
                    Some(e @ Expression::ArrayExpression(entries)) => {
                        let pairs = name == "Set"
                            || entries.elements.iter().all(|el| {
                                matches!(
                                    el.as_expression().map(strip),
                                    Some(Expression::ArrayExpression(_))
                                )
                            });
                        if pairs { self.expr(e) } else { runs }
                    }
                    _ => runs,
                }
            }
            _ => runs,
        }
    }

    /// Whether `e` is unevaluated without any condition.
    fn quiet(&self, e: &Expression<'_>) -> bool {
        let before = self.conds.borrow().len();
        let why = self.expr(e);
        let mut conds = self.conds.borrow_mut();
        let clean = why.is_none() && conds.len() == before;
        conds.truncate(before);
        clean
    }

    /// The symbols of plain identifier parameters; `None` when a parameter is a
    /// pattern, has a default value or a decorator.
    fn plain_params(p: &FormalParameters<'_>) -> Option<Vec<SymbolId>> {
        let mut out = Vec::new();
        for i in &p.items {
            let BindingPattern::BindingIdentifier(id) = &i.pattern else {
                return None;
            };
            if i.initializer.is_some() || !i.decorators.is_empty() {
                return None;
            }
            out.extend(id.symbol_id.get());
        }
        if let Some(r) = &p.rest {
            let BindingPattern::BindingIdentifier(id) = &r.rest.argument else {
                return None;
            };
            out.extend(id.symbol_id.get());
        }
        Some(out)
    }

    /// How many calls in a row of the function value `e` only build a value,
    /// without any condition.
    fn depth(&self, e: &Expression<'_>) -> u8 {
        match strip(e) {
            Expression::ArrowFunctionExpression(a) => self.arrow_depth(a),
            Expression::FunctionExpression(f) => self.function_depth(f),
            _ => 0,
        }
    }

    /// The depth of a function that returns `value` after doing nothing else.
    /// `strict`: the value must be unevaluated without conditions (else
    /// conditions are collected).
    fn returning(&self, value: Option<&Expression<'_>>, strict: bool) -> u8 {
        match value {
            None => 1,
            Some(e) => {
                let ok = if strict {
                    self.quiet(e)
                } else {
                    self.expr(e).is_none()
                };
                if ok {
                    1 + self.depth(e).min(u8::MAX - 1)
                } else {
                    0
                }
            }
        }
    }

    /// The depth of a function with this body: constant declarations from
    /// unevaluated expressions, then a `return` of one.
    fn body_depth(&self, body: &[Statement<'_>], strict: bool) -> u8 {
        let inert_vars = |s: &Statement<'_>| match s {
            Statement::VariableDeclaration(v) => v.declarations.iter().all(|d| {
                matches!(d.id, BindingPattern::BindingIdentifier(_))
                    && d.init.as_ref().is_none_or(|i| {
                        if strict {
                            self.quiet(i)
                        } else {
                            self.expr(i).is_none()
                        }
                    })
            }),
            _ => false,
        };
        let Some((last, first)) = body.split_last() else {
            return 1;
        };
        if !first.iter().all(inert_vars) {
            return 0;
        }
        match last {
            Statement::ReturnStatement(r) => self.returning(r.argument.as_ref(), strict),
            s if inert_vars(s) => 1,
            _ => 0,
        }
    }

    fn function_depth(&self, f: &Function<'_>) -> u8 {
        let Some(body) = &f.body else { return 0 };
        if Self::plain_params(&f.params).is_none() {
            return 0;
        }
        let d = self.body_depth(&body.statements, true);
        // The result of a generator is not the returned value: nothing runs until it is
        // iterated. An async function resolves its promise with what it returns, which
        // reads and calls `then` of a thenable: calling it does more than build a
        // value.
        if f.r#async && !f.generator {
            0
        } else if f.generator {
            d.min(1)
        } else {
            d
        }
    }

    fn arrow_depth(&self, a: &ArrowFunctionExpression<'_>) -> u8 {
        if Self::plain_params(&a.params).is_none() {
            return 0;
        }
        let d = match &a.body {
            ArrowFunctionBody::FunctionBody(b) => self.body_depth(&b.statements, true),
            body => body
                .as_expression()
                .map_or(0, |e| self.returning(Some(e), true)),
        };
        // An async function hands what it returns to its promise (see
        // `function_depth`).
        if a.r#async { 0 } else { d }
    }

    /// What calling a top-level function does, when that only builds a value.
    /// Its own body may call other builders (collected as `needs`) and
    /// spread its parameters; the functions it returns must build values
    /// without conditions.
    fn purity(
        &self,
        params: &FormalParameters<'_>,
        body: Result<&[Statement<'_>], &Expression<'_>>,
        wraps: bool,
    ) -> Option<Pure> {
        let own = Self::plain_params(params)?;
        let outer = self.params.replace(own);
        let spread = self.spreads.replace(false);
        let before = self.conds.borrow().len();
        let depth = match body {
            Ok(statements) => self.body_depth(statements, false),
            Err(e) => self.returning(Some(e), false),
        };
        let needs: Vec<Cond> = self
            .conds
            .borrow_mut()
            .split_off(before)
            .into_iter()
            .map(|p| p.cond)
            .collect();
        let spreads = self.spreads.replace(spread);
        self.params.replace(outer);
        // A class the function defines extends a binding of the module: calling the
        // function throws when that is not a constructor.
        if !self.extends.borrow().is_empty() {
            self.extends.borrow_mut().clear();
            return None;
        }
        (depth > 0).then_some(Pure::Function {
            depth: if wraps { 1 } else { depth },
            spreads,
            needs,
        })
    }

    /// [`Eval::purity`] of the value `e` of a constant: a function, or the
    /// result of a chain on another top-level binding (then calling the
    /// constant continues that chain).
    fn value_purity(&self, e: &Expression<'_>) -> Option<Pure> {
        match strip(e) {
            Expression::ArrowFunctionExpression(a) => {
                let body = match &a.body {
                    ArrowFunctionBody::FunctionBody(b) => Ok(b.statements.as_slice()),
                    body => Err(body.as_expression()?),
                };
                if a.r#async {
                    return None;
                }
                self.purity(&a.params, body, false)
            }
            Expression::FunctionExpression(f) => self.function_purity(f),
            Expression::CallExpression(c) => {
                let before = self.conds.borrow().len();
                let (mut steps, mut literal) = (Vec::new(), true);
                let root = self.chain_call(c, &mut steps, &mut literal);
                self.conds.borrow_mut().truncate(before);
                match self.target(root.ok()?) {
                    Some(Some(root)) => Some(Pure::Alias { root, steps }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn function_purity(&self, f: &Function<'_>) -> Option<Pure> {
        let body = f.body.as_ref()?;
        if f.r#async && !f.generator {
            return None;
        }
        self.purity(&f.params, Ok(body.statements.as_slice()), f.generator)
    }

    /// Whether `e` names a parameter of the function being judged.
    fn is_param(&self, e: &Expression<'_>) -> bool {
        match strip(e) {
            Expression::Identifier(id) => self
                .symbol(id)
                .is_some_and(|s| self.params.borrow().contains(&s)),
            _ => false,
        }
    }

    /// Whether `e` is a primitive value that operators can combine without
    /// running code.
    fn prim(&self, e: &Expression<'_>) -> bool {
        match e {
            Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::StringLiteral(_) => true,
            Expression::TemplateLiteral(t) => t.expressions.iter().all(|x| self.prim(x)),
            Expression::Identifier(id) => {
                if matches!(id.name.as_str(), "undefined" | "NaN" | "Infinity") {
                    return true;
                }
                let Some(s) = self.symbol(id) else {
                    return false;
                };
                if self.prims.contains(&s) {
                    return true;
                }
                // An import: a primitive on the condition that its module says so.
                match self.index.get(&s) {
                    Some(&binding) if self.scoping.symbol_flags(s).is_import() => {
                        self.immediate.borrow_mut().push(binding);
                        self.conds.borrow_mut().push(Pending {
                            cond: Cond::Prim { binding },
                            span: id.span,
                        });
                        true
                    }
                    _ => false,
                }
            }
            Expression::ParenthesizedExpression(p) => self.prim(&p.expression),
            Expression::TSAsExpression(x) => self.prim(&x.expression),
            Expression::TSSatisfiesExpression(x) => self.prim(&x.expression),
            Expression::TSNonNullExpression(x) => self.prim(&x.expression),
            Expression::UnaryExpression(u) => match u.operator {
                // `typeof` of a global that does not exist is `'undefined'`.
                UnaryOperator::Typeof if matches!(strip(&u.argument), Expression::Identifier(id) if self.symbol(id).is_none()) => {
                    true
                }
                UnaryOperator::LogicalNot | UnaryOperator::Typeof | UnaryOperator::Void => {
                    self.expr(&u.argument).is_none()
                }
                UnaryOperator::UnaryNegation
                | UnaryOperator::UnaryPlus
                | UnaryOperator::BitwiseNot => self.prim(&u.argument),
                UnaryOperator::Delete => false,
            },
            Expression::BinaryExpression(b) => {
                !b.operator.is_in()
                    && !b.operator.is_instance_of()
                    && self.prim(&b.left)
                    && self.prim(&b.right)
            }
            Expression::LogicalExpression(l) => self.prim(&l.left) && self.prim(&l.right),
            Expression::ConditionalExpression(c) => {
                self.expr(&c.test).is_none() && self.prim(&c.consequent) && self.prim(&c.alternate)
            }
            _ => false,
        }
    }

    /// Why evaluating `e` runs code (`None`: it only builds a value).
    fn expr(&self, e: &Expression<'_>) -> Option<Why> {
        match e {
            Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::RegExpLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::FunctionExpression(_) => None,
            Expression::Identifier(id) => self.read(id),
            Expression::TemplateLiteral(t) => {
                if t.expressions.iter().all(|x| self.prim(x)) {
                    None
                } else {
                    Some(Why::Is("a template literal over non-primitive values"))
                }
            }
            Expression::ClassExpression(c) => self.class(c),
            Expression::ParenthesizedExpression(p) => self.expr(&p.expression),
            Expression::TSAsExpression(x) => self.expr(&x.expression),
            Expression::TSSatisfiesExpression(x) => self.expr(&x.expression),
            Expression::TSNonNullExpression(x) => self.expr(&x.expression),
            Expression::TSTypeAssertion(x) => self.expr(&x.expression),
            Expression::TSInstantiationExpression(x) => self.expr(&x.expression),
            Expression::ArrayExpression(a) => a.elements.iter().find_map(|el| match el {
                ArrayExpressionElement::SpreadElement(_) => Some(Why::Is("a spread")),
                ArrayExpressionElement::Elision(_) => None,
                _ => el.as_expression().and_then(|x| self.expr(x)),
            }),
            Expression::ObjectExpression(o) => o.properties.iter().find_map(|p| match p {
                // A spread of a parameter of the function being judged runs no code when
                // every call passes a literal (`spreads` of [`Pure::Function`]).
                ObjectPropertyKind::SpreadProperty(s) if self.is_param(&s.argument) => {
                    self.spreads.set(true);
                    None
                }
                ObjectPropertyKind::SpreadProperty(_) => Some(Why::Is("a spread")),
                ObjectPropertyKind::ObjectProperty(p) => {
                    if p.computed && !self.key_is_literal(&p.key) {
                        return Some(Why::Is("a computed key"));
                    }
                    self.expr(&p.value)
                }
            }),
            Expression::UnaryExpression(u) => match u.operator {
                // `typeof` of a global that does not exist is `'undefined'`.
                UnaryOperator::Typeof if matches!(strip(&u.argument), Expression::Identifier(id) if self.symbol(id).is_none()) => {
                    None
                }
                UnaryOperator::LogicalNot | UnaryOperator::Typeof | UnaryOperator::Void => {
                    self.expr(&u.argument)
                }
                UnaryOperator::Delete => Some(Why::Is("a delete")),
                _ if self.prim(&u.argument) => None,
                _ => Some(Why::Is("an operator on a non-primitive value")),
            },
            Expression::BinaryExpression(_) => {
                if self.prim(e) {
                    None
                } else {
                    Some(Why::Is("an operator on a non-primitive value"))
                }
            }
            Expression::LogicalExpression(l) => self.expr(&l.left).or_else(|| self.expr(&l.right)),
            Expression::ConditionalExpression(c) => self
                .expr(&c.test)
                .or_else(|| self.expr(&c.consequent))
                .or_else(|| self.expr(&c.alternate)),
            Expression::SequenceExpression(s) => s.expressions.iter().find_map(|x| self.expr(x)),
            Expression::CallExpression(c) => self.call(c),
            Expression::NewExpression(c) => self.construct(c),
            Expression::TaggedTemplateExpression(t) => Some(Why::Tag(t.tag.span())),
            Expression::AwaitExpression(_) => Some(Why::Is("an `await`")),
            Expression::AssignmentExpression(_) | Expression::UpdateExpression(_) => {
                Some(Why::Is("an assignment"))
            }
            Expression::JSXElement(_) | Expression::JSXFragment(_) => Some(Why::Is("JSX")),
            Expression::ImportExpression(_) => Some(Why::Is("a dynamic import")),
            Expression::StaticMemberExpression(_)
            | Expression::ComputedMemberExpression(_)
            | Expression::PrivateFieldExpression(_)
            | Expression::ChainExpression(_) => Some(Why::Is("a member access")),
            _ => Some(Why::Is("an expression that runs code")),
        }
    }

    fn key_is_literal(&self, k: &PropertyKey<'_>) -> bool {
        k.as_expression().is_some_and(|x| {
            matches!(
                x,
                Expression::StringLiteral(_) | Expression::NumericLiteral(_)
            ) || self.prim(x)
        })
    }

    /// Why declaring the class `c` runs code.
    fn class(&self, c: &Class<'_>) -> Option<Why> {
        if !c.decorators.is_empty() {
            return Some(Why::Is("a decorator"));
        }
        // Defining a class reads what it extends, and throws when that is not a
        // constructor.
        if let Some(h) = &c.heritage {
            match strip(&h.expression) {
                Expression::NullLiteral(_) => {}
                Expression::Identifier(id) => match self.symbol(id) {
                    None => {
                        if !GLOBAL_CLASSES.contains(&id.name.as_str()) {
                            return Some(Why::Is("a class that extends a global"));
                        }
                    }
                    Some(s) => {
                        if let Some(why) = self.read(id) {
                            return Some(why);
                        }
                        match self.index.get(&s) {
                            Some(&b) => self.extends.borrow_mut().push(b),
                            // A parameter or a local of a function: whatever it holds.
                            None => return Some(Why::Is("a class that extends a computed value")),
                        }
                    }
                },
                other => {
                    return Some(
                        self.expr(other)
                            .unwrap_or(Why::Is("a class that extends a computed value")),
                    );
                }
            }
        }
        for el in &c.body.body {
            match el {
                ClassElement::StaticBlock(_) => return Some(Why::Is("a static block")),
                ClassElement::MethodDefinition(m) => {
                    // A decorator of the method or of one of its parameters runs when the
                    // class is defined.
                    if !m.decorators.is_empty()
                        || m.value
                            .params
                            .items
                            .iter()
                            .any(|p| !p.decorators.is_empty())
                    {
                        return Some(Why::Is("a decorator"));
                    }
                    if m.computed && !self.key_is_literal(&m.key) {
                        return Some(Why::Is("a computed key"));
                    }
                }
                ClassElement::PropertyDefinition(p) => {
                    if !p.decorators.is_empty() {
                        return Some(Why::Is("a decorator"));
                    }
                    if p.computed && !self.key_is_literal(&p.key) {
                        return Some(Why::Is("a computed key"));
                    }
                    if p.r#static
                        && let Some(v) = &p.value
                        && let Some(why) = self.expr(v)
                    {
                        return Some(why);
                    }
                }
                ClassElement::AccessorProperty(p) => {
                    if !p.decorators.is_empty() {
                        return Some(Why::Is("a decorator"));
                    }
                    if p.computed && !self.key_is_literal(&p.key) {
                        return Some(Why::Is("a computed key"));
                    }
                    if p.r#static
                        && let Some(v) = &p.value
                        && let Some(why) = self.expr(v)
                    {
                        return Some(why);
                    }
                }
                ClassElement::TSIndexSignature(_) => {}
            }
        }
        None
    }

    /// Why the `const` / `let` / `var` declaration runs code; records primitive
    /// constants.
    fn vars(&mut self, v: &VariableDeclaration<'_>) -> Option<Why> {
        if v.declare {
            return None;
        }
        // The value is disposed when the module has run.
        if v.kind.is_using() {
            return Some(Why::Is("a `using` declaration"));
        }
        let mut why = None;
        for d in &v.declarations {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                why = why.or(Some(Why::Is("a destructuring")));
                continue;
            };
            let Some(init) = &d.init else { continue };
            match self.expr(init) {
                Some(w) => why = why.or(Some(w)),
                None => {
                    // A primitive without conditions (the probe's own are dropped).
                    let before = self.conds.borrow().len();
                    let prim = self.prim(init) && self.conds.borrow().len() == before;
                    self.conds.borrow_mut().truncate(before);
                    if v.kind.is_const()
                        && prim
                        && let Some(s) = id.symbol_id.get()
                    {
                        self.prims.insert(s);
                    }
                }
            }
        }
        why
    }

    fn declaration(&mut self, d: &Declaration<'_>) -> Option<Why> {
        match d {
            Declaration::VariableDeclaration(v) => self.vars(v),
            Declaration::FunctionDeclaration(_) => None,
            Declaration::ClassDeclaration(c) => {
                if c.declare {
                    None
                } else {
                    self.class(c)
                }
            }
            Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_) => None,
            Declaration::TSEnumDeclaration(e) => {
                if e.declare {
                    return None;
                }
                e.body.members.iter().find_map(|m| {
                    m.initializer
                        .as_ref()
                        .filter(|i| !self.prim(i))
                        .map(|_| Why::Is("an enum member computed from another value"))
                })
            }
            Declaration::TSGlobalDeclaration(_) | Declaration::TSExternalModuleDeclaration(_) => {
                None
            }
            Declaration::TSNamespaceDeclaration(n) => {
                if n.declare {
                    None
                } else {
                    Some(Why::Is("a namespace"))
                }
            }
            Declaration::TSImportEqualsDeclaration(_) => Some(Why::Is("an `import =`")),
        }
    }

    /// Why the top-level statement `s` runs code at load time.
    fn statement(&mut self, s: &Statement<'_>) -> Option<Why> {
        match s {
            Statement::ImportDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::EmptyStatement(_) => None,
            Statement::ExportDeclaration(e) => self.declaration(&e.declaration),
            Statement::ExportDefaultDeclaration(e) => match &e.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(_)
                | ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => None,
                ExportDefaultDeclarationKind::ClassDeclaration(c) => self.class(c),
                other => other.as_expression().and_then(|x| self.expr(x)),
            },
            Statement::ExpressionStatement(e) => self.expr(&e.expression),
            _ => match s.as_declaration() {
                Some(d) => self.declaration(d),
                None => Some(Why::Is("a statement that runs at load time")),
            },
        }
    }
}

/// What a program uses that the per-statement facts cannot express.
#[derive(Default)]
struct Scan {
    /// A decorator: with decorator metadata, types are emitted as values.
    decorators: bool,
    /// A call of `eval`: the code it runs can read every binding of the module.
    eval: bool,
    /// JSX elements and fragments: each one calls the JSX factory, which the
    /// code does not name.
    jsx: Vec<Span>,
    /// `this` in a plain function (not a method of a class or of an object
    /// literal, not a class body): for an exported function called as a
    /// member of the module's namespace (`ns.f()`), `this` is the
    /// namespace, and through it the function reads any export.
    this: Vec<Span>,
    /// What `this` belongs to at the position of the walk: `true` for a plain
    /// function, `false` for a class or a method.
    owners: Vec<bool>,
    /// The function visited next is a method.
    method: bool,
}

impl<'a> Visit<'a> for Scan {
    fn visit_decorator(&mut self, it: &oxc_ast::ast::Decorator<'a>) {
        self.decorators = true;
        oxc_ast_visit::walk::walk_decorator(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if matches!(strip(&it.callee), Expression::Identifier(id) if id.name == "eval") {
            self.eval = true;
        }
        oxc_ast_visit::walk::walk_call_expression(self, it);
    }

    fn visit_jsx_element(&mut self, it: &oxc_ast::ast::JSXElement<'a>) {
        self.jsx.push(it.span);
        oxc_ast_visit::walk::walk_jsx_element(self, it);
    }

    fn visit_jsx_fragment(&mut self, it: &oxc_ast::ast::JSXFragment<'a>) {
        self.jsx.push(it.span);
        oxc_ast_visit::walk::walk_jsx_fragment(self, it);
    }

    fn visit_this_expression(&mut self, it: &oxc_ast::ast::ThisExpression) {
        if self.owners.last() == Some(&true) {
            self.this.push(it.span);
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        let method = std::mem::take(&mut self.method);
        self.owners.push(!method);
        oxc_ast_visit::walk::walk_function(self, it, flags);
        self.owners.pop();
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        self.owners.push(false);
        oxc_ast_visit::walk::walk_class(self, it);
        self.owners.pop();
    }

    fn visit_method_definition(&mut self, it: &oxc_ast::ast::MethodDefinition<'a>) {
        self.method = true;
        oxc_ast_visit::walk::walk_method_definition(self, it);
        self.method = false;
    }

    fn visit_object_property(&mut self, it: &oxc_ast::ast::ObjectProperty<'a>) {
        // `{ f() {} }` and `{ f: function () {} }`: called as `o.f()`, `this` is the
        // object.
        self.method = matches!(strip(&it.value), Expression::FunctionExpression(_));
        oxc_ast_visit::walk::walk_object_property(self, it);
        self.method = false;
    }
}

/// Whether a top-level import is declared again by the module (`import { f }
/// from './m'` next to `const f = 1`). TypeScript's binder lets that pass (a
/// type may share a name with an import; its checker reports the rest), so the
/// parser's own checks don't see it, but every transpiler either rejects the
/// file or emits JavaScript that does not load.
pub fn import_redeclared(scoping: &Scoping) -> bool {
    scoping.iter_bindings_in(scoping.root_scope_id()).any(|s| {
        scoping.symbol_flags(s).is_import()
            && scoping
                .symbol_redeclarations(s)
                .iter()
                .any(|r| r.flags.is_value())
    })
}

/// The names a transpiler's configuration gives the JSX factory and the
/// fragment when no pragma does (`jsxFactory: "h"`, `pragma: "createElement"`):
/// a binding of one of these names is read by every statement with JSX,
/// whatever else the file does with it.
const JSX_FACTORIES: &[&str] = &[
    "React",
    "h",
    "jsx",
    "jsxs",
    "jsxDEV",
    "createElement",
    "Fragment",
    "preact",
    "Preact",
];

/// The names a `@jsx` or `@jsxFrag` pragma of a comment gives the JSX factory
/// (`h` for `@jsx h`, `React` for `@jsx React.createElement`).
fn jsx_pragmas(comment: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut words = comment.split_whitespace();
    while let Some(w) = words.next() {
        if matches!(w, "@jsx" | "@jsxFrag")
            && let Some(name) = words.next()
        {
            out.push(name.split('.').next().unwrap_or(name));
        }
    }
    out
}

/// The members of an enum whose members all have literal initializers, or of a
/// `const` initialized with an object literal of plain keys: the span of the
/// body and each member's name and span. `None` for any other statement.
fn table_of(s: &Statement<'_>) -> Option<(Span, Vec<(String, Span)>)> {
    let d = match s {
        Statement::ExportDeclaration(e) => &e.declaration,
        _ => s.as_declaration()?,
    };
    match d {
        Declaration::TSEnumDeclaration(e) if !e.declare => {
            let mut members = Vec::new();
            for m in &e.body.members {
                let name = match &m.id {
                    oxc_ast::ast::TSEnumMemberName::Identifier(id) => id.name.to_string(),
                    oxc_ast::ast::TSEnumMemberName::String(s) => lit(s),
                    _ => return None,
                };
                // Without a literal initializer the value depends on the members before.
                let literal = match m.initializer.as_ref().map(strip) {
                    Some(Expression::StringLiteral(_) | Expression::NumericLiteral(_)) => true,
                    Some(Expression::UnaryExpression(u)) => {
                        matches!(u.argument, Expression::NumericLiteral(_))
                    }
                    _ => false,
                };
                if !literal {
                    return None;
                }
                members.push((name, m.span));
            }
            Some((e.body.span, members))
        }
        Declaration::VariableDeclaration(v) if v.kind.is_const() && v.declarations.len() == 1 => {
            let init = v.declarations[0].init.as_ref()?;
            let Expression::ObjectExpression(o) = strip(init) else {
                return None;
            };
            let mut members = Vec::new();
            for p in &o.properties {
                let ObjectPropertyKind::ObjectProperty(p) = p else {
                    return None;
                };
                if p.computed || !matches!(p.kind, PropertyKind::Init) {
                    return None;
                }
                let name = match &p.key {
                    PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                    PropertyKey::StringLiteral(s) => lit(s),
                    _ => return None,
                };
                // `__proto__: x` is not a member: it sets the prototype, whose properties
                // every member read can fall through to.
                if name == "__proto__" {
                    return None;
                }
                members.push((name, p.span));
            }
            Some((o.span, members))
        }
        _ => None,
    }
}

fn export_name(n: &ModuleExportName<'_>) -> String {
    n.name().to_string()
}

/// The value of a string literal (a module specifier).
fn lit(s: &StringLiteral<'_>) -> String {
    match s.value.as_str() {
        Some(v) => v.to_owned(),
        None => format!("{:?}", s.value),
    }
}

/// Substrings that make a comment a directive some tool reads.
pub const DIRECTIVES: &[&str] = &[
    "@jsx",
    "@jest",
    "@vitest",
    "@flow",
    "@noflow",
    "@ngInject",
    "@__",
    "#__",
    "istanbul",
    "c8 ignore",
    "v8 ignore",
    "node:coverage",
    "webpack",
    "turbopack",
    "@vite",
    "sourceMappingURL",
    "sourceURL",
    "@preserve",
    "@swc",
    "@babel",
    "@refresh",
    "<reference",
    "<amd-",
];

/// Names of test-runner objects whose mock calls replace modules behind the
/// import graph.
const MOCKERS: &[&str] = &["jest", "vi", "vitest"];

/// Names whose use means CommonJS exports or loads.
const COMMONJS: &[&str] = &["module", "exports", "require"];

impl Module {
    /// Reads the facts of `src` (the contents of `path`); the error says why
    /// there are none. `texts`: keep the statements' tokens (needed for
    /// [`diff`], not for [`Module::taint`]).
    pub fn read(path: &str, src: &str, texts: bool) -> Result<Module, &'static str> {
        let st = SourceType::from_path(path).map_err(|_| "not a JS or TS file")?;
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, st).parse();
        if ret.fatal_error || !ret.diagnostics.is_empty() || ret.is_flow_language {
            return Err("parse errors");
        }
        let program = ret.program;
        let sem = SemanticBuilder::new()
            .with_check_syntax_error(true)
            .with_build_nodes(true)
            .build(&program);
        if !sem.diagnostics.is_empty() {
            return Err("parse errors");
        }
        let semantic = sem.semantic;
        let scoping = semantic.scoping();
        for name in scoping.root_unresolved_references().keys() {
            let n = name.as_str();
            if COMMONJS.contains(&n) {
                return Err("CommonJS");
            }
            if MOCKERS.contains(&n) {
                return Err("test-runner mocks");
            }
        }
        if import_redeclared(scoping) {
            return Err("parse errors");
        }
        let mut scan = Scan::default();
        scan.visit_program(&program);
        // Code handed to `eval` reads and writes the bindings of the module by name.
        if scan.eval {
            return Err("a call of `eval`");
        }
        // A statement owns everything from the end of the statement before it:
        // decorators written before `export` are outside the span of the export
        // declaration.
        let mut spans: Vec<Span> = Vec::with_capacity(program.body.len());
        let mut end = 0;
        for st in &program.body {
            let span = st.span();
            spans.push(Span::new(end.min(span.start), span.end));
            end = span.end;
        }
        let root = scoping.root_scope_id();
        // Decorator metadata turns the types of decorated members into values.
        let types_are_values = scan.decorators;
        let mut index: HashMap<SymbolId, u32> = HashMap::new();
        let mut m = Module {
            stmts: vec![Stmt::default(); spans.len()],
            ..Module::default()
        };
        let mut symbols: Vec<SymbolId> = scoping.iter_bindings_in(root).collect();
        symbols.sort_unstable();
        let mut by_name: HashMap<&str, u32> = HashMap::new();
        // Per symbol: the statement that declares it, and whether it is uninitialized
        // before.
        let mut declared: HashMap<SymbolId, (u32, bool)> = HashMap::new();
        for s in &symbols {
            let b = m.bindings.len() as u32;
            index.insert(*s, b);
            by_name.insert(scoping.symbol_name(*s), b);
            let flags = scoping.symbol_flags(*s);
            let merged = !scoping.symbol_redeclarations(*s).is_empty();
            let kind = if flags.is_import() {
                Kind::Import
            } else if merged {
                Kind::Other
            } else if flags.is_class() {
                Kind::Class
            } else if flags.intersects(oxc_semantic::SymbolFlags::BlockScopedVariable) {
                Kind::Lexical
            } else if flags.is_function()
                && !flags.intersects(oxc_semantic::SymbolFlags::AsyncOrGeneratorFunction)
            {
                Kind::Function
            } else {
                Kind::Other
            };
            if let Some(at) = stmt_at(&spans, scoping.symbol_span(*s).start) {
                // A merged declaration counts as uninitialized when any part of it is.
                let lexical = flags.is_class()
                    || flags.intersects(oxc_semantic::SymbolFlags::BlockScopedVariable);
                declared.insert(*s, (at, lexical));
            }
            m.bindings.push(Binding {
                kind,
                import: None,
                pure: None,
                prim: false,
            });
        }
        // Declarations and references per statement.
        for (&s, &b) in &index {
            let mut decl = |span: Span| {
                if let Some(i) = stmt_at(&spans, span.start) {
                    let d = &mut m.stmts[i as usize].declares;
                    if !d.contains(&b) {
                        d.push(b);
                    }
                }
            };
            decl(scoping.symbol_span(s));
            for r in scoping.symbol_redeclarations(s) {
                decl(r.span);
            }
            for r in scoping.get_resolved_references(s) {
                let span = semantic.reference_span(r);
                let Some(i) = stmt_at(&spans, span.start) else {
                    continue;
                };
                let value = types_are_values || !r.flags().is_type_only();
                // `X.name`, read: a use of one member. `X.name(...)` is not: the call hands
                // the whole of `X` to the function as `this` (a method of the object, or
                // one it inherits such as `hasOwnProperty`).
                let nodes = semantic.nodes();
                let member = match nodes.parent_kind(r.node_id()) {
                    AstKind::StaticMemberExpression(m)
                        if m.object.span() == span
                            && !r.flags().is_write()
                            && !r.flags().is_member_write_target()
                            && !types_are_values =>
                    {
                        let mut at = nodes.parent_id(r.node_id());
                        let mut inner = m.span;
                        let called = loop {
                            let up = nodes.parent_id(at);
                            if up == at {
                                break false;
                            }
                            match nodes.kind(up) {
                                AstKind::ParenthesizedExpression(_)
                                | AstKind::TSNonNullExpression(_)
                                | AstKind::TSAsExpression(_)
                                | AstKind::TSSatisfiesExpression(_)
                                | AstKind::TSInstantiationExpression(_) => {
                                    inner = nodes.kind(up).span();
                                    at = up;
                                }
                                AstKind::CallExpression(c) => break c.callee.span() == inner,
                                AstKind::TaggedTemplateExpression(t) => {
                                    break t.tag.span() == inner;
                                }
                                _ => break false,
                            }
                        };
                        (!called).then(|| m.property.name.to_string())
                    }
                    _ => None,
                };
                let refs = &mut m.stmts[i as usize].refs;
                let at = match refs.iter().position(|x| x.binding == b) {
                    Some(at) => at,
                    None => {
                        refs.push(Ref {
                            binding: b,
                            value: false,
                            members: Some(Vec::new()),
                        });
                        refs.len() - 1
                    }
                };
                let x = &mut refs[at];
                if value {
                    x.value = true;
                    match (member, &mut x.members) {
                        (Some(name), Some(names)) => {
                            if !names.contains(&name) {
                                names.push(name);
                            }
                        }
                        _ => x.members = None,
                    }
                }
            }
        }
        // The JSX factory is named by a pragma or by the transpiler's configuration,
        // never by the code: every statement with JSX reads the bindings the
        // pragmas name, `React`, and every import nothing else in the file uses
        // (`import { h } from './h'`). Bindings that some statement of the
        // source reads as a value, before the reads that JSX stands for are
        // added.
        let mut written_use = vec![false; m.bindings.len()];
        for st in &m.stmts {
            for r in st.refs.iter().filter(|r| r.value) {
                written_use[r.binding as usize] = true;
            }
        }
        // Without a `@jsx` pragma the factory is a guess (see below).
        let mut guessed_factory = false;
        if !scan.jsx.is_empty() {
            let pragmas: Vec<&str> = program
                .comments
                .iter()
                .flat_map(|c| {
                    jsx_pragmas(
                        src.get(c.span.start as usize..c.span.end as usize)
                            .unwrap_or(""),
                    )
                })
                .collect();
            // A pragma says what the factory is. Without one the configuration does, and
            // the file does not: the usual names, and every import nothing else uses.
            guessed_factory = pragmas.is_empty();
            let mut factories: Vec<u32> = index
                .iter()
                .filter(|(s, _)| {
                    let name = scoping.symbol_name(**s);
                    if !guessed_factory {
                        return pragmas.contains(&name);
                    }
                    JSX_FACTORIES.contains(&name)
                        || (scoping.symbol_flags(**s).is_import()
                            && scoping.get_resolved_references(**s).next().is_none())
                })
                .map(|(_, b)| *b)
                .collect();
            factories.sort_unstable();
            for span in &scan.jsx {
                let Some(i) = stmt_at(&spans, span.start) else {
                    continue;
                };
                let refs = &mut m.stmts[i as usize].refs;
                for &b in &factories {
                    match refs.iter_mut().find(|x| x.binding == b) {
                        Some(x) => {
                            x.value = true;
                            x.members = None;
                        }
                        None => refs.push(Ref {
                            binding: b,
                            value: true,
                            members: None,
                        }),
                    }
                }
            }
        }
        // Imports, exports and evaluation, in source order.
        let mut eval = Eval {
            scoping,
            index: &index,
            declared: &declared,
            current: Cell::new(0),
            extends: RefCell::new(Vec::new()),
            immediate: RefCell::new(Vec::new()),
            prims: BTreeSet::new(),
            conds: RefCell::new(Vec::new()),
            params: RefCell::new(Vec::new()),
            spreads: Cell::new(false),
        };
        // Bindings whose calls only build a value.
        let pure_fn = |m: &mut Module,
                       id: Option<&oxc_ast::ast::BindingIdentifier<'_>>,
                       pure: Option<Pure>| {
            let Some(s) = id.and_then(|id| id.symbol_id.get()) else {
                return;
            };
            if pure.is_some()
                && !scoping.symbol_is_mutated(s)
                && scoping.symbol_redeclarations(s).is_empty()
                && let Some(&b) = index.get(&s)
            {
                m.bindings[b as usize].pure = pure;
            }
        };
        // Directive comments (JSX and test-environment pragmas, triple-slash
        // references): they change what the whole file means.
        for c in &program.comments {
            let text = src
                .get(c.span.start as usize..c.span.end as usize)
                .unwrap_or("");
            if DIRECTIVES.iter().any(|d| text.contains(d)) {
                m.directives.push(text.to_owned());
            }
        }
        for d in &program.directives {
            m.directives.push(d.directive.to_string());
        }
        // Bindings that some statement reads as a value.
        let mut value_used = vec![false; m.bindings.len()];
        for st in &m.stmts {
            for r in st.refs.iter().filter(|r| r.value) {
                value_used[r.binding as usize] = true;
            }
        }
        // Records an export name, in source order.
        fn export(m: &mut Module, name: String, def: Export) {
            if !m.exports.contains_key(&name) {
                m.order.push(name.clone());
            }
            m.exports.insert(name, def);
        }
        for (i, s) in program.body.iter().enumerate() {
            let i = i as u32;
            eval.current.set(i);
            let binding = |id: &oxc_ast::ast::BindingIdentifier<'_>| {
                id.symbol_id.get().and_then(|s| index.get(&s).copied())
            };
            match s {
                Statement::ImportDeclaration(d) => {
                    let all_types = d.import_kind == ImportOrExportKind::Type;
                    if matches!(lit(&d.source).as_str(), "vitest" | "@jest/globals")
                        && d.specifiers.iter().flatten().any(|x| {
                            let imported = match x {
                                ImportDeclarationSpecifier::ImportSpecifier(x) => {
                                    export_name(&x.imported)
                                }
                                _ => "vi".to_owned(),
                            };
                            MOCKERS.contains(&imported.as_str())
                        })
                    {
                        return Err("test-runner mocks");
                    }
                    let kept = |used: &[bool]| match &d.specifiers {
                        None => true,
                        Some(list) => list.iter().any(|sp| {
                            let local = match sp {
                                ImportDeclarationSpecifier::ImportSpecifier(x) => &x.local,
                                ImportDeclarationSpecifier::ImportDefaultSpecifier(x) => &x.local,
                                ImportDeclarationSpecifier::ImportNamespaceSpecifier(x) => &x.local,
                            };
                            binding(local).is_some_and(|b| used[b as usize])
                        }),
                    };
                    if kept(&value_used) && !all_types {
                        m.loads.push(lit(&d.source));
                    }
                    // Where the factory is a guess, an import that only the guess uses may
                    // be dropped after all: both readings are kept.
                    let written = if guessed_factory {
                        &written_use
                    } else {
                        &value_used
                    };
                    if kept(written) && !all_types {
                        m.written_loads.push(lit(&d.source));
                    }
                    for sp in d.specifiers.iter().flatten() {
                        let (local, imported, type_only) = match sp {
                            ImportDeclarationSpecifier::ImportSpecifier(x) => (
                                &x.local,
                                Imported::Name(export_name(&x.imported)),
                                all_types || x.import_kind == ImportOrExportKind::Type,
                            ),
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(x) => {
                                (&x.local, Imported::Default, all_types)
                            }
                            ImportDeclarationSpecifier::ImportNamespaceSpecifier(x) => {
                                (&x.local, Imported::Namespace, all_types)
                            }
                        };
                        if let Some(b) = binding(local) {
                            m.bindings[b as usize].import = Some(Import {
                                start: d.span.start,
                                spec: lit(&d.source),
                                imported,
                                type_only,
                            });
                        }
                    }
                }
                Statement::ExportAllDeclaration(d) => match &d.exported {
                    Some(name) => {
                        export(
                            &mut m,
                            export_name(name),
                            Export::From(format!("* of {:?}", lit(&d.source))),
                        );
                    }
                    None => m.stars.push(lit(&d.source)),
                },
                Statement::ExportFromDeclaration(d) => {
                    for sp in &d.specifiers {
                        export(
                            &mut m,
                            export_name(&sp.exported),
                            Export::From(format!(
                                "{} of {:?}",
                                export_name(&sp.local),
                                lit(&d.source)
                            )),
                        );
                    }
                }
                Statement::ExportNamedDeclaration(d) => {
                    for sp in &d.specifiers {
                        let local = export_name(&sp.local);
                        match by_name.get(local.as_str()).copied() {
                            Some(b) => {
                                export(&mut m, export_name(&sp.exported), Export::Binding(b));
                            }
                            None => return Err("an export of an undeclared name"),
                        }
                    }
                }
                Statement::ExportDeclaration(_) => {
                    for &b in &m.stmts[i as usize].declares.clone() {
                        let name = scoping.symbol_name(symbols[b as usize]).to_owned();
                        export(&mut m, name, Export::Binding(b));
                    }
                }
                Statement::ExportDefaultDeclaration(d) => {
                    let id = match &d.declaration {
                        ExportDefaultDeclarationKind::FunctionDeclaration(f) => f.id.as_ref(),
                        ExportDefaultDeclarationKind::ClassDeclaration(c) => c.id.as_ref(),
                        ExportDefaultDeclarationKind::TSInterfaceDeclaration(t) => Some(&t.id),
                        _ => None,
                    };
                    let def = match id.and_then(binding) {
                        Some(b) => Export::Binding(b),
                        None => Export::Stmt(i),
                    };
                    export(&mut m, "default".to_owned(), def);
                }
                Statement::TSExportAssignment(_) | Statement::TSNamespaceExportDeclaration(_) => {
                    return Err("`export =`");
                }
                Statement::TSImportEqualsDeclaration(_) => return Err("`import =`"),
                _ => {}
            }
            // Functions the statement declares.
            let declared = match s {
                Statement::ExportDeclaration(e) => Some(&e.declaration),
                _ => s.as_declaration(),
            };
            match declared {
                Some(Declaration::FunctionDeclaration(f)) => {
                    pure_fn(&mut m, f.id.as_ref(), eval.function_purity(f));
                }
                Some(Declaration::VariableDeclaration(v)) if v.kind.is_const() => {
                    for d in &v.declarations {
                        if let (BindingPattern::BindingIdentifier(id), Some(init)) =
                            (&d.id, &d.init)
                        {
                            pure_fn(&mut m, Some(id), eval.value_purity(init));
                        }
                    }
                }
                _ => {}
            }
            if let Statement::ExportDefaultDeclaration(d) = s
                && let ExportDefaultDeclarationKind::FunctionDeclaration(f) = &d.declaration
            {
                pure_fn(&mut m, f.id.as_ref(), eval.function_purity(f));
            }
            eval.conds.borrow_mut().clear();
            eval.extends.borrow_mut().clear();
            eval.immediate.borrow_mut().clear();
            let why = eval.statement(s);
            let conds = std::mem::take(&mut *eval.conds.borrow_mut());
            let mut extends = std::mem::take(&mut *eval.extends.borrow_mut());
            extends.sort_unstable();
            extends.dedup();
            let mut immediate = std::mem::take(&mut *eval.immediate.borrow_mut());
            immediate.sort_unstable();
            immediate.dedup();
            m.stmts[i as usize].extends = extends;
            m.stmts[i as usize].immediate = immediate;
            m.stmts[i as usize].evaluated = match (why, conds.first()) {
                (Some(w), _) => Some(Evaluated {
                    why: w.text(src),
                    unless: Vec::new(),
                }),
                (None, Some(first)) => Some(Evaluated {
                    why: match first.cond {
                        Cond::Builds { .. } => Why::Call(first.span).text(src),
                        Cond::Prim { .. } => {
                            "a template literal or operator over an imported value".to_owned()
                        }
                    },
                    unless: conds.into_iter().map(|p| p.cond).collect(),
                }),
                (None, None) => None,
            };
        }
        for (s, &b) in &index {
            m.bindings[b as usize].prim = eval.prims.contains(s);
        }
        // A function that reads `this` and that an export hands out (the export is the
        // function, a constant that holds it, or what a builder made of it) reads the
        // namespace it is called on: `ns.f()`.
        let mut through_this: Vec<u32> = scan
            .this
            .iter()
            .filter_map(|span| stmt_at(&spans, span.start))
            .collect();
        through_this.sort_unstable();
        through_this.dedup();
        let mut reached = vec![false; m.stmts.len()];
        if !through_this.is_empty() {
            let declared_in = m.declared_in();
            for e in m.exports.values() {
                let (stmts, _) = match e {
                    Export::Binding(b) => m.closure(&[], &[*b], &declared_in),
                    Export::Stmt(at) => m.closure(&[*at], &[], &declared_in),
                    Export::From(_) => continue,
                };
                for (r, inside) in reached.iter_mut().zip(stmts) {
                    *r |= inside;
                }
            }
        }
        for i in through_this {
            if !reached[i as usize] {
                continue;
            }
            m.reads_this = true;
            let mut all: Vec<u32> = Vec::new();
            for e in m.exports.values() {
                match e {
                    Export::Binding(b) => all.push(*b),
                    // Not a binding a statement can be said to read.
                    Export::Stmt(_) | Export::From(_) => {
                        return Err("an exported function that reads `this`");
                    }
                }
            }
            if !m.stars.is_empty() {
                return Err("an exported function that reads `this`");
            }
            let refs = &mut m.stmts[i as usize].refs;
            for b in all {
                match refs.iter_mut().find(|x| x.binding == b) {
                    Some(x) => {
                        x.value = true;
                        x.members = None;
                    }
                    None => refs.push(Ref {
                        binding: b,
                        value: true,
                        members: None,
                    }),
                }
            }
        }
        if texts {
            let toks = tokens(path, src).ok_or("not tokenizable")?;
            // The lexer skims JSX: an attribute name, an attribute string and JSX text are
            // in no token, so a change of markup alone would compare equal. The
            // source of the outermost JSX elements of a span is part of its
            // text, byte for byte: white space inside an attribute string or
            // between two words of JSX text is output.
            let mut outer: Vec<Span> = Vec::new();
            let mut all = scan.jsx.clone();
            all.sort_unstable_by_key(|s| (s.start, std::cmp::Reverse(s.end)));
            let mut end = 0u32;
            for s in all {
                if s.start >= end {
                    end = s.end;
                    outer.push(s);
                }
            }
            let markup = |within: Span| -> String {
                let mut out = String::new();
                for s in outer
                    .iter()
                    .filter(|s| s.start >= within.start && s.end <= within.end)
                {
                    out.push('\u{4}');
                    out.push_str(src.get(s.start as usize..s.end as usize).unwrap_or(""));
                }
                out
            };
            for (i, span) in spans.iter().enumerate() {
                m.stmts[i].text = span_text(src, &toks, *span);
                m.stmts[i].text.push_str(&markup(*span));
                // An enum or an object table: the statement around its members, and each
                // member.
                if let Some((body, members)) = table_of(&program.body[i])
                    && m.stmts[i].evaluated.is_none()
                {
                    let mut shell = span_text(src, &toks, Span::new(span.start, body.start));
                    shell.push_str(" {…} ");
                    shell.push_str(&span_text(src, &toks, Span::new(body.end, span.end)));
                    m.stmts[i].table = Some(Table {
                        shell,
                        members: members
                            .into_iter()
                            .map(|(name, span)| {
                                let mut text = span_text(src, &toks, span);
                                text.push_str(&markup(span));
                                (name, text)
                            })
                            .collect(),
                    });
                }
            }
        }
        for s in &mut m.stmts {
            s.declares.sort_unstable();
            for r in &mut s.refs {
                if let Some(names) = &mut r.members {
                    names.sort_unstable();
                }
            }
            s.refs.sort_unstable();
        }
        Ok(m)
    }

    /// Statements that declare each binding.
    fn declared_in(&self) -> Vec<Vec<u32>> {
        let mut out = vec![Vec::new(); self.bindings.len()];
        for (i, s) in self.stmts.iter().enumerate() {
            for &b in &s.declares {
                out[b as usize].push(i as u32);
            }
        }
        out
    }

    /// The canonical text of the definition closure of `roots` (statements) and
    /// `bindings`: the closure's statements in source order, then the
    /// imports it refers to.
    fn closure_text(&self, roots: &[u32], bindings: &[u32], declared_in: &[Vec<u32>]) -> String {
        self.closure_text_with(roots, bindings, declared_in, None)
    }

    /// [`Module::closure_text`]; the table statement `shell` contributes only
    /// what stands around its members.
    fn closure_text_with(
        &self,
        roots: &[u32],
        bindings: &[u32],
        declared_in: &[Vec<u32>],
        shell: Option<u32>,
    ) -> String {
        let (in_closure, imports) = self.closure(roots, bindings, declared_in);
        let mut out = String::new();
        for (i, s) in self.stmts.iter().enumerate() {
            if in_closure[i] {
                match (&s.table, shell) {
                    (Some(t), Some(at)) if at as usize == i => out.push_str(&t.shell),
                    _ => out.push_str(&s.text),
                }
                out.push('\u{0}');
            }
        }
        for i in imports {
            out.push_str(&i);
            out.push('\u{0}');
        }
        out
    }

    /// The definition closure of `roots` (statements) and `bindings`: the
    /// statements in it, and the imports it refers to.
    fn closure(
        &self,
        roots: &[u32],
        bindings: &[u32],
        declared_in: &[Vec<u32>],
    ) -> (Vec<bool>, BTreeSet<String>) {
        let mut in_closure = vec![false; self.stmts.len()];
        let mut seen = vec![false; self.bindings.len()];
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut work: Vec<u32> = Vec::new();
        let mut add_binding = |b: u32, work: &mut Vec<u32>, seen: &mut Vec<bool>| {
            if std::mem::replace(&mut seen[b as usize], true) {
                return;
            }
            match &self.bindings[b as usize].import {
                Some(i) => {
                    imports.insert(format!(
                        "import {:?} of {:?}{}",
                        i.imported,
                        i.spec,
                        if i.type_only { " (type)" } else { "" }
                    ));
                }
                None => work.extend(declared_in[b as usize].iter().copied()),
            }
        };
        for &b in bindings {
            add_binding(b, &mut work, &mut seen);
        }
        work.extend(roots.iter().copied());
        while let Some(i) = work.pop() {
            if std::mem::replace(&mut in_closure[i as usize], true) {
                continue;
            }
            for r in &self.stmts[i as usize].refs {
                add_binding(r.binding, &mut work, &mut seen);
            }
        }
        (in_closure, imports)
    }

    /// The canonical definition of export `name`.
    fn export_text(&self, name: &str, declared_in: &[Vec<u32>]) -> Option<String> {
        Some(match self.exports.get(name)? {
            Export::Binding(b) => self.closure_text(&[], &[*b], declared_in),
            Export::Stmt(i) => self.closure_text(&[*i], &[], declared_in),
            Export::From(fact) => format!("from {fact}"),
        })
    }

    /// For an export that is an enum or an object table declared by one
    /// statement: that statement, its members, and the export's definition
    /// without the members.
    fn export_table(
        &self,
        name: &str,
        declared_in: &[Vec<u32>],
    ) -> Option<(&[(String, String)], String)> {
        let Some(Export::Binding(b)) = self.exports.get(name) else {
            return None;
        };
        let [at] = declared_in[*b as usize].as_slice() else {
            return None;
        };
        let table = self.stmts[*at as usize].table.as_ref()?;
        // A member that reads the table itself (`g: () => T.a`, or a helper that does)
        // changes with the other members.
        let (in_closure, _) = self.closure(&[], &[*b], declared_in);
        if self
            .stmts
            .iter()
            .zip(&in_closure)
            .any(|(s, inside)| *inside && s.refs.iter().any(|r| r.binding == *b))
        {
            return None;
        }
        Some((
            &table.members,
            self.closure_text_with(&[], &[*b], declared_in, Some(*at)),
        ))
    }

    /// The canonical text of everything the module does at load time; `load`:
    /// the statements that are load-time code ([`Module::load_time`]).
    ///
    /// Statements that are not load-time code still do two things when the
    /// module is loaded, and both are part of the text:
    ///
    /// - a class definition reads what it extends and throws when that is not a
    ///   constructor: the imports and the bindings (other than classes and
    ///   plain functions of the module, which always are constructors) that
    ///   classes extend;
    /// - `cyclic` (the module is part of an import cycle, so an import can be
    ///   uninitialized while the module is loaded): the statements that read an
    ///   import at load time;
    /// - a read of an import that may be a getter ([`Module::getter_reads`]).
    fn evaluation_text(
        &self,
        declared_in: &[Vec<u32>],
        load: &[bool],
        cyclic: bool,
        oracle: &dyn Oracle,
    ) -> String {
        let roots: Vec<u32> = (0..self.stmts.len() as u32)
            .filter(|i| load[*i as usize])
            .collect();
        let mut out = self.directives.join("\u{0}");
        out.push('\u{1}');
        out.push_str(&self.closure_text(&roots, &[], declared_in));
        let mut bases: BTreeSet<String> = BTreeSet::new();
        for (s, _) in self.stmts.iter().zip(load).filter(|(_, l)| !**l) {
            for &b in &s.extends {
                let binding = &self.bindings[b as usize];
                if !matches!(binding.kind, Kind::Class | Kind::Function) {
                    bases.insert(self.closure_text(&[], &[b], declared_in));
                }
            }
        }
        for b in bases {
            out.push('\u{2}');
            out.push_str(&b);
        }
        if cyclic {
            for (s, _) in self
                .stmts
                .iter()
                .zip(load)
                .filter(|(s, l)| !**l && !s.immediate.is_empty())
            {
                out.push('\u{3}');
                out.push_str(&s.text);
                for &b in &s.immediate {
                    out.push_str(&self.closure_text(&[], &[b], declared_in));
                }
            }
        }
        for r in self.getter_reads(load, oracle) {
            out.push('\u{5}');
            out.push_str(&r);
        }
        out
    }

    /// The imports that statements other than load-time code read when the
    /// module is loaded (`const table = { a, b }`, the callee of a chain
    /// that only builds a value), and that do not come from an ES module of
    /// the repository. Where modules are transpiled to CommonJS, reading an
    /// import is reading a property of the module object: a CommonJS module
    /// or a package may define it with a getter (a lazy `require`, a getter
    /// that someone wrote), which then runs when this module loads.
    /// Which of them a module reads is therefore part of what it does at load
    /// time.
    fn getter_reads(&self, load: &[bool], oracle: &dyn Oracle) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for (s, _) in self.stmts.iter().zip(load).filter(|(_, l)| !**l) {
            for &b in &s.immediate {
                if let Some(i) = &self.bindings[b as usize].import
                    && !oracle.esm(Self::callee(i))
                {
                    out.insert(format!("{:?} of {:?}", i.imported, i.spec));
                }
            }
        }
        out
    }

    /// Whether loading the module runs code beyond declarations: the first
    /// reason, chains that may turn out to only build a value included.
    #[cfg(test)]
    fn evaluated(&self) -> Option<&str> {
        self.stmts
            .iter()
            .find_map(|s| s.evaluated.as_ref().map(|e| e.why.as_str()))
    }

    fn callee<'a>(i: &'a Import) -> Callee<'a> {
        Callee {
            spec: &i.spec,
            name: match &i.imported {
                Imported::Name(n) => n,
                Imported::Default => "default",
                Imported::Namespace => "*",
            },
        }
    }

    /// Whether `c` holds. `tainted`: bindings that carry a change (a condition
    /// on one of them does not hold: what changed may no longer build a
    /// value the same way).
    fn holds(&self, c: &Cond, tainted: Option<&[bool]>, oracle: &dyn Oracle, fuel: u8) -> bool {
        let changed = |b: u32| tainted.is_some_and(|t| t[b as usize]);
        match c {
            Cond::Prim { binding } => {
                let b = &self.bindings[*binding as usize];
                !changed(*binding)
                    && match &b.import {
                        None => b.prim,
                        Some(i) => oracle.prim(Self::callee(i)),
                    }
            }
            Cond::Builds {
                root,
                steps,
                literal_args,
                arg,
            } => {
                !changed(*root)
                    && match &self.bindings[*root as usize].import {
                        None => self.local_builds(*root, steps, *literal_args, oracle, fuel),
                        Some(i) => match arg {
                            None => oracle.builds(Self::callee(i), steps, *literal_args),
                            Some(props) => {
                                let props: Vec<(String, Prop<'_>)> = props
                                    .iter()
                                    .map(|(name, fact)| {
                                        let prop = match fact {
                                            PropFact::Function => Prop::Function,
                                            PropFact::Spread => Prop::Spread,
                                            PropFact::Other => Prop::Other,
                                            PropFact::Chain { root, steps } => {
                                                match self.chain_on_import(*root, steps, FUEL) {
                                                    Some((i, all)) => {
                                                        Prop::Chain(Self::callee(i), all)
                                                    }
                                                    None => Prop::Other,
                                                }
                                            }
                                        };
                                        (name.clone(), prop)
                                    })
                                    .collect();
                                oracle.builds_with(Self::callee(i), steps, *literal_args, &props)
                            }
                        },
                    }
            }
        }
    }

    /// Whether the chain `steps` on the local binding `b` only builds a value.
    fn local_builds(
        &self,
        b: u32,
        steps: &[Step],
        literal_args: bool,
        oracle: &dyn Oracle,
        fuel: u8,
    ) -> bool {
        if fuel == 0 {
            return false;
        }
        match &self.bindings[b as usize].pure {
            None => false,
            Some(Pure::Function {
                depth,
                spreads,
                needs,
            }) => {
                steps.len() <= usize::from(*depth)
                    && steps.iter().all(|s| *s == Step::Call)
                    && (!spreads || literal_args)
                    && needs.iter().all(|n| self.holds(n, None, oracle, fuel - 1))
            }
            // The chain goes on from the value an import's chain made.
            Some(Pure::Alias { root, steps: first })
                if self.bindings[*root as usize].import.is_some() =>
            {
                let import = self.bindings[*root as usize].import.as_ref();
                import.is_some_and(|i| oracle.continues(Self::callee(i), first, steps))
            }
            Some(Pure::Alias { root, steps: first }) => {
                let all: Vec<Step> = first.iter().chain(steps).cloned().collect();
                // The literal-ness of the first calls' arguments is not known here.
                self.holds(
                    &Cond::Builds {
                        root: *root,
                        steps: all,
                        literal_args: false,
                        arg: None,
                    },
                    None,
                    oracle,
                    fuel - 1,
                )
            }
        }
    }

    /// The chain `steps` on binding `b` as a chain on an import: `b` is an
    /// import, or a constant that holds the result of a chain on one
    /// (through other such constants).
    fn chain_on_import(&self, b: u32, steps: &[Step], fuel: u8) -> Option<(&Import, Vec<Step>)> {
        let binding = &self.bindings[b as usize];
        if let Some(i) = &binding.import {
            return Some((i, steps.to_vec()));
        }
        match &binding.pure {
            Some(Pure::Alias { root, steps: first }) if fuel > 0 => {
                let all: Vec<Step> = first.iter().chain(steps).cloned().collect();
                self.chain_on_import(*root, &all, fuel - 1)
            }
            _ => None,
        }
    }

    /// The chain `steps` on export `name` as a chain on an import of this
    /// module (specifier, imported name, steps): the export is a constant
    /// that holds the result of such a chain.
    pub fn export_chain(&self, name: &str, steps: &[Step]) -> Option<(String, String, Vec<Step>)> {
        let Some(Export::Binding(b)) = self.exports.get(name) else {
            return None;
        };
        if self.bindings[*b as usize].import.is_some() {
            return None;
        }
        let (i, all) = self.chain_on_import(*b, steps, FUEL)?;
        let c = Self::callee(i);
        Some((c.spec.to_owned(), c.name.to_owned(), all))
    }

    /// Whether the chain `steps` on export `name` only builds a value (see
    /// "Calls that only build a value"): the export is a function of this
    /// file with such a body, a constant holding the result of such a
    /// chain, or a re-exported import that `oracle` vouches for.
    pub fn builds(
        &self,
        name: &str,
        steps: &[Step],
        literal_args: bool,
        oracle: &dyn Oracle,
    ) -> bool {
        match self.exports.get(name) {
            Some(Export::Binding(b)) => self.holds(
                &Cond::Builds {
                    root: *b,
                    steps: steps.to_vec(),
                    literal_args,
                    arg: None,
                },
                None,
                oracle,
                FUEL,
            ),
            _ => false,
        }
    }

    /// Whether an export hands out a function that reads `this` (the namespace
    /// it is called on).
    pub fn reads_this(&self) -> bool {
        self.reads_this
    }

    /// Whether export `name` is a constant of this file with a primitive value.
    pub fn prim(&self, name: &str) -> bool {
        match self.exports.get(name) {
            Some(Export::Binding(b)) => self.bindings[*b as usize].prim,
            _ => false,
        }
    }

    /// Per statement: whether it is load-time code, given what `oracle` knows
    /// about the chains and constants it depends on. `cyclic`: the module
    /// is part of an import cycle; a chain then counts as load-time code
    /// (what it calls may not be initialized).
    fn load_time(&self, oracle: &dyn Oracle, cyclic: bool) -> Vec<bool> {
        self.stmts
            .iter()
            .map(|s| {
                s.evaluated.as_ref().is_some_and(|e| {
                    cyclic
                        || e.unless.is_empty()
                        || !e.unless.iter().all(|c| self.holds(c, None, oracle, FUEL))
                })
            })
            .collect()
    }

    /// What the module derives from tainted imports. `imports`: the `import`
    /// declarations (by the byte offset of their keyword) of modules with
    /// changed exports, each with the changed names (`None`: every name).
    /// `None` when one of the imports is not an import declaration of this
    /// module (a `require`, a re-export): the caller cannot narrow.
    #[cfg(test)]
    fn taint(
        &self,
        imports: &[(u32, Option<&BTreeSet<String>>)],
        oracle: &dyn Oracle,
    ) -> Option<Taint> {
        let lists: Vec<Option<Vec<String>>> = imports
            .iter()
            .map(|(_, names)| names.map(|n| n.iter().cloned().collect()))
            .collect();
        let asked: Vec<Asked<'_>> = imports
            .iter()
            .zip(&lists)
            .map(|((start, _), names)| Asked {
                start: *start,
                names: names.as_deref(),
                rebound: &[],
            })
            .collect();
        self.taint_members(&asked, &BTreeMap::new(), oracle)
    }

    /// [`Module::taint`] with member-level changes: `members` maps a changed
    /// name to its changed members ([`Changed::members`]); an import of
    /// such a name is tainted only in the statements that read a changed
    /// member or use the import as a whole.
    ///
    /// Two uses of a tainted import count as load-time code whatever the
    /// statement is: a class definition that extends it (the definition
    /// throws when the value is no longer a constructor), and an import by
    /// name of a binding that was added, removed or redeclared
    /// ([`Asked::rebound`]: linking the module can fail, and in an import cycle
    /// the binding can be uninitialized where it was a hoisted function
    /// before).
    pub fn taint_members(
        &self,
        imports: &[Asked<'_>],
        members: &BTreeMap<String, Vec<String>>,
        oracle: &dyn Oracle,
    ) -> Option<Taint> {
        let mut tainted = vec![false; self.bindings.len()];
        // Per binding: the changed members, when only those carry the change.
        let mut only: Vec<Option<&Vec<String>>> = vec![None; self.bindings.len()];
        let mut relinked = false;
        for Asked {
            start,
            names,
            rebound,
        } in imports
        {
            let mut found = false;
            for (b, binding) in self.bindings.iter().enumerate() {
                let Some(i) = binding.import.as_ref().filter(|i| i.start == *start) else {
                    continue;
                };
                found = true;
                let hit = match (&i.imported, names) {
                    (_, None) | (Imported::Namespace, _) => true,
                    (Imported::Name(n), Some(names)) => names.contains(n),
                    (Imported::Default, Some(names)) => names.iter().any(|n| n == "default"),
                };
                relinked |= !i.type_only
                    && match &i.imported {
                        Imported::Name(n) => rebound.contains(n),
                        Imported::Default => rebound.iter().any(|r| r == "default"),
                        Imported::Namespace => false,
                    };
                if hit && !tainted[b] {
                    tainted[b] = true;
                    only[b] = match &i.imported {
                        Imported::Name(n) if names.is_some() => members.get(n),
                        _ => None,
                    };
                }
            }
            if !found {
                // `import 'x'` binds nothing and reads nothing; anything else is not an
                // import declaration this module knows.
                return None;
            }
        }
        // Whether the statement's use of a tainted binding reads the change.
        let reads =
            |r: &Ref, only: &[Option<&Vec<String>>]| match (only[r.binding as usize], &r.members) {
                (Some(changed), Some(used)) => used.iter().any(|m| changed.contains(m)),
                _ => true,
            };
        let mut stmt_tainted = vec![false; self.stmts.len()];
        let mut changed = true;
        while changed {
            changed = false;
            for (i, s) in self.stmts.iter().enumerate() {
                if stmt_tainted[i]
                    || !s
                        .refs
                        .iter()
                        .any(|r| r.value && tainted[r.binding as usize] && reads(r, &only))
                {
                    continue;
                }
                stmt_tainted[i] = true;
                changed = true;
                for &b in &s.declares {
                    if self.bindings[b as usize].import.is_none() {
                        tainted[b as usize] = true;
                    }
                }
            }
        }
        // A chain that only builds a value is not load-time code, unless its root is
        // what changed.
        let mut pure_calls = 0;
        let mut evaluation = relinked
            .then(|| "an import of a name that was added, removed or redeclared".to_owned());
        for (s, _) in self.stmts.iter().zip(&stmt_tainted).filter(|(_, t)| **t) {
            // A class that extends a tainted value, other than a class or a plain function
            // of this module (those stay constructors whatever they read).
            if evaluation.is_none()
                && s.extends.iter().any(|&b| {
                    tainted[b as usize]
                        && !matches!(self.bindings[b as usize].kind, Kind::Class | Kind::Function)
                })
            {
                evaluation = Some("a class that extends a changed value".to_owned());
            }
            let Some(e) = &s.evaluated else { continue };
            if !e.unless.is_empty()
                && e.unless
                    .iter()
                    .all(|c| self.holds(c, Some(&tainted), oracle, FUEL))
            {
                pure_calls += 1;
            } else if evaluation.is_none() {
                evaluation = Some(e.why.clone());
            }
        }
        let exports = self
            .exports
            .iter()
            .filter(|(_, def)| match def {
                Export::Binding(b) => tainted[*b as usize],
                Export::Stmt(i) => stmt_tainted[*i as usize],
                Export::From(_) => false,
            })
            .map(|(name, _)| name.clone())
            .collect();
        Some(Taint {
            evaluation,
            exports,
            pure_calls,
            any: relinked || stmt_tainted.contains(&true),
        })
    }
}

/// What changed between the `base` and `head` versions of a module (both read
/// with texts). The oracles answer for the imports of the respective version.
/// `cyclic`: the module is part of an import cycle (or may be): a statement
/// that reads an import at load time, and every call, is then compared as
/// load-time code.
pub fn diff(
    base: &Module,
    head: &Module,
    at_base: &dyn Oracle,
    at_head: &dyn Oracle,
    cyclic: bool,
) -> Changed {
    let (db, dh) = (base.declared_in(), head.declared_in());
    let (lb, lh) = (
        base.load_time(at_base, cyclic),
        head.load_time(at_head, cyclic),
    );
    let evaluation = base.evaluation_text(&db, &lb, cyclic, at_base)
        != head.evaluation_text(&dh, &lh, cyclic, at_head)
        || base.stars != head.stars
        || base.loads != head.loads
        || base.written_loads != head.written_loads;
    let load_time = evaluation.then(|| {
        let texts = |m: &'_ Module, load: &[bool]| -> Vec<(String, String)> {
            m.stmts
                .iter()
                .zip(load)
                .filter(|(_, l)| **l)
                .filter_map(|(s, _)| {
                    s.evaluated
                        .as_ref()
                        .map(|e| (s.text.clone(), e.why.clone()))
                })
                .collect()
        };
        let (b, h) = (texts(base, &lb), texts(head, &lh));
        if let Some((_, why)) = h.iter().find(|(t, _)| !b.iter().any(|(x, _)| x == t)) {
            format!("{why} is new or changed")
        } else if let Some((_, why)) = b.iter().find(|(t, _)| !h.iter().any(|(x, _)| x == t)) {
            format!("{why} was removed")
        } else if base.directives != head.directives {
            "the directives differ".to_owned()
        } else if base.stars != head.stars {
            "the `export *` list differs".to_owned()
        } else if base.loads != head.loads || base.written_loads != head.written_loads {
            "an import is used as a value at one revision only (a transpiler drops an unused \
             import)"
                .to_owned()
        } else if base.getter_reads(&lb, at_base) != head.getter_reads(&lh, at_head) {
            "an import of a package or a CommonJS module is read when the file is loaded at one \
             revision only (it may be a getter)"
                .to_owned()
        } else {
            "a declaration that load-time code uses changed".to_owned()
        }
    });
    let names: BTreeSet<&String> = base.exports.keys().chain(head.exports.keys()).collect();
    let exports: Vec<String> = names
        .into_iter()
        .filter(|n| base.export_text(n, &db) != head.export_text(n, &dh))
        .cloned()
        .collect();
    // Enums and object tables that differ only in their members.
    let mut members = BTreeMap::new();
    for n in &exports {
        let (Some((mb, rest_b)), Some((mh, rest_h))) =
            (base.export_table(n, &db), head.export_table(n, &dh))
        else {
            continue;
        };
        if rest_b != rest_h {
            continue;
        }
        let text = |m: &[(String, String)], name: &str| {
            let mut it = m.iter().filter(|x| x.0 == name);
            // A duplicate key is not a table.
            (it.next().map(|x| x.1.clone()), it.next().is_some())
        };
        let mut changed: Vec<String> = Vec::new();
        let mut plain = true;
        for (name, _) in mb.iter().chain(mh) {
            let ((b, dup_b), (h, dup_h)) = (text(mb, name), text(mh, name));
            plain &= !dup_b && !dup_h;
            if b != h && !changed.contains(name) {
                changed.push(name.clone());
            }
        }
        if plain {
            changed.sort_unstable();
            members.insert(n.clone(), changed);
        }
    }
    // Names whose binding changed: at one version only, or declared in another way.
    let kind = |m: &Module, n: &str| {
        m.exports.get(n).map(|e| match e {
            Export::Binding(b) => Some(m.bindings[*b as usize].kind),
            Export::Stmt(_) | Export::From(_) => None,
        })
    };
    let from = |m: &Module, n: &str| matches!(m.exports.get(n), Some(Export::From(_)));
    let rebound: Vec<String> = exports
        .iter()
        .filter(|n| kind(base, n) != kind(head, n) || from(base, n) != from(head, n))
        .cloned()
        .collect();
    // The order of the exports both versions have.
    let shared = |m: &Module, other: &Module| -> Vec<String> {
        m.order
            .iter()
            .filter(|n| other.exports.contains_key(*n))
            .cloned()
            .collect()
    };
    let reordered = shared(base, head) != shared(head, base);
    Changed {
        evaluation,
        load_time,
        exports,
        members,
        rebound,
        reordered,
    }
}

/// [`diff`] of two sources of `path`; the error says which version has no facts
/// and why.
pub fn changed(
    path: &str,
    base: &str,
    head: &str,
    at_base: &dyn Oracle,
    at_head: &dyn Oracle,
    cyclic: bool,
) -> Result<Changed, String> {
    let h = Module::read(path, head, true).map_err(|e| format!("head: {e}"))?;
    let b = Module::read(path, base, true).map_err(|e| format!("base: {e}"))?;
    Ok(diff(&b, &h, at_base, at_head, cyclic))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(base: &str, head: &str) -> Changed {
        changed("a.ts", base, head, &Alone, &Alone, false).expect("facts")
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    /// One import declaration with these changed names.
    fn ask(start: u32, names: &[String]) -> [Asked<'_>; 1] {
        [Asked {
            start,
            names: Some(names),
            rebound: &[],
        }]
    }

    /// The shape of the real site (a utility module of a tracking package): one
    /// unused export and its import are removed, the other exports are
    /// untouched.
    #[test]
    fn a_removed_export_changes_only_itself() {
        let base = "import { COOKIES, PARAMS } from './constants';\nimport type { CookieKey, \
                    ParamKey, UtmKey } from './types';\nexport const isParam = (p: string): p is \
                    ParamKey => p in PARAMS;\nexport const isUtm = (p: string): p is UtmKey => \
                    p.startsWith('utm_');\n/** doc */\nexport const isCookie = (k: string): k is \
                    CookieKey => k in COOKIES;\nexport function hasSignal(t: Record<string, \
                    string>): boolean {\n  return Object.keys(t).some((k) => isUtm(k) || \
                    isParam(k));\n}\n";
        let head = "import { PARAMS } from './constants';\nimport type { ParamKey, UtmKey } from \
                    './types';\nexport const isParam = (p: string): p is ParamKey => p in \
                    PARAMS;\nexport const isUtm = (p: string): p is UtmKey => \
                    p.startsWith('utm_');\nexport function hasSignal(t: Record<string, string>): \
                    boolean {\n  return Object.keys(t).some((k) => isUtm(k) || isParam(k));\n}\n";
        assert_eq!(
            ch(base, head),
            Changed {
                evaluation: false,
                load_time: None,
                exports: names(&["isCookie"]),
                members: BTreeMap::new(),
                rebound: names(&["isCookie"]),
                reordered: false,
            }
        );
    }

    #[test]
    fn a_change_reaches_the_exports_that_refer_to_it() {
        let base = "const LIMIT = 3;\nfunction clamp(n: number) { return Math.min(n, LIMIT); \
                    }\nexport function a(n: number) { return clamp(n); }\nexport function b(n: \
                    number) { return n + 1; }\nexport const c = { a };\nexport default b;\n";
        // A helper's body.
        let head = base.replace("Math.min", "Math.max");
        assert_eq!(ch(base, &head).exports, names(&["a", "c"]));
        // A constant two steps away.
        let head = base.replace("LIMIT = 3", "LIMIT = 4");
        assert_eq!(ch(base, &head).exports, names(&["a", "c"]));
        // The export itself; `default` is the same binding.
        let head = base.replace("n + 1", "n + 2");
        assert_eq!(ch(base, &head).exports, names(&["b", "default"]));
        // Comments, blank lines and spacing inside a line are not changes.
        let head = base.replace("const LIMIT = 3;", "// the limit\n\nconst  LIMIT  =  3;");
        assert_eq!(ch(base, &head), Changed::default());
        // A type that an export's signature names.
        let base2 =
            "type N = number;\nexport function a(n: N) { return n; }\nexport const k = 1;\n";
        let head2 = base2.replace("type N = number", "type N = number | string");
        assert_eq!(ch(base2, &head2).exports, names(&["a"]));
    }

    #[test]
    fn an_import_is_compared_by_where_it_comes_from() {
        let base =
            "import { x, y } from './m';\nexport const a = () => x;\nexport const b = () => y;\n";
        // Another name is added to the declaration: nothing that exists changed.
        let head = "import { x, y, z } from './m';\nexport const a = () => x;\nexport const b = \
                    () => y;\nexport const c = () => z;\n";
        assert_eq!(ch(base, head).exports, names(&["c"]));
        // The same local name now comes from another module.
        let head = "import { y } from './m';\nimport { x } from './n';\nexport const a = () => \
                    x;\nexport const b = () => y;\n";
        assert_eq!(ch(base, head).exports, names(&["a"]));
        // ... or is another export of the same module.
        let head = "import { w as x, y } from './m';\nexport const a = () => x;\nexport const b = \
                    () => y;\n";
        assert_eq!(ch(base, head).exports, names(&["a"]));
    }

    #[test]
    fn load_time_code_is_compared_with_what_it_reads() {
        let base = "const KEY = lookup('k');\nfunction make() { return create(); }\nconst cache = \
                    make();\nexport function get(k: string) { return cache.get(k) ?? KEY; \
                    }\nexport function other() { return 1; }\n";
        // Only an export's body: load-time code is the same.
        let head = base.replace("return 1", "return 2");
        assert_eq!(
            ch(base, &head),
            Changed {
                evaluation: false,
                load_time: None,
                exports: names(&["other"]),
                members: BTreeMap::new(),
                ..Changed::default()
            }
        );
        // A function that load-time code calls.
        let head = base.replace("create()", "create(1)");
        let c = ch(base, &head);
        assert!(c.evaluation);
        assert_eq!(
            c.load_time.as_deref(),
            Some("a declaration that load-time code uses changed")
        );
        // The load-time statement itself.
        let head = base.replace("'k'", "'k2'");
        let c = ch(base, &head);
        assert!(c.evaluation);
        assert_eq!(
            c.load_time.as_deref(),
            Some("a call of lookup is new or changed")
        );
        assert_eq!(c.exports, names(&["get"]));
        // A new load-time statement.
        let head = format!("{base}console.log('loaded');\n");
        assert_eq!(
            ch(base, &head).load_time.as_deref(),
            Some("a call of console.log is new or changed")
        );
        assert_eq!(
            ch(&head, base).load_time.as_deref(),
            Some("a call of console.log was removed")
        );
        // A directive, and a directive comment (an ordinary comment is not a change).
        let head = format!("'use client';\n{base}");
        assert!(ch(base, &head).evaluation);
        let head = format!("/** @jest-environment jsdom */\n{base}");
        assert_eq!(
            ch(base, &head).load_time.as_deref(),
            Some("the directives differ")
        );
        let head = format!("/** A registry. */\n{base}");
        assert_eq!(ch(base, &head), Changed::default());
    }

    #[test]
    fn what_counts_as_evaluated() {
        let ev = |src: &str| {
            Module::read("a.tsx", src, false)
                .expect("facts")
                .evaluated()
                .map(str::to_owned)
        };
        for inert in [
            "export const a = 1, b = 'x', c = `t${a}`, d = -a, e = a + 1;",
            "const o = { a: 1, f() {}, g: () => call(), ['k']: [1, 2, { z: null }] };",
            "export function f() { return g(); }\nexport class K { static x = 1; y = h(); m() {} }",
            "import { x } from './m';\nexport const table = { x, y: x };\nexport default x;",
            "export const C = () => <div />;\nexport type T = { a: string };\nexport enum E { A, \
             B = 2 }",
            "const cond = 1, other = null;\nexport const v = cond ? 1 : 2, w = other || null, n = \
             !other, t = typeof other;",
            "export const t = typeof window, u = typeof window === 'undefined' ? 1 : 2;",
            "class Base {}\nfunction Old() {}\nexport class A extends Base {}\nexport class B \
             extends Old {}\nexport class E extends Error {}\nexport class N extends null {}",
            "declare const g: number;\nexport abstract class A {}\nexport interface I {}",
        ] {
            assert_eq!(ev(inert), None, "{inert}");
        }
        for (run, why) in [
            ("export const a = f();", "a call of f"),
            ("export const a = new Date();", "a `new Date`"),
            ("export const a = process.env.X;", "a member access"),
            ("const { a } = o;", "a destructuring"),
            ("export const a = { ...o };", "a spread"),
            ("export const a = <div />;", "JSX"),
            ("export const a = css`x`;", "a template tagged css"),
            (
                "export const a = `${o}`;",
                "a template literal over non-primitive values",
            ),
            (
                "export const a = o + 1;",
                "an operator on a non-primitive value",
            ),
            ("export class K { static x = f(); }", "a call of f"),
            ("@d class K {}\nexport { K };", "a decorator"),
            (
                "function d(): any {}\nexport class K { m(@d() x: number) {} }",
                "a decorator",
            ),
            (
                "class B {}\nexport class K extends mixin(B) {}",
                "a call of mixin",
            ),
            (
                "export const a = notDefinedAnywhere;",
                "a read of a global that may not exist",
            ),
            (
                "export const a = [window];",
                "a read of a global that may not exist",
            ),
            (
                "export const a = [later];\nconst later = 1;",
                "a read of a binding before its declaration",
            ),
            (
                "export class K extends Later {}\nclass Later {}",
                "a read of a binding before its declaration",
            ),
            (
                "export const a = make();\nfunction make() { return 1; }",
                "a call of make",
            ),
            (
                "export class K extends HTMLElement {}",
                "a class that extends a global",
            ),
            (
                "export class K extends Math {}",
                "a class that extends a global",
            ),
            ("if (x) { y(); }", "a statement that runs at load time"),
            ("f();", "a call of f"),
            (
                "const C = () => null;\nexport default memo(C);",
                "a call of memo",
            ),
            ("let a; a = 1;", "an assignment"),
        ] {
            assert_eq!(ev(run).as_deref(), Some(why), "{run}");
        }
    }

    /// Every import builds a value for up to this many calls in a row.
    struct Calls(u8);

    impl Oracle for Calls {
        fn builds(&self, _: Callee<'_>, steps: &[Step], _: bool) -> bool {
            steps.len() <= usize::from(self.0) && steps.iter().all(|s| *s == Step::Call)
        }
        fn prim(&self, _: Callee<'_>) -> bool {
            false
        }
    }

    /// React's builders (by name or as a member of the default import), `flag`
    /// of `flags`, chains on `z` of `zod` that don't parse, and the
    /// primitive constants of `./constants`.
    struct Known;

    impl Oracle for Known {
        fn builds(&self, c: Callee<'_>, steps: &[Step], _: bool) -> bool {
            let builder = |n: &str| matches!(n, "memo" | "cache" | "forwardRef");
            match (c.spec, steps) {
                ("react", [Step::Call]) => builder(c.name),
                ("react", [Step::Member(m), Step::Call]) => c.name == "default" && builder(m),
                ("flags", [Step::Call]) => c.name == "flag",
                ("zod", _) => {
                    c.name == "z"
                        && !steps
                            .iter()
                            .any(|s| matches!(s, Step::Member(m) if m == "parse"))
                }
                _ => false,
            }
        }
        fn prim(&self, c: Callee<'_>) -> bool {
            c.spec == "./constants"
        }
    }

    fn taint_with(
        src: &str,
        start_of: &str,
        tainted: &[&str],
        oracle: &dyn Oracle,
    ) -> Option<Taint> {
        let m = Module::read("a.tsx", src, false).expect("facts");
        let start = src.find(start_of).expect("import") as u32;
        let set: BTreeSet<String> = tainted.iter().map(|s| (*s).to_owned()).collect();
        m.taint(&[(start, Some(&set))], oracle)
    }

    fn taint(src: &str, start_of: &str, tainted: &[&str]) -> Option<Taint> {
        taint_with(src, start_of, tainted, &Alone)
    }

    #[test]
    fn taint_follows_value_references_to_exports() {
        let src = "import { changed, same, type T } from './f';\nimport dflt, * as ns from \
                   './g';\nconst helper = (x: T) => changed(x);\nexport function uses() { return \
                   helper(1); }\nexport function free() { return same(); }\nexport const table = \
                   { uses };\nexport const typed = (x: T) => x;\nexport class K { m() { return \
                   table; } }\nconst local = () => free();\nexport { local as renamed };\nexport \
                   default free;\n";
        let t = taint(src, "import { changed", &["changed"]).expect("taint");
        assert_eq!(t.evaluation, None);
        assert_eq!(t.exports, names(&["K", "table", "uses"]));
        // A name the file does not import, and a type.
        let t = taint(src, "import { changed", &["missing", "T"]).expect("taint");
        assert_eq!(t, Taint::default());
        let t = taint(src, "import { changed", &["same"]).expect("taint");
        assert_eq!(t.exports, names(&["default", "free", "renamed"]));
        // Default and namespace imports of another declaration are not touched.
        let t = taint(src, "import dflt", &["default"]).expect("taint");
        assert_eq!(t, Taint::default());
    }

    #[test]
    fn taint_read_at_load_time_is_reported() {
        let src = "import { a, b } from './f';\nexport const made = build(a);\nexport const later \
                   = () => b();\n";
        let t = taint(src, "import {", &["a"]).expect("taint");
        assert_eq!(t.evaluation.as_deref(), Some("a call of build"));
        let t = taint(src, "import {", &["b"]).expect("taint");
        assert_eq!(
            t,
            Taint {
                evaluation: None,
                exports: names(&["later"]),
                pure_calls: 0,
                any: true,
            }
        );
        // Through a local function that load-time code calls.
        let src = "import { a } from './f';\nfunction init() { return a(); }\ninit();\nexport \
                   const x = 1;\n";
        assert_eq!(
            taint(src, "import {", &["a"])
                .expect("taint")
                .evaluation
                .as_deref(),
            Some("a call of init")
        );
        // In a file with decorators a type may be emitted as a value (decorator
        // metadata).
        let src =
            "import { Dep } from './f';\nexport class Plain { constructor(private d: Dep) {} }\n";
        assert_eq!(
            taint(src, "import {", &["Dep"]).expect("taint"),
            Taint::default()
        );
        let src = "import { Dep } from './f';\n@Injectable()\nexport class Svc { \
                   constructor(private d: Dep) {} }\n";
        let t = taint(src, "import {", &["Dep"]).expect("taint");
        assert_eq!(t.evaluation.as_deref(), Some("a decorator"));
        assert_eq!(t.exports, names(&["Svc"]));
        // A namespace import is tainted by any name; an import the module does not have
        // is unknown.
        let src =
            "import * as ns from './f';\nexport const f = () => ns.a();\nexport const g = 1;\n";
        assert_eq!(
            taint(src, "import *", &["zzz"]).expect("taint").exports,
            names(&["f"])
        );
        let m = Module::read("a.ts", src, false).expect("facts");
        assert_eq!(m.taint(&[(999, None)], &Alone), None);
    }

    #[test]
    fn functions_that_only_build_a_value() {
        // The longest row of calls of an export that only builds a value (arguments are
        // literals, nothing is known about imports).
        let depth = |src: &str, name: &str| {
            let m = Module::read("a.tsx", src, false).expect("facts");
            (1..=4u8)
                .rev()
                .find(|k| m.builds(name, &vec![Step::Call; usize::from(*k)], true, &Alone))
                .unwrap_or(0)
        };
        // A wrapper that returns a closure (the handler is not called).
        let wrap = "export function withTrace(handler, options) {\n  return (req, ctx) => \
                    trace(() => handler(req, ctx), options);\n}\n";
        assert_eq!(depth(wrap, "withTrace"), 1);
        // A wrapper factory: two calls build values, the third runs the component.
        let hoc = "export function withInterceptors<P>() {\n  return (interceptors, create, \
                   options) => {\n    return function Intercepted(props) {\n      return trace(() \
                   => create(props), options);\n    };\n  };\n}\n";
        assert_eq!(depth(hoc, "withInterceptors"), 2);
        for (src, name, d) in [
            ("export const id = (x) => x;", "id", 1),
            (
                "export const make = (a, ...rest) => ({ a, rest, f() { return go(); } });",
                "make",
                1,
            ),
            (
                "export const k = (a) => { const b = [a, 1]; return { b }; };",
                "k",
                1,
            ),
            ("export function noop() {}", "noop", 1),
            // An async function hands what it returns to its promise: a thenable's `then`
            // is read and called.
            ("export async function later(x) { return x; }", "later", 0),
            (
                "export const curry = (a) => (b) => (c) => [a, b, c];",
                "curry",
                3,
            ),
            (
                "export const asyncCurry = async (a) => (b) => b;",
                "asyncCurry",
                0,
            ),
            // Runs something.
            ("export const call = (f) => f();", "call", 0),
            (
                "export function reg(h) { handlers.push(h); return h; }",
                "reg",
                0,
            ),
            ("export const get = (o) => o.value;", "get", 0),
            // A spread of a parameter: fine as long as the call sites pass literals.
            (
                "export const spread = (o) => ({ ...o, more: 1 });",
                "spread",
                1,
            ),
            (
                "export const spreads = (o) => ({ ...other });",
                "spreads",
                0,
            ),
            (
                "export function branch(x) { if (x) return 1; return 2; }",
                "branch",
                0,
            ),
            (
                "export const withDefault = (a = make()) => a;",
                "withDefault",
                0,
            ),
            (
                "export const destructured = ({ a }) => a;",
                "destructured",
                0,
            ),
            ("export const tpl = (a) => `${a}`;", "tpl", 0),
            // Not a function, or one that is replaced later.
            ("export const value = 1;", "value", 0),
            ("export let f = (x) => x;\nf = other;", "f", 0),
            ("export function g(x) { return x; }\ng = other;", "g", 0),
        ] {
            assert_eq!(depth(src, name), d, "{src}");
        }
        let m = Module::read(
            "a.ts",
            "export const spread = (o) => ({ ...o, more: 1 });",
            false,
        )
        .expect("facts");
        assert!(!m.builds("spread", &[Step::Call], false, &Alone));
        // `wrap(c)` and `wrap(c)()` both only hand a value back.
        assert_eq!(
            depth(
                "export default function wrap(c) { return () => c; }",
                "default"
            ),
            2
        );
    }

    #[test]
    fn a_call_that_only_builds_a_value_is_not_load_time_code() {
        // A local wrapper, a known builder of a package, and a member of its default
        // import.
        let src = "import React, { cache } from 'react';\nimport { changed, other } from \
                   './f';\nconst wrap = (fn) => (...a) => fn(...a);\nexport const GET = \
                   wrap((req) => changed(req));\nexport const load = cache(async () => \
                   changed());\nexport const View = React.memo(function View() { return \
                   changed(); });\nexport const untouched = wrap(() => other());\nexport const \
                   TAG = 'tag';\n";
        let t = taint_with(src, "import { changed", &["changed"], &Known).expect("taint");
        assert_eq!(t.evaluation, None);
        assert_eq!(t.exports, names(&["GET", "View", "load"]));
        assert_eq!(t.pure_calls, 3);
        // Without knowledge of the package only the local wrapper is known.
        let t = taint(src, "import { changed", &["changed"]).expect("taint");
        assert_eq!(t.evaluation.as_deref(), Some("a call of cache"));
        // The wrapper itself is what changed: its calls are load-time code.
        let src = "import { wrap, handler } from './f';\nexport const GET = wrap(handler);\n";
        let pure = Calls(1);
        let t = taint_with(src, "import {", &["handler"], &pure).expect("taint");
        assert_eq!((t.evaluation, t.pure_calls), (None, 1));
        let t = taint_with(src, "import {", &["wrap"], &pure).expect("taint");
        assert_eq!(t.evaluation.as_deref(), Some("a call of wrap"));
        // More calls in a row than the callee is known for, an evaluated argument, a
        // callee that is not a plain name.
        let src = "import { wrap, handler } from './f';\nexport const GET = wrap()(handler);\n";
        let t = taint_with(src, "import {", &["handler"], &pure).expect("taint");
        assert_eq!(t.evaluation.as_deref(), Some("a call of wrap()"));
        let two = Calls(2);
        let t = taint_with(src, "import {", &["handler"], &two).expect("taint");
        assert_eq!(t.evaluation, None);
        let src = "import { wrap, handler } from './f';\nexport const GET = wrap(handler());\n";
        let t = taint_with(src, "import {", &["handler"], &two).expect("taint");
        assert_eq!(t.evaluation.as_deref(), Some("a call of handler"));
        let src =
            "import { reg, handler } from './f';\nexport const GET = reg.items[0](handler);\n";
        let t = taint_with(src, "import {", &["handler"], &two).expect("taint");
        assert_eq!(t.evaluation.as_deref(), Some("a call of reg.items[0]"));
    }

    #[test]
    fn a_builder_may_call_other_builders_and_spread_its_parameter() {
        // The shape of a flag declarator: it hands its argument, plus an adapter made
        // by another builder, to a builder of a package.
        let declare = "import { flag } from 'flags';\nimport { adapter } from \
                       './adapter';\nimport { identify } from './identify';\nexport function \
                       declare(declaration) {\n  return flag({ ...declaration, adapter: \
                       adapter(), identify });\n}\n";
        let m = Module::read("declare.ts", declare, false).expect("facts");
        // Both `flag` and `adapter()` have to build values.
        struct Both;
        impl Oracle for Both {
            fn builds(&self, c: Callee<'_>, steps: &[Step], _: bool) -> bool {
                matches!((c.name, steps), ("flag" | "adapter", [Step::Call]))
            }
            fn prim(&self, _: Callee<'_>) -> bool {
                false
            }
        }
        assert!(m.builds("declare", &[Step::Call], true, &Both));
        assert!(
            !m.builds("declare", &[Step::Call], true, &Known),
            "adapter() is unknown"
        );
        assert!(
            !m.builds("declare", &[Step::Call], false, &Both),
            "it spreads its argument"
        );
        assert!(
            !m.builds("declare", &[Step::Call, Step::Call], true, &Both),
            "a flag is called"
        );
        // A constant holding what a builder made: calling it continues the chain.
        let adapter = "import { createAdapter } from 'pkg';\nimport { client } from \
                       './client';\nexport const adapter = createAdapter(client);\nexport { \
                       createAdapter as make };\n";
        let m = Module::read("adapter.ts", adapter, false).expect("facts");
        assert!(m.builds("adapter", &[Step::Call], true, &Calls(2)));
        assert!(!m.builds("adapter", &[Step::Call], true, &Calls(1)));
        assert!(m.builds("adapter", &[], true, &Calls(1)));
        // A re-exported import is the oracle's to judge.
        assert!(m.builds("make", &[Step::Call], true, &Calls(1)));
        assert!(!m.builds("make", &[Step::Call], true, &Alone));
        assert!(
            !m.builds("client", &[Step::Call], true, &Calls(9)),
            "not an export"
        );
    }

    #[test]
    fn chains_and_constants_of_other_modules_hold_on_the_oracles_word() {
        // A schema built by a chain on `z`, and a table with a label made from an
        // imported constant.
        let src = "import { z } from 'zod';\nimport { NAME } from './constants';\nimport { inner, \
                   other } from './f';\nexport const schema = z.object({ a: z.string().min(1), \
                   inner }).optional();\nexport const table = { '/a': { label: `${NAME} credits`, \
                   visible: () => inner } };\nexport const untouched = z.enum(['x', \
                   'y']);\nexport const parsed = z.string().parse(other);\n";
        let m = Module::read("a.ts", src, false).expect("facts");
        assert!(m.evaluated().is_some_and(|w| w.starts_with("a call of z.")));
        let t = taint_with(src, "import { inner", &["inner"], &Known).expect("taint");
        assert_eq!(t.evaluation, None);
        assert_eq!(t.exports, names(&["schema", "table"]));
        assert_eq!(t.pure_calls, 2);
        // Nothing known about `zod` or `./constants`.
        let t = taint_with(src, "import { inner", &["inner"], &Alone).expect("taint");
        assert!(t.evaluation.is_some());
        // A chain that parses runs the schema.
        let t = taint_with(src, "import { inner", &["other"], &Known).expect("taint");
        assert_eq!(t.evaluation.as_deref(), Some("a call of z.string().parse"));
        // The constant is what changed: the template is evaluated with a new value,
        // which only its own statement uses.
        let t = taint_with(src, "import { NAME", &["NAME"], &Known).expect("taint");
        assert_eq!(
            t.evaluation.as_deref(),
            Some("a template literal or operator over an imported value")
        );
        // A local constant made from an import is not known to be primitive.
        let src = "import { NAME } from './constants';\nconst alias = NAME;\nexport const label = \
                   `${alias}!`;\n";
        assert_eq!(
            Module::read("a.ts", src, false).expect("facts").evaluated(),
            Some("a template literal over non-primitive values")
        );
    }

    #[test]
    fn load_time_chains_that_build_values_are_compared_by_export() {
        // The shape of a flag registry: one declaration per export.
        let base = "import { declare } from './declare';\nexport const a = declare({ key: 'a', \
                    defaultValue: false });\nexport const b = declare({ key: 'b', defaultValue: \
                    true });\nexport const c = declare({ key: 'c', defaultValue: false, decide() \
                    { return a(); } });\n";
        // `b` goes away, `d` is new, `a` changes its default (and with it `c`, which
        // calls it).
        let head = "import { declare } from './declare';\nexport const a = declare({ key: 'a', \
                    defaultValue: true });\nexport const c = declare({ key: 'c', defaultValue: \
                    false, decide() { return a(); } });\nexport const d = declare({ key: 'd', \
                    defaultValue: 1 });\n";
        let known = changed("flags.ts", base, head, &Calls(1), &Calls(1), false).expect("facts");
        assert_eq!(
            known,
            Changed {
                evaluation: false,
                load_time: None,
                exports: names(&["a", "b", "c", "d"]),
                members: BTreeMap::new(),
                rebound: names(&["b", "d"]),
                reordered: false,
            }
        );
        // Nothing known about `declare`: every declaration is load-time code.
        let unknown = ch(base, head);
        assert!(unknown.evaluation);
        assert_eq!(
            unknown.load_time.as_deref(),
            Some("a call of declare is new or changed")
        );
        // Known at one revision only (the declarator changed in between).
        assert!(
            changed("flags.ts", base, head, &Alone, &Calls(1), false)
                .expect("facts")
                .evaluation
        );
        // A declaration that runs something in its argument stays load-time code.
        let head2 = format!("{head}export const e = declare({{ key: lookup() }});\n");
        let c = changed("flags.ts", head, &head2, &Calls(1), &Calls(1), false).expect("facts");
        assert_eq!(
            c.load_time.as_deref(),
            Some("a call of lookup is new or changed")
        );
        // A declarator of the file itself, around a builder of a package.
        let local = |flags: &str| {
            format!(
                "import {{ flag }} from 'flags';\nfunction declare(d) {{\n  return flag({{ ...d, \
                 origin: 'x' }});\n}}\n{flags}"
            )
        };
        let c = changed(
            "flags.ts",
            &local("export const a = declare({ key: 'a' });\n"),
            &local(
                "export const a = declare({ key: 'a' });\nexport const b = declare({ key: 'b' \
                 });\n",
            ),
            &Known,
            &Known,
            false,
        )
        .expect("facts");
        assert_eq!((c.evaluation, c.exports), (false, names(&["b"])));
        // ... called with something that is not a literal: the spread may run a getter.
        let c = changed(
            "flags.ts",
            &local("export const a = declare({ key: 'a' });\n"),
            &local("export const a = declare({ key: 'a' });\nexport const b = declare(shared);\n"),
            &Known,
            &Known,
            false,
        )
        .expect("facts");
        assert!(c.evaluation);
    }

    #[test]
    fn values_of_the_language_are_not_load_time_code() {
        let ev = |src: &str| {
            Module::read("a.ts", src, false)
                .expect("facts")
                .evaluated()
                .map(str::to_owned)
        };
        for inert in [
            "export const K = Symbol.for('k'), L = Symbol('l'), M = Symbol();",
            "export const frozen = Object.freeze({ a: 1, b: [2] });",
            "export const m = new Map(), s = new Set([1, 2]), w = new WeakMap();",
            "export const pairs = new Map([['a', 1], ['b', 2]]);",
        ] {
            assert_eq!(ev(inert), None, "{inert}");
        }
        for (run, why) in [
            ("export const m = new Map(entries);", "a `new Map`"),
            ("export const m = new Map([[key(), 1]]);", "a call of key"),
            (
                "const Map = Object;\nexport const m = new Map();",
                "a `new Map`",
            ),
            // Entries that are not pairs throw; so do keys of a weak collection that are
            // not objects, and a symbol description that cannot be converted.
            ("export const m = new Map([1]);", "a `new Map`"),
            ("export const m = new Map([['a', 1], x]);", "a `new Map`"),
            ("export const w = new WeakMap([[1, 2]]);", "a `new WeakMap`"),
            ("export const w = new WeakSet([1]);", "a `new WeakSet`"),
            (
                "const o = {};\nexport const k = Symbol(o);",
                "a call of Symbol",
            ),
            ("export const k = Symbol.for({});", "a call of Symbol.for"),
            ("export const k = Symbol.for(name());", "a call of name"),
            (
                "const a = {};\nexport const o = Object.assign({}, a);",
                "a call of Object.assign",
            ),
            ("export const d = new Date();", "a `new Date`"),
            (
                "const shared = {};\nexport const f = Object.freeze(shared);",
                "a call of Object.freeze",
            ),
        ] {
            assert_eq!(ev(run).as_deref(), Some(why), "{run}");
        }
    }

    #[test]
    fn members_of_enums_and_tables_are_compared_one_by_one() {
        // The shape of a permissions enum: one member is added.
        let base = "export enum Action { Read = 'read', Write = 'write' }\nexport enum Resource \
                    {\n  Team = 'team',\n  Project = 'project',\n}\nexport type Permissions = \
                    Partial<Record<Resource, Action[]>>;\nexport const LIMITS = { free: 1, pro: \
                    10 } as const;\n";
        let head = base.replace(
            "  Project = 'project',\n",
            "  Project = 'project',\n  Rules = 'rules',\n",
        );
        let c = ch(base, &head);
        assert_eq!(c.exports, names(&["Permissions", "Resource"]));
        assert_eq!(c.members.get("Resource"), Some(&names(&["Rules"])));
        assert_eq!(c.members.len(), 1);
        // A changed value, a removed member; an object table.
        let head = base
            .replace("Team = 'team'", "Team = 'teams'")
            .replace("  Project = 'project',\n", "")
            .replace("pro: 10", "pro: 20, max: 99");
        let c = ch(base, &head);
        assert_eq!(
            c.members.get("Resource"),
            Some(&names(&["Project", "Team"]))
        );
        assert_eq!(c.members.get("LIMITS"), Some(&names(&["max", "pro"])));
        // Not member by member: a member without a literal value (its value depends on
        // its position), a change outside the members, an object with a spread.
        for (b, h) in [
            ("export enum E { A, B }\n", "export enum E { A, X, B }\n"),
            (
                "export enum E { A = 1, B = A + 1 }\n",
                "export enum E { A = 2, B = A + 1 }\n",
            ),
            (
                "export enum E { A = 'a' }\n",
                "export const enum E { A = 'a', B = 'b' }\n",
            ),
            (
                "const base = { a: 1 };\nexport const T = { ...base, b: 2 };\n",
                "const base = { a: 1 };\nexport const T = { ...base, b: 3 };\n",
            ),
            (
                "const K = 1;\nexport const T = { a: K, b: 2 };\n",
                "const K = 5;\nexport const T = { a: K, b: 2 };\n",
            ),
        ] {
            let c = ch(b, h);
            assert!(
                !c.exports.is_empty() && c.members.is_empty(),
                "{b} -> {h}: {c:?}"
            );
        }
    }

    #[test]
    fn an_import_of_a_table_is_tainted_where_a_changed_member_is_read() {
        let src = "import { Resource, Action } from './permissions';\nexport const canEditTeam = \
                   (p) => p[Resource.Team]?.includes(Action.Write);\nexport const canSeeRules = \
                   (p) => Boolean(p[Resource.Rules]);\nexport const all = () => \
                   Object.values(Resource);\nexport const label = (r: Resource) => \
                   String(r);\nexport function check(r) { return Resource[r] ?? Resource.Team; }\n";
        let m = Module::read("a.ts", src, false).expect("facts");
        let start = src.find("import {").expect("import") as u32;
        let tracked: Vec<String> = ["Resource".to_owned()].into();
        let with = |changed: &[&str]| {
            let members: BTreeMap<String, Vec<String>> =
                [("Resource".to_owned(), names(changed))].into();
            m.taint_members(&ask(start, &tracked), &members, &Alone)
                .expect("taint")
        };
        // A new member: the reader of that member, and the uses of the enum as a whole.
        assert_eq!(
            with(&["Rules"]).exports,
            names(&["all", "canSeeRules", "check"])
        );
        assert_eq!(
            with(&["Team"]).exports,
            names(&["all", "canEditTeam", "check"])
        );
        // A member nobody reads by name.
        let t = with(&["Billing"]);
        assert_eq!(t.exports, names(&["all", "check"]));
        assert!(t.any);
        // Without member information every value use counts; a type use never does.
        let t = m
            .taint_members(&ask(start, &tracked), &BTreeMap::new(), &Alone)
            .expect("taint");
        assert_eq!(
            t.exports,
            names(&["all", "canEditTeam", "canSeeRules", "check"])
        );
        // A file that only reads other members is not touched at all.
        let src = "import { Resource } from './permissions';\ntest('t', () => \
                   expect(can(Resource.Team)).toBe(true));\n";
        let m = Module::read("a.test.ts", src, false).expect("facts");
        let members: BTreeMap<String, Vec<String>> =
            [("Resource".to_owned(), names(&["Rules"]))].into();
        let t = m
            .taint_members(&ask(0, &tracked), &members, &Alone)
            .expect("taint");
        assert!(!t.any && t.evaluation.is_none() && t.exports.is_empty());
        let members: BTreeMap<String, Vec<String>> =
            [("Resource".to_owned(), names(&["Team"]))].into();
        let t = m
            .taint_members(&ask(0, &tracked), &members, &Alone)
            .expect("taint");
        assert!(t.any && t.evaluation.is_some());
        // A write to a member, and a member read through a wrapper, take the whole
        // export.
        for src in [
            "import { T } from './t';\nexport const f = () => { T.a = 1; };\n",
            "import { T } from './t';\nexport const f = () => (T as any).a;\n",
            "import { T } from './t';\nexport const f = () => { const { a } = T; return a; };\n",
        ] {
            let m = Module::read("a.ts", src, false).expect("facts");
            let tracked: Vec<String> = ["T".to_owned()].into();
            let members: BTreeMap<String, Vec<String>> = [("T".to_owned(), names(&["zzz"]))].into();
            let t = m
                .taint_members(&ask(0, &tracked), &members, &Alone)
                .expect("taint");
            assert_eq!(t.exports, names(&["f"]), "{src}");
        }
    }

    #[test]
    fn a_class_definition_reads_what_it_extends() {
        // In the modified file: the base is compared, the body of the class is not.
        let base = "import { Base } from './base';\nconst local = Base;\nexport class A extends \
                    Base { m() { return 1; } }\nexport const other = 1;\n";
        let c = ch(base, &base.replace("return 1", "return 2"));
        assert!(!c.evaluation);
        assert_eq!(c.exports, names(&["A"]));
        // A new class on a base that no class extended before, and a constant as base
        // whose value changes.
        let head = format!("{base}const k: any = 1;\nexport class Z extends k {{}}\n");
        assert!(ch(base, &head).evaluation);
        let base2 = "const k: any = Object;\nexport class Z extends k {}\nexport const o = 1;\n";
        assert!(ch(base2, &base2.replace("= Object", "= 1")).evaluation);
        // A class of the module stays a constructor whatever its body is.
        let base3 =
            "class B { m() { return 1; } }\nexport class Z extends B {}\nexport const o = 1;\n";
        let c = ch(base3, &base3.replace("return 1", "return 2"));
        assert!(!c.evaluation);
        assert_eq!(c.exports, names(&["Z"]));
        // In an importer: a class that extends a changed import ends the walk, a class
        // that extends a class of the importer does not.
        let src = "import { Base, helper } from './base';\nexport class A extends Base {}\nexport \
                   const other = 1;\n";
        let t = taint(src, "import {", &["Base"]).expect("taint");
        assert_eq!(
            t.evaluation.as_deref(),
            Some("a class that extends a changed value")
        );
        let src = "import { helper } from './base';\nclass Mid { m() { return helper(); } \
                   }\nexport class A extends Mid {}\nexport const other = 1;\n";
        let t = taint(src, "import {", &["helper"]).expect("taint");
        assert_eq!(t.evaluation, None);
        assert_eq!(t.exports, names(&["A"]));
        // A mixin's class extends its parameter: calling it is load-time code.
        let src = "const mix = (B: any) => class extends B {};\nexport const K = mix(Object);\n";
        let m = Module::read("a.ts", src, false).expect("facts");
        assert_eq!(m.evaluated(), Some("a call of mix"));
        assert_eq!(m.load_time(&Alone, false), [false, true]);
    }

    #[test]
    fn a_rebound_export_is_read_by_every_import_of_its_name() {
        // A function that becomes a constant, a removed and an added name are rebound;
        // a changed body is not.
        let base = "export function f() { return 1; }\nexport function g() { return 1; }\nexport \
                    const gone = 1;\nexport class K {}\n";
        let head = "export const f = () => 1;\nexport function g() { return 2; }\nexport const \
                    fresh = 1;\nexport function K() {}\n";
        let c = ch(base, head);
        assert_eq!(c.exports, names(&["K", "f", "fresh", "g", "gone"]));
        assert_eq!(c.rebound, names(&["K", "f", "fresh", "gone"]));
        // The importer: naming a rebound export is a load-time read, a type import is
        // not.
        let src = "import { f, g } from './m';\nimport type { K } from './m';\nexport const a = \
                   () => g();\nexport const b = (k: K) => f();\n";
        let m = Module::read("a.ts", src, false).expect("facts");
        let set: Vec<String> = ["f".to_owned(), "g".to_owned(), "K".to_owned()].into();
        let ask = |start: &str, rebound: &'static [&'static str]| {
            let rebound = names(rebound);
            m.taint_members(
                &[Asked {
                    start: src.find(start).expect("import") as u32,
                    names: Some(&set),
                    rebound: &rebound,
                }],
                &BTreeMap::new(),
                &Alone,
            )
            .expect("taint")
        };
        assert_eq!(ask("import { f", &[]).evaluation, None);
        assert_eq!(
            ask("import { f", &["f"]).evaluation.as_deref(),
            Some("an import of a name that was added, removed or redeclared")
        );
        assert_eq!(ask("import type", &["K"]).evaluation, None);
    }

    #[test]
    fn the_order_of_the_exports_is_part_of_the_module() {
        let base = "export const a = 1;\nexport const b = 2;\nexport function c() {}\n";
        let head = "export const b = 2;\nexport const a = 1;\nexport function c() {}\n";
        let c = ch(base, head);
        assert!(c.reordered && !c.evaluation);
        assert_eq!(c.exports, names(&[]));
        // A new name between the others is not a reordering of what both versions have.
        let head = "export const a = 1;\nexport const n = 0;\nexport const b = 2;\nexport \
                    function c() {}\n";
        assert!(!ch(base, head).reordered);
    }

    #[test]
    fn decorators_before_export_belong_to_the_statement() {
        let base = "function dec(tag: string) { return (t: any) => { log(tag); }; \
                    }\n@dec('a')\nexport class K {}\nexport const other = 1;\n";
        let c = ch(base, &base.replace("@dec('a')", "@dec('b')"));
        assert!(c.evaluation, "{c:?}");
        assert_eq!(c.exports, names(&["K"]));
    }

    #[test]
    fn an_exported_function_that_reads_this_reads_the_namespace() {
        // `ns.viaThis()` hands the namespace to the function.
        let base = "export function hidden() { return 1; }\nexport function viaThis(this: any) { \
                    return this.hidden(); }\nexport function plain() { return 1; }\n";
        let c = ch(
            base,
            &base.replace(
                "return 1; }\nexport function viaThis",
                "return 2; }\nexport function viaThis",
            ),
        );
        assert_eq!(c.exports, names(&["hidden", "viaThis"]));
        // `this` of a method or of a class is not the namespace.
        let base = "export function hidden() { return 1; }\nexport const o = { m() { return this; \
                    }, f: function () { return this; } };\nexport class K { m() { return this; } \
                    }\n";
        let c = ch(base, &base.replace("return 1", "return 2"));
        assert_eq!(c.exports, names(&["hidden"]));
        // With a default expression or a re-export there is no binding to name: no
        // facts.
        assert_eq!(
            Module::read(
                "a.ts",
                "export function f(this: any) { return this; }\nexport default 1;\n",
                false
            )
            .err(),
            Some("an exported function that reads `this`")
        );
    }

    #[test]
    fn an_import_whose_last_value_use_goes_away_is_no_longer_loaded() {
        // A transpiler drops an import no value reads: the module is then not loaded.
        let base = "import { a } from './a';\nimport { b } from './b';\nexport const x = () => \
                    a;\nexport const y = () => b;\n";
        let head =
            "import { a } from './a';\nimport { b } from './b';\nexport const x = () => a;\n";
        let c = ch(base, head);
        assert!(c.evaluation);
        assert_eq!(
            c.load_time.as_deref(),
            Some(
                "an import is used as a value at one revision only (a transpiler drops an unused \
                 import)"
            )
        );
        // Another use of the same import goes away: nothing about loading changes.
        let head = "import { a } from './a';\nimport { b } from './b';\nexport const x = () => \
                    a;\nexport const y = () => [b, b];\n";
        assert!(!ch(base, head).evaluation);
        // A use in a type only is no use.
        let head = "import { a } from './a';\nimport { b } from './b';\nexport const x = () => \
                    a;\nexport const y = (v: typeof b) => v;\n";
        assert!(ch(base, head).evaluation);
    }

    #[test]
    fn jsx_reads_the_factory() {
        // A pragma names the factory; an import nothing else uses may be one.
        let src = "/** @jsx h */\nimport { h } from './h';\nimport { other } from './o';\nexport \
                   const El = () => <a x=\"1\" />;\nexport const plain = () => other();\n";
        let m = Module::read("a.tsx", src, false).expect("facts");
        let set: BTreeSet<String> = ["h".to_owned()].into();
        let t = m
            .taint(
                &[(src.find("import { h").expect("import") as u32, Some(&set))],
                &Alone,
            )
            .expect("taint");
        assert_eq!(t.exports, names(&["El"]));
        let src =
            "import React from './react';\nexport const El = () => <a />;\nexport const k = 1;\n";
        let m = Module::read("a.tsx", src, false).expect("facts");
        let set: BTreeSet<String> = ["default".to_owned()].into();
        let t = m.taint(&[(0, Some(&set))], &Alone).expect("taint");
        assert_eq!(t.exports, names(&["El"]));
    }

    #[test]
    fn a_table_whose_members_see_each_other_is_one_value() {
        for (b, h) in [
            // A member that reads the table by name.
            (
                "export const T = { a: 1, g: () => T.a };",
                "export const T = { a: 2, g: () => T.a };",
            ),
            // ... through a helper.
            (
                "function get() { return T.a; }\nexport const T = { a: 1, g: get };",
                "function get() { return T.a; }\nexport const T = { a: 2, g: get };",
            ),
            // The prototype.
            (
                "export const T = { __proto__: { p: 1 }, a: 1 };",
                "export const T = { __proto__: { p: 2 }, a: 1 };",
            ),
        ] {
            let c = ch(b, h);
            assert!(
                c.exports == names(&["T"]) && c.members.is_empty(),
                "{b} -> {h}: {c:?}"
            );
        }
        // A call through a member hands the table to the function as `this`: it reads
        // every member. A member that is only read does not.
        let src = "import { T } from './t';\nexport const viaThis = () => T.f();\nexport const \
                   inherited = () => T.hasOwnProperty('b');\nexport const tagged = () => \
                   T.tag`x`;\nexport const one = () => T.a;\nexport const wrapped = () => \
                   (T.f)();\n";
        let m = Module::read("a.ts", src, false).expect("facts");
        let tracked: Vec<String> = ["T".to_owned()].into();
        let members: BTreeMap<String, Vec<String>> = [("T".to_owned(), names(&["b"]))].into();
        let t = m
            .taint_members(&ask(0, &tracked), &members, &Alone)
            .expect("taint");
        assert_eq!(
            t.exports,
            names(&["inherited", "tagged", "viaThis", "wrapped"])
        );
    }

    #[test]
    fn a_module_on_an_import_cycle_is_compared_more_strictly() {
        // A statement that only names an import, and a chain that only builds a value.
        let base =
            "import { x, wrap } from './m';\nexport const a = () => x;\nexport const k = 1;\n";
        let named = format!("{base}export const list = [x];\n");
        let built = format!("{base}export const w = wrap(() => 1);\n");
        struct Builds;
        impl Oracle for Builds {
            fn builds(&self, _: Callee<'_>, _: &[Step], _: bool) -> bool {
                true
            }
            fn prim(&self, _: Callee<'_>) -> bool {
                false
            }
            fn esm(&self, _: Callee<'_>) -> bool {
                true
            }
        }
        for head in [&named, &built] {
            let free = changed("a.ts", base, head, &Builds, &Builds, false).expect("facts");
            assert!(!free.evaluation, "{head}");
            // An import of a package or of a CommonJS module may be a getter: reading it
            // when the module loads is something the module does.
            struct Getters;
            impl Oracle for Getters {
                fn builds(&self, _: Callee<'_>, _: &[Step], _: bool) -> bool {
                    true
                }
                fn prim(&self, _: Callee<'_>) -> bool {
                    false
                }
            }
            let read = changed("a.ts", base, head, &Getters, &Getters, false).expect("facts");
            assert!(read.evaluation, "{head}");
            assert_eq!(
                read.load_time.as_deref(),
                Some(
                    "an import of a package or a CommonJS module is read when the file is loaded \
                     at one revision only (it may be a getter)"
                ),
                "{head}"
            );
            let cyclic = changed("a.ts", base, head, &Builds, &Builds, true).expect("facts");
            assert!(cyclic.evaluation, "{head}");
        }
        // A function body is the same on a cycle.
        let head = base.replace("() => x", "() => [x]");
        assert!(
            !changed("a.ts", base, &head, &Builds, &Builds, true)
                .expect("facts")
                .evaluation
        );
    }

    #[test]
    fn modules_without_facts() {
        for (src, why) in [
            ("module.exports = { a: 1 };", "CommonJS"),
            ("const x = require('./x');\nexport const a = x;", "CommonJS"),
            (
                "import { a } from './a';\njest.mock('./a');\nexport const b = a;",
                "test-runner mocks",
            ),
            (
                "import { vi } from 'vitest';\nexport const m = () => vi.fn();",
                "test-runner mocks",
            ),
            ("export const a = ;", "parse errors"),
            (
                "const hidden = 1;\nexport function f() { return eval('hidden'); }",
                "a call of `eval`",
            ),
            ("export function f() { let x; let x; }", "parse errors"),
            (
                "import { f } from './m';\nexport const f = 1;",
                "parse errors",
            ),
        ] {
            assert_eq!(Module::read("a.ts", src, false).err(), Some(why), "{src}");
        }
        assert_eq!(
            Module::read(
                "a.ts",
                "import x = require('y');\nexport const a = x;",
                false
            )
            .err(),
            Some("`import =`")
        );
    }
}
