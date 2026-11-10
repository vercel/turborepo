//! The value-flow engine: tier 2 of JS / TS extraction.
//!
//! The recognizer ([`crate::extract`]) finds every site (a module load or a
//! file read through a tracked API) with a fast token pass and evaluates its
//! argument with a token evaluator that only knows top-level constants. When
//! that leaves a site unresolved, guessed, or dependent on a binding it read by
//! name (it does not know scopes), this module parses the file with
//! `oxc_parser`, resolves bindings with `oxc_semantic`, and evaluates the
//! argument expression again as an abstract value.
//!
//! # Interface
//!
//! [`evaluate_sites`] takes the source, the dialect and the spans of the
//! argument expressions (or, for a call whose callee decides where its argument
//! is resolved from, the span of the call: [`Ask::Load`]) and returns one
//! [`SiteValue`] per span: a [`JsVal`] of the token evaluator's value domain
//! (known strings, patterns, alternatives, paths anchored at the file's
//! directory, the working directory or the temporary directory, or
//! next to a module a resolver call finds). The recognizer lowers it
//! exactly like a token result ([`crate::extract::evaluator::lower`]) into
//! references with bases, patterns and uncertainty records, so resolution,
//! globbing and memberships are unchanged. `None` for the whole file (it does
//! not parse, is too large or too deeply nested) or for one span (no expression
//! starts there) keeps the token result.
//!
//! # What is evaluated
//!
//! Evaluation is flow-insensitive and over-approximating: **the value of a
//! binding is the join of all its definitions**, wherever they are. A
//! definition that cannot be evaluated makes that alternative unknown; nothing
//! is dropped.
//!
//! - Bindings at any depth: `const` / `let` / `var` with or without
//!   initializer, every assignment (`=`, `||=`, `??=`, `&&=`, `+=`),
//!   destructuring of objects and arrays with defaults and renames, `for…of` /
//!   `for…in` variables, function and class declarations, imports of the
//!   builtin modules (`path`, `url`, `os`, `fs`, `process`, `module`).
//! - Expressions: string and template literals, `+`, `?:`, `||`, `??`, `&&`,
//!   `await`, parentheses, TypeScript `as` / `!` / `satisfies`, object and
//!   array literals, property reads (a computed key that is not known reads
//!   every value), spread, `String(x)`, string methods (known receivers are
//!   computed, others derive an unknown of the same origin), `path.*`, `url.*`,
//!   `new URL(rel, base)`, `fileURLToPath`, `require.resolve`, `import.meta.*`,
//!   `process.cwd()`, `process.env.X`, `process.argv`, `os.tmpdir()`,
//!   `fs.mkdtemp*`, `fs.realpath*`, `fs.readdir*`.
//! - Modules found by a resolver call and what is built on them (a file next to
//!   `require.resolve('pkg')` or `import.meta.resolve('pkg')`),
//!   `require.resolve(x, { paths: [...] })` (one alternative per entry; Node
//!   reads no other option) and `createRequire(x)`: a value ([`V::Loader`])
//!   that carries where its calls resolve from, the file's own `require` when
//!   `x` is the file or a file in its directory, and unknown when `x` cannot be
//!   evaluated. `node:` requests are builtin modules for every `require`
//!   function.
//! - Collections: array and object literals, `Set` / `Map` built with `.add` /
//!   `.set` / `.push` on their binding, `.map` / `.filter` / `.concat` /
//!   `.flatMap` / `.find`, `Object.keys` / `values` / `entries`, iteration with
//!   `for…of`, `.forEach`, and `describe.each(table)(…)`-style tables.
//! - Local functions: a call of a function, arrow function or object-literal
//!   method of the file is evaluated by binding its parameters to the argument
//!   values and joining its `return` values (bounded depth, recursion guard).
//! - Parameters: inside a function the value of a parameter is the join of the
//!   arguments at all call sites, **if every use of the function is a direct
//!   call in this file**. If the function escapes (exported, passed as a value,
//!   a method), the join also contains an unknown with origin `Param`. A
//!   callback's parameters are bound by the callee: known for array iteration
//!   methods and `.each` tables, `Param` otherwise.
//! - Guards ([`guard`]): the one place where control flow counts. At a use of a
//!   binding that never changes (a parameter or a `const`), the unknown part of
//!   its value is replaced by what a test around the use allows:
//!   `x.startsWith(prefix)` (the prefix and anything after it),
//!   `list.includes(x)` / `set.has(x)` (the elements), an anchored regular
//!   expression literal (its literal start). The test holds inside the branch
//!   it guards and after an `if` that throws, returns, continues or breaks.
//! - Classes: `this.x` is the join of the field initializer and every `this.x =
//!   …` in the class when the field cannot be written from outside or
//!   redeclared by a subclass (`#x`, TypeScript `private`) and nothing in the
//!   file writes it through another reference (`other.x = …`, `self.x = …`,
//!   `this.x++`); otherwise unknown. `readonly` alone is not enough: a subclass
//!   can declare the field again.
//!
//! # Origins
//!
//! An unknown part of a value carries where it comes from as a mark in its
//! reason ([`crate::extract::origin`]); when a site cannot be lowered, the
//! recognizer turns the marks into [`turborepo_trace_core::Origin`] on the
//! uncertainty record (`Param`, `Env`, `Argv`, `Imported`, `Call`, `Data`,
//! `Unknown`; `Cwd` for paths relative to the working directory) and puts the
//! payload (the variable, parameter or imported binding) into the
//! detail. A value below the temporary directory ([`Anchor::Tmp`]:
//! `os.tmpdir()`, `mkdtemp`, `tmp` / `tempy` helpers, `process.env.TMPDIR`) is
//! outside the repository: a file read there produces neither a reference nor a
//! record (what a test copies there is a dependency through the copy's source,
//! which is a tracked read). A module loaded from there (`require`, `import()`,
//! `new Worker`, `fork`) was generated at runtime from files the site cannot
//! name and keeps a record with origin `Data`.
//!
//! `undefined` and `null` ([`V::Void`]) are never a path: as a site argument
//! they produce nothing, and a concatenation with one is dropped. A number or a
//! boolean ([`V::scalar`]) is nothing as a site argument either, but inside a
//! concatenation it is some text (`` `chunk-${i}.json` ``, `'out-' + minify`)
//! and as a key it selects some property. Numeric literals are kept as their
//! digits; `+` and `+=` on operands that can both be numbers are unknown.
//!
//! # Soundness
//!
//! A site loses its uncertainty only if the value covers every string the
//! expression can evaluate to, under these assumptions:
//!
//! - no `eval` / `with` (files using them keep their token results);
//! - a wildcard after a known directory names something below that directory
//!   (the existing rule for partial specifiers), and an unknown segment of
//!   `path.resolve` or an unknown `new URL` input is relative;
//! - the temporary directory and `TMPDIR` are outside the repository, and a
//!   path built below it stays there;
//! - TypeScript `private` and `as const` are honoured by other modules;
//! - a path built from `undefined` or `null` is not a dependency;
//! - an iteration callback defined outside the file does not change the array
//!   it is given as its last argument;
//! - the receiver of a `startsWith` test that guards a use is a string, and a
//!   value used as a specifier converts to the same string each time
//!   ([`guard`]).
//!
//! Mutable state is handled conservatively: a binding is the join of all its
//! writes; a container bound to a name is trusted only if every use of the name
//! is a read, a tracked mutation (`.push`, `.add`, `.set`, property assignment)
//! or an alias with the same property, and it is neither exported unfrozen nor
//! passed to code that is not evaluated. The same holds for the alias of a
//! nested container (`binding.list` passed to an unknown callee, returned or
//! exported), for the result of a method that returns its receiver
//! (`binding.sort()`, `binding.add(x)`), and for the container an iteration
//! callback receives. A method the object literal defines itself is not a
//! builtin of that name. Containers reached through other containers,
//! parameters or aliases additionally read every untracked property write and
//! array mutation of the file (field-based), and lose their element positions
//! when the file reorders an untracked array.
//!
//! The runtime oracle (`crates/turborepo-trace-lang-js/tests/flow_oracle`) runs
//! hand-written and generated programs under Node and checks
//! that every value a site receives is covered; the cases it found are the
//! tests in `soundness.rs`.
//!
//! # Bounds
//!
//! Files larger than [`MAX_BYTES`], nested deeper than [`MAX_NESTING`] brackets
//! or with an expression chain longer than [`MAX_CHAIN`] tokens are not parsed
//! (the parser and the scope analysis are recursive). Parsing and evaluation
//! run on a thread with its own stack; evaluation is bounded by a step budget
//! per site and per file, an inlining depth and an alternatives cap. A parser
//! panic is contained and keeps the token results.

