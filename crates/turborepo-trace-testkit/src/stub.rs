//! A trivial line-oriented "language" for testing the tracer end to end. Not a
//! real language and not a default CLI feature.
//!
//! Files with the `.stub` extension (or starting with `#!stub`) contain one
//! directive per line:
//!
//! ```text
//! # comment
//! [modifier...] keyword argument
//! ```
//!
//! | keyword    | kind                    | resolution                                     |
//! |------------|-------------------------|------------------------------------------------|
//! | `dep`      | [`RefKind::Import`]     | `.stub` extension probing, `index.stub`, bare specifiers under `<base>/stub_modules/` (emits `stub.json` as a manifest) |
//! | `asset`    | [`RefKind::FsRead`]     | exact file                                     |
//! | `dir`      | [`RefKind::FsReadDir`]  | directory, all files below it                  |
//! | `manifest` | [`RefKind::Manifest`]   | exact file                                     |
//! | `lib`      | [`RefKind::SharedLib`]  | exact file                                     |
//!
//! Modifiers: `optional`, `cond`, `lazy`, `type` (TYPE_ONLY), `build` (Build
//! phase).
//!
//! The argument is a `+`-separated concatenation of double-quoted strings (JS
//! escapes) and `?name` placeholders, which stand for unknown runtime values:
//! `dep "./locales/" + ?lang + ".json"` becomes the pattern `./locales/*.json`.
//! A string containing `*` is also a pattern. Unknown keywords are ignored;
//! malformed lines produce a `ParseIncomplete` diagnostic and are skipped.

use std::path::Path;

use turborepo_trace_core::{
    ExtractCx, FileId, LangId, Language, Phase, ProjectCx, RawRef, RefFlags, RefKind, RefSink,
    Resolution, ResolveCx, Resolver, Span, Specifier, Target, Uncertainty, UncertaintyKind,
    eval::Value,
    lexkit::{Cursor, EscapeRules, decode, skip},
    resolve_path::{PathResolver, PathResolverConfig, SpecKind, spec_kind},
};

/// The stub language. See the crate docs for the syntax.
#[derive(Copy, Clone, Debug, Default)]
pub struct StubLanguage;

impl Language for StubLanguage {
    fn id(&self) -> LangId {
        LangId("stub")
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["stub"]
    }

    fn sniff(&self, _path: &Path, head: &[u8]) -> bool {
        head.starts_with(b"#!stub")
    }

    fn extract<'s>(&self, src: &'s [u8], _cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
        extract(src, out);
    }

    fn resolver(&self, project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        let modules = project.options.base.join("stub_modules");
        Box::new(StubResolver {
            modules: modules.clone(),
            code: PathResolver::new(PathResolverConfig {
                roots: vec![modules],
                extensions: vec!["stub".into()],
                index_files: vec!["index.stub".into()],
                ..Default::default()
            }),
            files: PathResolver::new(PathResolverConfig::default()),
            dirs: PathResolver::new(PathResolverConfig {
                allow_dirs: true,
                ..Default::default()
            }),
        })
    }
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Extracts directives from `src`. Linear; never panics.
fn extract<'s>(src: &'s [u8], out: &mut RefSink<'s>) {
    let mut c = Cursor::new(src);
    while !c.is_eof() {
        c.skip_inline_whitespace();
        match c.peek() {
            None => break,
            Some(b'\n' | b'\r') => {
                c.advance(1);
                continue;
            }
            Some(b'#') => {
                skip::hash_comment(&mut c);
                continue;
            }
            _ => {}
        }
        let line_start = c.pos();
        let mut flags = RefFlags::empty();
        let mut phase = None;
        let kind = loop {
            let start = c.pos();
            c.skip_while(is_word);
            let word = &src[start..c.pos()];
            c.skip_inline_whitespace();
            match word {
                b"optional" => flags |= RefFlags::OPTIONAL,
                b"cond" => flags |= RefFlags::CONDITIONAL,
                b"lazy" => flags |= RefFlags::LAZY,
                b"type" => flags |= RefFlags::TYPE_ONLY,
                b"build" => phase = Some(Phase::Build),
                b"dep" => break Some(RefKind::Import),
                b"asset" => break Some(RefKind::FsRead),
                b"dir" => break Some(RefKind::FsReadDir),
                b"manifest" => break Some(RefKind::Manifest),
                b"lib" => break Some(RefKind::SharedLib),
                _ => break None,
            }
        };
        let Some(kind) = kind else {
            // Unknown directive: ignore the rest of the line.
            c.skip_until(b'\n');
            continue;
        };
        match parse_arg(&mut c) {
            Ok((spec, span)) => {
                let mut r = RawRef::new(kind, spec, span).with_flags(flags);
                if let Some(p) = phase {
                    r = r.with_phase(p);
                }
                r.uncertainty = match &r.spec {
                    Specifier::Unknown => Some(Uncertainty::new(
                        UncertaintyKind::ComputedSpecifier,
                        span,
                        format!(
                            "computed argument {}",
                            String::from_utf8_lossy(&src[span.range()])
                        ),
                    )),
                    Specifier::Pattern(_) => Some(Uncertainty::new(
                        UncertaintyKind::PartialSpecifier,
                        span,
                        format!("partially known argument {}", r.spec),
                    )),
                    _ => None,
                };
                out.push(r);
            }
            Err(msg) => {
                c.skip_until(b'\n');
                out.diagnostic(Uncertainty::new(
                    UncertaintyKind::ParseIncomplete,
                    c.span_from(line_start),
                    msg,
                ));
            }
        }
        // Anything after the argument on the same line is ignored.
        c.skip_until(b'\n');
    }
}

