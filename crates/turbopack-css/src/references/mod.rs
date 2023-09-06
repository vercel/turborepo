use std::convert::Infallible;

use anyhow::Result;
use lightningcss::{
    rules::CssRule,
    stylesheet::StyleSheet,
    values::url::Url,
    visitor::{Visit, Visitor},
};
use swc_core::common::{source_map::Pos, Spanned};
use turbo_tasks::{Value, Vc};
use turbopack_core::{
    issue::{IssueSeverity, IssueSource},
    reference::{ModuleReference, ModuleReferences},
    reference_type::{CssReferenceSubType, ReferenceType},
    resolve::{
        handle_resolve_error,
        origin::{ResolveOrigin, ResolveOriginExt},
        parse::Request,
        ModuleResolveResult,
    },
    source::Source,
};

use crate::references::{
    import::{ImportAssetReference, ImportAttributes},
    url::UrlAssetReference,
};

pub(crate) mod compose;
pub(crate) mod import;
pub(crate) mod internal;
pub(crate) mod url;

/// Returns `(all_references, urls)`.
pub fn analyze_references(
    stylesheet: &mut StyleSheet<'static, 'static>,
    source: Vc<Box<dyn Source>>,
    origin: Vc<Box<dyn ResolveOrigin>>,
) -> Result<(
    Vec<Vc<Box<dyn ModuleReference>>>,
    Vec<(String, Vc<Box<dyn ModuleReference>>)>,
)> {
    let mut references = Vec::new();
    let mut urls = Vec::new();

    let mut visitor = ModuleReferencesVisitor::new(source, origin, &mut references, &mut urls);
    stylesheet.visit(&mut visitor);

    Ok((references, urls))
}

struct ModuleReferencesVisitor<'a> {
    source: Vc<Box<dyn Source>>,
    origin: Vc<Box<dyn ResolveOrigin>>,
    references: &'a mut Vec<Vc<Box<dyn ModuleReference>>>,
    urls: &'a mut Vec<(String, Vc<Box<dyn ModuleReference>>)>,
    is_import: bool,
}

impl<'a> ModuleReferencesVisitor<'a> {
    fn new(
        source: Vc<Box<dyn Source>>,
        origin: Vc<Box<dyn ResolveOrigin>>,
        references: &'a mut Vec<Vc<Box<dyn ModuleReference>>>,
        urls: &'a mut Vec<(String, Vc<Box<dyn ModuleReference>>)>,
    ) -> Self {
        Self {
            source,
            origin,
            references,
            urls,
            is_import: false,
        }
    }
}

impl<'a> Visitor<'_> for ModuleReferencesVisitor<'a> {
    type Error = Infallible;

    const TYPES: lightningcss::visitor::VisitTypes = lightningcss::visitor::VisitTypes::all();

    fn visit_rule(&mut self, rule: &mut CssRule) -> std::result::Result<(), Self::Error> {
        match rule {
            CssRule::Import(i) => {
                let src = &*i.url;

                let issue_span = i.loc;

                self.references.push(Vc::upcast(ImportAssetReference::new(
                    self.origin,
                    Request::parse(Value::new(src.to_string().into())),
                    ImportAttributes::new_from_prelude(i).into(),
                    IssueSource::from_byte_offset(
                        Vc::upcast(self.source),
                        issue_span.lo.to_usize(),
                        issue_span.hi.to_usize(),
                    ),
                )));

                self.is_import = true;
                let res = i.visit_children(self);
                self.is_import = false;
                res
            }

            _ => rule.visit_children(self),
        }
    }

    fn visit_url(&mut self, u: &mut Url) -> std::result::Result<(), Self::Error> {
        if self.is_import {
            return u.visit_children(self);
        }

        let src = &*u.url;

        // ignore internal urls like `url(#noiseFilter)`
        // ignore server-relative urls like `url(/foo)`
        if !matches!(src.bytes().next(), Some(b'#') | Some(b'/')) {
            let issue_span = u.loc;

            let vc = UrlAssetReference::new(
                self.origin,
                Request::parse(Value::new(src.to_string().into())),
                IssueSource::from_byte_offset(
                    Vc::upcast(self.source),
                    issue_span.lo.to_usize(),
                    issue_span.hi.to_usize(),
                ),
            );

            self.references.push(Vc::upcast(vc));
            self.urls.push((u.url.to_string(), Vc::upcast(vc)))
        }

        u.visit_children(self);

        Ok(())
    }
}

#[turbo_tasks::function]
pub async fn css_resolve(
    origin: Vc<Box<dyn ResolveOrigin>>,
    request: Vc<Request>,
    ty: Value<CssReferenceSubType>,
    issue_source: Option<Vc<IssueSource>>,
) -> Result<Vc<ModuleResolveResult>> {
    let ty = Value::new(ReferenceType::Css(ty.into_value()));
    let options = origin.resolve_options(ty.clone());
    let result = origin.resolve_asset(request, options, ty.clone());

    handle_resolve_error(
        result,
        ty,
        origin.origin_path(),
        request,
        options,
        IssueSeverity::Error.cell(),
        issue_source,
    )
    .await
}