pub mod anchors;
mod engine;
pub(crate) mod env;
#[cfg(test)]
mod env_tests;
pub(crate) mod facts;
mod guard;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod soundness;
mod stable;
mod sym;
#[cfg(test)]
mod tests;
mod value;

use std::collections::HashMap;

use engine::Engine;
use oxc_allocator::Allocator;
use oxc_ast::{
    AstKind,
    ast::{Argument, Expression},
};
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType};
use turborepo_trace_core::{
    Span,
    env::{EnvFact, EnvValue},
};
use value::V;

use crate::{
    extract::{
        evaluator::{Anchor, JsVal},
        origin::{self, Tag},
    },
    lexer::{Dialect, Jsx, Kw, Lexer, NEWLINE_BEFORE, TokKind},
};

/// Files larger than this keep their token results.
pub const MAX_BYTES: usize = 4 << 20;
/// Maximum bracket nesting of a file handed to the parser.
pub const MAX_NESTING: usize = 256;
/// Maximum tokens of one expression chain (no separator at its bracket level),
/// and of the chains of all open bracket levels together: every token of an
/// open chain can be one more level of the syntax tree (`a => b => (c => d =>
/// (...))`), so the sum bounds its depth.
pub const MAX_CHAIN: usize = 3000;
/// Stack of the analysis thread.
const STACK_BYTES: usize = 256 << 20;

