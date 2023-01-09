use std::io::Write;

use anyhow::Result;
use turbo_tasks::primitives::StringsVc;
use turbo_tasks_fs::{rope::RopeBuilder, File, FileSystemPathVc};
use turbopack::evaluate_context::node_evaluate_asset_context;
use turbopack_core::{
    asset::{Asset, AssetContentVc, AssetVc},
    chunk::{dev::DevChunkingContextVc, ChunkGroupVc, ChunkReferenceVc, ChunksVc},
    reference::AssetReferencesVc,
    resolve::{
        find_context_file,
        options::{ImportMap, ImportMapping},
        FindContextFileResult,
    },
    source_asset::SourceAssetVc,
    virtual_asset::VirtualAssetVc,
};
use turbopack_dev_server::source::{asset_graph::AssetGraphContentSourceVc, ContentSourceVc};
use turbopack_ecmascript::{
    EcmascriptInputTransform, EcmascriptInputTransformsVc, EcmascriptModuleAssetType,
    EcmascriptModuleAssetVc,
};

#[turbo_tasks::function]
pub async fn create_middleware_loader(
    project_root: FileSystemPathVc,
    server_root: FileSystemPathVc,
    output_root: FileSystemPathVc,
) -> Result<ContentSourceVc> {
    let middleware_asset_result = find_context_file(
        project_root,
        StringsVc::cell(
            [
                "middleware.mts",
                "middleware.ts",
                "middleware.mjs",
                "middleware.js",
            ]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
        ),
    );
    let middleware_asset = match &*middleware_asset_result.await? {
        FindContextFileResult::Found(config_path, _) => Some(SourceAssetVc::new(*config_path)),
        FindContextFileResult::NotFound(_) => None,
    };
    let asset = MiddlewareAsset {
        project_root,
        server_root,
        entry: middleware_asset,
        output_root,
    }
    .cell();

    Ok(AssetGraphContentSourceVc::new_lazy(project_root, asset.into()).into())
}

#[turbo_tasks::value(shared)]
pub struct MiddlewareAsset {
    pub project_root: FileSystemPathVc,
    pub server_root: FileSystemPathVc,
    pub output_root: FileSystemPathVc,
    pub entry: Option<SourceAssetVc>,
}

#[turbo_tasks::value_impl]
impl MiddlewareAssetVc {
    #[turbo_tasks::function]
    async fn get_middleware_entry_asset(self) -> Result<AssetVc> {
        let this = &*self.await?;

        let mut result = RopeBuilder::default();
        if this.entry.is_none() {
            // https://github.com/vercel/next.js/blob/v13.1.2-canary.3/packages/next/src/build/webpack/loaders/next-middleware-loader.ts#L42
            writeln!(
                result,
                r#"import {{ adapter }} from 'next/dist/esm/server/web/adapter'
                if (typeof handler !== 'function') {{
                  throw new Error('The Middleware "./middleware" must export a \`middleware\` or a \`default\` function');
                }}
                export default function (opts) {{
                  return adapter({{
                      ...opts,
                      page: "./src/middleware",
                      handler: (req) => req,
                  }})
                }}"#,
            )?;
        } else {
            // https://github.com/vercel/next.js/blob/v13.1.2-canary.3/packages/next/src/build/webpack/loaders/next-middleware-loader.ts#L42
            writeln!(
                result,
                r#"import {{ adapter, enhanceGlobals }} from 'next/dist/esm/server/web/adapter'
                enhanceGlobals()
                var mod = require("../middleware")
                var handler = mod.middleware || mod.default;
                if (typeof handler !== 'function') {{
                  throw new Error('The Middleware "./middleware" must export a \`middleware\` or a \`default\` function');
                }}
                global._ENTRIES['middleware'] = function (opts) {{
                  return adapter({{
                      ...opts,
                      page: "./src/middleware",
                      handler,
                  }})
                }}"#,
            )?;
        }

        let file = File::from(result.build());

        Ok(VirtualAssetVc::new(this.server_root.join("middleware-loader.ts"), file.into()).into())
    }

    #[turbo_tasks::function]
    async fn get_middleware_chunks(self) -> Result<ChunksVc> {
        let this = &*self.await?;

        let loader_entry_asset = self.get_middleware_entry_asset();

        let mut import_map = ImportMap::default();

        import_map.insert_exact_alias("next", ImportMapping::External(None).into());
        import_map.insert_wildcard_alias("next/", ImportMapping::External(None).into());

        let context = node_evaluate_asset_context(Some(import_map.cell()));
        let asset = EcmascriptModuleAssetVc::new(
            loader_entry_asset,
            context,
            turbo_tasks::Value::new(EcmascriptModuleAssetType::Typescript),
            EcmascriptInputTransformsVc::cell(vec![EcmascriptInputTransform::TypeScript]),
            context.environment(),
        );
        let intermediate_output_path = this.output_root;
        let chunking_context = DevChunkingContextVc::builder(
            this.project_root,
            intermediate_output_path,
            intermediate_output_path.join("chunks"),
            intermediate_output_path.join("assets"),
        )
        .build();
        let chunk_group =
            ChunkGroupVc::from_chunk(asset.as_evaluated_chunk(chunking_context, None));

        Ok(chunk_group.chunks())
    }
}

#[turbo_tasks::value_impl]
impl Asset for MiddlewareAsset {
    #[turbo_tasks::function]
    async fn path(&self) -> Result<FileSystemPathVc> {
        Ok(self.server_root.join(".next/server/src/middleware.js"))
    }

    #[turbo_tasks::function]
    async fn content(self_vc: MiddlewareAssetVc) -> Result<AssetContentVc> {
        use std::fmt::Write;
        let chunks = self_vc.get_middleware_chunks().await?;

        let mut output = String::new();
        for chunk in chunks.iter() {
            let path = chunk.path().await?;
            write!(&mut output, "__turbopack_require__({});\n", &path.path)?;
        }

        Ok(AssetContentVc::from(File::from(output)))
    }

    #[turbo_tasks::function]
    async fn references(self_vc: MiddlewareAssetVc) -> Result<AssetReferencesVc> {
        let chunks = self_vc.get_middleware_chunks().await?;

        let mut references = Vec::with_capacity(chunks.len());
        for chunk in chunks.iter() {
            references.push(ChunkReferenceVc::new(*chunk).into());
        }

        Ok(AssetReferencesVc::cell(references))
    }
}
