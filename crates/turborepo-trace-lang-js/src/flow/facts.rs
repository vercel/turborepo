//! Extraction of cross-file facts ([`crate::xfile::facts`]) with the engine in
//! symbolic mode ([`super::sym`]).
//!
//! One pass over the syntax tree of a file collects:
//!
//! - its import declarations, and for every imported binding whether the file
//!   uses it other than by calling it (an escape);
//! - re-exports, and the values of its own exports;
//! - every call whose callee is an imported binding, a hole or the result of
//!   such a call, with the values of the arguments;
//! - the sites that kept a parameter-origin record, and the resolved sites that
//!   read a lookup table with a key that depends on holes, as expressions over
//!   holes;
//! - lookup tables and how they are read;
//! - for every function those expressions mention: how its callers are known.
//!
//! Everything is an over-approximation of what the file can do: a use that is
//! not recognized as a direct call is an escape, a value that cannot be
//! evaluated is unknown.

use std::collections::{HashMap, HashSet};

use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::{NodeId, SemanticBuilder, SymbolId};
use oxc_span::GetSpan;
use turborepo_trace_core::{RefKind, Span as CoreSpan, api::BaseRule, lexkit::LineIndex};

use super::{
    engine::{Engine, Reach},
    stable::any_written,
    sym::{Callee, Op, ReachStep, Sym, sym_of, syms_in},
    value::{Builtin, FnRef, Trust, V},
};
use crate::{
    extract::{
        evaluator::{Anchor, JsVal},
        origin::{self, Tag},
    },
    lexer::Dialect,
    xfile::facts::{
        Call, Callable, ELEMENT, Export, FileFacts, FnFact, Handoff, Import, RECEIVER, ReExport,
        Read, Sink, Source, Table, Writes, X,
    },
};

/// A site of the recognizer that the value-flow pass evaluated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SiteIn {
    pub kind: RefKind,
    pub rule: BaseRule,
    /// The expression whose value the site uses.
    pub expr: CoreSpan,
    /// `expr` only marks where the expression starts.
    pub open_end: bool,
    /// `expr` is a call asked about as a load ([`super::Ask::Load`]): its value
    /// is not the value of an expression other files could supply, so it is
    /// no sink.
    pub load: bool,
    /// The span of the site's uncertainty record.
    pub arg_span: CoreSpan,
    /// The span of the site's references.
    pub span: CoreSpan,
    /// The site kept a record whose value other files decide (origin parameter,
    /// call or imported binding).
    pub param: bool,
}

/// Calls recorded per file.
const MAX_CALLS: usize = 20_000;
/// Functions described per file.
const MAX_FNS: usize = 2_000;
/// Nodes of one expression.
const MAX_X: usize = 4_000;
/// Nodes of a callee named next to a read ([`X::Changed`]).
const MAX_CALLEE_X: usize = 24;
/// Nodes of the value of `module.exports` (a route table is one big literal).
const MAX_EXPORT_X: usize = 60_000;

/// The facts of a file, or `None` when it cannot be analyzed (it does not
/// parse, is too large or too deeply nested, or uses `eval` / `with`).
pub(crate) fn extract(
    src: &[u8],
    dialect: Dialect,
    file_name: Option<&str>,
    sites: &[SiteIn],
) -> Option<FileFacts> {
    if src.len() > super::MAX_BYTES || dialect.dts {
        return None;
    }
    let text = std::str::from_utf8(src).ok()?;
    if !super::shallow_enough(src, dialect) {
        return None;
    }
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("turbo-trace-facts".into())
            .stack_size(super::STACK_BYTES)
            .spawn_scoped(scope, || analyze(text, dialect, file_name, sites))
            .ok()?
            .join()
            .ok()?
    })
}

fn analyze(
    text: &str,
    dialect: Dialect,
    file_name: Option<&str>,
    sites: &[SiteIn],
) -> Option<FileFacts> {
    for st in super::source_types(dialect) {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, text, st)
            .with_options(ParseOptions {
                allow_return_outside_function: true,
                ..ParseOptions::default()
            })
            .parse();
        if ret.is_flow_language {
            return None;
        }
        if ret.fatal_error || !ret.diagnostics.is_empty() {
            continue;
        }
        let sem = SemanticBuilder::new()
            .with_build_nodes(true)
            .build(&ret.program)
            .semantic;
        let nodes = sem.nodes();
        let scoping = sem.scoping();
        let mut engine = Engine::new(nodes, scoping, text, file_name);
        engine.symbolic();
        let mut cx = Cx {
            eng: engine,
            lines: LineIndex::new(text.as_bytes()),
            out: FileFacts::default(),
            fns: HashMap::new(),
            classes: HashMap::new(),
            imports: HashMap::new(),
            targets: HashMap::new(),
            memo: HashMap::new(),
            room: MAX_X,
            module: false,
            pending: Vec::new(),
            unrecorded: Vec::new(),
            recorded: HashSet::new(),
            calls_overflow: false,
            taints: HashMap::new(),
            taint_unknown: HashSet::new(),
            callee_memo: HashMap::new(),
            callee_busy: Vec::new(),
            exported: false,
        };
        if !cx.index() {
            // `eval` and `with` can rebind anything.
            return None;
        }
        let facts = cx.eng.facts();
        cx.out.writes = Writes {
            props: facts.written_props(),
            any: facts.writes_any_prop(),
            arrays: facts.array_mutation(),
        };
        // Every call is evaluated before any value is described: a value handed to a
        // callee can be changed by it, which is part of the description of every read
        // through that value.
        cx.eval_calls();
        cx.imports_and_exports();
        cx.record_unrecorded();
        cx.sinks(sites);
        cx.tables(sites);
        cx.calls();
        cx.functions();
        return Some(cx.out);
    }
    None
}

/// What a site's expression is.
enum Target<'a> {
    Arg(&'a Argument<'a>),
    Expr(&'a Expression<'a>),
    New(&'a NewExpression<'a>),
}

/// How a reference uses a binding.
enum Use {
    /// The callee of a call or `new` (through the member path, for namespaces).
    Call,
    /// A value: the binding itself (empty path) or a member of it.
    Value(Option<String>),
    /// Exported under this name.
    Export(String),
    /// A type position.
    Type,
}

struct Cx<'a, 's> {
    eng: Engine<'a, 's>,
    lines: LineIndex,
    out: FileFacts,
    /// Functions by the offset where they start.
    fns: HashMap<u32, (FnRef<'a>, NodeId)>,
    /// Class symbols with the class's constructor (`None`: the implicit one).
    classes: HashMap<SymbolId, Option<u32>>,
    /// Imported bindings: index into `out.imports` and the export path
    /// (`default`, a name, `*`).
    imports: HashMap<SymbolId, (usize, String)>,
    /// Site expressions by start offset.
    targets: HashMap<u32, Target<'a>>,
    /// Expression nodes already converted.
    memo: HashMap<u32, (X, usize)>,
    /// Nodes the expression being converted may still take ([`Cx::value`]).
    room: usize,
    /// The file has `import` / `export` syntax.
    module: bool,
    /// The calls of the file with their values ([`Cx::eval_calls`]), described
    /// later ([`Cx::calls`]).
    pending: Vec<Pending<'a>>,
    /// Callees of calls that are not recorded although they start at an
    /// imported binding.
    unrecorded: Vec<&'a Expression<'a>>,
    /// The calls (start offsets) that are recorded ([`Pending::record`]).
    recorded: HashSet<u32>,
    /// The file has more calls than are recorded.
    calls_overflow: bool,
    /// Nodes handed to a recorded call: `(index into pending, argument
    /// position)`.
    taints: HashMap<u32, Vec<(usize, u32)>>,
    /// Nodes handed to code that is not described.
    taint_unknown: HashSet<u32>,
    /// The callees of pending calls as expressions.
    callee_memo: HashMap<usize, X>,
    callee_busy: Vec<usize>,
    /// The value being described is an export: every container in it is
    /// reachable by other code of the file (through `module.exports`),
    /// whatever its own trust says.
    exported: bool,
}

/// A parameter with the pending calls it is handed to (index, argument
/// position) and whether it is also handed to code that is not described.
type RawHandoff = (u32, Vec<(usize, u32)>, bool);

/// What a pending call calls.
enum PCallee<'a> {
    /// The constructor of a class of the file.
    Ctor(u32),
    /// An imported binding, a hole, the result of a call.
    Val(V<'a>),
}

/// A call that will be recorded.
struct Pending<'a> {
    callee: PCallee<'a>,
    args: Vec<V<'a>>,
    rest: Option<V<'a>>,
    line: u32,
    new: bool,
    within: Option<u32>,
    /// Whether the call is one of the file's [`Call`]s (its callee can be a
    /// function of another file); otherwise it only says what its arguments
    /// are handed to.
    record: bool,
    /// The callee is the callback of an array method. When it is the result of
    /// a call (`xs.map(merge(spec))`), it is described without that call's
    /// arguments: they are often what the callback is handed, and their
    /// description would name this callee again.
    shallow: bool,
}

/// Globals whose calls do not change their arguments in a way the file-wide
/// scan of writes does not see (`Object.assign` and the like are part of that
/// scan).
const QUIET_GLOBALS: &[&str] = &[
    "console",
    "JSON",
    "Math",
    "Object",
    "Array",
    "String",
    "Number",
    "Boolean",
    "Promise",
    "Symbol",
    "Date",
    "RegExp",
    "Error",
    "TypeError",
    "RangeError",
    "Set",
    "Map",
    "WeakMap",
    "WeakSet",
    "BigInt",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "require",
    "process",
    "Buffer",
    "URL",
    "URLSearchParams",
    "describe",
    "it",
    "test",
    "expect",
    "beforeAll",
    "beforeEach",
    "afterAll",
    "afterEach",
    "jest",
    "vi",
    "setTimeout",
    "setInterval",
    "setImmediate",
    "clearTimeout",
    "clearInterval",
    "queueMicrotask",
    "structuredClone",
    "encodeURIComponent",
    "decodeURIComponent",
    "encodeURI",
    "decodeURI",
    "fetch",
    "Reflect",
    "Intl",
    "TextEncoder",
    "TextDecoder",
    "AbortController",
    "import",
];

/// Methods that add to their receiver: part of the file-wide scan of writes.
const ADDER_METHODS: &[&str] = &[
    "push",
    "unshift",
    "add",
    "splice",
    "fill",
    "copyWithin",
    "set",
];

fn unknown(text: &str) -> X {
    X::Const(JsVal::Unknown(origin::mark(Tag::Unknown, "", text)))
}

fn export_name(n: &ModuleExportName<'_>) -> Option<String> {
    match n {
        ModuleExportName::IdentifierName(i) => Some(i.name.as_str().to_owned()),
        ModuleExportName::IdentifierReference(i) => Some(i.name.as_str().to_owned()),
        ModuleExportName::StringLiteral(l) => l.value.as_str().map(str::to_owned),
    }
}