/// What a site asks about the expression at its span.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The value of the expression (a call argument, an `import()` source, a
    /// `new URL`).
    Value,
    /// The same, when only the start of the span is exact (a truncated
    /// capture).
    ValueFrom,
    /// The span is a call whose callee decides where its first argument is
    /// resolved from: a call of a `require` function that is not known to
    /// be the file's own (`createRequire(x)(spec)`, a name bound to it), or
    /// a `resolve` call with options (`require.resolve(spec, { paths })`).
    /// The answer is the path the call resolves.
    Load,
}

/// The value of the expression at one site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteValue {
    /// The value; unknown parts carry origin marks
    /// ([`crate::extract::origin`]).
    pub val: JsVal,
    /// The value relies on an assumption (an unknown object's `.join` taken as
    /// `path.join`): the text for a `PartialSpecifier` record.
    pub guess: Option<Box<str>>,
    /// The alternatives the recognizer cannot lower and records as uncertainty
    /// of origin `env` or `call`, as templates over environment variables
    /// and imported calls ([`env`]): one value per origin.
    pub uses: Vec<(turborepo_trace_core::Origin, EnvValue)>,
    /// For a load ([`Ask::Load`]): where the `require` function that is asked
    /// searches from, which decides what a relative value of `uses` names.
    /// The file for any other site.
    pub from: turborepo_trace_core::env::EnvFrom,
}

/// The result of [`evaluate`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlowOutput {
    /// One entry per requested site, as [`evaluate_sites`] returns them.
    pub sites: Vec<Option<SiteValue>>,
    /// The file's assignment and helper facts ([`env`]), when requested.
    pub env: Vec<EnvFact>,
    /// Where the file's anchors go ([`anchors`]), when requested.
    pub anchors: Option<anchors::Anchors>,
}

