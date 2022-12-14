use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use turbo_tasks_fs::{rope::Rope, to_sys_path, FileSystemPathVc};
use turbopack_core::chunk::{ChunkGroupVc, ChunkingContextVc};
use turbopack_ecmascript::{chunk::EcmascriptChunkPlaceablesVc, EcmascriptModuleAssetVc};

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
pub async fn load_config(
    entry_asset: EcmascriptModuleAssetVc,
    config_type: String,
    intermediate_output_path: FileSystemPathVc,
    chunking_context: ChunkingContextVc,
    path: FileSystemPathVc,
    runtime_entries: Option<EcmascriptChunkPlaceablesVc>,
) -> Result<JavaScriptValueVc> {
    if let (Some(cwd), Some(entrypoint)) = (
        to_sys_path(intermediate_output_path).await?,
        to_sys_path(intermediate_output_path.join("read-config.js")).await?,
    ) {
        let bootstrap = NodeJsBootstrapAsset {
            path: intermediate_output_path.join("read-config.js"),
            chunk_group: ChunkGroupVc::from_chunk(
                entry_asset.as_evaluated_chunk(chunking_context, runtime_entries),
            ),
        };
        emit(bootstrap.cell().into(), intermediate_output_path).await?;
        let pool = NodeJsPool::new(cwd, entrypoint, HashMap::new(), 1);
        let mut operation = pool.operation().await?;
        let output = eval_js_operation(
            &mut operation,
            EvalJavaScriptOutgoingMessage::LoadConfig {
                path: to_sys_path(path)
                    .await?
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .ok_or_else(|| anyhow!("Invalid config path"))?,
                config_type,
            },
        )
        .await?;
        Ok(JavaScriptValue::Value(output.into()).cell())
    } else {
        panic!("can only render from a disk filesystem");
    }
}
