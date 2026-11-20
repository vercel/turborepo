//! A compact binary form of [`FileFacts`], for caches keyed by a file's content
//! hash.
//!
//! The format is private to one build: [`VERSION`] changes whenever the facts
//! or their extraction change, and [`decode`] rejects anything else (a rejected
//! entry is extracted again).

use turborepo_trace_core::{
    RefKind,
    api::BaseRule,
    eval::{PatPart, Value},
};

use super::facts::{
    Call, Callable, Export, FileFacts, FnFact, Handoff, Import, ReExport, Read, Sink, Source,
    Table, Writes, X,
};
use crate::extract::evaluator::{Anchor, Func, JsVal};

/// The format version: bump on any change to the facts or to what the
/// extraction produces.
pub const VERSION: u8 = 6;

const FUNCS: [Func; 16] = [
    Func::PathJoin,
    Func::PathResolve,
    Func::PathDirname,
    Func::PathBasename,
    Func::PathExtname,
    Func::PathRelative,
    Func::PathNormalize,
    Func::FileUrlToPath,
    Func::PathToFileUrl,
    Func::ProcessCwd,
    Func::Resolve,
    Func::StringRaw,
    Func::ResolveFrom,
    Func::GuessJoin,
    Func::GuessResolve,
    Func::MetaResolve,
];

const RULES: [BaseRule; 4] = [
    BaseRule::Path,
    BaseRule::Module,
    BaseRule::FileDir,
    BaseRule::PackageRoot,
];

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    fn u32(&mut self, mut v: u32) {
        while v >= 0x80 {
            self.0.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.0.push(v as u8);
    }

    fn len(&mut self, n: usize) {
        self.u32(n as u32);
    }

    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.0.extend_from_slice(s.as_bytes());
    }

    fn bool(&mut self, b: bool) {
        self.u8(u8::from(b));
    }

    fn value(&mut self, v: &Value) {
        match v {
            Value::Str(s) => {
                self.u8(0);
                self.str(s);
            }
            Value::Pattern(parts) => {
                self.u8(1);
                self.len(parts.len());
                for p in parts {
                    match p {
                        PatPart::Lit(s) => {
                            self.u8(0);
                            self.str(s);
                        }
                        PatPart::Wild { reason } => {
                            self.u8(1);
                            self.str(reason);
                        }
                    }
                }
            }
            Value::Alternatives(alts) => {
                self.u8(2);
                self.len(alts.len());
                alts.iter().for_each(|a| self.value(a));
            }
            Value::Unknown { reason } => {
                self.u8(3);
                self.str(reason);
            }
        }
    }

    fn anchor(&mut self, a: Anchor) {
        self.u8(match a {
            Anchor::FileDir => 0,
            Anchor::Cwd => 1,
            Anchor::Tmp => 2,
        });
    }

    fn js(&mut self, j: &JsVal) {
        match j {
            JsVal::Str(v) => {
                self.u8(0);
                self.value(v);
            }
            JsVal::Path(a, v) => {
                self.u8(1);
                self.anchor(*a);
                self.value(v);
            }
            JsVal::Url(a, v) => {
                self.u8(2);
                self.anchor(*a);
                self.value(v);
            }
            JsVal::File => self.u8(3),
            JsVal::FileUrl => self.u8(4),
            JsVal::Unknown(r) => {
                self.u8(5);
                self.str(r);
            }
            JsVal::Alts(vs) => {
                self.u8(6);
                self.len(vs.len());
                vs.iter().for_each(|v| self.js(v));
            }
            // Builtin objects and functions, arrays of the token evaluator, paths next to
            // a resolved package: never a path a caller can supply.
            JsVal::Resolved(..) | JsVal::Obj(_) | JsVal::Func(_) | JsVal::Arr(_) => {
                self.u8(5);
                self.str("@unknown[] not a string");
            }
        }
    }

    fn xs(&mut self, xs: &[X]) {
        self.len(xs.len());
        xs.iter().for_each(|x| self.x(x));
    }

    fn opt_x(&mut self, x: Option<&X>) {
        match x {
            Some(x) => {
                self.u8(1);
                self.x(x);
            }
            None => self.u8(0),
        }
    }

    fn x(&mut self, x: &X) {
        match x {
            X::Const(j) => {
                self.u8(0);
                self.js(j);
            }
            X::Void => self.u8(1),
            X::Param { func, index } => {
                self.u8(2);
                self.u32(*func);
                self.u32(*index);
            }
            X::Field(b, n) => {
                self.u8(3);
                self.x(b);
                self.str(n);
            }
            X::Elem(b) => {
                self.u8(4);
                self.x(b);
            }
            X::AnyField(b) => {
                self.u8(5);
                self.x(b);
            }
            X::Concat(a, b) => {
                self.u8(6);
                self.x(a);
                self.x(b);
            }
            X::Apply(f, xs) => {
                self.u8(7);
                self.u8(FUNCS.iter().position(|g| g == f).unwrap_or(0) as u8);
                self.xs(xs);
            }
            X::Opaque(xs) => {
                self.u8(8);
                self.xs(xs);
            }
            X::Index { table, key } => {
                self.u8(9);
                self.x(table);
                self.x(key);
            }
            X::Object { props, open } => {
                self.u8(10);
                self.len(props.len());
                for (k, v) in props {
                    self.str(k);
                    self.x(v);
                }
                self.opt_x(open.as_deref());
            }
            X::Array { elems, rest } => {
                self.u8(11);
                self.xs(elems);
                self.opt_x(rest.as_deref());
            }
            X::Func(id) => {
                self.u8(12);
                self.u32(*id);
            }
            X::Import { module, name } => {
                self.u8(13);
                self.str(module);
                self.str(name);
            }
            X::Call { callee, args, rest } => {
                self.u8(14);
                self.x(callee);
                self.xs(args);
                self.opt_x(rest.as_deref());
            }
            X::Alts(xs) => {
                self.u8(15);
                self.xs(xs);
            }
            X::Reach { base, steps } => {
                self.u8(17);
                self.x(base);
                self.len(steps.len());
                for s in steps {
                    match s {
                        Read::Field(n) => {
                            self.u8(0);
                            self.str(n);
                        }
                        Read::Elem => self.u8(1),
                        Read::Any => self.u8(2),
                    }
                }
            }
            X::Changed { by, prop } => {
                self.u8(16);
                self.pairs(by);
                match prop {
                    Some(p) => {
                        self.u8(1);
                        self.str(p);
                    }
                    None => self.u8(0),
                }
            }
        }
    }

    fn pairs(&mut self, by: &[(X, u32)]) {
        self.len(by.len());
        for (x, pos) in by {
            self.x(x);
            self.u32(*pos);
        }
    }
}