/// What [`follow_anchors`] is asked about: the anchor uses of the file
/// ([`crate::names::scan_parts`]) and the argument spans of the sites the
/// recognizer evaluates.
#[derive(Copy, Clone, Debug)]
struct AnchorRequest<'r> {
    uses: &'r [crate::names::AnchorUse],
    sites: &'r [Span],
    /// Byte offsets of the templates whose leading hole is asked for.
    templates: &'r [u32],
}

/// Follows the anchor uses of the file `src` ([`anchors`]). `None` when the
/// file cannot be analyzed (it does not parse, is too large or too deep, uses
/// `eval` or `with`).
pub fn follow_anchors(
    src: &[u8],
    dialect: Dialect,
    file_name: Option<&str>,
    uses: &[crate::names::AnchorUse],
    sites: &[Span],
    templates: &[u32],
) -> Option<anchors::Anchors> {
    if uses.is_empty() && templates.is_empty() {
        return None;
    }
    run(
        src,
        dialect,
        file_name,
        &[],
        false,
        Some(AnchorRequest {
            uses,
            sites,
            templates,
        }),
    )
    .and_then(|o| o.anchors)
}

/// Evaluates the expressions at `sites` (the span of each expression and what
/// is asked about it). `None` when the file cannot be analyzed; a `None` entry
/// when no expression starts at that span or the step budget ran out.
pub fn evaluate_sites(
    src: &[u8],
    dialect: Dialect,
    file_name: Option<&str>,
    sites: &[(Span, Ask)],
) -> Option<Vec<Option<SiteValue>>> {
    if sites.is_empty() {
        return None;
    }
    evaluate(src, dialect, file_name, sites, false).map(|o| o.sites)
}

/// [`evaluate_sites`], and with `env_facts` also the file's environment
/// assignment and helper facts ([`env`]) from the same parse. `sites` may then
/// be empty.
pub fn evaluate(
    src: &[u8],
    dialect: Dialect,
    file_name: Option<&str>,
    sites: &[(Span, Ask)],
    env_facts: bool,
) -> Option<FlowOutput> {
    if sites.is_empty() && !env_facts {
        return None;
    }
    run(src, dialect, file_name, sites, env_facts, None)
}

fn run(
    src: &[u8],
    dialect: Dialect,
    file_name: Option<&str>,
    sites: &[(Span, Ask)],
    env_facts: bool,
    anchors: Option<AnchorRequest<'_>>,
) -> Option<FlowOutput> {
    if src.len() > MAX_BYTES || dialect.dts {
        return None;
    }
    let text = std::str::from_utf8(src).ok()?;
    let timing =
        turborepo_trace_core::debug::var_os("FLOW_TIMING").map(|_| std::time::Instant::now());
    if !shallow_enough(src, dialect) {
        return None;
    }
    let guard = timing.map(|t| t.elapsed());
    // The parser and the scope analysis recurse over the syntax tree: give them
    // their own stack, sized for the nesting bound above. A panic inside (a
    // parser bug on unusual input) is contained by the join.
    let out = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("turbotrace-flow".into())
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, || {
                analyze(text, dialect, file_name, sites, env_facts, anchors)
            })
            .ok()?
            .join()
            .ok()?
    });
    if let (Some(t), Some(g)) = (timing, guard) {
        // Debug aid: `flow <total µs> <guard µs> <sites> <bytes> <ok>` per analyzed
        // file.
        eprintln!(
            "flow {} {} {} {} {}",
            t.elapsed().as_micros(),
            g.as_micros(),
            sites.len(),
            src.len(),
            u8::from(out.is_some())
        );
    }
    out
}

