use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use turbo_tasks_fs::{rope::Rope, to_sys_path, FileSystemPathVc};
use turbopack_core::chunk::{ChunkGroupVc, ChunkingContextVc};
use turbopack_ecmascript::EcmascriptModuleAssetVc;

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

#[turbo_tasks::value_trait]
pub trait JavaScriptConfig {
    fn path(&self) -> FileSystemPathVc;
    fn entry(&self) -> EcmascriptModuleAssetVc;
    fn chunking_context(&self) -> ChunkingContextVc;
    fn intermediate_output_path(&self) -> FileSystemPathVc;

    async fn load(&self) -> Result<JavaScriptValueVc> {
        if let (Some(cwd), Some(entrypoint)) = (
            to_sys_path(self.intermediate_output_path()).await?,
            to_sys_path(self.intermediate_output_path().join("read-config.js")).await?,
        ) {
            let bootstrap = NodeJsBootstrapAsset {
                path: self.intermediate_output_path().join("read-config.js"),
                chunk_group: ChunkGroupVc::from_chunk(
                    self.entry()
                        .as_evaluated_chunk(self.chunking_context(), None),
                ),
            };
            emit(bootstrap.cell().into(), self.intermediate_output_path()).await?;
            let pool = NodeJsPool::new(cwd, entrypoint, HashMap::new(), 1);
            let mut operation = pool.operation().await?;
            let output = eval_js_operation(
                &mut operation,
                EvalJavaScriptOutgoingMessage::LoadNextConfig {
                    path: to_sys_path(self.path())
                        .await?
                        .and_then(|p| p.to_str().map(|s| s.to_string()))
                        .ok_or_else(|| anyhow!("Invalid config path"))?,
                },
            )
            .await?;
            Ok(JavaScriptValue::Value(output.into()).cell())
        } else {
            panic!("can only render from a disk filesystem");
        }
    }
}
