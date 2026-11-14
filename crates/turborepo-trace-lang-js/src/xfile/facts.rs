//! The facts of one file: what cross-file binding needs to know about it.
//!
//! Facts are a pure function of the file's text. They describe values as
//! expressions over *holes* ([`X::Param`]: a parameter of a function of the
//! file, as bound by callers the file cannot see), so that another file's
//! arguments can be substituted later ([`super::bind`]).

use turborepo_trace_core::{RefKind, api::BaseRule};

use crate::extract::evaluator::{Func, JsVal};

/// The module name of the marker import that stands for `require` and
/// `import()`: a [`X::Call`] whose callee is `X::Import { module: REQUIRE, ..
/// }` is the module whose specifier is the call's argument.
pub const REQUIRE: &str = "\u{0}require";

/// A value as an expression over holes.
#[derive(Clone, Debug, PartialEq)]
pub enum X {
    /// A value without holes, in the extractor's domain. Unknown parts carry
    /// origin marks; paths are anchored at the directory of the file the
    /// facts belong to.
    Const(JsVal),
    /// `undefined`, `null`, a boolean, a number: never part of a path.
    Void,
    /// Parameter `index` of the function `func` ([`FnFact::id`]).
    Param {
        /// The function.
        func: u32,
        /// The parameter's position.
        index: u32,
    },
    /// A property.
    Field(Box<X>, String),
    /// Any element.
    Elem(Box<X>),
    /// Any property.
    AnyField(Box<X>),
    /// `a + b` (also template literals).
    Concat(Box<X>, Box<X>),
    /// A path builtin (`path.join`, `path.resolve`, `path.dirname`, ...).
    Apply(Func, Vec<X>),
    /// Some function of the operands that is not modelled (a string method).
    Opaque(Vec<X>),
    /// An object literal: known properties, and the value of every other
    /// property.
    Object {
        /// Known properties.
        props: Vec<(String, X)>,
        /// Every other property.
        open: Option<Box<X>>,
    },
    /// An array literal: elements by position, and elements at unknown
    /// positions.
    Array {
        /// Elements by position.
        elems: Vec<X>,
        /// Elements at unknown positions.
        rest: Option<Box<X>>,
    },
    /// `table[key]`: the properties of an object of the file that the key
    /// selects.
    Index {
        /// The object.
        table: Box<X>,
        /// The key.
        key: Box<X>,
    },
    /// A function of the file ([`FnFact::id`]).
    Func(u32),
    /// An imported binding: the specifier as written and the export path
    /// (`default`, `a`, `a.b`, `*` for the namespace).
    Import {
        /// The specifier as written.
        module: String,
        /// The export path.
        name: String,
    },
    /// The result of a call.
    Call {
        /// What is called.
        callee: Box<X>,
        /// Positional arguments.
        args: Vec<X>,
        /// Further arguments (a spread).
        rest: Option<Box<X>>,
    },
    /// One of several values.
    Alts(Vec<X>),
    /// `base` and everything reached from it by any sequence of the reads
    /// `steps`: what a function that walks a structure by calling itself
    /// sees of its argument.
    Reach {
        /// Where the reads start.
        base: Box<X>,
        /// The reads.
        steps: Vec<Read>,
    },
    /// An unknown value when one of the callees `by` can change the object it
    /// is handed as argument `.1` ([`RECEIVER`]: as `this`), no value
    /// otherwise. Stands next to a read of a property or element of an
    /// object that the file hands to those callees: what they do to it is
    /// in their facts.
    Changed {
        /// The callees and the position of the object among their arguments.
        by: Vec<(X, u32)>,
        /// The property read ([`ELEMENT`]: an element; `None`: any property or
        /// element).
        prop: Option<String>,
    },
}

/// A read that [`X::Reach`] repeats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// `.name` ([`AS_ARRAY`]: as the receiver of an array method).
    Field(String),
    /// Any element.
    Elem,
    /// Any property.
    Any,
}

/// The property name that stands for "as the receiver of an array method"
/// (`xs.map(f)` hands `f` the elements of an array; a string or a plain object
/// in its place has no such method): an array is itself, anything else nothing.
pub const AS_ARRAY: &str = "\u{0}array";

