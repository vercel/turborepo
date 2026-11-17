//! The abstract values of the value-flow engine.
//!
//! [`V`] extends the token evaluator's string domain ([`JsVal`]: known strings, patterns,
//! anchored paths, URLs, the path/url/process builtins, unknowns with an origin
//! mark) with what scoped evaluation needs: object and array values, user
//! functions, a few more builtins, and [`V::Void`] for values that can never be
//! a path.

use std::rc::Rc;

use oxc_ast::ast::{ArrowFunctionExpression, Function};
use oxc_span::Span;
use turborepo_trace_core::eval::Value;

use crate::extract::{
    evaluator::JsVal,
    origin::{self, Tag},
};

/// Maximum alternatives of one value. A join with more becomes unknown.
pub(crate) const MAX_ALTS: usize = 64;
/// The reason of [`V::scalar`]. Source text never produces it: reasons made
/// from text go through [`origin::clean`], which collapses whitespace to single
/// spaces.
const SCALAR: &str = "@unknown[] number or\tboolean";
/// Maximum properties or elements kept per container.
pub(crate) const MAX_ITEMS: usize = 256;

/// An abstract value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum V<'a> {
    /// A string-like leaf of the token evaluator's domain. Never
    /// [`JsVal::Alts`].
    Js(JsVal),
    /// `undefined`, `null`, a boolean, a file descriptor: never a path. Using
    /// it as one throws, so it adds no reference.
    Void,
    /// An object literal (also `Map`s with known keys).
    Object(Rc<ObjV<'a>>),
    /// An array literal (also `Set`s): elements by position plus elements at
    /// unknown positions.
    Array(Rc<ArrV<'a>>),
    /// A function or arrow function of the file.
    Func(FnRef<'a>),
    /// A builtin object the token evaluator does not model.
    Builtin(Builtin),
    /// A member of a builtin object (`os.tmpdir`, `fs.mkdtempSync`), not called
    /// yet.
    Method(Builtin, Rc<str>),
    /// A `require` function made by `createRequire(filename)` for a known place
    /// that is not the file's own directory: what it loads and resolves is
    /// searched from there ([`crate::extract::evaluator::resolve_from`]).
    /// The file's own `require` (also `createRequire(import.meta.url)`) is
    /// [`crate::extract::evaluator::Obj::Require`].
    Loader(Rc<crate::extract::evaluator::Start>),

    /// A binding imported from another module (`name` is the export path, `*`
    /// the module itself).
    Import(Rc<ImportRef>),
    /// One of several values (flat, deduplicated, at most [`MAX_ALTS`]).
    Alts(Rc<Vec<V<'a>>>),
}

/// How far a container can be trusted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Trust {
    /// A literal that has not been stored anywhere: nothing else can have
    /// changed it.
    Fresh,
    /// The value of a binding whose every use was checked (its direct mutations
    /// are part of the value).
    Tracked,
    /// Reached through another container, a parameter or an alias: other code
    /// of the file may have written to it. Reads also consult the file's
    /// untracked writes.
    Shared,
}

/// An object value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ObjV<'a> {
    /// Known properties, in definition order.
    pub props: Vec<(Rc<str>, V<'a>)>,
    /// The value of every other property (a spread of an unknown object, a
    /// write with a computed key). `None`: there are no other properties.
    pub open: Option<V<'a>>,
    /// Known properties written by name in the object's literal after a spread
    /// (`{...rest, name: v}`): what widened the object before does not apply to
    /// them, only what widened it since the first of them was written
    /// ([`ObjV::late`]). Only cross-file facts read it ([`super::sym`]).
    pub exact: Vec<Rc<str>>,
    /// The part of [`ObjV::open`] that was added after the first exact
    /// property.
    pub late: Option<V<'a>>,
    pub trust: Trust,
    /// A `Map`: `.set(k, v)` writes entries.
    pub map: bool,
}

/// An array value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ArrV<'a> {
    /// Elements at known positions.
    pub elems: Vec<V<'a>>,
    /// Elements at unknown positions (pushed, spread, mapped).
    pub rest: Option<V<'a>>,
    pub trust: Trust,
}

/// A user function.
#[derive(Copy, Clone, Debug)]
pub(crate) enum FnRef<'a> {
    Function(&'a Function<'a>),
    Arrow(&'a ArrowFunctionExpression<'a>),
}