/// Whether the syntax tree of `src` is certainly shallow enough to parse
/// recursively: the bracket nesting and the number of tokens since the last
/// separator (`;`, `,`, a statement start), summed over the open bracket
/// levels, bound the depth of every tree the parser can build. (Bounding each
/// level alone is not enough: 250 levels of 1,500 curried arrows are
/// 375,000 nested functions, which overflowed the analysis thread's stack.)
fn shallow_enough(src: &[u8], dialect: Dialect) -> bool {
    let mut lx = Lexer::new(src, dialect);
    // Per open bracket: tokens since the last separator, and whether it is the head
    // of an `if (...)`-like statement (its `)` does not end a value).
    let mut levels: Vec<(u32, bool)> = vec![(0, false)];
    // Tokens of the open chains of all levels.
    let mut total: usize = 0;
    let mut prev_kind = TokKind::Eof;
    let mut prev_kw = Kw::None;
    let mut header_closed = false;
    loop {
        let t = lx.next_token();
        if t.kind == TokKind::Eof {
            break;
        }
        // Closers pop first (the lexer's depth is the truth; the stack follows it).
        let depth = t.depth as usize + 1;
        let mut closed_header = false;
        while levels.len() > depth {
            let closed = levels.pop();
            total -= closed.map_or(0, |l| l.0 as usize);
            closed_header = closed.is_some_and(|l| l.1);
        }
        while levels.len() < depth {
            levels.push((0, false));
        }
        let value_end = match prev_kind {
            TokKind::Ident => !matches!(
                prev_kw,
                Kw::Typeof
                    | Kw::Void
                    | Kw::Delete
                    | Kw::Await
                    | Kw::New
                    | Kw::Yield
                    | Kw::Return
                    | Kw::Throw
                    | Kw::Case
                    | Kw::Else
                    | Kw::Do
                    | Kw::In
                    | Kw::Of
                    | Kw::Instanceof
                    | Kw::Extends
                    | Kw::As
            ),
            TokKind::Str
            | TokKind::Number
            | TokKind::Template
            | TokKind::TemplateTail
            | TokKind::RBracket
            | TokKind::RBrace
            | TokKind::Regex
            | TokKind::Jsx => true,
            TokKind::RParen => !header_closed,
            _ => false,
        };
        let starts_statement = t.flags & NEWLINE_BEFORE != 0
            && value_end
            && match t.kind {
                TokKind::Str | TokKind::Number => true,
                TokKind::Ident => {
                    !matches!(
                        t.kw,
                        Kw::Else
                            | Kw::In
                            | Kw::Of
                            | Kw::Instanceof
                            | Kw::As
                            | Kw::From
                            | Kw::Extends
                    ) && !matches!(
                        t.text(src),
                        b"satisfies" | b"catch" | b"finally" | b"implements" | b"while"
                    )
                }
                _ => false,
            };
        let Some(level) = levels.last_mut() else {
            return false;
        };
        if matches!(t.kind, TokKind::Semi | TokKind::Comma) || starts_statement {
            total -= level.0 as usize;
            level.0 = 0;
        } else {
            let mut add: u32 = 1;
            if t.kind == TokKind::Jsx {
                // One token for a whole element tree: its nesting is at most its tags.
                let tags = t.text(src).iter().filter(|&&b| b == b'<').count();
                add = add.saturating_add(tags.min(u32::MAX as usize - 1) as u32);
            }
            let before = level.0;
            level.0 = level.0.saturating_add(add);
            total += (level.0 - before) as usize;
            if total > MAX_CHAIN {
                return false;
            }
        }
        if matches!(
            t.kind,
            TokKind::LParen
                | TokKind::LBracket
                | TokKind::LBrace
                | TokKind::TemplateHead
                | TokKind::TemplateMiddle
        ) {
            if levels.len() >= MAX_NESTING {
                return false;
            }
            let header = t.kind == TokKind::LParen
                && prev_kind == TokKind::Ident
                && (matches!(prev_kw, Kw::If | Kw::While | Kw::For | Kw::With)
                    || matches!(lx_prev_text(src, &t), b"switch" | b"catch"));
            levels.push((0, header));
        }
        header_closed = t.kind == TokKind::RParen && closed_header;
        prev_kind = t.kind;
        prev_kw = t.kw;
    }
    !lx.jsx_failed()
}