/// Parses `term ('+' term)*` where `term` is a string or `?name`.
fn parse_arg<'s>(c: &mut Cursor<'s>) -> Result<(Specifier<'s>, Span), String> {
    let src = c.src();
    let start = c.pos();
    let mut terms: Vec<Value> = Vec::new();
    let mut single_borrowed: Option<&'s str> = None;
    loop {
        c.skip_inline_whitespace();
        match c.peek() {
            Some(b'"') => {
                let lit = skip::quoted(c, skip::QuoteOpts::single_line(b'"'))
                    .ok_or("expected string")?
                    .map_err(|e| e.to_string())?;
                let text = decode(c.slice(lit.content), EscapeRules::JS)
                    .map_err(|e| format!("bad string: {e}"))?;
                if terms.is_empty()
                    && let std::borrow::Cow::Borrowed(s) = text
                {
                    single_borrowed = Some(s);
                }
                terms.push(glob_value(&text));
            }
            Some(b'?') => {
                let s = c.pos();
                c.advance(1);
                c.skip_while(is_word);
                let name = String::from_utf8_lossy(&src[s..c.pos()]).into_owned();
                terms.push(Value::unknown(name));
            }
            _ => return Err("expected a string or ?name argument".into()),
        }
        let end = c.pos();
        c.skip_inline_whitespace();
        if !c.eat(b'+') {
            c.set_pos(end);
            break;
        }
    }
    let span = c.span_from(start);
    let value = Value::concat_all(terms.iter().cloned());
    // Keep the zero-copy borrowed literal for the common single-string case.
    let spec = match (&value, single_borrowed, terms.len()) {
        (Value::Str(_), Some(s), 1) => Specifier::literal(s),
        _ => value.to_specifier(),
    };
    Ok((spec, span))
}

/// A string term; each `*` inside it is a wildcard.
fn glob_value(text: &str) -> Value {
    if !text.contains('*') {
        return Value::str(text);
    }
    let mut parts = Vec::new();
    for (i, seg) in text.split('*').enumerate() {
        if i > 0 {
            parts.push(Value::unknown("*"));
        }
        parts.push(Value::str(seg));
    }
    Value::concat_all(parts)
}

/// Resolver for the stub language.
struct StubResolver {
    modules: std::path::PathBuf,
    code: PathResolver,
    files: PathResolver,
    dirs: PathResolver,
}

