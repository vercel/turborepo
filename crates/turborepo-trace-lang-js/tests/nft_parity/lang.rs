//! Adapters that express nft test options turbotrace has no flag for, without
//! touching the JS plugin: `exportsOnly` (via
//! [`JsResolveOptions::exports_only`]) and nft's `paths` aliases.
//!
//! Every case without those options uses [`JsLanguage`] unchanged, so the
//! harness measures exactly what the CLI does.

use std::path::Path;

use turborepo_trace_core::{
    ExtractCx, FileId, LangId, Language, ProjectCx, RawRef, RefKind, RefSink, Resolution,
    ResolveCx, Resolver, Specifier,
};
use turborepo_trace_lang_js::{
    JsLanguage,
    resolve::{JsResolveOptions, JsResolver},
};

/// [`JsLanguage`] with nft-only options.
pub struct NftJs {
    /// nft `exportsOnly`.
    pub exports_only: bool,
    /// nft `paths`: `(specifier or prefix ending in '/', absolute
    /// replacement)`.
    pub paths: Vec<(String, String)>,
}

impl Language for NftJs {
    fn id(&self) -> LangId {
        JsLanguage.id()
    }

    fn extensions(&self) -> &'static [&'static str] {
        JsLanguage.extensions()
    }

    fn sniff(&self, path: &Path, head: &[u8]) -> bool {
        JsLanguage.sniff(path, head)
    }

    fn extract<'s>(&self, src: &'s [u8], cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
        JsLanguage.extract(src, cx, out)
    }

    fn resolver(&self, project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        let inner: Box<dyn Resolver> = if self.exports_only {
            Box::new(JsResolver::new(
                project,
                &JsResolveOptions {
                    exports_only: true,
                    ..JsResolveOptions::default()
                },
            ))
        } else {
            JsLanguage.resolver(project)
        };
        if self.paths.is_empty() {
            inner
        } else {
            Box::new(PathsAlias {
                inner,
                paths: self.paths.clone(),
            })
        }
    }
}

/// Rewrites aliased module specifiers to absolute paths, then delegates.
struct PathsAlias {
    inner: Box<dyn Resolver>,
    paths: Vec<(String, String)>,
}

impl PathsAlias {
    fn rewrite(&self, spec: &str) -> Option<String> {
        self.paths.iter().find_map(|(from, to)| {
            if let Some(prefix) = from.strip_suffix('/') {
                spec.strip_prefix(prefix)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .map(|rest| format!("{to}{rest}"))
            } else {
                (spec == from).then(|| to.clone())
            }
        })
    }
}

impl Resolver for PathsAlias {
    fn resolve(&self, r: &RawRef<'_>, from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        let module = matches!(
            r.kind,
            RefKind::Import | RefKind::Require | RefKind::DynamicImport | RefKind::ReExport
        );
        if module && let Some(to) = r.spec.as_literal().and_then(|s| self.rewrite(s)) {
            let mut aliased = r.clone();
            aliased.spec = Specifier::literal(to);
            return self.inner.resolve(&aliased, from, cx);
        }
        self.inner.resolve(r, from, cx)
    }

    fn clear_cache(&self) {
        self.inner.clear_cache()
    }
}
