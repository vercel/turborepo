use anyhow::Result;
use lightningcss::{
    printer::Printer,
    rules::{
        import::ImportRule,
        layer::{LayerName, LayerStatementRule},
        media::MediaRule,
        supports::SupportsRule,
        unknown::UnknownAtRule,
        CssRule,
    },
    stylesheet::PrinterOptions,
    traits::ToCss,
};
use turbo_tasks::{Value, ValueToString, Vc};
use turbopack_core::{
    chunk::{ChunkableModuleReference, ChunkingContext},
    issue::IssueSource,
    reference::ModuleReference,
    reference_type::CssReferenceSubType,
    resolve::{origin::ResolveOrigin, parse::Request, ModuleResolveResult},
};

use crate::{
    chunk::CssImport,
    code_gen::{CodeGenerateable, CodeGeneration},
    references::{css_resolve, AstPath},
};

#[turbo_tasks::value(into = "new")]
pub struct ImportAttributes {
    #[turbo_tasks(trace_ignore)]
    pub layer_name: Option<LayerStatementRule<'static>>,
    #[turbo_tasks(trace_ignore)]
    pub supports: Option<SupportsRule<'static>>,
    #[turbo_tasks(trace_ignore)]
    pub media: Option<MediaRule<'static>>,
}

impl ImportAttributes {
    pub fn new_from_prelude(prelude: &ImportRule<'static>) -> Self {
        let layer_name = prelude
            .layer
            .as_ref()
            .map(|v| v.as_ref())
            .flatten()
            .and_then(|l| match l {
                LayerName::Ident(_) => LayerName(Default::default()),
                LayerName::Function(f) => {
                    assert_eq!(f.value.len(), 1);
                    assert!(matches!(&f.value[0], ComponentValue::LayerName(_)));
                    if let ComponentValue::LayerName(layer_name) = &f.value[0] {
                        *layer_name.clone()
                    } else {
                        unreachable!()
                    }
                }
            });

        let supports = prelude.supports.clone();

        let media = prelude.media.clone();

        Self {
            layer_name,
            supports,
            media,
        }
    }

    pub fn print_block(&self) -> Result<(String, String)> {
        // something random that's never gonna be in real css
        // Box::new(ListOfComponentValues {
        //     span: DUMMY_SP,
        //     children: vec![ComponentValue::PreservedToken(Box::new(token(
        //         Token::String {
        //             value: Default::default(),
        //             raw: r#""""__turbopack_placeholder__""""#.into(),
        //         },
        //     )))],
        // })
        let mut rule = CssRule::Unknown(UnknownAtRule {});

        if let Some(media) = &self.media {
            rule = CssRule::Media(media.clone())
        }

        if let Some(supports) = &self.supports {
            rule = CssRule::Supports(supports.clone())
        }
        if let Some(layer_name) = &self.layer_name {
            rule = CssRule::LayerStatement(layer_name.clone());
        }

        let mut output = String::new();
        let mut printer = Printer::new(&mut output, PrinterOptions::default());
        rule.to_css(&mut printer)?;

        let (open, close) = output
            .split_once(r#""""__turbopack_placeholder__""""#)
            .unwrap();

        Ok((open.trim().into(), close.trim().into()))
    }
}

#[turbo_tasks::value]
#[derive(Hash, Debug)]
pub struct ImportAssetReference {
    pub origin: Vc<Box<dyn ResolveOrigin>>,
    pub request: Vc<Request>,
    pub path: Vc<AstPath>,
    pub attributes: Vc<ImportAttributes>,
    pub issue_source: Vc<IssueSource>,
}

#[turbo_tasks::value_impl]
impl ImportAssetReference {
    #[turbo_tasks::function]
    pub fn new(
        origin: Vc<Box<dyn ResolveOrigin>>,
        request: Vc<Request>,
        path: Vc<AstPath>,
        attributes: Vc<ImportAttributes>,
        issue_source: Vc<IssueSource>,
    ) -> Vc<Self> {
        Self::cell(ImportAssetReference {
            origin,
            request,
            path,
            attributes,
            issue_source,
        })
    }
}

#[turbo_tasks::value_impl]
impl ModuleReference for ImportAssetReference {
    #[turbo_tasks::function]
    fn resolve_reference(&self) -> Vc<ModuleResolveResult> {
        css_resolve(
            self.origin,
            self.request,
            Value::new(CssReferenceSubType::AtImport),
            Some(self.issue_source),
        )
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for ImportAssetReference {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<Vc<String>> {
        Ok(Vc::cell(format!(
            "import(url) {}",
            self.request.to_string().await?,
        )))
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for ImportAssetReference {
    #[turbo_tasks::function]
    async fn code_generation(
        self: Vc<Self>,
        _context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<CodeGeneration>> {
        let this = &*self.await?;
        let mut imports = vec![];
        if let Request::Uri {
            protocol,
            remainder,
        } = &*this.request.await?
        {
            imports.push(CssImport::External(Vc::cell(format!(
                "{}{}",
                protocol, remainder
            ))))
        }

        Ok(CodeGeneration {
            visitors: vec![],
            imports,
        }
        .into())
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModuleReference for ImportAssetReference {}
