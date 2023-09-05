use std::convert::Infallible;

use anyhow::Result;
use lightningcss::{rules::CssRule, stylesheet::StyleSheet, values::url::Url, visitor::Visitor};
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

use crate::{
    process::{process_css, ProcessCssResult},
    references::{
        import::{ImportAssetReference, ImportAttributes},
        url::UrlAssetReference,
    },
    CssModuleAssetType,
};

pub(crate) mod compose;
pub(crate) mod import;
pub(crate) mod internal;
pub(crate) mod url;

#[turbo_tasks::function]
pub async fn analyze_references(
    stylesheet: &mut StyleSheet<'static, 'static>,
    source: Vc<Box<dyn Source>>,
    origin: Vc<Box<dyn ResolveOrigin>>,
) -> Result<Vc<ModuleReferences>> {
    let mut references = Vec::new();

    let mut visitor = ModuleReferencesVisitor::new(source, origin, &mut references);
    stylesheet.visit(&mut visitor);

    Ok(ModuleReferences::new(references))
}

struct ModuleReferencesVisitor<'a> {
    source: Vc<Box<dyn Source>>,
    origin: Vc<Box<dyn ResolveOrigin>>,
    references: &'a mut Vec<Vc<Box<dyn ModuleReference>>>,
    is_import: bool,
}

impl<'a> ModuleReferencesVisitor<'a> {
    fn new(
        source: Vc<Box<dyn Source>>,
        origin: Vc<Box<dyn ResolveOrigin>>,
        references: &'a mut Vec<Vc<Box<dyn ModuleReference>>>,
    ) -> Self {
        Self {
            source,
            origin,
            references,
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

                let issue_span = i.href.span();

                self.references.push(Vc::upcast(ImportAssetReference::new(
                    self.origin,
                    Request::parse(Value::new(src.to_string().into())),
                    Vc::cell(ast_path),
                    ImportAttributes::new_from_prelude(i).into(),
                    IssueSource::from_byte_offset(
                        Vc::upcast(self.source),
                        issue_span.lo.to_usize(),
                        issue_span.hi.to_usize(),
                    ),
                )));

                self.is_import = true;
                let res = i.visit_children_with_path(self, ast_path);
                self.is_import = false;
                res
            }

            _ => rule.visit_children_with(self),
        }
    }

    fn visit_url(&mut self, u: &Url, ast_path: &mut AstKindPath) {
        if self.is_import {
            return u.visit_children_with_path(self, ast_path);
        }

        let src = &*u.url;

        // ignore internal urls like `url(#noiseFilter)`
        // ignore server-relative urls like `url(/foo)`
        if !matches!(src.bytes().next(), Some(b'#') | Some(b'/')) {
            let issue_span = u.span;
            self.references.push(Vc::upcast(UrlAssetReference::new(
                self.origin,
                Request::parse(Value::new(src.to_string().into())),
                Vc::cell(ast_path),
                IssueSource::from_byte_offset(
                    Vc::upcast(self.source),
                    issue_span.lo.to_usize(),
                    issue_span.hi.to_usize(),
                ),
            )));
        }

        u.visit_children_with_path(self, ast_path);
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

// TODO enable serialization
#[turbo_tasks::value(transparent, serialization = "none")]
pub struct AstPath(#[turbo_tasks(trace_ignore)] Vec<AstParentKind>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstParentKind {}

pub type AstKindPath = swc_core::common::pass::AstKindPath<AstParentKind>;

impl swc_core::common::pass::ParentKind for AstParentKind {
    fn set_index(&mut self, _: usize) {}
}
