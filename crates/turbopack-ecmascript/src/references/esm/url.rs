use anyhow::{bail, Result};
use swc_core::{
    ecma::ast::{Expr, NewExpr},
    quote,
};
use turbo_tasks::{primitives::StringVc, ValueToString, ValueToStringVc};
use turbo_tasks_fs::{FileContent, FileSystemPathVc};
use turbo_tasks_hash::{encode_hex, hash_xxh3_hash64};
use turbopack_core::{
    asset::{Asset, AssetContent, AssetContentVc, AssetVc},
    chunk::{
        ChunkItem, ChunkItemVc, ChunkVc, ChunkableAsset, ChunkableAssetReference,
        ChunkableAssetReferenceVc, ChunkableAssetVc, ChunkingContextVc, ChunkingType,
        ChunkingTypeOptionVc,
    },
    reference::{AssetReference, AssetReferenceVc, AssetReferencesVc, SingleAssetReferenceVc},
    resolve::{
        pattern::{Pattern, PatternVc},
        ResolveResult, ResolveResultVc,
    },
    virtual_asset::VirtualAssetVc,
};

use crate::{
    chunk::{
        EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkItemContentVc,
        EcmascriptChunkItemVc, EcmascriptChunkPlaceable, EcmascriptChunkPlaceableVc,
        EcmascriptChunkVc, EcmascriptExports, EcmascriptExportsVc,
    },
    code_gen::{CodeGenerateable, CodeGenerateableVc, CodeGeneration, CodeGenerationVc},
    create_visitor,
    references::AstPathVc,
    utils::{module_id_to_lit, stringify_str},
};

/// URL Asset References are injected during code analysis when we find a
/// (staticly analyzable) `new URL("path", import.meta.url)`.
///
/// It's responsible for resolving a reference to InertUrlAsset (which isn't
/// itself useful), and rewriting the `URL` constructor's arguments to allow the
/// referenced file to be imported/fetched/etc.
#[turbo_tasks::value]
pub struct UrlAssetReference {
    pub source: AssetVc,
    pub pattern: PatternVc,
    pub ast_path: AstPathVc,
}

/// Inert URL Assets are used to have a EcmascriptChunkPlaceable impl, so that
/// we can generate a real UrlAssetChunk item (with an appropriate path). That's
/// it, the inert asset doesn't really do anything, besides act as a holder for
/// the referenced file path as we wait for a call to create a
/// EcmascriptChunkItemVc via EcmascriptChunkPlaceable trait.
#[turbo_tasks::value]
struct InertUrlAsset {
    source: FileSystemPathVc,
}

/// UrlAssetChunk is the real URL Asset. It generates a devserver-addressable
/// file path, links to a virtual file of the referenced URL's contents, and
/// generates a module exporting the file path.
///
/// This is differentiated from a regular StaticAsset/StaticModuleAsset because
/// the generated module's export is usable to construct a `new URL` in both
/// server and node environments.
#[turbo_tasks::value]
struct UrlAssetChunk {
    asset: AssetVc,
    context: ChunkingContextVc,
}

#[turbo_tasks::value_impl]
impl UrlAssetReferenceVc {
    #[turbo_tasks::function]
    pub fn new(source: AssetVc, pattern: PatternVc, ast_path: AstPathVc) -> Self {
        UrlAssetReference {
            source,
            pattern,
            ast_path,
        }
        .cell()
    }

    #[turbo_tasks::function]
    async fn inner_asset(self) -> Result<OptionInertUrlAssetVc> {
        let this = self.await?;
        Ok(OptionInertUrlAssetVc::cell(match &*this.pattern.await? {
            Pattern::Constant(path) => {
                let path = this.source.path().parent().join(path);
                Some(InertUrlAssetVc::new(path))
            }
            _ => None,
        }))
    }
}