/// The identifier right before the token `t` (whitespace skipped), or empty.
fn lx_prev_text<'s>(src: &'s [u8], t: &crate::lexer::Token) -> &'s [u8] {
    let mut e = t.start as usize;
    while e > 0 && src[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    let mut s = e;
    while s > 0 && (src[s - 1].is_ascii_alphanumeric() || src[s - 1] == b'_' || src[s - 1] == b'$')
    {
        s -= 1;
    }
    &src[s..e]
}

/// The source types to try, most likely first.
fn source_types(dialect: Dialect) -> Vec<SourceType> {
    if dialect.ts {
        let st = if dialect.jsx == Jsx::Off {
            SourceType::ts()
        } else {
            SourceType::tsx()
        };
        return vec![st];
    }
    // Plain JS: JSX is harmless there; an ES module first, then a sloppy script
    // (`with`, octal literals, top-level `return`).
    let base = SourceType::unambiguous().with_jsx(true);
    vec![base, SourceType::script().with_jsx(true)]
}

fn analyze(
    text: &str,
    dialect: Dialect,
    file_name: Option<&str>,
    sites: &[(Span, Ask)],
    env_facts: bool,
    anchors: Option<AnchorRequest<'_>>,
) -> Option<FlowOutput> {
    for st in source_types(dialect) {
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
        let t_parse = std::time::Instant::now();
        let sem = SemanticBuilder::new()
            .with_build_nodes(true)
            .build(&ret.program)
            .semantic;
        let t_sem = std::time::Instant::now();
        let nodes = sem.nodes();
        // Expressions by the offset where they start: call and `new` arguments,
        // `import()` sources, and `new` expressions themselves (`new URL(...)`
        // sites). Several can start at one offset (`f(new URL(a, b).href)`: the
        // argument and the `new` expression), so a site picks the one that also
        // ends where it does.
        let mut index: HashMap<u32, Vec<Target<'_>>> = HashMap::new();
        // The calls asked about as loads ([`Ask::Load`]), by where they start.
        let loads: Vec<u32> = sites
            .iter()
            .filter(|s| s.1 == Ask::Load)
            .map(|s| s.0.start)
            .collect();
        let mut dynamic = false;
        for node in nodes.iter() {
            match node.kind() {
                AstKind::CallExpression(c) => {
                    if loads.contains(&c.span.start) {
                        index.entry(c.span.start).or_default().push(Target::Call(c));
                    }
                    for a in &c.arguments {
                        index
                            .entry(a.span().start)
                            .or_default()
                            .push(Target::Arg(a));
                    }
                    if let Expression::Identifier(id) = &c.callee
                        && id.name == "eval"
                        && id
                            .reference_id
                            .get()
                            .is_none_or(|r| !sem.scoping().has_binding(r))
                    {
                        dynamic = true;
                    }
                }
                AstKind::NewExpression(n) => {
                    for a in &n.arguments {
                        index
                            .entry(a.span().start)
                            .or_default()
                            .push(Target::Arg(a));
                    }
                    index.entry(n.span.start).or_default().push(Target::New(n));
                }
                AstKind::ImportExpression(i) => {
                    index
                        .entry(i.source.span().start)
                        .or_default()
                        .push(Target::Expr(&i.source));
                }
                AstKind::WithStatement(_) => dynamic = true,
                _ => {}
            }
        }
        if dynamic {
            // `eval` and `with` can rebind anything.
            return None;
        }
        let mut engine = Engine::new(nodes, sem.scoping(), text, file_name);
        let mut out = Vec::with_capacity(sites.len());
        for (span, ask) in sites {
            let end_of = |t: &Target<'_>| match t {
                Target::Arg(a) => a.span().end,
                Target::Expr(e) => e.span().end,
                Target::New(n) => n.span.end,
                Target::Call(c) => c.span.end,
            };
            let load = *ask == Ask::Load;
            let found = index.get(&span.start).and_then(|ts| {
                // The one that ends where the site does; for a truncated capture, the
                // outermost (the argument itself comes before what it starts with). A load
                // is a call, and only a load is.
                let mut ts = ts.iter().filter(|t| matches!(t, Target::Call(_)) == load);
                ts.clone()
                    .find(|t| end_of(t) == span.end)
                    .or_else(|| ts.find(|_| *ask == Ask::ValueFrom))
            });
            let v = found.and_then(|t| {
                let v = match t {
                    Target::Arg(Argument::SpreadElement(s)) => engine.site_spread(&s.argument),
                    Target::Arg(a) => engine.site(a.as_expression()?),
                    Target::Expr(e) => engine.site(e),
                    Target::New(n) => engine.site_new(n),
                    Target::Call(c) => engine.site_load(c),
                }?;
                let val = to_js(v);
                Some(SiteValue {
                    uses: engine.env.uses(&val),
                    val,
                    guess: engine.take_guess(),
                    from: engine.take_from(),
                })
            });
            out.push(v);
        }
        let mut facts = if env_facts {
            env::collect(&mut engine, nodes)
        } else {
            Vec::new()
        };
        env::set_lines(&mut facts, text);
        let anchors =
            anchors.map(|a| anchors::follow(&mut engine, text, a.uses, a.sites, a.templates));
        if turborepo_trace_core::debug::var_os("FLOW_TIMING").is_some() {
            // Debug aid: `flowparts <semantic µs> <index + evaluation µs> <steps>`.
            eprintln!(
                "flowparts {} {} {}",
                (t_sem - t_parse).as_micros(),
                t_sem.elapsed().as_micros(),
                engine.steps_used()
            );
        }
        return Some(FlowOutput {
            sites: out,
            env: facts,
            anchors,
        });
    }
    None
}