impl StubResolver {
    fn resolve_spec(
        &self,
        r: &RawRef<'_>,
        spec: &Specifier<'_>,
        base: &Path,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        match spec {
            Specifier::Literal(s) => {
                let mut res = match r.kind {
                    RefKind::Import => self.code.resolve_to(s, base, cx, r.span),
                    RefKind::FsReadDir => self.dirs.resolve_to(s, base, cx, r.span),
                    _ => self.files.resolve_to(s, base, cx, r.span),
                };
                if r.kind == RefKind::Import && spec_kind(s) == SpecKind::Bare {
                    let pkg = s.split('/').next().unwrap_or(s);
                    let manifest = self.modules.join(pkg).join("stub.json");
                    if cx.fs.is_file(&manifest) {
                        res.manifests.push(cx.intern(&manifest));
                    }
                }
                res
            }
            Specifier::Pattern(parts) => {
                turborepo_trace_core::eval::resolve_pattern(parts, base, cx, r.span)
            }
            Specifier::Alternatives(alts) => {
                let mut out = Resolution::default();
                for a in alts {
                    let res = self.resolve_spec(r, a, base, cx);
                    out.targets.extend(
                        res.targets
                            .into_iter()
                            .filter(|t| !matches!(t, Target::Unresolved)),
                    );
                    out.manifests.extend(res.manifests);
                    out.uncertainty = out.uncertainty.or(res.uncertainty);
                }
                out.uncertainty.get_or_insert_with(|| {
                    Uncertainty::new(
                        UncertaintyKind::ConditionalResolution,
                        r.span,
                        format!("one of: {spec}"),
                    )
                });
                out
            }
            Specifier::Unknown => {
                Resolution::unresolved(r.uncertainty.clone().unwrap_or_else(|| {
                    Uncertainty::new(UncertaintyKind::ComputedSpecifier, r.span, "unknown")
                }))
            }
        }
    }
}

impl Resolver for StubResolver {
    fn resolve(&self, r: &RawRef<'_>, _from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        let Some(base) = cx.base_dir(&r.base) else {
            return Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::UnsupportedSyntax,
                r.span,
                "unsupported base",
            ));
        };
        self.resolve_spec(r, &r.spec, &base, cx)
    }
}

#[cfg(test)]
mod tests {
    use turborepo_trace_core::Part;

    use super::*;

    fn refs(src: &[u8]) -> (Vec<RawRef<'_>>, Vec<Uncertainty>) {
        let mut sink = RefSink::new();
        extract(src, &mut sink);
        sink.into_parts()
    }

    #[test]
    fn parses_directives() {
        let src = b"# c\ndep \"./a\"\n  optional build asset \"./b.txt\" trailing\nunknown \"x\"\n";
        let (r, d) = refs(src);
        assert!(d.is_empty());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].kind, RefKind::Import);
        assert_eq!(r[0].spec, Specifier::literal("./a"));
        assert!(matches!(
            r[0].spec,
            Specifier::Literal(std::borrow::Cow::Borrowed(_))
        ));
        assert_eq!(&src[r[0].span.range()], b"\"./a\"");
        assert_eq!(r[1].kind, RefKind::FsRead);
        assert_eq!(r[1].flags, RefFlags::OPTIONAL);
        assert_eq!(r[1].phase, Phase::Build);
    }

    #[test]
    fn concatenation_and_unknown() {
        let (r, _) = refs(b"dep \"./l/\" + ?lang + \".json\"\ndep ?x\nasset \"./d/*.txt\"\n");
        assert_eq!(r[0].spec.to_string(), "./l/*.json");
        assert_eq!(
            r[0].uncertainty.as_ref().map(|u| u.kind),
            Some(UncertaintyKind::PartialSpecifier)
        );
        assert_eq!(r[1].spec, Specifier::Unknown);
        assert_eq!(
            r[1].uncertainty.as_ref().map(|u| u.kind),
            Some(UncertaintyKind::ComputedSpecifier)
        );
        assert_eq!(r[2].spec.to_string(), "./d/*.txt");
        assert!(matches!(&r[2].spec, Specifier::Pattern(p) if matches!(p[1], Part::Wild { .. })));
    }

    #[test]
    fn escapes_and_errors() {
        let (r, d) = refs(b"asset \"./a\\u0020b\"\ndep \"unterminated\ndep 42\ndep \"./ok\"");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].spec, Specifier::literal("./a b"));
        assert_eq!(r[1].spec, Specifier::literal("./ok"));
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|u| u.kind == UncertaintyKind::ParseIncomplete));
    }

    #[test]
    fn comments_produce_nothing() {
        let (r, d) = refs(b"# dep \"./a\"\n   # asset \"./b\"\n");
        assert!(r.is_empty() && d.is_empty());
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        let mut seed: u32 = 0x1234_5678;
        for len in 0..300 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let pick = b"dep asset \"?+*#\n\\\r\xff ";
                    pick[(seed as usize) % pick.len()]
                })
                .collect();
            let (r, _) = refs(&bytes);
            for x in r {
                assert!(x.span.end as usize <= bytes.len());
                assert!(x.span.start <= x.span.end);
            }
        }
    }
}