impl FnRef<'_> {
    pub(crate) fn span(&self) -> Span {
        match self {
            FnRef::Function(f) => f.span,
            FnRef::Arrow(f) => f.span,
        }
    }
}

impl PartialEq for FnRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.span() == other.span()
    }
}

/// An imported binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportRef {
    /// The module specifier.
    pub module: Box<str>,
    /// The export path inside the module (`default`, `a`, `a.b`), `*` for the
    /// namespace.
    pub name: Box<str>,
}

/// Builtin objects beyond the token evaluator's
/// ([`crate::extract::evaluator::Obj`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Builtin {
    /// `os`.
    Os,
    /// `fs`, `fs-extra`, `graceful-fs`.
    Fs,
    /// `fs/promises`, `fs.promises`.
    FsPromises,
    /// `module`.
    Module,
    /// `Object`.
    Object,
    /// `Array`.
    Array,
    /// `JSON`.
    Json,
    /// `Promise`.
    Promise,
    /// `Set` / `Map` constructors.
    SetCtor,
    MapCtor,
    /// A temp-file library (`tmp`, `tmp-promise`, `tempy`, `temp-dir`, `temp`)
    /// or a value obtained from one: every path derived from it lies in the
    /// temporary directory.
    Tmp,
}

impl<'a> V<'a> {
    pub(crate) fn str(s: impl Into<String>) -> V<'a> {
        V::Js(JsVal::Str(Value::Str(s.into())))
    }

    /// An unknown value with origin `tag`; `text` is cleaned source text.
    pub(crate) fn unknown(tag: Tag, payload: &str, text: &str) -> V<'a> {
        V::Js(JsVal::Unknown(origin::mark(tag, payload, text)))
    }

    /// A number or a boolean: never a path by itself (like [`V::Void`], a site
    /// that gets one has no reference), but inside a concatenation it is
    /// some text (`` `chunk-${i}.json` ``, `'out-' + minify`) and as a key
    /// it selects some property. Represented as an unknown with a reserved
    /// reason, so every place that handles unknowns over-approximates it.
    pub(crate) fn scalar() -> V<'a> {
        V::Js(JsVal::Unknown(SCALAR.into()))
    }

    /// Whether the value is [`V::scalar`].
    pub(crate) fn is_scalar(&self) -> bool {
        matches!(self, V::Js(JsVal::Unknown(r)) if &**r == SCALAR)
    }

    /// Whether the value may be a number at runtime: a scalar, an unknown, or a
    /// string of digits (numeric literals are kept as their text).
    pub(crate) fn maybe_number(&self) -> bool {
        match self {
            V::Js(JsVal::Unknown(_)) => true,
            V::Js(JsVal::Str(Value::Str(s))) => {
                !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
            }
            _ => false,
        }
    }

    pub(crate) fn object(props: Vec<(Rc<str>, V<'a>)>, open: Option<V<'a>>) -> V<'a> {
        V::Object(Rc::new(ObjV {
            props,
            open,
            exact: Vec::new(),
            late: None,
            trust: Trust::Fresh,
            map: false,
        }))
    }

    pub(crate) fn array(elems: Vec<V<'a>>, rest: Option<V<'a>>) -> V<'a> {
        V::Array(Rc::new(ArrV {
            elems,
            rest,
            trust: Trust::Fresh,
        }))
    }

    /// The leaves of the value (itself unless it is [`V::Alts`]).
    pub(crate) fn leaves(&self) -> &[V<'a>] {
        match self {
            V::Alts(vs) => vs,
            v => std::slice::from_ref(v),
        }
    }

    pub(crate) fn into_leaves(self) -> Vec<V<'a>> {
        match self {
            V::Alts(vs) => Rc::try_unwrap(vs).unwrap_or_else(|rc| (*rc).clone()),
            v => vec![v],
        }
    }

    /// Whether some leaf is a container.
    pub(crate) fn has_container(&self) -> bool {
        self.leaves()
            .iter()
            .any(|l| matches!(l, V::Object(_) | V::Array(_)))
    }

    /// The value with every container leaf marked [`Trust::Shared`] (unless it
    /// already is).
    pub(crate) fn shared(self) -> V<'a> {
        if !self.has_container() {
            return self;
        }
        join(self.into_leaves().into_iter().map(|l| match l {
            V::Object(o) if o.trust != Trust::Shared => {
                let mut o = (*o).clone();
                o.trust = Trust::Shared;
                V::Object(Rc::new(o))
            }
            V::Array(a) if a.trust != Trust::Shared => {
                let mut a = (*a).clone();
                a.trust = Trust::Shared;
                V::Array(Rc::new(a))
            }
            l => l,
        }))
    }
}

