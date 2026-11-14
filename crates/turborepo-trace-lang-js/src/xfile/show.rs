//! A compact text form of facts, for tests and debugging.

use std::fmt::Write;

use turborepo_trace_core::eval::{PatPart, Value};

use super::facts::{FileFacts, Source, X};
use crate::extract::{
    evaluator::{Anchor, Func, JsVal},
    origin,
};

fn value(v: &Value, out: &mut String) {
    match v {
        Value::Str(s) => out.push_str(s),
        Value::Pattern(parts) => {
            for p in parts {
                match p {
                    PatPart::Lit(s) => out.push_str(s),
                    PatPart::Wild { .. } => out.push('*'),
                }
            }
        }
        Value::Alternatives(alts) => {
            out.push('{');
            for (i, a) in alts.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                value(a, out);
            }
            out.push('}');
        }
        Value::Unknown { .. } => out.push('*'),
    }
}

/// A value of the extractor's domain: strings in quotes, anchored paths as
/// `<dir>/x`, `<cwd>/x`, `<tmp>/x`, unknowns as `?origin`.
pub fn js(j: &JsVal) -> String {
    let mut out = String::new();
    match j {
        JsVal::Str(v) => {
            out.push('"');
            value(v, &mut out);
            out.push('"');
        }
        JsVal::Path(a, v) | JsVal::Url(a, v) => {
            out.push_str(match a {
                Anchor::FileDir => "<dir>/",
                Anchor::Cwd => "<cwd>/",
                Anchor::Tmp => "<tmp>/",
            });
            value(v, &mut out);
        }
        JsVal::File => out.push_str("<file>"),
        JsVal::FileUrl => out.push_str("<file url>"),
        JsVal::Unknown(r) => {
            let (o, payload) = origin::combine([&**r]);
            let _ = write!(out, "?{}", o.as_str());
            if !payload.is_empty() {
                let _ = write!(out, "({payload})");
            }
        }
        JsVal::Alts(vs) => {
            let parts: Vec<String> = vs.iter().map(js).collect();
            let _ = write!(out, "{}", parts.join(" | "));
        }
        other => {
            let _ = write!(out, "{other:?}");
        }
    }
    out
}

fn func(f: Func) -> &'static str {
    match f {
        Func::PathJoin => "join",
        Func::PathResolve => "resolve",
        Func::PathDirname => "dirname",
        Func::PathBasename => "basename",
        Func::PathExtname => "extname",
        Func::PathRelative => "relative",
        Func::PathNormalize => "normalize",
        Func::FileUrlToPath => "fileURLToPath",
        Func::PathToFileUrl => "pathToFileURL",
        _ => "fn",
    }
}