/// The property name that stands for "an element, as an array method hands it
/// out": the elements of an array, nothing of anything else ([`AS_ARRAY`] and
/// an element read as one step of an [`X::Reach`]).
pub const ARRAY_ELEMENT: &str = "\u{0}array[]";

/// The property name that stands for "an element" in [`X::Changed::prop`].
pub const ELEMENT: &str = "\u{0}[]";

/// The argument position that stands for the receiver of a method call.
pub const RECEIVER: u32 = u32::MAX;

impl X {
    /// Whether the expression contains a hole or a call (something binding can
    /// change).
    pub fn is_open(&self) -> bool {
        match self {
            X::Const(_) | X::Void | X::Func(_) => false,
            X::Param { .. } | X::Call { .. } | X::Import { .. } | X::Changed { .. } => true,
            X::Field(b, _) | X::Elem(b) | X::AnyField(b) => b.is_open(),
            X::Reach { base, .. } => base.is_open(),
            X::Concat(a, b) => a.is_open() || b.is_open(),
            X::Index { table, key } => table.is_open() || key.is_open(),
            X::Apply(_, xs) | X::Opaque(xs) | X::Alts(xs) => xs.iter().any(X::is_open),
            X::Object { props, open } => {
                props.iter().any(|(_, x)| x.is_open()) || open.as_ref().is_some_and(|x| x.is_open())
            }
            X::Array { elems, rest } => {
                elems.iter().any(X::is_open) || rest.as_ref().is_some_and(|x| x.is_open())
            }
        }
    }

    /// The functions whose parameters the expression mentions, and the
    /// functions it mentions as values.
    pub fn funcs(&self, out: &mut Vec<u32>) {
        let mut push = |f: u32| {
            if !out.contains(&f) {
                out.push(f);
            }
        };
        match self {
            X::Const(_) | X::Void | X::Import { .. } => {}
            X::Param { func, .. } | X::Func(func) => push(*func),
            X::Field(b, _) | X::Elem(b) | X::AnyField(b) => b.funcs(out),
            X::Reach { base, .. } => base.funcs(out),
            X::Concat(a, b) | X::Index { table: a, key: b } => {
                a.funcs(out);
                b.funcs(out);
            }
            X::Apply(_, xs) | X::Opaque(xs) | X::Alts(xs) => xs.iter().for_each(|x| x.funcs(out)),
            X::Object { props, open } => {
                props.iter().for_each(|(_, x)| x.funcs(out));
                if let Some(x) = open {
                    x.funcs(out);
                }
            }
            X::Array { elems, rest } => {
                elems.iter().for_each(|x| x.funcs(out));
                if let Some(x) = rest {
                    x.funcs(out);
                }
            }
            X::Call { callee, args, rest } => {
                callee.funcs(out);
                args.iter().for_each(|x| x.funcs(out));
                if let Some(x) = rest {
                    x.funcs(out);
                }
            }
            X::Changed { by, .. } => by.iter().for_each(|(x, _)| x.funcs(out)),
        }
    }
}

/// One import declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    /// Byte offset of the declaration (the start of the graph edge's span).
    pub at: u32,
    /// The specifier as written.
    pub spec: String,
    /// Export paths of the module that the file uses as values other than by
    /// calling them directly (`*`: the namespace object itself, so any
    /// export). A path the file only calls (`f(...)`, `new C(...)`,
    /// `ns.f(...)`) is not listed: those calls are [`Call`]s.
    pub escapes: Vec<String>,
    /// The file uses a binding of this import in a way that can change a
    /// container it reaches through it (anything but reads, calls and
    /// reader methods): what the module exports can differ from what its
    /// own text says wherever this file ran first.
    pub writes: bool,
}