impl<'a> Cx<'a, '_> {
    fn line(&self, offset: u32) -> u32 {
        self.lines.line_col(offset).line
    }

    /// Indexes functions, classes and site expressions. `false`: the file uses
    /// `eval` or `with`.
    fn index(&mut self) -> bool {
        let nodes = self.eng.nodes;
        for node in nodes.iter() {
            match node.kind() {
                AstKind::Function(f) if f.body.is_some() => {
                    self.fns
                        .insert(f.span.start, (FnRef::Function(f), node.id()));
                }
                AstKind::ArrowFunctionExpression(f) => {
                    self.fns.insert(f.span.start, (FnRef::Arrow(f), node.id()));
                }
                AstKind::Class(c) => {
                    let ctor = c.body.body.iter().find_map(|el| match el {
                        ClassElement::MethodDefinition(m)
                            if m.kind == MethodDefinitionKind::Constructor
                                && m.value.body.is_some() =>
                        {
                            Some(m.value.span.start)
                        }
                        _ => None,
                    });
                    let mut syms: Vec<SymbolId> = Vec::new();
                    syms.extend(c.id.as_ref().and_then(|id| id.symbol_id.get()));
                    if let (_, AstKind::VariableDeclarator(d)) = self.eng.parent(node.id())
                        && let BindingPattern::BindingIdentifier(id) = &d.id
                    {
                        syms.extend(id.symbol_id.get());
                    }
                    for s in syms {
                        self.classes.insert(s, ctor);
                    }
                }
                AstKind::CallExpression(c) => {
                    for a in &c.arguments {
                        self.targets.insert(a.span().start, Target::Arg(a));
                    }
                    if let Expression::Identifier(id) = &c.callee
                        && id.name == "eval"
                        && id
                            .reference_id
                            .get()
                            .is_none_or(|r| !self.eng.scoping.has_binding(r))
                    {
                        return false;
                    }
                }
                AstKind::NewExpression(n) => {
                    for a in &n.arguments {
                        self.targets.insert(a.span().start, Target::Arg(a));
                    }
                }
                AstKind::ImportExpression(i) => {
                    self.targets
                        .insert(i.source.span().start, Target::Expr(&i.source));
                }
                AstKind::WithStatement(_) => return false,
                AstKind::ImportDeclaration(_)
                | AstKind::ExportNamedDeclaration(_)
                | AstKind::ExportFromDeclaration(_)
                | AstKind::ExportDeclaration(_)
                | AstKind::ExportAllDeclaration(_)
                | AstKind::ExportDefaultDeclaration(_) => self.module = true,
                AstKind::IdentifierReference(id)
                    if matches!(id.name.as_str(), "module" | "exports")
                        && id
                            .reference_id
                            .get()
                            .is_none_or(|r| !self.eng.scoping.has_binding(r)) =>
                {
                    if !self.listed_export(node.id(), id.name.as_str()) {
                        self.out.exports_open = true;
                    }
                }
                AstKind::TSExportAssignment(_) => self.out.exports_open = true,
                _ => {}
            }
        }
        for node in nodes.iter() {
            if let AstKind::NewExpression(n) = node.kind() {
                self.targets.entry(n.span.start).or_insert(Target::New(n));
            }
        }
        true
    }

    /// Whether the reference `node` to the global `module` or `exports` is the
    /// target of one of the assignments [`Cx::cjs_export`] records:
    /// `module.exports = v`, `module.exports.name = v`, `exports.name = v`.
    fn listed_export(&self, node: NodeId, name: &str) -> bool {
        let span = self.eng.nodes.kind(node).span();
        let assigned = |id: NodeId, mspan: oxc_span::Span| {
            matches!(
                self.eng.parent(id),
                (_, AstKind::AssignmentExpression(a))
                    if a.operator == AssignmentOperator::Assign && a.left.span() == mspan
            )
        };
        let (p1, AstKind::StaticMemberExpression(m1)) = self.eng.parent(node) else {
            return false;
        };
        if m1.object.get_inner_expression().span() != span {
            return false;
        }
        if name == "exports" {
            return assigned(p1, m1.span);
        }
        if m1.property.name != "exports" {
            return false;
        }
        if assigned(p1, m1.span) {
            return true;
        }
        match self.eng.parent(p1) {
            (p2, AstKind::StaticMemberExpression(m2))
                if m2.object.get_inner_expression().span() == m1.span =>
            {
                assigned(p2, m2.span)
            }
            _ => false,
        }
    }

    // ---- values ----

    /// What can have changed the object the node `base` stands for before a
    /// read of `prop` (`None`: an element, any property) through it: a
    /// write of the file itself (unknown), or a callee the object, or one
    /// it was reached through, is handed to ([`X::Changed`]: the callee's
    /// facts decide).
    fn changed(&mut self, base: u32, prop: Option<&str>, depth: u32) -> Option<X> {
        if self.out.writes.hits(prop) {
            return Some(unknown("written elsewhere in the file"));
        }
        let handed = || Some(unknown("handed to code that can change it"));
        if self.calls_overflow {
            return handed();
        }
        let mut by: Vec<(usize, u32)> = Vec::new();
        let mut id = base;
        for _ in 0..64 {
            if self.taint_unknown.contains(&id) {
                return handed();
            }
            if let Some(l) = self.taints.get(&id) {
                for b in l {
                    if !by.contains(b) {
                        by.push(*b);
                    }
                }
            }
            match self.eng.sym_node(id) {
                Some(Sym::Field(b, _) | Sym::Elem(b) | Sym::AnyField(b)) => id = *b,
                Some(Sym::Reach { base, .. }) => id = *base,
                _ => break,
            }
        }
        if by.is_empty() {
            return None;
        }
        if by.len() > 16 {
            return handed();
        }
        let by: Vec<(X, u32)> = by
            .into_iter()
            .map(|(i, pos)| (self.pending_callee(i, depth), pos))
            .collect();
        // A callee that takes a large expression to describe is not worth asking: the
        // read is repeated at every use of the object.
        if by
            .iter()
            .any(|(c, _)| matches!(c, X::Const(JsVal::Unknown(_))) || size(c) > MAX_CALLEE_X)
        {
            return handed();
        }
        Some(X::Changed {
            by,
            prop: prop.map(str::to_owned),
        })
    }

    /// The callee of the pending call `i` as an expression.
    fn pending_callee(&mut self, i: usize, depth: u32) -> X {
        if let Some(x) = self.callee_memo.get(&i) {
            return x.clone();
        }
        // A callee that is itself read through an object it is handed (`o.fix()`).
        if self.callee_busy.contains(&i) {
            return unknown("callee");
        }
        let x = match self.pending.get(i).map(|p| &p.callee) {
            Some(PCallee::Ctor(c)) => X::Func(*c),
            Some(PCallee::Val(v)) => {
                let v = v.clone();
                let shallow = self.pending[i].shallow;
                self.callee_busy.push(i);
                let x = if shallow {
                    self.callback_x(&v, depth + 1)
                } else {
                    self.value_x(&v, depth + 1)
                };
                self.callee_busy.pop();
                x
            }
            None => unknown("callee"),
        };
        if self.room > 0 {
            self.callee_memo.insert(i, x.clone());
        }
        x
    }

    /// The callback `v` of an array method as a callee ([`Pending::shallow`]):
    /// the result of a call is that call with unknown arguments. What the
    /// call returns is then evaluated with unknown arguments: that decides
    /// the function when the callee returns one whatever it is passed (a
    /// curried function), and is an unknown callee (one that can change
    /// anything) otherwise.
    fn callback_x(&mut self, v: &V<'a>, depth: u32) -> X {
        let mut alts: Vec<X> = Vec::new();
        for l in v.leaves() {
            let call = match l {
                V::Js(j) => sym_of(j).and_then(|id| match self.eng.sym_node(id) {
                    Some(Sym::Call { callee, args, rest }) => {
                        Some((callee.clone(), args.len(), rest.is_some()))
                    }
                    _ => None,
                }),
                _ => None,
            };
            let x = match call {
                Some((callee, args, rest)) => {
                    let hidden = || unknown("an argument that is not described");
                    X::Call {
                        callee: Box::new(match callee {
                            Callee::Sym(i) => self.node_x(i, depth + 1),
                            Callee::Import(i) => X::Import {
                                module: i.module.to_string(),
                                name: i.name.to_string(),
                            },
                        }),
                        args: (0..args).map(|_| hidden()).collect(),
                        rest: rest.then(|| Box::new(hidden())),
                    }
                }
                None => self.value_x(l, depth),
            };
            if !alts.contains(&x) {
                alts.push(x);
            }
        }
        if alts.len() == 1 {
            alts.remove(0)
        } else {
            X::Alts(alts)
        }
    }

    /// `read` (a property or element of the node `base`) with what can have
    /// changed it.
    fn read_x(&mut self, base: u32, prop: Option<&str>, read: X, depth: u32) -> X {
        match self.changed(base, prop, depth) {
            Some(c) => X::Alts(vec![read, c]),
            None => read,
        }
    }

    fn node_x(&mut self, id: u32, depth: u32) -> X {
        // Nodes are shared (a DAG); an expression is a tree. Every copy counts against
        // the size bound, so a deeply shared node cannot unfold exponentially.
        if let Some((x, n)) = self.memo.get(&id) {
            if *n > self.room {
                self.room = 0;
                return unknown("value too large");
            }
            self.room -= n;
            return x.clone();
        }
        if self.room == 0 {
            return unknown("value too large");
        }
        self.room -= 1;
        if depth > 48 {
            return unknown("expression too deep");
        }
        let Some(node) = self.eng.sym_node(id).cloned() else {
            return unknown("expression");
        };
        let d = depth + 1;
        let x = match node {
            Sym::Param { func, index } => X::Param { func, index },
            Sym::Field(b, n) => {
                let x = X::Field(Box::new(self.node_x(b, d)), n.to_string());
                self.read_x(b, Some(&n), x, d)
            }
            Sym::Elem(b) => {
                let x = X::Elem(Box::new(self.node_x(b, d)));
                self.read_x(b, Some(ELEMENT), x, d)
            }
            Sym::AnyField(b) => {
                let x = X::AnyField(Box::new(self.node_x(b, d)));
                self.read_x(b, None, x, d)
            }
            Sym::Reach { base, steps } => {
                let x = X::Reach {
                    base: Box::new(self.node_x(base, d)),
                    steps: steps
                        .into_iter()
                        .map(|s| match s {
                            ReachStep::Field(n) => Read::Field(n.to_string()),
                            ReachStep::Elem => Read::Elem,
                            ReachStep::Any => Read::Any,
                        })
                        .collect(),
                };
                // Each of the reads on the way can see something that was changed.
                let mut alts = vec![x];
                let kinds: Vec<Option<String>> = match &alts[0] {
                    X::Reach { steps, .. } => steps
                        .iter()
                        .map(|s| match s {
                            Read::Field(n) => Some(n.clone()),
                            Read::Elem => Some(ELEMENT.to_owned()),
                            Read::Any => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                // Asked of every node of the walk that the file hands on: they are parts
                // of what the walk reads.
                let mut family: Vec<u32> = vec![id];
                let mut tainted: Vec<u32> = self.taints.keys().copied().collect();
                tainted.extend(self.taint_unknown.iter().copied());
                tainted.sort_unstable();
                tainted.dedup();
                for t in tainted {
                    if t != id && self.eng.sym_chain(t).0 == base {
                        family.push(t);
                    }
                }
                for k in kinds {
                    for n in &family {
                        if let Some(c) = self.changed(*n, k.as_deref(), d)
                            && !alts.contains(&c)
                        {
                            alts.push(c);
                        }
                    }
                }
                if alts.len() == 1 {
                    alts.remove(0)
                } else {
                    X::Alts(alts)
                }
            }
            Sym::Concat(a, b) => X::Concat(Box::new(self.op_x(a, d)), Box::new(self.op_x(b, d))),
            Sym::Apply(f, ops) => X::Apply(f, ops.into_iter().map(|o| self.op_x(o, d)).collect()),
            Sym::Opaque(ids) => X::Opaque(ids.into_iter().map(|i| self.node_x(i, d)).collect()),
            Sym::Index { obj, key } => X::Index {
                table: Box::new(self.value_x(&obj, d)),
                key: Box::new(self.node_x(key, d)),
            },
            Sym::Call { callee, args, rest } => X::Call {
                callee: Box::new(match callee {
                    Callee::Sym(i) => self.node_x(i, d),
                    Callee::Import(i) => X::Import {
                        module: i.module.to_string(),
                        name: i.name.to_string(),
                    },
                }),
                args: args.iter().map(|a| self.value_x(a, d)).collect(),
                rest: rest.map(|r| Box::new(self.value_x(&r, d))),
            },
        };
        // A truncated conversion is not reused.
        if self.room > 0 {
            let n = size(&x);
            self.memo.insert(id, (x.clone(), n));
        }
        x
    }

    fn op_x(&mut self, op: Op, depth: u32) -> X {
        match op {
            Op::Sym(i) => self.node_x(i, depth),
            Op::Const(j) => X::Const(j),
        }
    }

    fn js_x(&mut self, j: &JsVal, depth: u32) -> X {
        if let Some(id) = sym_of(j) {
            return self.node_x(id, depth);
        }
        let mut ids = Vec::new();
        syms_in(j, &mut ids);
        if !ids.is_empty() {
            return X::Opaque(ids.into_iter().map(|i| self.node_x(i, depth + 1)).collect());
        }
        match j {
            JsVal::Alts(vs) => X::Alts(vs.iter().map(|v| self.js_x(v, depth + 1)).collect()),
            // Builtin objects and functions, a path next to a resolved package: never a
            // value another file can use.
            JsVal::Resolved(..) | JsVal::Obj(_) | JsVal::Func(_) | JsVal::Arr(_) => {
                unknown("not a string")
            }
            j => X::Const(j.clone()),
        }
    }

    /// The engine value `v` as an expression.
    fn value_x(&mut self, v: &V<'a>, depth: u32) -> X {
        if depth > 48 {
            return unknown("value too deep");
        }
        if self.room == 0 {
            return unknown("value too large");
        }
        self.room -= 1;
        let d = depth + 1;
        match v {
            V::Js(j) => self.js_x(j, d),
            V::Void => X::Void,
            V::Object(o) => {
                let mut props = Vec::with_capacity(o.props.len());
                for (k, _) in &o.props {
                    let pv = self.eng.obj_get(o, k);
                    props.push((k.to_string(), self.value_x(&pv, d)));
                }
                let mut open = o.open.as_ref().map(|x| Box::new(self.value_x(x, d)));
                if o.map && open.is_none() {
                    // A `Map`: its elements are entries, which the object form does not show.
                    open = Some(Box::new(unknown("map entries")));
                }
                let facts = self.eng.facts();
                if any_written(o, &facts) || (self.exported && facts.any_prop_write()) {
                    open = Some(Box::new(match open {
                        Some(x) => X::Alts(vec![*x, unknown("property written elsewhere")]),
                        None => unknown("property written elsewhere"),
                    }));
                }
                X::Object { props, open }
            }
            V::Array(a) => {
                let elems = a.elems.iter().map(|e| self.value_x(e, d)).collect();
                let mut rest = a.rest.as_ref().map(|x| Box::new(self.value_x(x, d)));
                if (a.trust == Trust::Shared || self.exported) && self.eng.facts().array_mutation()
                {
                    rest = Some(Box::new(match rest {
                        Some(x) => X::Alts(vec![*x, unknown("array modified elsewhere")]),
                        None => unknown("array modified elsewhere"),
                    }));
                }
                X::Array { elems, rest }
            }
            V::Func(f) => X::Func(f.span().start),
            V::Import(i) => X::Import {
                module: i.module.to_string(),
                name: i.name.to_string(),
            },
            V::Builtin(Builtin::Tmp) => X::Const(JsVal::Path(
                Anchor::Tmp,
                turborepo_trace_core::eval::Value::unknown(origin::mark(
                    Tag::Unknown,
                    "",
                    "temporary path",
                )),
            )),
            V::Builtin(_) | V::Method(..) | V::Loader(_) => unknown("not a string"),
            V::Alts(vs) => X::Alts(vs.iter().map(|x| self.value_x(x, d)).collect()),
        }
    }

    /// `v` as an expression, bounded in size.
    fn value(&mut self, v: &V<'a>) -> X {
        self.value_max(v, MAX_X)
    }

    /// `v` as an expression of at most `max` nodes.
    fn value_max(&mut self, v: &V<'a>, max: usize) -> X {
        self.room = max;
        let x = self.value_x(v, 0);
        if self.room == 0 || size(&x) > max {
            return unknown("value too large");
        }
        x
    }

    /// The value of the binding `sym` (a class is its constructor).
    fn symbol_x(&mut self, sym: SymbolId) -> X {
        if let Some(ctor) = self.classes.get(&sym) {
            return match ctor {
                Some(c) => X::Func(*c),
                None => X::Void,
            };
        }
        let v = self.eng.site_symbol(sym);
        self.value(&v)
    }

    // ---- uses of bindings ----

    /// How the reference at `node` uses its binding.
    fn use_of(&self, node: NodeId, read: bool, write: bool) -> Use {
        if !read {
            return if write { Use::Value(None) } else { Use::Type };
        }
        let span = self.eng.nodes.kind(node).span();
        let (pid, pk) = self.eng.parent(node);
        match pk {
            AstKind::CallExpression(c) if c.callee.get_inner_expression().span() == span => {
                Use::Call
            }
            AstKind::NewExpression(n) if n.callee.get_inner_expression().span() == span => {
                Use::Call
            }
            AstKind::StaticMemberExpression(m)
                if m.object.get_inner_expression().span() == span =>
            {
                match self.eng.parent(pid) {
                    (_, AstKind::CallExpression(c))
                        if c.callee.get_inner_expression().span() == m.span =>
                    {
                        Use::Call
                    }
                    (_, AstKind::NewExpression(n))
                        if n.callee.get_inner_expression().span() == m.span =>
                    {
                        Use::Call
                    }
                    _ => Use::Value(Some(m.property.name.as_str().to_owned())),
                }
            }
            AstKind::ExportSpecifier(s) => match export_name(&s.exported) {
                Some(n) => Use::Export(n),
                None => Use::Value(None),
            },
            AstKind::ExportDefaultDeclaration(_) => Use::Export("default".to_owned()),
            AstKind::TSTypeQuery(_) | AstKind::TSTypeReference(_) | AstKind::TSQualifiedName(_) => {
                Use::Type
            }
            _ => Use::Value(None),
        }
    }

    /// Whether the binding `sym` is used as a value (other than called or
    /// exported), and the names it is exported under.
    fn uses(&self, sym: SymbolId) -> (bool, Vec<String>) {
        let mut escapes = false;
        let mut exported = Vec::new();
        for r in self.eng.scoping.get_resolved_references(sym) {
            match self.use_of(r.node_id(), r.is_read(), r.is_write()) {
                Use::Call | Use::Type => {}
                Use::Export(n) => exported.push(n),
                Use::Value(_) => escapes = true,
            }
        }
        (escapes, exported)
    }

    // ---- imports and exports ----

    fn imports_and_exports(&mut self) {
        let nodes = self.eng.nodes;
        for node in nodes.iter() {
            match node.kind() {
                AstKind::ImportDeclaration(d) => self.import(d),
                AstKind::ExportAllDeclaration(d) => {
                    if d.export_kind != ImportOrExportKind::Value {
                        continue;
                    }
                    let Some(spec) = d.source.value.as_str() else {
                        self.out.exports_open = true;
                        continue;
                    };
                    let (names, star) = match &d.exported {
                        Some(n) => match export_name(n) {
                            Some(n) => (vec![("*".to_owned(), n)], false),
                            None => {
                                self.out.exports_open = true;
                                continue;
                            }
                        },
                        None => (Vec::new(), true),
                    };
                    self.out.reexports.push(ReExport {
                        at: d.span.start,
                        spec: spec.to_owned(),
                        names,
                        star,
                    });
                }
                AstKind::AssignmentExpression(a) => self.cjs_export(a),
                AstKind::ExportNamedDeclaration(d) => self.export_named(d),
                AstKind::ExportFromDeclaration(d) => self.export_from(d),
                AstKind::ExportDeclaration(d) => self.export_declaration(d),
                AstKind::ExportDefaultDeclaration(d) => {
                    let value = match &d.declaration {
                        ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                            if f.body.is_some() {
                                X::Func(f.span.start)
                            } else {
                                continue;
                            }
                        }
                        ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                            match self.class_ctor(c) {
                                Some(id) => X::Func(id),
                                None => X::Void,
                            }
                        }
                        ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => continue,
                        other => match other.as_expression() {
                            // An imported binding exported again is a re-export (recorded
                            // with the import's uses).
                            Some(Expression::Identifier(id))
                                if self
                                    .symbol_of(id)
                                    .is_some_and(|s| self.imports.contains_key(&s)) =>
                            {
                                continue;
                            }
                            Some(Expression::Identifier(id)) => match self.symbol_of(id) {
                                Some(s) => self.symbol_x(s),
                                None => unknown(id.name.as_str()),
                            },
                            Some(e) => {
                                let v = self.eng.site(e);
                                self.exported = true;
                                let x = match v {
                                    Some(v) => self.value(&v.shared()),
                                    None => unknown("default export"),
                                };
                                self.exported = false;
                                x
                            }
                            None => continue,
                        },
                    };
                    self.out.exports.push(Export {
                        name: "default".to_owned(),
                        value,
                    });
                }
                _ => {}
            }
        }
    }

    /// `module.exports = v` (the export named `module.exports`: what `require`
    /// returns), `module.exports.name = v` and `exports.name = v`.
    fn cjs_export(&mut self, a: &'a AssignmentExpression<'a>) {
        if a.operator != AssignmentOperator::Assign {
            return;
        }
        let AssignmentTarget::StaticMemberExpression(m) = &a.left else {
            return;
        };
        let global = |e: &Expression<'a>, name: &str| match e.get_inner_expression() {
            Expression::Identifier(id) => id.name == name && self.symbol_of(id).is_none(),
            _ => false,
        };
        let prop = m.property.name.as_str();
        let name = if global(&m.object, "module") && prop == "exports" {
            "module.exports".to_owned()
        } else if global(&m.object, "exports") {
            prop.to_owned()
        } else {
            match m.object.get_inner_expression() {
                Expression::StaticMemberExpression(o)
                    if global(&o.object, "module") && o.property.name == "exports" =>
                {
                    prop.to_owned()
                }
                _ => return,
            }
        };
        // The exported value is reachable through `module.exports` from now on: the
        // file's untracked writes apply to it.
        self.exported = true;
        let value = match self.eng.site(&a.right) {
            Some(v) => self.value_max(&v.shared(), MAX_EXPORT_X),
            None => unknown("export"),
        };
        self.exported = false;
        self.out.exports.push(Export { name, value });
    }

    fn symbol_of(&self, id: &IdentifierReference<'a>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|r| self.eng.scoping.get_reference(r).symbol_id())
    }

    fn class_ctor(&self, c: &Class<'a>) -> Option<u32> {
        c.body.body.iter().find_map(|el| match el {
            ClassElement::MethodDefinition(m)
                if m.kind == MethodDefinitionKind::Constructor && m.value.body.is_some() =>
            {
                Some(m.value.span.start)
            }
            _ => None,
        })
    }

    fn import(&mut self, d: &'a ImportDeclaration<'a>) {
        if d.import_kind != ImportOrExportKind::Value {
            return;
        }
        let Some(spec) = d.source.value.as_str() else {
            return;
        };
        let ix = self.out.imports.len();
        let mut imp = Import {
            at: d.span.start,
            spec: spec.to_owned(),
            escapes: Vec::new(),
            writes: false,
        };
        let mut reexported: Vec<(String, String)> = Vec::new();
        for s in d.specifiers.iter().flatten() {
            let (sym, path) = match s {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    if s.import_kind != ImportOrExportKind::Value {
                        continue;
                    }
                    match export_name(&s.imported) {
                        Some(n) => (s.local.symbol_id.get(), n),
                        None => (None, "*".to_owned()),
                    }
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                    (s.local.symbol_id.get(), "default".to_owned())
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                    (s.local.symbol_id.get(), "*".to_owned())
                }
            };
            let Some(sym) = sym else {
                // A binding that cannot be followed: anything may happen to the module.
                push_unique(&mut imp.escapes, "*".to_owned());
                imp.writes = true;
                continue;
            };
            self.imports.insert(sym, (ix, path.clone()));
            let refs: Vec<(NodeId, bool, bool)> = self
                .eng
                .scoping
                .get_resolved_references(sym)
                .map(|r| (r.node_id(), r.is_read(), r.is_write()))
                .collect();
            for (node, read, write) in refs {
                match self.use_of(node, read, write) {
                    // Calling an export is the module's own code at work; a builtin
                    // method that changes its receiver is not (`list.push(x)`).
                    Use::Call => imp.writes |= self.changes_receiver(node),
                    Use::Type => {}
                    Use::Export(n) => reexported.push((path.clone(), n)),
                    Use::Value(None) => {
                        push_unique(&mut imp.escapes, path.clone());
                        imp.writes |= !self.eng.site_use_only_reads(node);
                    }
                    Use::Value(Some(member)) => {
                        let p = if path == "*" {
                            member
                        } else {
                            format!("{path}.{member}")
                        };
                        push_unique(&mut imp.escapes, p);
                        imp.writes |= !self.eng.site_use_only_reads(node);
                    }
                }
            }
        }
        if !reexported.is_empty() {
            self.out.reexports.push(ReExport {
                at: d.span.start,
                spec: spec.to_owned(),
                names: reexported,
                star: false,
            });
        }
        self.out.imports.push(imp);
    }

    /// Whether the reference `node` is the receiver of a call of a builtin
    /// method that changes its receiver (`binding.push(x)`,
    /// `binding.sort()`).
    fn changes_receiver(&self, node: NodeId) -> bool {
        let span = self.eng.nodes.kind(node).span();
        match self.eng.parent(node) {
            (_, AstKind::StaticMemberExpression(m))
                if m.object.get_inner_expression().span() == span =>
            {
                matches!(
                    m.property.name.as_str(),
                    "push"
                        | "unshift"
                        | "add"
                        | "splice"
                        | "fill"
                        | "copyWithin"
                        | "set"
                        | "sort"
                        | "reverse"
                        | "shift"
                        | "pop"
                        | "delete"
                        | "clear"
                )
            }
            _ => false,
        }
    }

    fn export_from(&mut self, d: &'a ExportFromDeclaration<'a>) {
        if d.export_kind != ImportOrExportKind::Value {
            return;
        }
        let Some(spec) = d.source.value.as_str() else {
            self.out.exports_open = true;
            return;
        };
        let mut names = Vec::new();
        for s in &d.specifiers {
            if s.export_kind != ImportOrExportKind::Value {
                continue;
            }
            match (export_name(&s.local), export_name(&s.exported)) {
                (Some(l), Some(e)) => names.push((l, e)),
                _ => self.out.exports_open = true,
            }
        }
        self.out.reexports.push(ReExport {
            at: d.span.start,
            spec: spec.to_owned(),
            names,
            star: false,
        });
    }

    fn export_declaration(&mut self, d: &'a ExportDeclaration<'a>) {
        match &d.declaration {
            Declaration::FunctionDeclaration(f) => {
                if let (Some(id), true) = (&f.id, f.body.is_some()) {
                    self.out.exports.push(Export {
                        name: id.name.as_str().to_owned(),
                        value: X::Func(f.span.start),
                    });
                }
            }
            Declaration::ClassDeclaration(c) => {
                if let Some(id) = &c.id {
                    let value = match self.class_ctor(c) {
                        Some(ctor) => X::Func(ctor),
                        None => X::Void,
                    };
                    self.out.exports.push(Export {
                        name: id.name.as_str().to_owned(),
                        value,
                    });
                }
            }
            Declaration::VariableDeclaration(v) => {
                for decl in &v.declarations {
                    match &decl.id {
                        BindingPattern::BindingIdentifier(id) => {
                            let value = match id.symbol_id.get() {
                                Some(s) => self.symbol_x(s),
                                None => unknown(id.name.as_str()),
                            };
                            self.out.exports.push(Export {
                                name: id.name.as_str().to_owned(),
                                value,
                            });
                        }
                        // Destructured exports: names the facts do not describe.
                        _ => self.out.exports_open = true,
                    }
                }
            }
            Declaration::TSEnumDeclaration(e) => {
                let value = match e.id.symbol_id.get() {
                    Some(s) => self.symbol_x(s),
                    None => unknown(e.id.name.as_str()),
                };
                self.out.exports.push(Export {
                    name: e.id.name.as_str().to_owned(),
                    value,
                });
            }
            _ => {}
        }
    }

    fn export_named(&mut self, d: &'a ExportNamedDeclaration<'a>) {
        if d.export_kind != ImportOrExportKind::Value {
            return;
        }
        for s in &d.specifiers {
            if s.export_kind != ImportOrExportKind::Value {
                continue;
            }
            let ModuleExportName::IdentifierReference(id) = &s.local else {
                continue;
            };
            let Some(sym) = self.symbol_of(id) else {
                continue;
            };
            if self.imports.contains_key(&sym) {
                continue;
            }
            let Some(name) = export_name(&s.exported) else {
                self.out.exports_open = true;
                continue;
            };
            let value = self.symbol_x(sym);
            self.out.exports.push(Export { name, value });
        }
    }

    // ---- sinks ----

    fn sinks(&mut self, sites: &[SiteIn]) {
        for s in sites {
            if s.load {
                continue;
            }
            let Some(t) = self.targets.get(&s.expr.start) else {
                continue;
            };
            let end = match t {
                Target::Arg(a) => a.span().end,
                Target::Expr(e) => e.span().end,
                Target::New(n) => n.span.end,
            };
            if !s.open_end && end != s.expr.end {
                continue;
            }
            // The argument as an expression, for the conditions around the site.
            let arg: Option<&'a Expression<'a>> = match t {
                Target::Arg(a) => {
                    let a: &'a Argument<'a> = a;
                    a.as_expression()
                }
                Target::Expr(e) => Some(*e),
                Target::New(_) => None,
            };
            let v = match t {
                Target::Arg(Argument::SpreadElement(sp)) => self.eng.site_spread(&sp.argument),
                Target::Arg(a) => match a.as_expression() {
                    Some(e) => self.eng.site(e),
                    None => None,
                },
                Target::Expr(e) => self.eng.site(e),
                Target::New(n) => self.eng.site_new(n),
            };
            let Some(v) = v else { continue };
            // Only the alternatives that depend on holes: the others are the file's own
            // references.
            let mut alts = Vec::new();
            let mut unknowns = Vec::new();
            for l in v.leaves() {
                let x = self.value(l);
                if x.is_open() {
                    if !alts.contains(&x) {
                        alts.push(x);
                    }
                } else if partly_unknown(&x) {
                    // A value with an unknown part: the site's record can stand for it
                    // too, so the record has to stay.
                    let x = match x {
                        X::Const(JsVal::Unknown(_)) => x,
                        _ => unknown("a partly unknown value"),
                    };
                    if !unknowns.contains(&x) {
                        unknowns.push(x);
                    }
                }
            }
            if alts.is_empty() {
                continue;
            }
            // An unknown alternative of the file itself stays part of the value: it can be
            // what the symbolic evaluation gave up on (a recursion through a parameter
            // that does not settle), and then the alternatives above are not all there is.
            alts.extend(unknowns);
            // A resolved site is only worth binding when a key selects among its values.
            if !s.param && !alts.iter().any(has_index) {
                continue;
            }
            let value = if alts.len() == 1 {
                alts.remove(0)
            } else {
                X::Alts(alts)
            };
            let (string_only, string_tests) = match arg {
                Some(e) => self.string_guards(e),
                None => (false, Vec::new()),
            };
            self.out.sinks.push(Sink {
                kind: s.kind,
                rule: s.rule,
                at: [s.arg_span.start, s.arg_span.end],
                site: [s.span.start, s.span.end],
                open: s.param,
                line: self.line(s.arg_span.start),
                value,
                string_only,
                string_tests,
            });
        }
    }

    // ---- tests for a string ----

    /// Whether the binding `sym` holds one value for as long as it lives: a
    /// parameter or a `const`, declared once and never assigned (a
    /// parameter of a function that uses `arguments` can be assigned
    /// through it).
    fn fixed_binding(&mut self, sym: SymbolId) -> bool {
        let scoping = self.eng.scoping;
        if !scoping.symbol_redeclarations(sym).is_empty()
            || scoping.get_resolved_references(sym).any(|r| r.is_write())
        {
            return false;
        }
        let nodes = self.eng.nodes;
        let decl = scoping.symbol_declaration(sym);
        match nodes.kind(decl) {
            AstKind::VariableDeclarator(_) => matches!(
                nodes.parent_kind(decl),
                AstKind::VariableDeclaration(v) if v.kind == VariableDeclarationKind::Const
            ),
            AstKind::FormalParameter(_) => {
                // The function the parameter belongs to: an arrow function has no
                // `arguments` of its own.
                for k in nodes.ancestor_kinds(decl) {
                    match k {
                        AstKind::ArrowFunctionExpression(_) => return true,
                        AstKind::Function(f) => {
                            return !self.eng.facts().uses_arguments(f.span.start);
                        }
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Whether `e` is `typeof <sym> === 'string'` (either order, `==` too).
    fn typeof_string(&self, e: &Expression<'a>, sym: SymbolId) -> bool {
        let Expression::BinaryExpression(b) = e.get_inner_expression() else {
            return false;
        };
        if !matches!(
            b.operator,
            BinaryOperator::StrictEquality | BinaryOperator::Equality
        ) {
            return false;
        }
        let pair = |t: &Expression<'a>, s: &Expression<'a>| {
            matches!(s.get_inner_expression(), Expression::StringLiteral(l) if l.value.as_str() == Some("string"))
                && matches!(
                    t.get_inner_expression(),
                    Expression::UnaryExpression(u)
                        if u.operator == UnaryOperator::Typeof
                            && matches!(
                                u.argument.get_inner_expression(),
                                Expression::Identifier(i) if self.symbol_of(i) == Some(sym)
                            )
                )
        };
        pair(&b.left, &b.right) || pair(&b.right, &b.left)
    }

    /// The conjuncts of the condition `e` (`a && b && c`).
    fn conjuncts(e: &'a Expression<'a>, out: &mut Vec<&'a Expression<'a>>) {
        match e.get_inner_expression() {
            Expression::LogicalExpression(l) if l.operator == LogicalOperator::And => {
                Self::conjuncts(&l.left, out);
                Self::conjuncts(&l.right, out);
            }
            e => out.push(e),
        }
    }

    /// What is known about the argument `e` of a site from the conditions
    /// around the site ([`Sink::string_only`], [`Sink::string_tests`]). The
    /// argument has to be a binding with one value ([`Cx::fixed_binding`]):
    /// the test then holds whenever the site runs, also inside a function
    /// made behind the test.
    fn string_guards(&mut self, e: &'a Expression<'a>) -> (bool, Vec<(X, u32)>) {
        let none = (false, Vec::new());
        let Expression::Identifier(id) = e.get_inner_expression() else {
            return none;
        };
        let Some(rid) = id.reference_id.get() else {
            return none;
        };
        let reference = self.eng.scoping.get_reference(rid);
        let (Some(sym), node) = (reference.symbol_id(), reference.node_id()) else {
            return none;
        };
        if !self.fixed_binding(sym) {
            return none;
        }
        let within =
            |outer: oxc_span::Span| outer.start <= id.span.start && id.span.end <= outer.end;
        // The conditions that hold where the site is.
        let nodes = self.eng.nodes;
        let mut conds: Vec<&'a Expression<'a>> = Vec::new();
        let mut cur = node;
        loop {
            let p = nodes.parent_id(cur);
            if p == cur {
                break;
            }
            match nodes.kind(p) {
                AstKind::IfStatement(i) if within(i.consequent.span()) => {
                    Self::conjuncts(&i.test, &mut conds);
                }
                AstKind::ConditionalExpression(c) if within(c.consequent.span()) => {
                    Self::conjuncts(&c.test, &mut conds);
                }
                AstKind::LogicalExpression(l)
                    if l.operator == LogicalOperator::And && within(l.right.span()) =>
                {
                    Self::conjuncts(&l.left, &mut conds);
                }
                _ => {}
            }
            cur = p;
        }
        let mut only = false;
        let mut tests: Vec<(X, u32)> = Vec::new();
        for c in conds {
            if self.typeof_string(c, sym) {
                only = true;
                continue;
            }
            let Expression::CallExpression(call) = c else {
                continue;
            };
            if call.optional {
                continue;
            }
            // The binding as a plain argument, with no spread before it.
            let mut pos = None;
            for (i, a) in call.arguments.iter().enumerate() {
                let Some(a) = a.as_expression() else { break };
                if matches!(a.get_inner_expression(), Expression::Identifier(x) if self.symbol_of(x) == Some(sym))
                {
                    pos = Some(i as u32);
                    break;
                }
            }
            let (Some(pos), Some(v)) = (pos, self.eng.site(&call.callee)) else {
                continue;
            };
            let x = self.value_max(&v, MAX_CALLEE_X);
            if !matches!(x, X::Const(JsVal::Unknown(_))) && tests.len() < 8 {
                tests.push((x, pos));
            }
        }
        (only, tests)
    }

    /// The position of the parameter that `f` tests for a string
    /// ([`FnFact::tests_string`]): `f` is a plain function whose body is one
    /// returned expression with the conjunct `typeof <parameter> ===
    /// 'string'`.
    fn tests_string(&mut self, f: FnRef<'a>) -> Option<u32> {
        // The one expression a body returns.
        fn single<'a>(body: &'a FunctionBody<'a>) -> Option<&'a Expression<'a>> {
            match body.statements.as_slice() {
                [Statement::ReturnStatement(r)] => r.argument.as_ref(),
                _ => None,
            }
        }
        let (params, returned): (_, &'a Expression<'a>) = match f {
            FnRef::Function(func) => {
                if func.r#async
                    || func.generator
                    || self.eng.facts().uses_arguments(func.span.start)
                {
                    return None;
                }
                (&func.params, single(func.body.as_ref()?)?)
            }
            FnRef::Arrow(a) => {
                if a.r#async {
                    return None;
                }
                match &a.body {
                    ArrowFunctionBody::FunctionBody(b) => (&a.params, single(b)?),
                    other => (&a.params, other.as_expression()?),
                }
            }
        };
        let mut conds = Vec::new();
        Self::conjuncts(returned, &mut conds);
        for (i, p) in params.items.iter().enumerate() {
            // A parameter with a default is not what the caller passed.
            if p.initializer.is_some() {
                continue;
            }
            let BindingPattern::BindingIdentifier(id) = &p.pattern else {
                continue;
            };
            let Some(sym) = id.symbol_id.get() else {
                continue;
            };
            if self
                .eng
                .scoping
                .get_resolved_references(sym)
                .any(|r| r.is_write())
            {
                continue;
            }
            if conds.iter().any(|c| self.typeof_string(c, sym)) {
                return Some(i as u32);
            }
        }
        None
    }

    // ---- calls ----

    /// The innermost function containing `node`.
    fn within(&self, node: NodeId) -> Option<u32> {
        let nodes = self.eng.nodes;
        let mut id = node;
        loop {
            let p = nodes.parent_id(id);
            if p == id {
                return None;
            }
            match nodes.kind(p) {
                AstKind::Function(f) => return Some(f.span.start),
                AstKind::ArrowFunctionExpression(f) => return Some(f.span.start),
                _ => id = p,
            }
        }
    }

    /// Whether the callee `e` can be something other than a function of the
    /// file or a global: its root identifier is a binding that is not a
    /// function declaration.
    fn callee_candidate(&self, e: &'a Expression<'a>) -> bool {
        let mut e = e.get_inner_expression();
        loop {
            match e {
                Expression::StaticMemberExpression(m) => e = m.object.get_inner_expression(),
                Expression::ComputedMemberExpression(m) => e = m.object.get_inner_expression(),
                Expression::CallExpression(c) => e = c.callee.get_inner_expression(),
                Expression::Identifier(id) => {
                    let Some(sym) = self.symbol_of(id) else {
                        return false;
                    };
                    return !self
                        .eng
                        .scoping
                        .symbol_declarations(sym)
                        .any(|d| matches!(self.eng.nodes.kind(d), AstKind::Function(_)));
                }
                _ => return false,
            }
        }
    }

    /// Whether the callee value `v` can be a function of another file: an
    /// imported binding, a hole, or the result of calling one.
    fn external(&self, v: &V<'a>) -> bool {
        v.leaves().iter().any(|l| match l {
            V::Import(_) => true,
            V::Js(j) => sym_of(j).is_some_and(|id| {
                matches!(
                    self.eng.sym_node(id),
                    Some(Sym::Param { .. } | Sym::Call { .. })
                )
            }),
            _ => false,
        })
    }

    /// Whether the callee value `v` is something only binding can know: an
    /// imported binding or any value read from a hole or a call result (a
    /// method of an object a caller passes).
    fn symbolic(&self, v: &V<'a>) -> bool {
        v.leaves().iter().any(|l| match l {
            V::Import(_) => true,
            V::Js(j) => sym_of(j).is_some(),
            _ => false,
        })
    }

    /// Whether the value of the expression at `node` (spanning `span`) can only
    /// end up as the argument of a site, or nowhere (a test, a dropped
    /// value): through a path builder, a local name, or a local set or list
    /// that is only filled and iterated. `out` gets the argument spans of
    /// the sites.
    fn sunk(
        &self,
        node: NodeId,
        span: oxc_span::Span,
        sites: &[SiteIn],
        depth: u32,
        out: &mut Vec<[u32; 2]>,
    ) -> bool {
        if depth > 8 {
            return false;
        }
        for s in sites {
            let inside = if s.open_end {
                s.expr.start == span.start
            } else {
                s.expr.start <= span.start && span.end <= s.expr.end
            };
            if inside {
                let at = [s.arg_span.start, s.arg_span.end];
                if !out.contains(&at) {
                    out.push(at);
                }
                return true;
            }
        }
        let (pid, pk) = self.eng.parent(node);
        let is = |e: &Expression<'a>| e.get_inner_expression().span() == span;
        let up = |out: &mut Vec<[u32; 2]>| self.sunk(pid, pk.span(), sites, depth + 1, out);
        match pk {
            // Tested or dropped.
            AstKind::IfStatement(s) if is(&s.test) => true,
            AstKind::ConditionalExpression(c) if is(&c.test) => true,
            AstKind::UnaryExpression(u)
                if matches!(
                    u.operator,
                    UnaryOperator::LogicalNot | UnaryOperator::Typeof | UnaryOperator::Void
                ) =>
            {
                true
            }
            AstKind::ExpressionStatement(_) => true,
            AstKind::BinaryExpression(b)
                if matches!(
                    b.operator,
                    BinaryOperator::Equality
                        | BinaryOperator::Inequality
                        | BinaryOperator::StrictEquality
                        | BinaryOperator::StrictInequality
                        | BinaryOperator::In
                        | BinaryOperator::Instanceof
                ) =>
            {
                true
            }
            // Part of a larger value.
            AstKind::ConditionalExpression(_)
            | AstKind::LogicalExpression(_)
            | AstKind::TemplateLiteral(_) => up(out),
            AstKind::BinaryExpression(b) if b.operator == BinaryOperator::Addition => up(out),
            AstKind::CallExpression(c) if !is(&c.callee) => {
                if !c
                    .arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(&is))
                {
                    return false;
                }
                match c.callee.get_inner_expression() {
                    // `join(dir, v)`, `path.resolve(v)`: the result carries the value.
                    Expression::Identifier(id)
                        if matches!(id.name.as_str(), "join" | "resolve" | "normalize") =>
                    {
                        up(out)
                    }
                    Expression::StaticMemberExpression(m)
                        if matches!(m.property.name.as_str(), "join" | "resolve" | "normalize")
                            && matches!(m.object.get_inner_expression(),
                                Expression::Identifier(o) if o.name == "path") =>
                    {
                        up(out)
                    }
                    // `set.add(v)`, `list.push(v)` on a local container.
                    Expression::StaticMemberExpression(m)
                        if matches!(m.property.name.as_str(), "add" | "push") =>
                    {
                        match m.object.get_inner_expression() {
                            Expression::Identifier(id) => match self.symbol_of(id) {
                                Some(sym) if self.eng.literal_binding(sym) => {
                                    self.container_sunk(sym, sites, depth + 1, out)
                                }
                                _ => false,
                            },
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }
            AstKind::VariableDeclarator(d) => match (&d.id, &d.init) {
                (BindingPattern::BindingIdentifier(id), Some(init)) if is(init) => {
                    match id.symbol_id.get() {
                        Some(sym) if !self.exported_declarator(pid) => {
                            self.refs_sunk(sym, sites, depth + 1, out)
                        }
                        _ => false,
                    }
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Whether the variable declarator `decl` is part of an `export`.
    fn exported_declarator(&self, decl: NodeId) -> bool {
        let list = self.eng.nodes.parent_id(decl);
        matches!(
            self.eng.nodes.parent_kind(list),
            AstKind::ExportDeclaration(_)
                | AstKind::ExportNamedDeclaration(_)
                | AstKind::ExportDefaultDeclaration(_)
        )
    }

    /// [`Cx::sunk`] for every use of the binding `sym`.
    fn refs_sunk(
        &self,
        sym: SymbolId,
        sites: &[SiteIn],
        depth: u32,
        out: &mut Vec<[u32; 2]>,
    ) -> bool {
        self.eng.scoping.get_resolved_references(sym).all(|r| {
            if r.is_write() {
                return false;
            }
            if !r.is_read() {
                return true;
            }
            let span = self.eng.nodes.kind(r.node_id()).span();
            self.sunk(r.node_id(), span, sites, depth, out)
        })
    }

    /// Whether the elements of the local set or list `sym` only end up at
    /// sites: it is filled, asked for its size or membership, and iterated
    /// with `for…of`.
    fn container_sunk(
        &self,
        sym: SymbolId,
        sites: &[SiteIn],
        depth: u32,
        out: &mut Vec<[u32; 2]>,
    ) -> bool {
        if self.exported_declarator(self.eng.scoping.symbol_declaration(sym)) {
            return false;
        }
        self.eng.scoping.get_resolved_references(sym).all(|r| {
            if r.is_write() {
                return false;
            }
            if !r.is_read() {
                return true;
            }
            let span = self.eng.nodes.kind(r.node_id()).span();
            match self.eng.parent(r.node_id()) {
                (mid, AstKind::StaticMemberExpression(m))
                    if m.object.get_inner_expression().span() == span =>
                {
                    let called = matches!(
                        self.eng.parent(mid),
                        (_, AstKind::CallExpression(c))
                            if c.callee.get_inner_expression().span() == m.span
                    );
                    match m.property.name.as_str() {
                        "add" | "push" | "has" | "includes" => called,
                        "size" | "length" => !called,
                        _ => false,
                    }
                }
                (_, AstKind::ForOfStatement(f))
                    if f.right.get_inner_expression().span() == span =>
                {
                    match &f.left {
                        ForStatementLeft::VariableDeclaration(v) => {
                            match v.declarations.as_slice() {
                                [d] => match &d.id {
                                    BindingPattern::BindingIdentifier(id) => {
                                        match id.symbol_id.get() {
                                            Some(s) => self.refs_sunk(s, sites, depth + 1, out),
                                            None => false,
                                        }
                                    }
                                    _ => false,
                                },
                                _ => false,
                            }
                        }
                        _ => false,
                    }
                }
                _ => false,
            }
        })
    }

    /// Lookup tables: object literals bound to a name that is read with a
    /// computed key.
    fn tables(&mut self, sites: &[SiteIn]) {
        const MAX_TABLES: usize = 32;
        let nodes = self.eng.nodes;
        for node in nodes.iter() {
            if self.out.tables.len() >= MAX_TABLES {
                break;
            }
            let AstKind::VariableDeclarator(d) = node.kind() else {
                continue;
            };
            let (BindingPattern::BindingIdentifier(id), Some(init)) = (&d.id, &d.init) else {
                continue;
            };
            let Expression::ObjectExpression(o) = init.get_inner_expression() else {
                continue;
            };
            let Some(sym) = id.symbol_id.get() else {
                continue;
            };
            // An exported table can be read whole by other files.
            let list = nodes.parent_id(node.id());
            let mut keyed_only = !matches!(nodes.parent_kind(list), AstKind::ExportDeclaration(_));
            let mut key_exprs: Vec<&'a Expression<'a>> = Vec::new();
            let mut keys: Vec<X> = Vec::new();
            // Where the values read by key end up.
            let mut sinks: Vec<[u32; 2]> = Vec::new();
            for r in self.eng.scoping.get_resolved_references(sym) {
                if !r.is_read() {
                    keyed_only &= !r.is_write();
                    continue;
                }
                let span = nodes.kind(r.node_id()).span();
                match self.eng.parent(r.node_id()) {
                    (mid, AstKind::ComputedMemberExpression(m))
                        if m.object.get_inner_expression().span() == span =>
                    {
                        keyed_only &= self.sunk(mid, m.span, sites, 0, &mut sinks);
                        key_exprs.push(&m.expression);
                    }
                    (mid, AstKind::StaticMemberExpression(m))
                        if m.object.get_inner_expression().span() == span =>
                    {
                        keyed_only &= self.sunk(mid, m.span, sites, 0, &mut sinks);
                        let k = X::Const(JsVal::Str(turborepo_trace_core::eval::Value::str(
                            m.property.name.as_str(),
                        )));
                        if !keys.contains(&k) {
                            keys.push(k);
                        }
                    }
                    _ => keyed_only = false,
                }
            }
            if key_exprs.is_empty() {
                continue;
            }
            let mut values = Vec::new();
            for p in &o.properties {
                let ObjectPropertyKind::ObjectProperty(p) = p else {
                    keyed_only = false;
                    continue;
                };
                let key = match &p.key {
                    PropertyKey::StaticIdentifier(i) => Some(i.name.as_str()),
                    PropertyKey::StringLiteral(s) if !p.computed => s.value.as_str(),
                    _ => None,
                };
                if let (Some(k), Expression::StringLiteral(v)) =
                    (key, p.value.get_inner_expression())
                    && let Some(v) = v.value.as_str()
                    && values.len() < 256
                {
                    values.push((k.to_owned(), v.to_owned()));
                }
            }
            if values.is_empty() {
                continue;
            }
            for e in key_exprs.into_iter().take(32) {
                let k = match self.eng.site(e) {
                    Some(v) => self.value(&v),
                    None => unknown("key"),
                };
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            sinks.sort_unstable();
            self.out.tables.push(Table {
                at: [o.span.start, o.span.end],
                keyed_only,
                keys,
                values,
                sinks,
            });
        }
    }

    /// A call that is not recorded although its callee starts at an imported
    /// binding (the evaluation ran out of budget, or the binding is used in
    /// a way that hides it): that import may be used in a way the facts do
    /// not show.
    ///
    /// A member of the result of a call that is recorded (`load(x).filter(f)`)
    /// is no such use: the binding is used by the recorded call, and what
    /// its result holds is a value of the module (a function the module
    /// hands out in a result is used as a value there, so its unknown
    /// callers are at home in the module).
    fn mark_unrecorded(&mut self, callee: &'a Expression<'a>) {
        let mut e = callee.get_inner_expression();
        let mut member = false;
        loop {
            match e {
                Expression::StaticMemberExpression(m) => {
                    member = true;
                    e = m.object.get_inner_expression();
                }
                Expression::ComputedMemberExpression(m) => {
                    member = true;
                    e = m.object.get_inner_expression();
                }
                Expression::CallExpression(c) => {
                    if member && self.recorded.contains(&c.span.start) {
                        return;
                    }
                    e = c.callee.get_inner_expression();
                }
                Expression::Identifier(id) => {
                    if let Some((ix, _)) = self.symbol_of(id).and_then(|s| self.imports.get(&s))
                        && let Some(i) = self.out.imports.get_mut(*ix)
                    {
                        push_unique(&mut i.escapes, "*".to_owned());
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    /// Applies what [`Cx::eval_calls`] found about calls that are not recorded
    /// (the imports are known only now).
    fn record_unrecorded(&mut self) {
        for callee in std::mem::take(&mut self.unrecorded) {
            self.mark_unrecorded(callee);
        }
        if self.calls_overflow {
            // Unrecorded calls: every imported binding may be used in a way the facts
            // do not show.
            for i in &mut self.out.imports {
                push_unique(&mut i.escapes, "*".to_owned());
            }
        }
    }

    /// Marks every node inside `v` (also inside its containers) as handed to
    /// the pending call `by` (`None`: to code that is not described).
    fn taint(&mut self, v: &V<'a>, by: Option<(usize, u32)>, depth: u32) {
        if depth > 6 {
            return;
        }
        for l in v.leaves() {
            match l {
                V::Js(j) => {
                    let mut ids = Vec::new();
                    syms_in(j, &mut ids);
                    for id in ids {
                        self.taint_node(id, by);
                    }
                }
                V::Object(o) => {
                    for (_, x) in &o.props {
                        self.taint(x, by, depth + 1);
                    }
                    if let Some(x) = &o.open {
                        self.taint(x, by, depth + 1);
                    }
                }
                V::Array(a) => {
                    for x in &a.elems {
                        self.taint(x, by, depth + 1);
                    }
                    if let Some(x) = &a.rest {
                        self.taint(x, by, depth + 1);
                    }
                }
                _ => {}
            }
        }
    }

    fn taint_node(&mut self, id: u32, by: Option<(usize, u32)>) {
        match by {
            Some(b) => {
                let l = self.taints.entry(id).or_default();
                if !l.contains(&b) {
                    l.push(b);
                }
            }
            None => {
                self.taint_unknown.insert(id);
            }
        }
    }

    /// Whether the root of the callee `e` is a global whose calls change
    /// nothing the file-wide scan of writes does not see.
    fn quiet_global(&self, e: &'a Expression<'a>) -> bool {
        let mut e = e.get_inner_expression();
        loop {
            match e {
                Expression::StaticMemberExpression(m) => e = m.object.get_inner_expression(),
                Expression::ComputedMemberExpression(m) => e = m.object.get_inner_expression(),
                Expression::CallExpression(c) => e = c.callee.get_inner_expression(),
                Expression::Identifier(id) => {
                    return self.symbol_of(id).is_none()
                        && QUIET_GLOBALS.contains(&id.name.as_str());
                }
                // `import(x)`, `super(...)`, a literal: nothing that keeps an argument.
                Expression::ImportExpression(_) | Expression::Super(_) => return true,
                _ => return false,
            }
        }
    }

    /// Whether the root identifier of the callee `e` is a function declaration
    /// of the file (its parameters are aliases of the arguments, which the
    /// engine follows).
    fn declared_function(&self, e: &'a Expression<'a>) -> bool {
        match e.get_inner_expression() {
            Expression::Identifier(id) => self.symbol_of(id).is_some_and(|sym| {
                self.eng
                    .scoping
                    .symbol_declarations(sym)
                    .any(|d| matches!(self.eng.nodes.kind(d), AstKind::Function(_)))
            }),
            _ => false,
        }
    }

    /// A builtin that calls a function with the elements of a list
    /// (`list.forEach(fix)`, `[{ inner: spec }].map(fix)`,
    /// `Array.from(list, fix)`) or with the value of a promise (`p.
    /// then(fix)`): the elements, and what they hold, are handed to that
    /// function. A function written in the file is part of the file-wide scan
    /// of writes; one that comes from elsewhere (a parameter, an import,
    /// the result of a call) is a callee like any other, and what it does
    /// to its arguments is in its facts.
    ///
    /// The list is often a copy (`[...list]`, `new Set(list)`,
    /// `list.filter(Boolean)`, `Object.values(table)`), whose value says
    /// nothing about where its elements come from, so everything the
    /// expression of the list mentions counts as handed ([`mentioned`]).
    fn element_callback(&mut self, c: &'a CallExpression<'a>) {
        let Expression::StaticMemberExpression(m) = c.callee.get_inner_expression() else {
            return;
        };
        let name = m.property.name.as_str();
        let array_from = name == "from"
            && matches!(
                m.object.get_inner_expression(),
                Expression::Identifier(i) if i.name == "Array" && self.symbol_of(i).is_none()
            );
        // The positions of the elements (and of the list itself) among the arguments of
        // the function, the position of the function among the arguments of the call,
        // and the expression of the list.
        let (positions, at, list): (&[u32], usize, Option<&'a Expression<'a>>) = match name {
            "from" if array_from => (&[0], 1, c.arguments.first().and_then(|a| a.as_expression())),
            "map" | "filter" | "forEach" | "find" | "findLast" | "findIndex" | "findLastIndex"
            | "some" | "every" | "flatMap" => (&[0, 2], 0, Some(&m.object)),
            "reduce" | "reduceRight" => (&[0, 1, 3], 0, Some(&m.object)),
            "sort" | "toSorted" => (&[0, 1], 0, Some(&m.object)),
            "then" | "finally" => (&[0], 0, Some(&m.object)),
            _ => return,
        };
        let (Some(cb), Some(list)) = (c.arguments.get(at).and_then(|a| a.as_expression()), list)
        else {
            return;
        };
        if matches!(
            cb.get_inner_expression(),
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
        ) || self.quiet_global(cb)
        {
            return;
        }
        let cbv = self.eng.site_args(&c.arguments).0.into_iter().nth(at);
        // A function of the file (also: no value at all, the call throws).
        if cbv
            .as_ref()
            .is_some_and(|v| v.leaves().iter().all(|l| matches!(l, V::Func(_))))
        {
            return;
        }
        let mut parts: Vec<&'a Expression<'a>> = Vec::new();
        mentioned(list, &mut parts, 0);
        let handed: Vec<V<'a>> = parts.into_iter().filter_map(|e| self.eng.site(e)).collect();
        match cbv {
            Some(v) if self.symbolic(&v) => {
                let ix = self.pending.len();
                for h in &handed {
                    for p in positions {
                        self.taint(h, Some((ix, *p)), 0);
                    }
                }
                self.pending.push(Pending {
                    callee: PCallee::Val(v),
                    args: Vec::new(),
                    rest: None,
                    line: self.line(c.span.start),
                    new: false,
                    within: None,
                    record: false,
                    shallow: true,
                });
            }
            // Not something binding can ask: code that is not described.
            _ => {
                for h in &handed {
                    self.taint(h, None, 0);
                }
            }
        }
    }

    /// Evaluates every call of the file: the ones to record ([`Cx::calls`]) are
    /// kept, and every value handed to a callee that is not a function of
    /// the file or a quiet builtin is marked ([`Cx::changed`]).
    fn eval_calls(&mut self) {
        let nodes = self.eng.nodes;
        for node in nodes.iter() {
            let (callee, args, span, call) = match node.kind() {
                AstKind::CallExpression(c) => (&c.callee, &c.arguments, c.span, Some(c)),
                AstKind::NewExpression(n) => (&n.callee, &n.arguments, n.span, None),
                _ => continue,
            };
            let new = call.is_none();
            if self.pending.len() >= MAX_CALLS {
                self.calls_overflow = true;
                break;
            }
            if let Some(c) = call {
                self.element_callback(c);
            }
            // `new C(...)` of a class of the file.
            let local_ctor = match callee.get_inner_expression() {
                Expression::Identifier(id) if new => self
                    .symbol_of(id)
                    .and_then(|s| self.classes.get(&s).copied())
                    .flatten(),
                _ => None,
            };
            // Whether argument `i` is handed to something that can change it.
            let pure = |cx: &Self, i: usize| call.is_some_and(|c| cx.eng.pure_callee(c, i));
            // Whether the call is one to record ([`Cx::calls`]).
            let mut record = true;
            let pc = match local_ctor {
                Some(ctor) => PCallee::Ctor(ctor),
                None => {
                    if !self.callee_candidate(callee) {
                        // A global, or a function declaration of the file.
                        if !self.quiet_global(callee) && !self.declared_function(callee) {
                            let (vals, rest) = self.eng.site_args(args);
                            for (i, v) in vals.iter().enumerate() {
                                if !pure(self, i) {
                                    self.taint(v, None, 0);
                                }
                            }
                            if let Some(r) = &rest {
                                self.taint(r, None, 0);
                            }
                        }
                        continue;
                    }
                    let v = self.eng.site(callee);
                    record = v.as_ref().is_some_and(|v| self.external(v));
                    let symbolic = v.as_ref().is_some_and(|v| self.symbolic(v));
                    if symbolic && !record {
                        // A method of an object read from a hole: not a call to record,
                        // but what it is handed is.
                        self.unrecorded.push(callee);
                    }
                    if !symbolic {
                        // A member of a builtin module is what it is, a function of the
                        // file gets aliases; anything else can keep or change what it is
                        // handed, and if it starts at an imported binding it is a use
                        // the facts do not show.
                        let (builtin, local) = match &v {
                            Some(v) => (
                                v.leaves().iter().all(|l| {
                                    matches!(
                                        l,
                                        V::Builtin(_)
                                            | V::Method(..)
                                            | V::Js(JsVal::Obj(_) | JsVal::Func(_))
                                    )
                                }),
                                v.leaves().iter().all(|l| matches!(l, V::Func(_))),
                            ),
                            None => (false, false),
                        };
                        if !builtin {
                            self.unrecorded.push(callee);
                        }
                        if !builtin && !local {
                            let (vals, rest) = self.eng.site_args(args);
                            for (i, v) in vals.iter().enumerate() {
                                if !pure(self, i) {
                                    self.taint(v, None, 0);
                                }
                            }
                            if let Some(r) = &rest {
                                self.taint(r, None, 0);
                            }
                        }
                        continue;
                    }
                    match v {
                        Some(v) => PCallee::Val(v),
                        None => continue,
                    }
                }
            };
            let (vals, rest) = self.eng.site_args(args);
            let ix = self.pending.len();
            if let PCallee::Val(_) = &pc {
                for (i, v) in vals.iter().enumerate() {
                    if !pure(self, i) {
                        self.taint(v, Some((ix, i as u32)), 0);
                    }
                }
                if let Some(r) = &rest {
                    self.taint(r, Some((ix, vals.len() as u32)), 0);
                }
            }
            // A method called on an object (`o.fix()`) gets it as `this` and can change it;
            // a function that was merely read from an object (`const { before } = opts;
            // before(cb)`) does not. Builtin readers and adders are part of the file-wide
            // scan of writes.
            let method = match callee.get_inner_expression() {
                Expression::StaticMemberExpression(m) => {
                    let name = m.property.name.as_str();
                    (!super::stable::READERS.contains(&name) && !ADDER_METHODS.contains(&name))
                        .then_some(&m.object)
                }
                Expression::ComputedMemberExpression(m) => Some(&m.object),
                _ => None,
            };
            if let (Some(object), PCallee::Val(_)) = (method, &pc)
                && let Some(recv) = self.eng.site(object)
            {
                for l in recv.leaves() {
                    if let V::Js(j) = l
                        && let Some(id) = sym_of(j)
                    {
                        self.taint_node(id, Some((ix, RECEIVER)));
                    }
                }
            }
            if record {
                self.recorded.insert(span.start);
            }
            self.pending.push(Pending {
                callee: pc,
                args: vals,
                rest,
                line: self.line(span.start),
                new,
                within: self.within(node.id()),
                record,
                shallow: false,
            });
        }
    }

    /// Describes the calls [`Cx::eval_calls`] kept.
    fn calls(&mut self) {
        for i in 0..self.pending.len() {
            if !self.pending[i].record {
                continue;
            }
            let callee = self.pending_callee(i, 0);
            let callee = match &callee {
                // Not described as a callee (it was being described when it was needed).
                X::Const(JsVal::Unknown(_)) => match &self.pending[i].callee {
                    PCallee::Val(v) => {
                        let v = v.clone();
                        self.value(&v)
                    }
                    PCallee::Ctor(c) => X::Func(*c),
                },
                _ => callee,
            };
            let vals = self.pending[i].args.clone();
            let rest = self.pending[i].rest.clone();
            let args = vals.iter().map(|v| self.value(v)).collect();
            let rest = rest.map(|r| self.value(&r));
            let p = &self.pending[i];
            self.out.calls.push(Call {
                callee,
                args,
                rest,
                line: p.line,
                new: p.new,
                within: p.within,
            });
        }
    }

    // ---- functions ----

    /// The call `c` as a value: `callee(args)`.
    fn call_x(&mut self, c: &'a CallExpression<'a>) -> Option<X> {
        let v = self.eng.site(&c.callee)?;
        if !self.external(&v) {
            return None;
        }
        let callee = self.value(&v);
        let (vals, rest) = self.eng.site_args(&c.arguments);
        Some(X::Call {
            callee: Box::new(callee),
            args: vals.iter().map(|v| self.value(v)).collect(),
            rest: rest.map(|r| Box::new(self.value(&r))),
        })
    }

    /// How the result of the call at `node` is used: bound to a name that is
    /// only called or exported, called at once, or dropped (`false`);
    /// anything else escapes.
    fn result_escapes(&self, node: NodeId, span: oxc_span::Span) -> bool {
        match self.eng.parent(node) {
            (_, AstKind::VariableDeclarator(d)) => match &d.id {
                BindingPattern::BindingIdentifier(id) => match id.symbol_id.get() {
                    Some(s) => self.uses(s).0,
                    None => true,
                },
                _ => true,
            },
            (_, AstKind::CallExpression(c)) => c.callee.get_inner_expression().span() != span,
            (_, AstKind::ExpressionStatement(_) | AstKind::ExportDefaultDeclaration(_)) => false,
            _ => true,
        }
    }

    fn fn_fact(&mut self, id: u32) -> Option<FnFact> {
        let (f, node) = *self.fns.get(&id)?;
        let params = match f {
            FnRef::Function(func) => &func.params,
            FnRef::Arrow(a) => &a.params,
        };
        let reach = self.eng.site_reach(f, node);
        let name;
        let declaration = matches!(f, FnRef::Function(func) if func.is_declaration());
        let (pid, pk) = self.eng.parent(node);
        let source = match &*reach {
            Reach::Named { name: n, .. } => {
                name = n.to_string();
                // The bindings that name the function.
                let mut syms: Vec<SymbolId> = Vec::new();
                if let FnRef::Function(func) = f {
                    syms.extend(func.id.as_ref().and_then(|i| i.symbol_id.get()));
                }
                let named_position = declaration
                    || match pk {
                        AstKind::VariableDeclarator(d) => match &d.id {
                            BindingPattern::BindingIdentifier(i) => {
                                syms.extend(i.symbol_id.get());
                                true
                            }
                            _ => false,
                        },
                        // An immediately invoked function.
                        AstKind::CallExpression(c) => {
                            c.callee.get_inner_expression().span() == f.span()
                        }
                        _ => false,
                    };
                if !named_position {
                    Source::Unseen
                } else {
                    let escapes = syms.iter().any(|s| self.uses(*s).0)
                        // Top-level functions of a classic script are globals.
                        || !self.module;
                    Source::Named { escapes }
                }
            }
            Reach::Callback { call, arg } => {
                name = format!("callback of {}", self.eng.text(call.callee.span()));
                match self.call_x(call) {
                    Some(x) => {
                        let escapes = self.result_escapes(pid, call.span);
                        if !self.out.callables.iter().any(|c| c.value == x) {
                            self.out.callables.push(Callable {
                                value: x.clone(),
                                escapes,
                            });
                        }
                        Source::Callback {
                            call: x,
                            arg: *arg as u32,
                        }
                    }
                    None => Source::Unseen,
                }
            }
            Reach::Unseen { name: n } => {
                name = n.to_string();
                self.ctor_source(pid, pk)
            }
        };
        // What it returns, for functions other files can call.
        let mut returns = None;
        if matches!(source, Source::Named { .. }) {
            let info = self.eng.fn_info(f);
            if !info.generator && info.returns.is_empty() {
                returns = Some(X::Void);
            } else if !info.generator {
                let mut alts = Vec::new();
                for e in info.returns.iter().take(16) {
                    if let Some(v) = self.eng.site(e) {
                        let x = self.value(&v);
                        if !alts.contains(&x) {
                            alts.push(x);
                        }
                    }
                }
                let x = if alts.len() == 1 {
                    alts.remove(0)
                } else {
                    X::Alts(alts)
                };
                returns = Some(x);
            }
        }
        // Parameters used other than as the callee of a direct call.
        let mut param_escapes = Vec::new();
        for (i, p) in params.items.iter().enumerate() {
            let escapes = match &p.pattern {
                BindingPattern::BindingIdentifier(id) => match id.symbol_id.get() {
                    Some(s) => self.uses(s).0,
                    None => true,
                },
                _ => true,
            };
            if escapes {
                param_escapes.push(i as u32);
            }
        }
        let handoffs = self.handoffs(id);
        let tests_string = self.tests_string(f);
        let facts = self.eng.facts();
        Some(FnFact {
            id,
            name,
            line: self.line(id),
            source,
            returns,
            param_escapes,
            rest: params.rest.as_ref().map(|_| params.items.len() as u32),
            handoffs,
            uses_this: matches!(f, FnRef::Function(_)) && facts.uses_this(id),
            uses_arguments: facts.uses_arguments(id),
            tests_string,
        })
    }

    /// The parameter `(function, index)` that the node `id` is, or is read
    /// from.
    fn root_param(&self, mut id: u32) -> Option<(u32, u32)> {
        for _ in 0..64 {
            match self.eng.sym_node(id)? {
                Sym::Param { func, index } => return Some((*func, *index)),
                Sym::Field(b, _) | Sym::Elem(b) | Sym::AnyField(b) => id = *b,
                Sym::Reach { base, .. } => id = *base,
                _ => return None,
            }
        }
        None
    }

    /// What the function `func` hands its parameters, or objects reached
    /// through them, to.
    fn handoffs(&mut self, func: u32) -> Vec<Handoff> {
        let mut raw: Vec<RawHandoff> = Vec::new();
        let slot = |raw: &mut Vec<RawHandoff>, param: u32| -> usize {
            match raw.iter().position(|r| r.0 == param) {
                Some(i) => i,
                None => {
                    raw.push((param, Vec::new(), false));
                    raw.len() - 1
                }
            }
        };
        let mut ids: Vec<u32> = self.taints.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            if let Some((f, index)) = self.root_param(id)
                && f == func
            {
                let i = slot(&mut raw, index);
                for b in self.taints.get(&id).into_iter().flatten() {
                    if !raw[i].1.contains(b) {
                        raw[i].1.push(*b);
                    }
                }
            }
        }
        let mut unknown: Vec<u32> = self.taint_unknown.iter().copied().collect();
        unknown.sort_unstable();
        for id in unknown {
            if let Some((f, index)) = self.root_param(id)
                && f == func
            {
                let i = slot(&mut raw, index);
                raw[i].2 = true;
            }
        }
        raw.sort_by_key(|r| r.0);
        let mut out = Vec::new();
        for (param, by, unknown) in raw {
            let mut many = by.len() > 16;
            self.room = MAX_X;
            let mut to: Vec<(X, u32)> = if many {
                Vec::new()
            } else {
                by.into_iter()
                    .map(|(i, pos)| (self.pending_callee(i, 0), pos))
                    .collect()
            };
            if to
                .iter()
                .any(|(c, _)| matches!(c, X::Const(JsVal::Unknown(_))) || size(c) > MAX_CALLEE_X)
            {
                many = true;
                to.clear();
            }
            out.push(Handoff {
                param,
                to,
                unknown: unknown || many || self.calls_overflow,
            });
        }
        out
    }

    /// The source of a function that is not reached through a name: a
    /// constructor is reached through its class.
    fn ctor_source(&self, pid: NodeId, pk: AstKind<'a>) -> Source {
        let AstKind::MethodDefinition(m) = pk else {
            return Source::Unseen;
        };
        if m.kind != MethodDefinitionKind::Constructor {
            return Source::Unseen;
        }
        // Method → class body → class.
        let body = self.eng.nodes.parent_id(pid);
        let class_id = self.eng.nodes.parent_id(body);
        let AstKind::Class(c) = self.eng.nodes.kind(class_id) else {
            return Source::Unseen;
        };
        let mut syms: Vec<SymbolId> = Vec::new();
        syms.extend(c.id.as_ref().and_then(|id| id.symbol_id.get()));
        let named_position = c.is_declaration()
            || match self.eng.parent(class_id) {
                (_, AstKind::VariableDeclarator(d)) => match &d.id {
                    BindingPattern::BindingIdentifier(i) => {
                        syms.extend(i.symbol_id.get());
                        true
                    }
                    _ => false,
                },
                _ => false,
            };
        if !named_position {
            return Source::Unseen;
        }
        Source::Ctor {
            escapes: syms.iter().any(|s| self.uses(*s).0) || !self.module,
        }
    }

    /// Describes every function the facts mention as a hole or export, and the
    /// functions those return; drops calls of constructors nothing refers
    /// to.
    fn functions(&mut self) {
        let mut todo: Vec<u32> = Vec::new();
        let mut holes: Vec<u32> = Vec::new();
        for s in &self.out.sinks {
            hole_funcs(&s.value, &mut holes);
            // The callees that guard the site: a function of the file is asked whether
            // it is a test for a string.
            for (callee, _) in &s.string_tests {
                callee.funcs(&mut todo);
            }
        }
        for t in &self.out.tables {
            t.keys.iter().for_each(|k| hole_funcs(k, &mut holes));
        }
        for c in &self.out.calls {
            hole_funcs(&c.callee, &mut holes);
            c.args.iter().for_each(|a| hole_funcs(a, &mut holes));
            if let Some(r) = &c.rest {
                hole_funcs(r, &mut holes);
            }
            // Functions handed to a callee (a `merge` option, a callback): the callee may
            // call them with an object, and what they do to it is in their facts.
            c.args.iter().for_each(|a| a.funcs(&mut todo));
        }
        for e in &self.out.exports {
            e.value.funcs(&mut todo);
        }
        todo.extend(holes);
        let mut done: Vec<u32> = Vec::new();
        while let Some(id) = todo.pop() {
            if done.contains(&id) || done.len() >= MAX_FNS {
                continue;
            }
            done.push(id);
            let Some(fact) = self.fn_fact(id) else {
                continue;
            };
            if let Some(r) = &fact.returns {
                r.funcs(&mut todo);
            }
            if let Source::Callback { call, .. } = &fact.source {
                hole_funcs(call, &mut todo);
            }
            self.out.fns.push(fact);
        }
        self.out.fns.sort_by_key(|f| f.id);
        let fns: Vec<u32> = self.out.fns.iter().map(|f| f.id).collect();
        self.out.calls.retain(|c| match &c.callee {
            X::Func(id) => fns.contains(id),
            _ => true,
        });
    }
}

/// The expressions whose values the list expression `e` can take its elements
/// from: `e` itself, the receiver and the arguments of the calls in it
/// (`list.filter(f)`, `Object.values(table)`, `new Set(list)`), the elements
/// and spreads of an array literal, the values of an object literal, the
/// branches of a condition.
fn mentioned<'a>(e: &'a Expression<'a>, out: &mut Vec<&'a Expression<'a>>, depth: u32) {
    if depth > 8 || out.len() >= 32 {
        return;
    }
    let e = e.get_inner_expression();
    out.push(e);
    let d = depth + 1;
    let args = |arguments: &'a [Argument<'a>], out: &mut Vec<&'a Expression<'a>>| {
        for a in arguments {
            match a {
                Argument::SpreadElement(s) => mentioned(&s.argument, out, d),
                a => {
                    if let Some(x) = a.as_expression() {
                        mentioned(x, out, d);
                    }
                }
            }
        }
    };
    match e {
        Expression::CallExpression(c) => {
            match c.callee.get_inner_expression() {
                Expression::StaticMemberExpression(m) => mentioned(&m.object, out, d),
                Expression::ComputedMemberExpression(m) => mentioned(&m.object, out, d),
                _ => {}
            }
            args(&c.arguments, out);
        }
        Expression::NewExpression(n) => args(&n.arguments, out),
        Expression::ArrayExpression(a) => {
            for el in &a.elements {
                match el {
                    ArrayExpressionElement::SpreadElement(s) => mentioned(&s.argument, out, d),
                    ArrayExpressionElement::Elision(_) => {}
                    el => {
                        if let Some(x) = el.as_expression() {
                            mentioned(x, out, d);
                        }
                    }
                }
            }
        }
        Expression::ObjectExpression(o) => {
            for p in &o.properties {
                match p {
                    ObjectPropertyKind::ObjectProperty(p) => mentioned(&p.value, out, d),
                    ObjectPropertyKind::SpreadProperty(s) => mentioned(&s.argument, out, d),
                }
            }
        }
        Expression::ConditionalExpression(c) => {
            mentioned(&c.consequent, out, d);
            mentioned(&c.alternate, out, d);
        }
        Expression::LogicalExpression(l) => {
            mentioned(&l.left, out, d);
            mentioned(&l.right, out, d);
        }
        Expression::AwaitExpression(a) => mentioned(&a.argument, out, d),
        _ => {}
    }
}

/// Whether the constant `x` has an unknown part.
fn partly_unknown(x: &X) -> bool {
    fn js(j: &JsVal) -> bool {
        match j {
            JsVal::Unknown(_) => true,
            JsVal::Str(v) | JsVal::Path(_, v) | JsVal::Url(_, v) | JsVal::Resolved(_, v) => {
                let mut rs = Vec::new();
                crate::extract::evaluator::wild_reasons(v, &mut rs);
                !rs.is_empty()
            }
            JsVal::Alts(vs) | JsVal::Arr(vs) => vs.iter().any(js),
            _ => false,
        }
    }
    match x {
        X::Const(j) => js(j),
        X::Alts(xs) => xs.iter().any(partly_unknown),
        _ => false,
    }
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.contains(&s) {
        v.push(s);
    }
}

/// The functions whose parameters `x` mentions.
fn hole_funcs(x: &X, out: &mut Vec<u32>) {
    match x {
        X::Param { func, .. } => {
            if !out.contains(func) {
                out.push(*func);
            }
        }
        X::Const(_) | X::Void | X::Func(_) | X::Import { .. } => {}
        X::Field(b, _) | X::Elem(b) | X::AnyField(b) => hole_funcs(b, out),
        X::Reach { base, .. } => hole_funcs(base, out),
        X::Concat(a, b) | X::Index { table: a, key: b } => {
            hole_funcs(a, out);
            hole_funcs(b, out);
        }
        X::Apply(_, xs) | X::Opaque(xs) | X::Alts(xs) => xs.iter().for_each(|x| hole_funcs(x, out)),
        X::Object { props, open } => {
            props.iter().for_each(|(_, x)| hole_funcs(x, out));
            if let Some(x) = open {
                hole_funcs(x, out);
            }
        }
        X::Array { elems, rest } => {
            elems.iter().for_each(|x| hole_funcs(x, out));
            if let Some(x) = rest {
                hole_funcs(x, out);
            }
        }
        X::Call { callee, args, rest } => {
            hole_funcs(callee, out);
            args.iter().for_each(|x| hole_funcs(x, out));
            if let Some(x) = rest {
                hole_funcs(x, out);
            }
        }
        X::Changed { by, .. } => by.iter().for_each(|(x, _)| hole_funcs(x, out)),
    }
}

/// Whether `x` reads a table by key.
fn has_index(x: &X) -> bool {
    match x {
        X::Index { .. } => true,
        X::Const(_) | X::Void | X::Param { .. } | X::Func(_) | X::Import { .. } => false,
        X::Field(b, _) | X::Elem(b) | X::AnyField(b) => has_index(b),
        X::Reach { base, .. } => has_index(base),
        X::Concat(a, b) => has_index(a) || has_index(b),
        X::Apply(_, xs) | X::Opaque(xs) | X::Alts(xs) => xs.iter().any(has_index),
        X::Object { props, open } => {
            props.iter().any(|(_, x)| has_index(x)) || open.as_ref().is_some_and(|x| has_index(x))
        }
        X::Array { elems, rest } => {
            elems.iter().any(has_index) || rest.as_ref().is_some_and(|x| has_index(x))
        }
        X::Call { callee, args, rest } => {
            has_index(callee)
                || args.iter().any(has_index)
                || rest.as_ref().is_some_and(|x| has_index(x))
        }
        X::Changed { .. } => false,
    }
}

/// Number of nodes of `x`.
fn size(x: &X) -> usize {
    1 + match x {
        X::Const(_) | X::Void | X::Param { .. } | X::Func(_) | X::Import { .. } => 0,
        X::Field(b, _) | X::Elem(b) | X::AnyField(b) => size(b),
        X::Reach { base, .. } => size(base),
        X::Concat(a, b) | X::Index { table: a, key: b } => size(a) + size(b),
        X::Apply(_, xs) | X::Opaque(xs) | X::Alts(xs) => xs.iter().map(size).sum(),
        X::Object { props, open } => {
            props.iter().map(|(_, x)| size(x)).sum::<usize>() + open.as_ref().map_or(0, |x| size(x))
        }
        X::Array { elems, rest } => {
            elems.iter().map(size).sum::<usize>() + rest.as_ref().map_or(0, |x| size(x))
        }
        X::Call { callee, args, rest } => {
            size(callee)
                + args.iter().map(size).sum::<usize>()
                + rest.as_ref().map_or(0, |x| size(x))
        }
        X::Changed { by, .. } => by.iter().map(|(x, _)| size(x)).sum(),
    }
}
