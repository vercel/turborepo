use anyhow::{bail, Result};
use futures::StreamExt;
use turbo_tasks::primitives::StringVc;
use turbo_tasks_env::ProcessEnvVc;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::{asset::AssetVc, chunk::ChunkingContextVc, error::PrettyPrintError};
use turbopack_dev_server::source::{BodyVc, ProxyResult, ProxyResultVc};
use turbopack_ecmascript::{chunk::EcmascriptChunkPlaceablesVc, EcmascriptModuleAssetVc};

use super::{
    issue::RenderingIssue, stream_body_chunks, RenderDataVc, RenderProxyIncomingMessage,
    RenderProxyOutgoingMessage, ResponseHeaders,
};
use crate::{
    get_intermediate_asset, get_renderer_pool, pool::NodeJsOperation,
    render::error_page::error_html, source_map::trace_stack,
};

/// Renders a module as static HTML in a node.js process.
#[turbo_tasks::function]
pub async fn render_proxy(
    cwd: FileSystemPathVc,
    env: ProcessEnvVc,
    path: FileSystemPathVc,
    module: EcmascriptModuleAssetVc,
    runtime_entries: EcmascriptChunkPlaceablesVc,
    chunking_context: ChunkingContextVc,
    intermediate_output_path: FileSystemPathVc,
    output_root: FileSystemPathVc,
    project_dir: FileSystemPathVc,
    data: RenderDataVc,
    body: BodyVc,
) -> Result<ProxyResultVc> {
    let intermediate_asset = get_intermediate_asset(
        module.as_evaluated_chunk(chunking_context, Some(runtime_entries)),
        intermediate_output_path,
    );

    let pool = get_renderer_pool(
        cwd,
        env,
        intermediate_asset,
        intermediate_output_path,
        output_root,
        project_dir,
        /* debug */ false,
    )
    .await?;

    let mut operation = match pool.operation().await {
        Ok(operation) => operation,
        Err(err) => {
            let (status, body) = proxy_error(path, err, None).await?;
            return Ok(proxy_error_result(status, body));
        }
    };

    let (status, headers) = match start_proxy_operation(
        &mut operation,
        data,
        body,
        intermediate_asset,
        intermediate_output_path,
        project_dir,
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            let (status, body) = proxy_error(path, err, Some(operation)).await?;
            return Ok(proxy_error_result(status, body));
        }
    };

    Ok(ProxyResult {
        status,
        headers,
        body: stream_body_chunks(operation, intermediate_asset, intermediate_output_path),
    }
    .cell())
}

async fn start_proxy_operation(
    operation: &mut NodeJsOperation,
    data: RenderDataVc,
    body: BodyVc,
    intermediate_asset: AssetVc,
    intermediate_output_path: FileSystemPathVc,
    project_dir: FileSystemPathVc,
) -> Result<(u16, Vec<(String, String)>)> {
    let data = data.await?;
    // First, send the render data.
    operation
        .send(RenderProxyOutgoingMessage::Headers { data: &data })
        .await?;

    let mut body = body.await?.read();
    // Then, send the binary body in chunks.
    while let Some(data) = body.next().await {
        operation
            .send(RenderProxyOutgoingMessage::BodyChunk { data: &data? })
            .await?;
    }

    operation.send(RenderProxyOutgoingMessage::BodyEnd).await?;

    match operation.recv().await? {
        RenderProxyIncomingMessage::Headers {
            data: ResponseHeaders { status, headers },
        } => Ok((status, headers)),
        RenderProxyIncomingMessage::Error(error) => {
            bail!(
                trace_stack(
                    error,
                    intermediate_asset,
                    intermediate_output_path,
                    project_dir
                )
                .await?
            )
        }
    }
}

async fn proxy_error(
    path: FileSystemPathVc,
    error: anyhow::Error,
    operation: Option<NodeJsOperation>,
) -> Result<(u16, String)> {
    let message = format!("{}", PrettyPrintError(&error));

    let status = match operation {
        Some(operation) => Some(operation.wait_or_kill().await?),
        None => None,
    };

    let mut details = vec![];
    if let Some(status) = status {
        details.push(format!("status: {status}"));
    }

    let status_code = 500;
    let body = error_html(
        status_code,
        "An error occurred while proxying the request to Node.js".to_string(),
        format!("{message}\n\n{}", details.join("\n")),
    )
    .await?
    .clone_value();

    RenderingIssue {
        context: path,
        message: StringVc::cell(message),
        status: status.and_then(|status| status.code()),
    }
    .cell()
    .as_issue()
    .emit();

    Ok((status_code, body))
}

fn proxy_error_result(status: u16, body: String) -> ProxyResultVc {
    ProxyResult {
        status,
        headers: vec![(
            "content-type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: body.clone().into(),
    }
    .cell()
}