/// Serializes facts.
pub fn encode(f: &FileFacts) -> Vec<u8> {
    let mut w = W(Vec::with_capacity(256));
    w.u8(VERSION);
    w.len(f.imports.len());
    for i in &f.imports {
        w.u32(i.at);
        w.str(&i.spec);
        w.len(i.escapes.len());
        i.escapes.iter().for_each(|e| w.str(e));
        w.bool(i.writes);
    }
    w.len(f.reexports.len());
    for r in &f.reexports {
        w.u32(r.at);
        w.str(&r.spec);
        w.len(r.names.len());
        for (a, b) in &r.names {
            w.str(a);
            w.str(b);
        }
        w.bool(r.star);
    }
    w.len(f.exports.len());
    for e in &f.exports {
        w.str(&e.name);
        w.x(&e.value);
    }
    w.len(f.fns.len());
    for func in &f.fns {
        w.u32(func.id);
        w.str(&func.name);
        w.u32(func.line);
        match &func.source {
            Source::Named { escapes } => {
                w.u8(0);
                w.bool(*escapes);
            }
            Source::Ctor { escapes } => {
                w.u8(1);
                w.bool(*escapes);
            }
            Source::Callback { call, arg } => {
                w.u8(2);
                w.x(call);
                w.u32(*arg);
            }
            Source::Unseen => w.u8(3),
        }
        w.opt_x(func.returns.as_ref());
        w.len(func.param_escapes.len());
        func.param_escapes.iter().for_each(|p| w.u32(*p));
        match func.rest {
            Some(r) => {
                w.u8(1);
                w.u32(r);
            }
            None => w.u8(0),
        }
        w.len(func.handoffs.len());
        for h in &func.handoffs {
            w.u32(h.param);
            w.pairs(&h.to);
            w.bool(h.unknown);
        }
        w.bool(func.uses_this);
        w.bool(func.uses_arguments);
        match func.tests_string {
            Some(p) => {
                w.u8(1);
                w.u32(p);
            }
            None => w.u8(0),
        }
    }
    w.len(f.sinks.len());
    for s in &f.sinks {
        w.u8(RefKind::ALL.iter().position(|k| *k == s.kind).unwrap_or(0) as u8);
        w.u8(RULES.iter().position(|r| *r == s.rule).unwrap_or(0) as u8);
        for v in s.at.iter().chain(&s.site) {
            w.u32(*v);
        }
        w.bool(s.open);
        w.u32(s.line);
        w.x(&s.value);
        w.bool(s.string_only);
        w.pairs(&s.string_tests);
    }
    w.len(f.calls.len());
    for c in &f.calls {
        w.x(&c.callee);
        w.xs(&c.args);
        w.opt_x(c.rest.as_ref());
        w.u32(c.line);
        w.bool(c.new);
        match c.within {
            Some(f) => {
                w.u8(1);
                w.u32(f);
            }
            None => w.u8(0),
        }
    }
    w.len(f.callables.len());
    for c in &f.callables {
        w.x(&c.value);
        w.bool(c.escapes);
    }
    w.len(f.tables.len());
    for t in &f.tables {
        w.u32(t.at[0]);
        w.u32(t.at[1]);
        w.bool(t.keyed_only);
        w.xs(&t.keys);
        w.len(t.values.len());
        for (k, v) in &t.values {
            w.str(k);
            w.str(v);
        }
        w.len(t.sinks.len());
        for s in &t.sinks {
            w.u32(s[0]);
            w.u32(s[1]);
        }
    }
    w.bool(f.exports_open);
    w.len(f.writes.props.len());
    f.writes.props.iter().for_each(|p| w.str(p));
    w.bool(f.writes.any);
    w.bool(f.writes.arrays);
    w.0
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
    depth: u32,
}