enum Target<'a> {
    Arg(&'a Argument<'a>),
    Expr(&'a Expression<'a>),
    New(&'a oxc_ast::ast::NewExpression<'a>),
    /// A call asked about as a load ([`Ask::Load`]).
    Call(&'a oxc_ast::ast::CallExpression<'a>),
}

/// The value of a site in the token evaluator's domain: the leaves that are
/// strings or paths; leaves that can never be a path are left out, and
/// containers and functions are unknown.
fn to_js(v: V<'_>) -> JsVal {
    let mut leaves: Vec<JsVal> = Vec::new();
    for l in v.into_leaves() {
        let j = match l {
            // A number or a boolean is no path.
            l if l.is_scalar() => continue,
            V::Js(j) => j,
            V::Void => continue,
            V::Builtin(value::Builtin::Tmp) => JsVal::Path(
                Anchor::Tmp,
                turborepo_trace_core::eval::Value::unknown(origin::mark(
                    Tag::Unknown,
                    "",
                    "temporary path",
                )),
            ),
            V::Import(i) => JsVal::Unknown(origin::mark(
                Tag::Import,
                &format!("{}#{}", i.module, i.name),
                &origin::clean(&i.name),
            )),
            V::Object(_)
            | V::Array(_)
            | V::Func(_)
            | V::Builtin(_)
            | V::Method(..)
            | V::Loader(_) => JsVal::Unknown(origin::mark(Tag::Unknown, "", "not a string")),

            V::Alts(_) => continue,
        };
        if !leaves.contains(&j) {
            leaves.push(j);
        }
    }
    match leaves.len() {
        // Never a path: an empty string lowers to nothing.
        0 => JsVal::Str(turborepo_trace_core::eval::Value::str("")),
        1 => leaves.pop().unwrap_or(JsVal::Alts(Vec::new())),
        _ => JsVal::Alts(leaves),
    }
}