/// One of `vals`: flattened and deduplicated. More than [`MAX_ALTS`] distinct
/// leaves become a single unknown value (which covers them all).
pub(crate) fn join<'a>(vals: impl IntoIterator<Item = V<'a>>) -> V<'a> {
    let mut out: Vec<V<'a>> = Vec::new();
    let mut over = false;
    for v in vals {
        for l in v.into_leaves() {
            // Split string-level alternatives so that every leaf is a single value.
            match l {
                V::Js(j) if has_alts(&j) => {
                    for x in crate::extract::evaluator::split_alts(j) {
                        push(&mut out, V::Js(x), &mut over);
                    }
                }
                l => push(&mut out, l, &mut over),
            }
        }
    }
    if over {
        return V::unknown(Tag::Unknown, "", "too many alternatives");
    }
    match out.len() {
        0 => V::Void,
        1 => out.pop().unwrap_or(V::Void),
        _ => V::Alts(Rc::new(out)),
    }
}

fn has_alts(j: &JsVal) -> bool {
    matches!(
        j,
        JsVal::Alts(_)
            | JsVal::Str(Value::Alternatives(_))
            | JsVal::Path(_, Value::Alternatives(_))
            | JsVal::Url(_, Value::Alternatives(_))
    )
}

fn push<'a>(out: &mut Vec<V<'a>>, v: V<'a>, over: &mut bool) {
    if out.contains(&v) {
        return;
    }
    if out.len() >= MAX_ALTS {
        *over = true;
        return;
    }
    out.push(v);
}

impl<'a> ObjV<'a> {
    pub(crate) fn get(&self, name: &str) -> Option<&V<'a>> {
        self.props
            .iter()
            .rev()
            .find(|(k, _)| &**k == name)
            .map(|(_, v)| v)
    }

    /// Sets `name` to `v`, joining with the current value when `weak` (a write
    /// that may or may not have happened).
    pub(crate) fn set(&mut self, name: &str, v: V<'a>, weak: bool) {
        if let Some(slot) = self.props.iter_mut().find(|(k, _)| &**k == name) {
            slot.1 = if weak { join([slot.1.clone(), v]) } else { v };
        } else if self.props.len() < MAX_ITEMS {
            let v = if weak { join([V::Void, v]) } else { v };
            self.props.push((name.into(), v));
        } else {
            self.widen(v);
        }
    }

    /// Records that any property may also hold `v`.
    pub(crate) fn widen(&mut self, v: V<'a>) {
        if !self.exact.is_empty() {
            self.late = Some(match self.late.take() {
                Some(o) => join([o, v.clone()]),
                None => v.clone(),
            });
        }
        self.open = Some(match self.open.take() {
            Some(o) => join([o, v]),
            None => v,
        });
    }

    /// Marks `name` as written by name after everything that widened the
    /// object.
    pub(crate) fn set_exact(&mut self, name: &str) {
        if !self.exact.iter().any(|k| &**k == name) {
            self.exact.push(name.into());
        }
    }

    /// Whether only the late part of the open value applies to the known
    /// property `name`.
    pub(crate) fn is_exact(&self, name: &str) -> bool {
        self.exact.iter().any(|k| &**k == name)
    }

    /// Every value a property can hold.
    pub(crate) fn values(&self) -> V<'a> {
        join(
            self.props
                .iter()
                .map(|(_, v)| v.clone())
                .chain(self.open.clone()),
        )
    }
}

impl<'a> ArrV<'a> {
    /// Every element.
    pub(crate) fn elements(&self) -> V<'a> {
        join(self.elems.iter().cloned().chain(self.rest.clone()))
    }

    /// Records an element at an unknown position.
    pub(crate) fn add(&mut self, v: V<'a>) {
        self.rest = Some(match self.rest.take() {
            Some(r) => join([r, v]),
            None => v,
        });
    }
}