impl R<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.at)?;
        self.at += 1;
        Some(v)
    }

    fn u32(&mut self) -> Option<u32> {
        let mut v = 0u32;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            v |= u32::from(b & 0x7f).checked_shl(shift)?;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }

    /// A count of items that each take at least one byte.
    fn len(&mut self) -> Option<usize> {
        let n = self.u32()? as usize;
        (n <= self.b.len() - self.at).then_some(n)
    }

    fn str(&mut self) -> Option<String> {
        let n = self.len()?;
        let s = std::str::from_utf8(self.b.get(self.at..self.at + n)?).ok()?;
        self.at += n;
        Some(s.to_owned())
    }

    fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.depth += 1;
        if self.depth > 256 {
            return None;
        }
        let v = match self.u8()? {
            0 => Value::Str(self.str()?),
            1 => {
                let n = self.len()?;
                let mut parts = Vec::with_capacity(n);
                for _ in 0..n {
                    parts.push(match self.u8()? {
                        0 => PatPart::Lit(self.str()?),
                        1 => PatPart::Wild {
                            reason: self.str()?.into(),
                        },
                        _ => return None,
                    });
                }
                Value::Pattern(parts)
            }
            2 => {
                let n = self.len()?;
                let mut alts = Vec::with_capacity(n);
                for _ in 0..n {
                    alts.push(self.value()?);
                }
                Value::Alternatives(alts)
            }
            3 => Value::Unknown {
                reason: self.str()?.into(),
            },
            _ => return None,
        };
        self.depth -= 1;
        Some(v)
    }

    fn anchor(&mut self) -> Option<Anchor> {
        Some(match self.u8()? {
            0 => Anchor::FileDir,
            1 => Anchor::Cwd,
            2 => Anchor::Tmp,
            _ => return None,
        })
    }

    fn js(&mut self) -> Option<JsVal> {
        self.depth += 1;
        if self.depth > 256 {
            return None;
        }
        let j = match self.u8()? {
            0 => JsVal::Str(self.value()?),
            1 => JsVal::Path(self.anchor()?, self.value()?),
            2 => JsVal::Url(self.anchor()?, self.value()?),
            3 => JsVal::File,
            4 => JsVal::FileUrl,
            5 => JsVal::Unknown(self.str()?.into()),
            6 => {
                let n = self.len()?;
                let mut vs = Vec::with_capacity(n);
                for _ in 0..n {
                    vs.push(self.js()?);
                }
                JsVal::Alts(vs)
            }
            _ => return None,
        };
        self.depth -= 1;
        Some(j)
    }

    fn xs(&mut self) -> Option<Vec<X>> {
        let n = self.len()?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.x()?);
        }
        Some(out)
    }

    fn opt_x(&mut self) -> Option<Option<X>> {
        match self.u8()? {
            0 => Some(None),
            1 => Some(Some(self.x()?)),
            _ => None,
        }
    }

    fn x(&mut self) -> Option<X> {
        self.depth += 1;
        if self.depth > 256 {
            return None;
        }
        let x = match self.u8()? {
            0 => X::Const(self.js()?),
            1 => X::Void,
            2 => X::Param {
                func: self.u32()?,
                index: self.u32()?,
            },
            3 => {
                let b = self.x()?;
                X::Field(Box::new(b), self.str()?)
            }
            4 => X::Elem(Box::new(self.x()?)),
            5 => X::AnyField(Box::new(self.x()?)),
            6 => {
                let a = self.x()?;
                X::Concat(Box::new(a), Box::new(self.x()?))
            }
            7 => {
                let f = *FUNCS.get(self.u8()? as usize)?;
                X::Apply(f, self.xs()?)
            }
            8 => X::Opaque(self.xs()?),
            9 => {
                let table = self.x()?;
                X::Index {
                    table: Box::new(table),
                    key: Box::new(self.x()?),
                }
            }
            10 => {
                let n = self.len()?;
                let mut props = Vec::with_capacity(n);
                for _ in 0..n {
                    let k = self.str()?;
                    props.push((k, self.x()?));
                }
                X::Object {
                    props,
                    open: self.opt_x()?.map(Box::new),
                }
            }
            11 => {
                let elems = self.xs()?;
                X::Array {
                    elems,
                    rest: self.opt_x()?.map(Box::new),
                }
            }
            12 => X::Func(self.u32()?),
            13 => {
                let module = self.str()?;
                X::Import {
                    module,
                    name: self.str()?,
                }
            }
            14 => {
                let callee = self.x()?;
                let args = self.xs()?;
                X::Call {
                    callee: Box::new(callee),
                    args,
                    rest: self.opt_x()?.map(Box::new),
                }
            }
            15 => X::Alts(self.xs()?),
            17 => {
                let base = self.x()?;
                let n = self.len()?;
                let mut steps = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    steps.push(match self.u8()? {
                        0 => Read::Field(self.str()?),
                        1 => Read::Elem,
                        2 => Read::Any,
                        _ => return None,
                    });
                }
                X::Reach {
                    base: Box::new(base),
                    steps,
                }
            }
            16 => {
                let by = self.pairs()?;
                let prop = match self.u8()? {
                    0 => None,
                    1 => Some(self.str()?),
                    _ => return None,
                };
                X::Changed { by, prop }
            }
            _ => return None,
        };
        self.depth -= 1;
        Some(x)
    }

    fn pairs(&mut self) -> Option<Vec<(X, u32)>> {
        let n = self.len()?;
        let mut out = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            let x = self.x()?;
            out.push((x, self.u32()?));
        }
        Some(out)
    }
}

