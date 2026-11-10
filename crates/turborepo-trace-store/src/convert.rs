//! Conversions between `turborepo_trace_core` values and shard records.

use std::{borrow::Cow, collections::HashSet, path::Path, sync::Mutex};

use turborepo_trace_core::{
    Base, CachedExtraction, LangId, Part, Phase, RawRef, RefFlags, RefKind, Span, Specifier,
    Tracer, Uncertainty, UncertaintyKind,
};
use turborepo_trace_graph as q;

use crate::shard::{
    ArchivedExtractionRec, ArchivedSpecTok, ArchivedUncRec, BaseRec, ExtractionRec, RawRefRec,
    SpecTok, UncRec,
};

/// Every uncertainty kind, in a fixed order (the index stored in shards).
pub const UNCERTAINTY_KINDS: [UncertaintyKind; 16] = [
    UncertaintyKind::ComputedSpecifier,
    UncertaintyKind::PartialSpecifier,
    UncertaintyKind::PatternTooBroad,
    UncertaintyKind::UnresolvedSpecifier,
    UncertaintyKind::AliasedLoader,
    UncertaintyKind::EvalLike,
    UncertaintyKind::EnvDependent,
    UncertaintyKind::CwdDependent,
    UncertaintyKind::ConditionalResolution,
    UncertaintyKind::GeneratedFile,
    UncertaintyKind::CaseMismatch,
    UncertaintyKind::ParseIncomplete,
    UncertaintyKind::UnsupportedSyntax,
    UncertaintyKind::OutsideBase,
    UncertaintyKind::UnknownCommand,
    UncertaintyKind::ShellUnsupported,
];

// Invariant: the table lists every kind (unit-tested).
#[allow(clippy::expect_used)]
fn unc_index(k: UncertaintyKind) -> u8 {
    UNCERTAINTY_KINDS
        .iter()
        .position(|x| *x == k)
        .expect("listed kind") as u8
}

// Invariant: the table lists every kind (unit-tested).
#[allow(clippy::expect_used)]
fn kind_index(k: RefKind) -> u8 {
    RefKind::ALL
        .iter()
        .position(|x| *x == k)
        .expect("listed kind") as u8
}

/// The query-side uncertainty kind of a stored index.
// Invariant: the graph crate's vocabulary has the same names as core's (unit-tested).
#[allow(clippy::expect_used)]
pub fn query_unc_kind(i: u8) -> q::UncertaintyKind {
    let k = UNCERTAINTY_KINDS
        .get(i as usize)
        .copied()
        .unwrap_or(UncertaintyKind::UnsupportedSyntax);
    q::UncertaintyKind::parse(k.as_str()).expect("same vocabulary")
}

/// The packed query attribute of a core edge.
// Invariant: the graph crate's vocabulary has the same names as core's (unit-tested).
#[allow(clippy::expect_used)]
pub fn edge_attr(kind: RefKind, phase: Phase, flags: RefFlags) -> u16 {
    let k = q::RefKind::parse(kind.as_str()).expect("same vocabulary");
    let p = match phase {
        Phase::Runtime => q::Phase::Runtime,
        Phase::Build => q::Phase::Build,
    };
    // Bits 0..5 are shared; core `NAMES` is not carried over, core `LOCATED` has
    // its own query bit (bits 6 and 7 of the query set are the store's marks).
    let mut f = q::RefFlags::from_bits_truncate(flags.bits() as u8 & 0x1f);
    if flags.contains(RefFlags::LOCATED) {
        f |= q::RefFlags::LOCATED;
    }
    let attr = q::EdgeAttr::new(k, p, f);
    if flags.contains(RefFlags::CONFIG) {
        attr.config().0
    } else {
        attr.0
    }
}

/// An uncertainty record with a line.
pub fn unc_rec(u: &Uncertainty, line: u32) -> UncRec {
    UncRec {
        kind: unc_index(u.kind),
        line,
        span: [u.span.start, u.span.end],
        detail: u.detail.to_string(),
        origin: origin_index(u.origin),
    }
}

// Invariant: the table lists every origin (unit-tested).
#[allow(clippy::expect_used)]
fn origin_index(o: turborepo_trace_core::Origin) -> u8 {
    turborepo_trace_core::Origin::ALL
        .iter()
        .position(|x| *x == o)
        .expect("listed origin") as u8
}

/// The query-side origin of a stored index.
// Invariant: the graph crate's vocabulary has the same names as core's (unit-tested).
#[allow(clippy::expect_used)]
pub fn query_origin(i: u8) -> q::Origin {
    let o = turborepo_trace_core::Origin::ALL
        .get(i as usize)
        .copied()
        .unwrap_or_default();
    q::Origin::parse(o.as_str()).expect("same vocabulary")
}

