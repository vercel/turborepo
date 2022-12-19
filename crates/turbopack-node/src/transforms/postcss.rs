use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use turbo_tasks::{primitives::JsonValueVc, TryJoinIterExt, Value};
use turbo_tasks_fs::{rope::Rope, File, FileContent, FileSystemEntryType, FileSystemPathVc};
use turbopack_core::{
    asset::{Asset, AssetContent, AssetContentVc, AssetVc},
    context::AssetContextVc,
    reference_type::{EntryReferenceSubType, ReferenceType},
    source_asset::SourceAssetVc,
    source_transform::{SourceTransform, SourceTransformVc},
    virtual_asset::VirtualAssetVc,
};
use turbopack_ecmascript::{
    chunk::EcmascriptChunkPlaceablesVc, EcmascriptInputTransformsVc, EcmascriptModuleAssetType,
    EcmascriptModuleAssetVc,
};

use crate::{
    embed_js::embed_file,
    evaluate::{evaluate, JavaScriptValue},
    execution_context::{ExecutionContext, ExecutionContextVc},
};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[turbo_tasks::value(transparent, serialization = "custom")]
struct ProcessedCSS {
    css: String,
    map: Option<String>,
}

#[turbo_tasks::value]
pub struct PostCssTransform {
    evaluate_context: AssetContextVc,
    execution_context: ExecutionContextVc,
}

#[turbo_tasks::value_impl]
impl PostCssTransformVc {
    #[turbo_tasks::function]
    pub fn new(evaluate_context: AssetContextVc, execution_context: ExecutionContextVc) -> Self {
        PostCssTransform {
            evaluate_context,
            execution_context,
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl SourceTransform for PostCssTransform {
    #[turbo_tasks::function]
    fn transform(&self, source: AssetVc) -> AssetVc {
        PostCssTransformedAsset {
            evaluate_context: self.evaluate_context,
            execution_context: self.execution_context,
            source,
        }
        .cell()
        .into()
    }
}

#[turbo_tasks::value]
struct PostCssTransformedAsset {
    evaluate_context: AssetContextVc,
    execution_context: ExecutionContextVc,
    source: AssetVc,
}

#[turbo_tasks::value_impl]
impl Asset for PostCssTransformedAsset {
    #[turbo_tasks::function]
    fn path(&self) -> FileSystemPathVc {
        self.source.path()
    }

    #[turbo_tasks::function]
    async fn content(&self) -> Result<AssetContentVc> {
        let ExecutionContext {
            project_root,
            intermediate_output_path,
        } = *self.execution_context.await?;
        let content = self.source.content().await?;
        let AssetContent::File(file) = *content else {
            bail!("PostCSS transform only support transforming files");
        };
        let FileContent::Content(content) = &*file.await? else {
            return Ok(AssetContent::File(FileContent::NotFound.cell()).cell());
        };
        let content = content.content().to_str()?;
        let context = self.evaluate_context;
        let config_paths = [
            project_root.join("postcss.config.js").realpath(),
            project_root.join("tailwind.config.js").realpath(),
        ];
        let configs = config_paths
            .into_iter()
            .map(|path| async move {
                Ok(
                    matches!(&*path.get_type().await?, FileSystemEntryType::File).then(|| {
                        EcmascriptModuleAssetVc::new(
                            SourceAssetVc::new(path).into(),
                            context,
                            Value::new(EcmascriptModuleAssetType::Ecmascript),
                            EcmascriptInputTransformsVc::cell(vec![]),
                            context.environment(),
                        )
                        .as_ecmascript_chunk_placeable()
                    }),
                )
            })
            .try_join()
            .await?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        let postcss_executor = context.process(
            VirtualAssetVc::new(
                project_root.join("postcss.config.js/transform.js"),
                AssetContent::File(embed_file("transforms/postcss.js")).cell(),
            )
            .into(),
            Value::new(ReferenceType::Entry(EntryReferenceSubType::Undefined)),
        );
        let css_fs_path = self.source.path().await?;
        let css_path = css_fs_path.path.as_str();
        let config_value = evaluate(
            project_root,
            postcss_executor,
            project_root,
            context,
            intermediate_output_path,
            Some(EcmascriptChunkPlaceablesVc::cell(configs)),
            vec![
                JsonValueVc::cell(content.into()),
                JsonValueVc::cell(css_path.into()),
            ],
        )
        .await?;
        let JavaScriptValue::Value(val) = &*config_value else {
            bail!("Expected a value from PostCSS transform");
        };
        let processed_css: ProcessedCSS = serde_json::from_reader(val.read())
            .context("Unable to deserializate response from PostCSS transform operation")?;
        let new_content = Rope::from(processed_css.css.clone());
        let file = File::from(new_content);
        // TODO handle SourceMap
        Ok(AssetContent::File(FileContent::Content(file).cell()).cell())
    }
}