/// One re-export: `export { a as b } from 'm'`, `export * from 'm'`, or an
/// imported binding that is exported again.
#[derive(Clone, Debug, PartialEq)]
pub struct ReExport {
    /// Byte offset of the declaration that loads the module (the `export …
    /// from`, or the `import` of a binding exported later).
    pub at: u32,
    /// The specifier as written.
    pub spec: String,
    /// `(name in the module, exported name)`; `*` as the module name is the
    /// namespace.
    pub names: Vec<(String, String)>,
    /// `export * from`.
    pub star: bool,
}

/// One export defined in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    /// The exported name (`default` for the default export).
    pub name: String,
    /// Its value.
    pub value: X,
}

/// How the callers of a function are known.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// Through its name. Its direct calls in the file are already part of every
    /// value; what remains is the callers of its exports ([`Export`]).
    /// `escapes`: the file also uses it as a value (passes it on, stores
    /// it), so not every call is a direct one.
    Named {
        /// Used as a value in the file.
        escapes: bool,
    },
    /// The constructor of a class: `new C(...)` in the file are [`Call`]s with
    /// this function as the callee; the rest is the users of the class's
    /// exports. `escapes`: the class is used other than with `new`
    /// (extended, passed on).
    Ctor {
        /// The class is used other than with `new`.
        escapes: bool,
    },
    /// Argument `arg` of the call `call` ([`X::Call`]): the callee decides.
    Callback {
        /// The call.
        call: X,
        /// The position of the function among its arguments.
        arg: u32,
    },
    /// A method, a function stored in a container, a returned closure: not
    /// known.
    Unseen,
}

/// A function of the file that facts refer to.
#[derive(Clone, Debug, PartialEq)]
pub struct FnFact {
    /// Byte offset of the function (its identity inside the file).
    pub id: u32,
    /// A name for reports.
    pub name: String,
    /// 1-based line.
    pub line: u32,
    /// How its callers are known.
    pub source: Source,
    /// What it returns, for functions reached through a name (`None`: not
    /// known).
    pub returns: Option<X>,
    /// Parameters the body uses other than by calling them directly (as a
    /// function value that is passed on or stored), by index. A parameter
    /// that is only called is a callback whose calls are all [`Call`]s.
    pub param_escapes: Vec<u32>,
    /// The index of the rest parameter (`...args`), if any.
    pub rest: Option<u32>,
    /// Parameters (or parts of them) that the function hands to other code.
    pub handoffs: Vec<Handoff>,
    /// A function with its own `this` that uses it (a method can change its
    /// receiver).
    pub uses_this: bool,
    /// The function uses `arguments`: every parameter can be handed anywhere.
    pub uses_arguments: bool,
    /// The function is a test for a string: it returns a truthy value only when
    /// its parameter at this position is a string (`typeof p === 'string'`
    /// is the returned value, or a conjunct of it; the function is neither
    /// `async` nor a generator and never assigns the parameter).
    pub tests_string: Option<u32>,
}

/// A parameter, or an object reached through it, that a function hands to other
/// code.
#[derive(Clone, Debug, PartialEq)]
pub struct Handoff {
    /// The parameter's position.
    pub param: u32,
    /// The callees it is handed to, with its position among their arguments.
    pub to: Vec<(X, u32)>,
    /// It is also handed to code the facts do not describe.
    pub unknown: bool,
}

/// Writes of a file that the engine does not attribute to a tracked binding:
/// they can hit any object the file holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Writes {
    /// Properties assigned by name (`o.name = v`), sorted.
    pub props: Vec<String>,
    /// A property is assigned with a computed key, or through `Object.assign`
    /// and the like.
    pub any: bool,
    /// Elements are added to an array or set.
    pub arrays: bool,
}

impl Writes {
    /// Whether a write of the file can change what a read of `prop` sees
    /// ([`ELEMENT`]: of an element; `None`: of any property or element). A
    /// property assigned by name does not change which elements an array
    /// has.
    pub fn hits(&self, prop: Option<&str>) -> bool {
        match prop {
            Some(ELEMENT) | Some(AS_ARRAY) | Some(ARRAY_ELEMENT) => self.any || self.arrays,
            Some(p) => self.any || self.props.iter().any(|x| x == p),
            None => self.any || self.arrays || !self.props.is_empty(),
        }
    }
}

