use anyhow::{bail, Result};
use swc_core::{
    ecma::ast::{Expr, NewExpr},
    quote,
};
use turbo_tasks::{primitives::StringVc, ValueToString, ValueToStringVc};
use turbo_tasks_fs::{FileContent, FileSystemPathVc};
use turbopack_core::{
    asset::{Asset, AssetContent, AssetContentVc, AssetOptionVc, AssetVc},
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

#[turbo_tasks::value]
pub struct UrlAssetReference {
    pub source: AssetVc,
    pub path: PatternVc,
    pub ast_path: AstPathVc,
}

#[turbo_tasks::value_impl]
impl UrlAssetReferenceVc {
    #[turbo_tasks::function]
    pub fn new(source: AssetVc, path: PatternVc, ast_path: AstPathVc) -> Self {
        UrlAssetReference {
            source,
            path,
            ast_path,
        }
        .cell()
    }

    #[turbo_tasks::function]
    async fn inner_asset(self) -> Result<AssetOptionVc> {
        let this = self.await?;
        Ok(AssetOptionVc::cell(match &*this.path.await? {
            Pattern::Constant(path) => {
                let path = this.source.path().parent().join(path);
                Some(UrlAssetVc::new(path).into())
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
            Some(a) => ResolveResult::Single(*a, vec![]).into(),
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
            self.path.await?,
        )))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAssetReference for UrlAssetReference {
    #[turbo_tasks::function]
    fn chunking_type(&self, _context: ChunkingContextVc) -> ChunkingTypeOptionVc {
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
            let Some(placeable) = EcmascriptChunkPlaceableVc::resolve_from(inner).await? else {
                bail!("failed to retrieve placeable from UrlAsset");
            };
            let chunk_item = placeable.as_chunk_item(context);
            let id = chunk_item.id().await?;

            let ast_path = &this.ast_path.await?;
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

#[turbo_tasks::value]
pub struct UrlAsset {
    source: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl UrlAssetVc {
    #[turbo_tasks::function]
    fn new(source: FileSystemPathVc) -> Self {
        UrlAsset { source }.cell()
    }
}

#[turbo_tasks::value_impl]
impl Asset for UrlAsset {
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
impl ValueToString for UrlAsset {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell(format!("URL Asset {}", self.source.await?,)))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAsset for UrlAsset {
    #[turbo_tasks::function]
    fn as_chunk(self_vc: UrlAssetVc, context: ChunkingContextVc) -> ChunkVc {
        EcmascriptChunkVc::new(context, self_vc.into()).into()
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for UrlAsset {
    #[turbo_tasks::function]
    fn as_chunk_item(self_vc: UrlAssetVc, context: ChunkingContextVc) -> EcmascriptChunkItemVc {
        UrlAssetChunkVc::new(self_vc.into(), context).into()
    }

    #[turbo_tasks::function]
    fn get_exports(&self) -> EcmascriptExportsVc {
        EcmascriptExports::Value.into()
    }
}

#[turbo_tasks::value]
struct UrlAssetChunk {
    asset: AssetVc,
    context: ChunkingContextVc,
}

#[turbo_tasks::value_impl]
impl UrlAssetChunkVc {
    #[turbo_tasks::function]
    fn new(asset: AssetVc, context: ChunkingContextVc) -> Self {
        UrlAssetChunk { asset, context }.cell()
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
impl Asset for UrlAssetChunk {
    #[turbo_tasks::function]
    async fn path(&self) -> Result<FileSystemPathVc> {
        let source_path = self.asset.path().await?;

        let content = self.asset.content();
        let content_hash = if let AssetContent::File(file) = &*content.await? {
            if let FileContent::Content(file) = &*file.await? {
                turbo_tasks_hash::hash_xxh3_hash64(file.content())
            } else {
                bail!("StaticAsset::path: not found");
            }
        } else {
            bail!("StaticAsset::path: unsupported file content");
        };
        let content_hash = turbo_tasks_hash::encode_hex(content_hash);

        let ext = source_path.extension().unwrap_or("bin");
        let asset_path = self.context.asset_path(&content_hash, ext);
        Ok(asset_path)
    }

    #[turbo_tasks::function]
    fn content(&self) -> AssetContentVc {
        self.asset.content()
    }

    #[turbo_tasks::function]
    fn references(&self) -> AssetReferencesVc {
        AssetReferencesVc::empty()
    }
}

#[turbo_tasks::value_impl]
impl ChunkItem for UrlAssetChunk {
    #[turbo_tasks::function]
    async fn references(self_vc: UrlAssetChunkVc) -> Result<AssetReferencesVc> {
        let path = self_vc.path();
        let asset_ref = SingleAssetReferenceVc::new(
            VirtualAssetVc::new(path, self_vc.as_asset().content()).into(),
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
        Ok(EcmascriptChunkItemContent {
            inner_code: format!(
                "__turbopack_export_value__({path});",
                path = stringify_str(&format!("/{}", &*self_vc.as_asset().path().await?))
            )
            .into(),
            ..Default::default()
        }
        .into())
    }
}
