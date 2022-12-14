use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use turbo_tasks::Value;
use turbo_tasks_fs::{embed_file, rope::Rope, to_sys_path, FileSystemPathVc};
use turbopack_core::{
    asset::AssetVc,
    chunk::{ChunkGroupVc, ChunkingContextVc},
    context::AssetContextVc,
    virtual_asset::VirtualAssetVc,
};
use turbopack_ecmascript::{
    chunk::EcmascriptChunkPlaceablesVc, EcmascriptInputTransform, EcmascriptInputTransformsVc,
    EcmascriptModuleAssetType, EcmascriptModuleAssetVc,
};

use crate::{
    bootstrap::NodeJsBootstrapAsset, emit, pool::NodeJsPool, EvalJavaScriptIncomingMessage,
    EvalJavaScriptOutgoingMessage, NodeJsOperation,
};

#[turbo_tasks::value(shared)]
#[derive(Clone)]
pub enum JavaScriptValue {
    Value(Rope),
    // TODO, support stream in the future
    Stream(#[turbo_tasks(trace_ignore)] Vec<u8>),
}

async fn eval_js_operation(
    operation: &mut NodeJsOperation,
    content: EvalJavaScriptOutgoingMessage,
) -> Result<Vec<u8>> {
    operation.send(content).await?;
    match operation.recv().await? {
        EvalJavaScriptIncomingMessage::Error(err) => {
            bail!(err.print(Default::default(), None).await?);
        }
        EvalJavaScriptIncomingMessage::JavaScriptValue { data } => Ok(data),
    }
}

#[turbo_tasks::function]
/// Pass the file you cared as `runtime_entries` to invalidate and reload the
/// evaluated result automatically.
pub async fn evaluate(
    asset: AssetVc,
    context: AssetContextVc,
    // TODO, serialize arguments
    arguments: Vec<String>,
    intermediate_output_path: FileSystemPathVc,
    chunking_context: ChunkingContextVc,
    runtime_entries: Option<EcmascriptChunkPlaceablesVc>,
) -> Result<JavaScriptValueVc> {
    let entry_module = EcmascriptModuleAssetVc::new(
        VirtualAssetVc::new(
            intermediate_output_path.join("evaluate.js"),
            embed_file!("js/src/evaluate.ts").into(),
        )
        .into(),
        context,
        Value::new(EcmascriptModuleAssetType::Typescript),
        EcmascriptInputTransformsVc::cell(vec![EcmascriptInputTransform::TypeScript]),
        context.environment(),
    );
    if let (Some(cwd), Some(entrypoint)) = (
        to_sys_path(intermediate_output_path).await?,
        to_sys_path(intermediate_output_path.join("evaluate.js")).await?,
    ) {
        let bootstrap = NodeJsBootstrapAsset {
            path: intermediate_output_path.join("evaluate.js"),
            chunk_group: ChunkGroupVc::from_chunk(
                entry_module.as_evaluated_chunk(chunking_context, runtime_entries),
            ),
        };
        emit(asset, intermediate_output_path).await?;
        emit(bootstrap.cell().into(), intermediate_output_path).await?;
        let pool = NodeJsPool::new(cwd, entrypoint, HashMap::new(), 1);
        let mut operation = pool.operation().await?;
        let output = eval_js_operation(
            &mut operation,
            EvalJavaScriptOutgoingMessage::Evaluate {
                filepath: to_sys_path(asset.path())
                    .await?
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .ok_or_else(|| anyhow!("Invalid JavaScript path to execute"))?,
                arguments,
            },
        )
        .await?;
        Ok(JavaScriptValue::Value(output.into()).cell())
    } else {
        panic!("can only render from a disk filesystem");
    }
}