/// Reads facts written by [`encode`]; `None` for another version or damaged
/// data.
pub fn decode(b: &[u8]) -> Option<FileFacts> {
    let mut r = R { b, at: 0, depth: 0 };
    if r.u8()? != VERSION {
        return None;
    }
    let mut f = FileFacts::default();
    for _ in 0..r.len()? {
        let at = r.u32()?;
        let spec = r.str()?;
        let mut escapes = Vec::new();
        for _ in 0..r.len()? {
            escapes.push(r.str()?);
        }
        f.imports.push(Import {
            at,
            spec,
            escapes,
            writes: r.bool()?,
        });
    }
    for _ in 0..r.len()? {
        let at = r.u32()?;
        let spec = r.str()?;
        let mut names = Vec::new();
        for _ in 0..r.len()? {
            let a = r.str()?;
            names.push((a, r.str()?));
        }
        f.reexports.push(ReExport {
            at,
            spec,
            names,
            star: r.bool()?,
        });
    }
    for _ in 0..r.len()? {
        let name = r.str()?;
        f.exports.push(Export {
            name,
            value: r.x()?,
        });
    }
    for _ in 0..r.len()? {
        let id = r.u32()?;
        let name = r.str()?;
        let line = r.u32()?;
        let source = match r.u8()? {
            0 => Source::Named { escapes: r.bool()? },
            1 => Source::Ctor { escapes: r.bool()? },
            2 => {
                let call = r.x()?;
                Source::Callback {
                    call,
                    arg: r.u32()?,
                }
            }
            3 => Source::Unseen,
            _ => return None,
        };
        let returns = r.opt_x()?;
        let mut param_escapes = Vec::new();
        for _ in 0..r.len()? {
            param_escapes.push(r.u32()?);
        }
        let rest = match r.u8()? {
            0 => None,
            1 => Some(r.u32()?),
            _ => return None,
        };
        let mut handoffs = Vec::new();
        for _ in 0..r.len()? {
            let param = r.u32()?;
            let to = r.pairs()?;
            handoffs.push(Handoff {
                param,
                to,
                unknown: r.bool()?,
            });
        }
        let uses_this = r.bool()?;
        let uses_arguments = r.bool()?;
        let tests_string = match r.u8()? {
            0 => None,
            1 => Some(r.u32()?),
            _ => return None,
        };
        f.fns.push(FnFact {
            id,
            name,
            line,
            source,
            returns,
            param_escapes,
            rest,
            handoffs,
            uses_this,
            uses_arguments,
            tests_string,
        });
    }
    for _ in 0..r.len()? {
        let kind = *RefKind::ALL.get(r.u8()? as usize)?;
        let rule = *RULES.get(r.u8()? as usize)?;
        let at = [r.u32()?, r.u32()?];
        let site = [r.u32()?, r.u32()?];
        let open = r.bool()?;
        let line = r.u32()?;
        let value = r.x()?;
        let string_only = r.bool()?;
        f.sinks.push(Sink {
            kind,
            rule,
            at,
            site,
            open,
            line,
            value,
            string_only,
            string_tests: r.pairs()?,
        });
    }
    for _ in 0..r.len()? {
        let callee = r.x()?;
        let args = r.xs()?;
        let rest = r.opt_x()?;
        let line = r.u32()?;
        let new = r.bool()?;
        let within = match r.u8()? {
            0 => None,
            1 => Some(r.u32()?),
            _ => return None,
        };
        f.calls.push(Call {
            callee,
            args,
            rest,
            line,
            new,
            within,
        });
    }
    for _ in 0..r.len()? {
        let value = r.x()?;
        f.callables.push(Callable {
            value,
            escapes: r.bool()?,
        });
    }
    for _ in 0..r.len()? {
        let at = [r.u32()?, r.u32()?];
        let keyed_only = r.bool()?;
        let keys = r.xs()?;
        let mut values = Vec::new();
        for _ in 0..r.len()? {
            let k = r.str()?;
            values.push((k, r.str()?));
        }
        let mut sinks = Vec::new();
        for _ in 0..r.len()? {
            sinks.push([r.u32()?, r.u32()?]);
        }
        f.tables.push(Table {
            at,
            keyed_only,
            keys,
            values,
            sinks,
        });
    }
    f.exports_open = r.bool()?;
    let mut props = Vec::new();
    for _ in 0..r.len()? {
        props.push(r.str()?);
    }
    let any = r.bool()?;
    f.writes = Writes {
        props,
        any,
        arrays: r.bool()?,
    };
    (r.at == b.len()).then_some(f)
}
