use std::collections::HashMap;

use anyhow::{anyhow, Result};
use turbo_tasks_fs::{rope::Rope, to_sys_path, FileSystemPathVc};
use turbopack_core::chunk::{ChunkGroupVc, ChunkingContextVc};
use turbopack_ecmascript::EcmascriptModuleAssetVc;

use crate::{
    bootstrap::NodeJsBootstrapAsset, emit, eval_js_operation, pool::NodeJsPool,
    EvalJavaScriptOutgoingMessage,
};

#[turbo_tasks::value(shared)]
#[derive(Clone)]
pub enum JavaScriptValue {
    Value(Rope),
    Stream(#[turbo_tasks(trace_ignore)] Vec<u8>),
}

#[turbo_tasks::value(transparent)]
pub struct JavaScriptValueArray(Vec<JavaScriptValueVc>);

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
            let pool = NodeJsPool::new(cwd, entrypoint, HashMap::new(), 1);
            let bootstrap = NodeJsBootstrapAsset {
                path: self.intermediate_output_path().join("read-config.js"),
                chunk_group: ChunkGroupVc::from_chunk(
                    self.entry()
                        .as_evaluated_chunk(self.chunking_context(), None),
                ),
            };
            emit(bootstrap.cell().into(), self.intermediate_output_path()).await?;
            let mut operation = pool.operation().await?;
            let output = eval_js_operation(
                &mut operation,
                EvalJavaScriptOutgoingMessage::LoadNextConfig {
                    path: to_sys_path(self.path())
                        .await?
                        .and_then(|p| p.to_str().map(|s| s.to_string()))
                        .ok_or_else(|| anyhow!("Invalid next.config.js path"))?,
                },
            )
            .await?;
            Ok(JavaScriptValue::Value(output.into()).cell())
        } else {
            Err(anyhow!("can only render from a disk filesystem"))
        }
    }
}