#[turbo_tasks::value_impl]
impl AssetReference for UrlAssetReference {
    #[turbo_tasks::function]
    async fn resolve_reference(self_vc: UrlAssetReferenceVc) -> Result<ResolveResultVc> {
        let asset = self_vc.inner_asset().await?;
        Ok(match &*asset {
            Some(a) => ResolveResult::Single(a.as_asset(), vec![]).into(),
            None => ResolveResult::Unresolveable(vec![]).cell(),
        })
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for UrlAssetReference {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell(format!(
            "URL Reference {} -> {}",
            self.source.path().to_string().await?,
            self.pattern.await?,
        )))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAssetReference for UrlAssetReference {
    #[turbo_tasks::function]
    fn chunking_type(&self, _context: ChunkingContextVc) -> ChunkingTypeOptionVc {
        // This cannot be Placed, I think because a Placed chunk item will use
        // the Asset's references instead of the ChunkItems? I'm not
        // fully sure, but I couldn't get this to wor work.
        ChunkingTypeOptionVc::cell(Some(ChunkingType::Parallel))
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for UrlAssetReference {
    #[turbo_tasks::function]
    async fn code_generation(
        self_vc: UrlAssetReferenceVc,
        context: ChunkingContextVc,
    ) -> Result<CodeGenerationVc> {
        let this = self_vc.await?;
        let mut visitors = vec![];

        let inner_asset = self_vc.inner_asset().await?;

        if let Some(inner) = &*inner_asset {
            let chunk_item = inner.as_chunk_item(context);

            // We rewrite the first `new URL()` arguments to be a require() of the chunk
            // item, which exports the static asset path to the linked file.
            let id = chunk_item.id().await?;

            let ast_path = this.ast_path.await?;
            visitors.push(create_visitor!(ast_path, visit_mut_expr(expr: &mut Expr) {
                if let Expr::New(NewExpr { args: Some(args), .. }) = expr {
                    args[0].expr = box quote!(
                        "__turbopack_require__($id)" as Expr,
                        id: Expr = module_id_to_lit(&id),
                    );
                }
            }));
        }

        Ok(CodeGeneration { visitors }.into())
    }
}

#[turbo_tasks::value(transparent)]
struct OptionInertUrlAsset(Option<InertUrlAssetVc>);

#[turbo_tasks::value_impl]
impl InertUrlAssetVc {
    #[turbo_tasks::function]
    fn new(source: FileSystemPathVc) -> Self {
        InertUrlAsset { source }.cell()
    }
}

#[turbo_tasks::value_impl]
impl Asset for InertUrlAsset {
    #[turbo_tasks::function]
    fn path(&self) -> FileSystemPathVc {
        self.source
    }

    #[turbo_tasks::function]
    fn content(&self) -> AssetContentVc {
        self.source.read().into()
    }

    #[turbo_tasks::function]
    fn references(&self) -> AssetReferencesVc {
        AssetReferencesVc::empty()
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for InertUrlAsset {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell(format!(
            "Inert URL Asset {}",
            self.source.await?,
        )))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAsset for InertUrlAsset {
    #[turbo_tasks::function]
    fn as_chunk(self_vc: InertUrlAssetVc, context: ChunkingContextVc) -> ChunkVc {
        EcmascriptChunkVc::new(context, self_vc.as_ecmascript_chunk_placeable()).into()
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for InertUrlAsset {
    #[turbo_tasks::function]
    fn as_chunk_item(
        self_vc: InertUrlAssetVc,
        context: ChunkingContextVc,
    ) -> EcmascriptChunkItemVc {
        UrlAssetChunkVc::new(self_vc.into(), context).into()
    }

    #[turbo_tasks::function]
    fn get_exports(&self) -> EcmascriptExportsVc {
        EcmascriptExports::Value.into()
    }
}

#[turbo_tasks::value_impl]
impl UrlAssetChunkVc {
    #[turbo_tasks::function]
    fn new(asset: AssetVc, context: ChunkingContextVc) -> Self {
        UrlAssetChunk { asset, context }.cell()
    }

    #[turbo_tasks::function]
    async fn static_path(self) -> Result<FileSystemPathVc> {
        let this = self.await?;
        let content = this.asset.content();

        let AssetContent::File(file) = &*content.await? else {
            bail!("UrlAssetChunk::path: unsupported file content");
        };
        let FileContent::Content(file) = &*file.await? else {
            bail!("UrlAssetChunk::path: not found");
        };

        let content_hash = encode_hex(hash_xxh3_hash64(file.content()));
        let source_path = this.asset.path().await?;
        let ext = source_path.extension().unwrap_or("bin");
        Ok(this.context.asset_path(&content_hash, ext))
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for UrlAssetChunk {
    #[turbo_tasks::function]
    fn to_string(&self) -> StringVc {
        self.asset.path().join("url-asset.js").to_string()
    }
}

#[turbo_tasks::value_impl]
impl ChunkItem for UrlAssetChunk {
    #[turbo_tasks::function]
    async fn references(self_vc: UrlAssetChunkVc) -> Result<AssetReferencesVc> {
        let path = self_vc.static_path();
        let content = self_vc.await?.asset.content();
        // Generating this asset references allows the real file to be accessed.
        let asset_ref = SingleAssetReferenceVc::new(
            VirtualAssetVc::new(path, content).into(),
            StringVc::cell(format!("static(url) {}", path.await?)),
        );
        Ok(AssetReferencesVc::cell(vec![asset_ref.into()]))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for UrlAssetChunk {
    #[turbo_tasks::function]
    fn chunking_context(&self) -> ChunkingContextVc {
        self.context
    }

    #[turbo_tasks::function]
    async fn content(self_vc: UrlAssetChunkVc) -> Result<EcmascriptChunkItemContentVc> {
        // Exports the path to where the real file can be accessed.
        Ok(EcmascriptChunkItemContent {
            inner_code: format!(
                "__turbopack_export_value__({path});",
                path = stringify_str(&format!("/{}", &*self_vc.static_path().await?))
            )
            .into(),
            ..Default::default()
        }
        .into())
    }
}