/// A site (a file read, a module load) whose argument depends on holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Sink {
    /// What the site does with the value.
    pub kind: RefKind,
    /// How a relative string is anchored.
    pub rule: BaseRule,
    /// The span of the argument (the span of the site's uncertainty record).
    pub at: [u32; 2],
    /// The span of the site's references.
    pub site: [u32; 2],
    /// The site kept a record whose value other files decide (origin parameter,
    /// call or imported binding). Otherwise its references already cover
    /// every value (an over-approximation that binding can only narrow).
    pub open: bool,
    /// 1-based line.
    pub line: u32,
    /// The alternatives of the argument that depend on holes.
    pub value: X,
    /// The site only runs where its argument is a string: the argument is a
    /// binding that is never assigned, and the site is inside the branch a
    /// `typeof <binding> === 'string'` test of that binding guards (`if`,
    /// `? :`, `&&`). A container, a function and an object with its own
    /// string form never reach it.
    pub string_only: bool,
    /// Like [`Sink::string_only`], where the test is a call with the binding as
    /// an argument: the callee and the position of the binding among its
    /// arguments. The site only runs where each of these calls returned a
    /// truthy value, so it only takes strings when one of the callees is a
    /// test for a string ([`FnFact::tests_string`]), which the callee's
    /// facts decide.
    pub string_tests: Vec<(X, u32)>,
}

/// A call whose callee is not a function of the file: an imported binding, a
/// hole, the result of another such call. Also `new C(...)` of a class of the
/// file whose constructor has holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// What is called.
    pub callee: X,
    /// Positional arguments.
    pub args: Vec<X>,
    /// Further arguments (a spread).
    pub rest: Option<X>,
    /// 1-based line.
    pub line: u32,
    /// `new`.
    pub new: bool,
    /// The innermost function of the file that contains the call
    /// ([`FnFact::id`]).
    pub within: Option<u32>,
}

/// A callable value that is not a function of the file (the result of a call of
/// an imported wrapper, `const cached = wrap(cache, fn)`) and how the file uses
/// it.
#[derive(Clone, Debug, PartialEq)]
pub struct Callable {
    /// The value.
    pub value: X,
    /// The file uses it other than by calling it directly or exporting it.
    pub escapes: bool,
}

/// A lookup table: an object literal bound to a name of the file that is read
/// by key (`const ROUTES = { a: './a', b: './b' }` ... `ROUTES[name]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// The span of the object literal.
    pub at: [u32; 2],
    /// Every use of the name is a read by key (`T[k]`, `T.k`) or a type
    /// position: a value can only leave the table through a key.
    pub keyed_only: bool,
    /// The keys it is read with.
    pub keys: Vec<X>,
    /// The properties whose value is a string literal: `(key, value)`.
    pub values: Vec<(String, String)>,
    /// The sites (spans of their arguments, [`Sink::at`]) that the values read
    /// by key flow into. With `keyed_only`, a value reaches nothing else.
    pub sinks: Vec<[u32; 2]>,
}

/// The facts of one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileFacts {
    /// Import declarations.
    pub imports: Vec<Import>,
    /// Re-exports.
    pub reexports: Vec<ReExport>,
    /// Exports defined in the file.
    pub exports: Vec<Export>,
    /// Functions the other facts refer to.
    pub fns: Vec<FnFact>,
    /// Sites that depend on holes.
    pub sinks: Vec<Sink>,
    /// Calls of imported bindings, holes and call results.
    pub calls: Vec<Call>,
    /// Call results bound to names.
    pub callables: Vec<Callable>,
    /// Lookup tables.
    pub tables: Vec<Table>,
    /// The file has exports the facts do not describe (`export =`, a
    /// destructured export, `module` or `exports` used other than in
    /// `module.exports = v`, `module.exports.name = v` and `exports.name =
    /// v`).
    pub exports_open: bool,
    /// Untracked writes.
    pub writes: Writes,
}

impl FileFacts {
    /// The function `id`.
    pub fn func(&self, id: u32) -> Option<&FnFact> {
        self.fns.iter().find(|f| f.id == id)
    }
}
