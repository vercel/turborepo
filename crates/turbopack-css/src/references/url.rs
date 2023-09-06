use std::{collections::HashMap, convert::Infallible};

use anyhow::Result;
use lightningcss::{
    stylesheet::StyleSheet,
    values::url::Url,
    visit_types,
    visitor::{Visit, Visitor},
};
use turbo_tasks::{Value, ValueToString, Vc};
use turbopack_core::{
    chunk::ChunkingContext,
    ident::AssetIdent,
    issue::{IssueSeverity, IssueSource},
    output::OutputAsset,
    reference::ModuleReference,
    reference_type::UrlReferenceSubType,
    resolve::{origin::ResolveOrigin, parse::Request, ModuleResolveResult},
};
use turbopack_ecmascript::resolve::url_resolve;

use crate::{
    code_gen::{CodeGenerateable, CodeGeneration},
    embed::{CssEmbed, CssEmbeddable},
    references::AstPath,
};

#[turbo_tasks::value(into = "new")]
pub enum ReferencedAsset {
    Some(Vc<Box<dyn OutputAsset>>),
    None,
}

#[turbo_tasks::value]
#[derive(Hash, Debug)]
pub struct UrlAssetReference {
    pub origin: Vc<Box<dyn ResolveOrigin>>,
    pub request: Vc<Request>,
    pub path: Vc<AstPath>,
    pub issue_source: Vc<IssueSource>,
}

#[turbo_tasks::value_impl]
impl UrlAssetReference {
    #[turbo_tasks::function]
    pub fn new(
        origin: Vc<Box<dyn ResolveOrigin>>,
        request: Vc<Request>,
        path: Vc<AstPath>,
        issue_source: Vc<IssueSource>,
    ) -> Vc<Self> {
        Self::cell(UrlAssetReference {
            origin,
            request,
            path,
            issue_source,
        })
    }

    #[turbo_tasks::function]
    async fn get_referenced_asset(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<ReferencedAsset>> {
        for &module in self.resolve_reference().primary_modules().await?.iter() {
            if let Some(embeddable) =
                Vc::try_resolve_sidecast::<Box<dyn CssEmbeddable>>(module).await?
            {
                return Ok(ReferencedAsset::Some(
                    embeddable.as_css_embed(chunking_context).embeddable_asset(),
                )
                .into());
            }
        }
        Ok(ReferencedAsset::cell(ReferencedAsset::None))
    }
}

#[turbo_tasks::value_impl]
impl ModuleReference for UrlAssetReference {
    #[turbo_tasks::function]
    fn resolve_reference(&self) -> Vc<ModuleResolveResult> {
        url_resolve(
            self.origin,
            self.request,
            Value::new(UrlReferenceSubType::CssUrl),
            self.issue_source,
            IssueSeverity::Error.cell(),
        )
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for UrlAssetReference {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<Vc<String>> {
        Ok(Vc::cell(
            format!("url {}", self.request.to_string().await?,),
        ))
    }
}

#[turbo_tasks::function]
pub async fn resolve_url_reference(
    url: Vc<UrlAssetReference>,
    chunking_context: Vc<Box<dyn ChunkingContext>>,
) -> Result<Vc<Option<String>>> {
    let this = url.await?;
    // TODO(WEB-662) This is not the correct way to get the current chunk path. It
    // currently works as all chunks are in the same directory.
    let chunk_path = chunking_context.chunk_path(
        AssetIdent::from_path(this.origin.origin_path()),
        ".css".to_string(),
    );
    let context_path = chunk_path.parent().await?;

    if let ReferencedAsset::Some(asset) = &*url.get_referenced_asset(chunking_context).await? {
        // TODO(WEB-662) This is not the correct way to get the path of the asset.
        // `asset` is on module-level, but we need the output-level asset instead.
        let path = asset.ident().path().await?;
        let relative_path = context_path
            .get_relative_path_to(&path)
            .unwrap_or_else(|| format!("/{}", path.path));

        return Ok(Vc::cell(Some(relative_path)));
    }

    Ok(Vc::cell(None))
}

pub fn replace_asset_references(
    ss: &mut StyleSheet<'static, 'static>,
    urls: &HashMap<String, String>,
) {
    let mut replacer = AssetReferenceReplacer { urls };
    ss.visit(&mut replacer);
}

struct AssetReferenceReplacer<'a> {
    urls: &'a HashMap<String, String>,
}

impl<'i> Visitor<'i> for AssetReferenceReplacer<'_> {
    type Error = Infallible;

    const TYPES: lightningcss::visitor::VisitTypes = visit_types!(URLS);

    fn visit_url(&mut self, u: &mut Url) -> std::result::Result<(), Self::Error> {
        u.visit_children(self);

        if let Some(new) = self.urls.get(&*u.url) {
            u.url = new.clone().into();
        }

        Ok(())
    }
}