/// An expression: holes as `name#index` (the function's name from `facts`).
pub fn x(e: &X, facts: &FileFacts) -> String {
    let list = |xs: &[X]| {
        xs.iter()
            .map(|a| x(a, facts))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let fname = |id: u32| match facts.func(id) {
        Some(f) => f.name.clone(),
        None => format!("fn@{id}"),
    };
    match e {
        X::Const(j) => js(j),
        X::Void => "void".to_owned(),
        X::Param { func, index } => format!("{}#{index}", fname(*func)),
        X::Field(b, n) if n == super::facts::AS_ARRAY => format!("{}(as array)", x(b, facts)),
        X::Field(b, n) => format!("{}.{n}", x(b, facts)),
        X::Elem(b) => format!("{}[]", x(b, facts)),
        X::AnyField(b) => format!("{}.*", x(b, facts)),
        X::Concat(a, b) => format!("({} + {})", x(a, facts), x(b, facts)),
        X::Index { table, key } => format!("{}[{}]", x(table, facts), x(key, facts)),
        X::Apply(f, xs) => format!("{}({})", func(*f), list(xs)),
        X::Opaque(xs) => format!("opaque({})", list(xs)),
        X::Object { props, open } => {
            let mut parts: Vec<String> = props
                .iter()
                .map(|(k, v)| format!("{k}: {}", x(v, facts)))
                .collect();
            if let Some(o) = open {
                parts.push(format!("...: {}", x(o, facts)));
            }
            format!("{{{}}}", parts.join(", "))
        }
        X::Array { elems, rest } => {
            let mut parts: Vec<String> = elems.iter().map(|v| x(v, facts)).collect();
            if let Some(r) = rest {
                parts.push(format!("...{}", x(r, facts)));
            }
            format!("[{}]", parts.join(", "))
        }
        X::Func(id) => format!("fn {}", fname(*id)),
        X::Import { module, .. } if module == crate::xfile::bind::REQUIRE => "require".to_owned(),
        X::Import { module, name } => format!("import({module}#{name})"),
        X::Call { callee, args, rest } => {
            let mut a = list(args);
            if let Some(r) = rest {
                if !a.is_empty() {
                    a.push_str(", ");
                }
                let _ = write!(a, "...{}", x(r, facts));
            }
            format!("{}({a})", x(callee, facts))
        }
        X::Alts(xs) => xs
            .iter()
            .map(|a| x(a, facts))
            .collect::<Vec<_>>()
            .join(" | "),
        X::Reach { base, steps } => {
            let steps: Vec<String> = steps
                .iter()
                .map(|s| match s {
                    super::facts::Read::Field(n) if n == super::facts::AS_ARRAY => {
                        "as array".to_owned()
                    }
                    super::facts::Read::Field(n) if n == super::facts::ARRAY_ELEMENT => {
                        "each".to_owned()
                    }
                    super::facts::Read::Field(n) => format!(".{n}"),
                    super::facts::Read::Elem => "[]".to_owned(),
                    super::facts::Read::Any => ".*".to_owned(),
                })
                .collect();
            format!("reach({}; {})", x(base, facts), steps.join(" "))
        }
        X::Changed { by, prop } => format!(
            "changed{}({})",
            prop.as_ref().map(|p| format!(" .{p}")).unwrap_or_default(),
            handed(by, facts)
        ),
    }
}

/// Callees with the position of what they are handed.
fn handed(by: &[(X, u32)], facts: &FileFacts) -> String {
    by.iter()
        .map(|(c, pos)| {
            if *pos == super::facts::RECEIVER {
                format!("this of {}", x(c, facts))
            } else {
                format!("{}@{pos}", x(c, facts))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The facts of a file, one line per fact.
pub fn facts(f: &FileFacts) -> String {
    let mut out = String::new();
    for i in &f.imports {
        let _ = write!(out, "import {}", i.spec);
        if !i.escapes.is_empty() {
            let _ = write!(out, " escapes {}", i.escapes.join(","));
        }
        if i.writes {
            out.push_str(" writes");
        }
        out.push('\n');
    }
    for r in &f.reexports {
        let names: Vec<String> = r
            .names
            .iter()
            .map(|(a, b)| {
                if a == b {
                    a.clone()
                } else {
                    format!("{a} as {b}")
                }
            })
            .collect();
        let _ = writeln!(
            out,
            "reexport {} {}{}",
            r.spec,
            if r.star { "*" } else { "" },
            names.join(",")
        );
    }
    for e in &f.exports {
        let _ = writeln!(out, "export {} = {}", e.name, x(&e.value, f));
    }
    for func in &f.fns {
        let src = match &func.source {
            Source::Named { escapes: false } => "named".to_owned(),
            Source::Named { escapes: true } => "named escapes".to_owned(),
            Source::Ctor { escapes: false } => "ctor".to_owned(),
            Source::Ctor { escapes: true } => "ctor escapes".to_owned(),
            Source::Callback { call, arg } => format!("callback {arg} of {}", x(call, f)),
            Source::Unseen => "unseen".to_owned(),
        };
        let _ = write!(out, "fn {}: {src}", func.name);
        if let Some(r) = &func.returns {
            let _ = write!(out, " returns {}", x(r, f));
        }
        for h in &func.handoffs {
            let _ = write!(
                out,
                " hands #{} to [{}]{}",
                h.param,
                handed(&h.to, f),
                if h.unknown { " and unknown code" } else { "" }
            );
        }
        if let Some(p) = func.tests_string {
            let _ = write!(out, " tests #{p} for a string");
        }
        out.push('\n');
    }
    for s in &f.sinks {
        let _ = write!(
            out,
            "sink {}:{}{} {}",
            s.kind.as_str(),
            s.line,
            if s.open { "" } else { " (resolved)" },
            x(&s.value, f)
        );
        if s.string_only {
            out.push_str(" if a string");
        }
        if !s.string_tests.is_empty() {
            let _ = write!(out, " if [{}]", handed(&s.string_tests, f));
        }
        out.push('\n');
    }
    for c in &f.calls {
        let mut args: Vec<String> = c.args.iter().map(|a| x(a, f)).collect();
        if let Some(r) = &c.rest {
            args.push(format!("...{}", x(r, f)));
        }
        let _ = writeln!(
            out,
            "call:{} {}{}({})",
            c.line,
            if c.new { "new " } else { "" },
            x(&c.callee, f),
            args.join(", ")
        );
    }
    for c in &f.callables {
        let _ = writeln!(
            out,
            "callable {}{}",
            x(&c.value, f),
            if c.escapes { " escapes" } else { "" }
        );
    }
    for t in &f.tables {
        let keys: Vec<String> = t.keys.iter().map(|k| x(k, f)).collect();
        let _ = writeln!(
            out,
            "table {} value(s){} keys {} into {} site(s)",
            t.values.len(),
            if t.keyed_only { " keyed only" } else { "" },
            keys.join(" ; "),
            t.sinks.len()
        );
    }
    if f.exports_open {
        out.push_str("exports open\n");
    }
    if f.writes.any || f.writes.arrays || !f.writes.props.is_empty() {
        let _ = writeln!(
            out,
            "writes {}{}{}",
            f.writes.props.join(","),
            if f.writes.any { " any" } else { "" },
            if f.writes.arrays { " arrays" } else { "" }
        );
    }
    out
}

/// The result of binding, one line per dependency, closed sink and closed
/// table.
pub fn bound(b: &super::bind::Bound, path: &dyn Fn(u32) -> String) -> String {
    use super::bind::DepValue;
    let mut lines: Vec<String> = Vec::new();
    for d in &b.deps {
        let v = match &d.value {
            DepValue::Path(p) => format!("path {p}"),
            DepValue::Below(p) => format!("below {p}"),
            DepValue::Cwd(p) => format!("cwd {p}"),
            DepValue::Spec(s) => format!("spec {s}"),
            DepValue::PerTestCwd(s) => format!("cwd-per-test {s}"),
            DepValue::Open { origin, detail } => format!("open {} ({detail})", origin.as_str()),
        };
        lines.push(format!(
            "dep {}:{} {} {v} <- {}:{}",
            path(d.file),
            d.line,
            d.kind.as_str(),
            path(d.sink_file),
            d.sink_line
        ));
    }
    for c in &b.closed {
        lines.push(format!(
            "closed {} {}{}",
            path(c.file),
            if c.open { "record" } else { "resolved" },
            if c.specs.is_empty() {
                String::new()
            } else {
                format!(" [{}]", c.specs.join(", "))
            }
        ));
    }
    for t in &b.tables {
        lines.push(format!("table {} [{}]", path(t.file), t.specs.join(", ")));
    }
    lines.sort();
    lines.dedup();
    lines.join("\n")
}