/// The stored effect ([`crate::shard::FileUncRec::effect`]) of a reference
/// kind.
// Invariant: the graph crate's vocabulary has the same names as core's (unit-tested).
#[allow(clippy::expect_used)]
pub fn effect_of(kind: RefKind) -> u8 {
    let k = q::RefKind::parse(kind.as_str()).expect("same vocabulary");
    match q::Effect::of(k) {
        q::Effect::None => crate::shard::EFFECT_NONE,
        q::Effect::Load => crate::shard::EFFECT_LOAD,
        q::Effect::Read => crate::shard::EFFECT_READ,
    }
}

/// The query-side effect of a stored value.
pub fn query_effect(e: u8) -> q::Effect {
    match e {
        crate::shard::EFFECT_LOAD => q::Effect::Load,
        crate::shard::EFFECT_READ => q::Effect::Read,
        _ => q::Effect::None,
    }
}

fn unc_from(r: &ArchivedUncRec) -> Uncertainty {
    let kind = UNCERTAINTY_KINDS
        .get(r.kind as usize)
        .copied()
        .unwrap_or(UncertaintyKind::UnsupportedSyntax);
    Uncertainty::new(
        kind,
        Span::new(r.span[0].to_native(), r.span[1].to_native()),
        r.detail.as_str(),
    )
    .with_origin(
        turborepo_trace_core::Origin::ALL
            .get(r.origin as usize)
            .copied()
            .unwrap_or_default(),
    )
}

/// Serializes an extraction for the cache.
pub fn extraction_rec(ex: &CachedExtraction) -> ExtractionRec {
    let refs = ex
        .refs
        .iter()
        .zip(&ex.ref_lines)
        .map(|(r, &line)| {
            let mut spec = Vec::new();
            spec_toks(&r.spec, &mut spec);
            RawRefRec {
                kind: kind_index(r.kind),
                spec,
                base: match &r.base {
                    Base::FileDir => BaseRec::FileDir,
                    Base::Cwd => BaseRec::Cwd,
                    Base::ProjectRoot => BaseRec::ProjectRoot,
                    // Resolved by the caller through `Tracer::path_of`; see `extraction_rec_in`.
                    Base::Dir(_) => BaseRec::Dir(String::new()),
                    Base::Language(s) => BaseRec::Language((*s).to_string()),
                },
                span: [r.span.start, r.span.end],
                flags: r.flags.bits(),
                phase: match r.phase {
                    Phase::Runtime => 0,
                    Phase::Build => 1,
                },
                uncertainty: r.uncertainty.as_ref().map(|u| unc_rec(u, line)),
                line,
            }
        })
        .collect();
    ExtractionRec {
        lang: ex.lang.0.to_string(),
        refs,
        diagnostics: ex
            .diagnostics
            .iter()
            .zip(&ex.diagnostic_lines)
            .map(|(d, &l)| unc_rec(d, l))
            .collect(),
        facts: ex.facts.clone(),
    }
}

/// [`extraction_rec`] with `Base::Dir` anchors written as paths.
pub fn extraction_rec_in(ex: &CachedExtraction, tracer: &Tracer) -> ExtractionRec {
    let mut rec = extraction_rec(ex);
    for (r, raw) in rec.refs.iter_mut().zip(&ex.refs) {
        if let Base::Dir(id) = raw.base {
            r.base = BaseRec::Dir(tracer.path_of(id).to_string_lossy().into_owned());
        }
    }
    rec
}

fn spec_toks(s: &Specifier<'_>, out: &mut Vec<SpecTok>) {
    match s {
        Specifier::Literal(l) => out.push(SpecTok::Literal(l.to_string())),
        Specifier::Pattern(parts) => {
            out.push(SpecTok::Pattern(parts.len() as u32));
            for p in parts {
                out.push(match p {
                    Part::Lit(l) => SpecTok::Lit(l.to_string()),
                    Part::Wild { reason } => SpecTok::Wild(reason.to_string()),
                });
            }
        }
        Specifier::Alternatives(alts) => {
            out.push(SpecTok::Alt(alts.len() as u32));
            for a in alts {
                spec_toks(a, out);
            }
        }
        Specifier::Unknown => out.push(SpecTok::Unknown),
    }
}

fn spec_from(toks: &[ArchivedSpecTok], pos: &mut usize) -> Specifier<'static> {
    let Some(t) = toks.get(*pos) else {
        return Specifier::Unknown;
    };
    *pos += 1;
    match t {
        ArchivedSpecTok::Literal(l) => Specifier::Literal(Cow::Owned(l.as_str().to_owned())),
        ArchivedSpecTok::Pattern(n) => {
            let mut parts = Vec::with_capacity(n.to_native() as usize);
            for _ in 0..n.to_native() {
                match toks.get(*pos) {
                    Some(ArchivedSpecTok::Lit(l)) => {
                        parts.push(Part::Lit(Cow::Owned(l.to_string())))
                    }
                    Some(ArchivedSpecTok::Wild(r)) => parts.push(Part::Wild {
                        reason: Cow::Owned(r.to_string()),
                    }),
                    _ => break,
                }
                *pos += 1;
            }
            Specifier::Pattern(parts)
        }
        ArchivedSpecTok::Alt(n) => {
            Specifier::Alternatives((0..n.to_native()).map(|_| spec_from(toks, pos)).collect())
        }
        ArchivedSpecTok::Unknown | ArchivedSpecTok::Lit(_) | ArchivedSpecTok::Wild(_) => {
            Specifier::Unknown
        }
    }
}

