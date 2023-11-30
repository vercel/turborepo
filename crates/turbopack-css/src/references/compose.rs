use anyhow::Result;
use turbo_tasks::{Value, ValueToString, Vc};
use turbopack_core::{
    chunk::ChunkableModuleReference,
    issue::IssueSeverity,
    reference::ModuleReference,
    reference_type::UrlReferenceSubType,
    resolve::{origin::ResolveOrigin, parse::Request, ModuleResolveResult},
};
use turbopack_ecmascript::resolve::url_resolve;

/// A `composes: ... from ...` CSS module reference.
#[turbo_tasks::value]
#[derive(Hash, Debug)]
pub struct CssModuleComposeReference {
    pub origin: Vc<Box<dyn ResolveOrigin>>,
    pub request: Vc<Request>,
}

#[turbo_tasks::value_impl]
impl CssModuleComposeReference {
    /// Creates a new [`CssModuleComposeReference`].
    #[turbo_tasks::function]
    pub fn new(origin: Vc<Box<dyn ResolveOrigin>>, request: Vc<Request>) -> Vc<Self> {
        Self::cell(CssModuleComposeReference { origin, request })
    }
}

#[turbo_tasks::value_impl]
impl ModuleReference for CssModuleComposeReference {
    #[turbo_tasks::function]
    fn resolve_reference(&self) -> Vc<ModuleResolveResult> {
        url_resolve(
            self.origin,
            self.request,
            Value::new(UrlReferenceSubType::CssUrl),
            // TODO: add real issue source, currently impossible because `CssClassName` doesn't
            // contain the source span
            // https://docs.rs/swc_css_modules/0.21.16/swc_css_modules/enum.CssClassName.html
            None,
            IssueSeverity::Error.cell(),
        )
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for CssModuleComposeReference {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<Vc<String>> {
        Ok(Vc::cell(format!(
            "compose(url) {}",
            self.request.to_string().await?,
        )))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModuleReference for CssModuleComposeReference {}