/// `&'static str` for language-defined base anchors, interned once per distinct
/// string.
fn static_str(s: &str) -> &'static str {
    static SET: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut g = SET.lock().unwrap_or_else(|e| e.into_inner());
    let set = g.get_or_insert_with(HashSet::new);
    if let Some(x) = set.get(s) {
        return x;
    }
    let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
    set.insert(leaked);
    leaked
}

/// Rebuilds a cached extraction for [`Tracer::analyze`]. `None` if the language
/// is not compiled in.
pub fn cached_extraction(rec: &ArchivedExtractionRec, tracer: &Tracer) -> Option<CachedExtraction> {
    let lang = tracer
        .languages()
        .iter()
        .map(|l| l.id())
        .find(|l| l.0 == rec.lang.as_str())?;
    let mut refs = Vec::with_capacity(rec.refs.len());
    let mut ref_lines = Vec::with_capacity(rec.refs.len());
    for r in rec.refs.iter() {
        let kind = *RefKind::ALL.get(r.kind as usize)?;
        let mut pos = 0;
        let spec = spec_from(&r.spec, &mut pos);
        let base = match &r.base {
            crate::shard::ArchivedBaseRec::FileDir => Base::FileDir,
            crate::shard::ArchivedBaseRec::Cwd => Base::Cwd,
            crate::shard::ArchivedBaseRec::ProjectRoot => Base::ProjectRoot,
            crate::shard::ArchivedBaseRec::Dir(p) => {
                Base::Dir(tracer.intern(Path::new(p.as_str())))
            }
            crate::shard::ArchivedBaseRec::Language(s) => Base::Language(static_str(s.as_str())),
        };
        refs.push(RawRef {
            kind,
            spec,
            base,
            span: Span::new(r.span[0].to_native(), r.span[1].to_native()),
            flags: RefFlags::from_bits_truncate(r.flags.to_native()),
            phase: if r.phase == 1 {
                Phase::Build
            } else {
                Phase::Runtime
            },
            uncertainty: r.uncertainty.as_ref().map(unc_from),
        });
        ref_lines.push(r.line.to_native());
    }
    let diagnostics = rec.diagnostics.iter().map(unc_from).collect();
    let diagnostic_lines = rec.diagnostics.iter().map(|d| d.line.to_native()).collect();
    Some(CachedExtraction {
        lang: LangId(lang.0),
        refs,
        ref_lines,
        diagnostics,
        diagnostic_lines,
        facts: rec.facts.iter().map(|f| f.as_str().to_owned()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabularies_match_query() {
        assert_eq!(UNCERTAINTY_KINDS.len(), q::UncertaintyKind::ALL.len());
        for (i, k) in UNCERTAINTY_KINDS.iter().enumerate() {
            assert_eq!(query_unc_kind(i as u8).as_str(), k.as_str());
        }
        for k in RefKind::ALL {
            assert!(q::RefKind::parse(k.as_str()).is_some(), "{k}");
        }
        let a = q::EdgeAttr(edge_attr(
            RefKind::Require,
            Phase::Runtime,
            RefFlags::OPTIONAL | RefFlags::TYPE_ONLY,
        ));
        assert_eq!(a.kind(), q::RefKind::Require);
        assert!(a.flags().contains(q::RefFlags::TYPE_ONLY));
    }

    #[test]
    fn extraction_roundtrip() {
        let ex = CachedExtraction {
            lang: LangId("js"),
            refs: vec![
                RawRef::new(
                    RefKind::Import,
                    Specifier::Alternatives(vec![
                        Specifier::literal("./a"),
                        Specifier::Pattern(vec![
                            Part::Lit("./l/".into()),
                            Part::Wild { reason: "x".into() },
                        ]),
                        Specifier::Unknown,
                    ]),
                    Span::new(3, 9),
                )
                .with_flags(RefFlags::LAZY)
                .with_base(Base::Language("crate_root")),
            ],
            ref_lines: vec![2],
            diagnostics: vec![Uncertainty::new(
                UncertaintyKind::ParseIncomplete,
                Span::new(0, 1),
                "x",
            )],
            diagnostic_lines: vec![1],
            facts: vec!["env:{}".to_owned()],
        };
        let rec = extraction_rec(&ex);
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&rec).unwrap();
        let archived = rkyv::access::<ArchivedExtractionRec, rkyv::rancor::Error>(&bytes).unwrap();
        let tracer = Tracer::builder()
            .language(turborepo_trace_lang_js::JsLanguage)
            .build()
            .unwrap();
        assert_eq!(cached_extraction(archived, &tracer).unwrap(), ex);
    }
}
