use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use mime::TEXT_HTML_UTF_8;
use serde::{Deserialize, Serialize};
use turbo_tasks::{primitives::StringVc, Value};
use turbo_tasks_fs::{File, FileContent, FileSystemEntryType, FileSystemPathVc};
use turbopack::{transition::TransitionsByNameVc, ModuleAssetContextVc};
use turbopack_core::{
    asset::{Asset, AssetContent, AssetContentVc, AssetVc},
    chunk::ChunkingContextVc,
    context::AssetContextVc,
    environment::{EnvironmentIntention, EnvironmentVc, ExecutionEnvironment, NodeJsEnvironment},
    reference_type::{EntryReferenceSubType, ReferenceType},
    source_asset::SourceAssetVc,
};
use turbopack_dev_server::html::DevHtmlAssetVc;
use turbopack_ecmascript::{
    chunk::EcmascriptChunkPlaceablesVc, EcmascriptInputTransformsVc, EcmascriptModuleAssetType,
    EcmascriptModuleAssetVc,
};
use turbopack_node::{
    evaluate::{evaluate, JavaScriptValue},
    get_intermediate_asset, get_renderer_pool, trace_stack, NodeJsOperation,
};

use super::{
    issue::RenderingIssue, RenderDataVc, RenderResult, RenderStaticIncomingMessage,
    RenderStaticOutgoingMessage,
};
use crate::{
    embed_js::next_asset,
    next_server::{get_build_module_options_context, get_build_resolve_options_context},
};

/// Renders a module as static HTML in a node.js process.
#[turbo_tasks::function]
pub async fn render_static(
    path: FileSystemPathVc,
    project_root: FileSystemPathVc,
    module: EcmascriptModuleAssetVc,
    runtime_entries: EcmascriptChunkPlaceablesVc,
    fallback_page: DevHtmlAssetVc,
    chunking_context: ChunkingContextVc,
    intermediate_output_path: FileSystemPathVc,
    data: RenderDataVc,
) -> Result<AssetContentVc> {
    let ecma_chunk = module.as_evaluated_chunk(chunking_context, Some(runtime_entries));
    for referenced_asset in ecma_chunk.references().await?.iter() {
        for asset in referenced_asset
            .resolve_reference()
            .primary_assets()
            .await?
            .iter()
        {
            // let mut inner_asset = HashMap::new();
            if &*asset.path().extension().await? == "css" {
                if let AssetContent::File(file) = &*asset.content().await? {
                    if let FileContent::Content(content) = &*file.await? {
                        let css_content = std::io::read_to_string(content.read())?;
                        let ProcessedCSS { css, .. } = &*postcss(
                            project_root,
                            intermediate_output_path.join(".next/config"),
                            asset.path(),
                            css_content,
                        )
                        .await?;
                        println!("{:?}", referenced_asset);
                    }
                }
            }
        }
    }
    let intermediate_asset = get_intermediate_asset(ecma_chunk, intermediate_output_path);
    let renderer_pool = get_renderer_pool(intermediate_asset, intermediate_output_path);
    // Read this strongly consistent, since we don't want to run inconsistent
    // node.js code.
    let pool = renderer_pool.strongly_consistent().await?;
    let mut operation = match pool.operation().await {
        Ok(operation) => operation,
        Err(err) => return static_error(path, err, None, fallback_page).await,
    };

    match run_static_operation(
        &mut operation,
        data,
        intermediate_asset,
        intermediate_output_path,
    )
    .await
    {
        Ok(asset) => Ok(asset),
        Err(err) => static_error(path, err, Some(operation), fallback_page).await,
    }
}

async fn run_static_operation(
    operation: &mut NodeJsOperation,
    data: RenderDataVc,
    intermediate_asset: AssetVc,
    intermediate_output_path: FileSystemPathVc,
) -> Result<AssetContentVc> {
    let data = data.await?;

    operation
        .send(RenderStaticOutgoingMessage::Headers { data: &data })
        .await
        .context("sending headers to node.js process")?;
    match operation
        .recv()
        .await
        .context("receiving from node.js process")?
    {
        RenderStaticIncomingMessage::Result {
            result: RenderResult::Simple(body),
        } => Ok(FileContent::Content(File::from(body).with_content_type(TEXT_HTML_UTF_8)).into()),
        RenderStaticIncomingMessage::Result {
            result: RenderResult::Advanced { body, content_type },
        } => Ok(FileContent::Content(
            File::from(body)
                .with_content_type(content_type.map_or(Ok(TEXT_HTML_UTF_8), |c| c.parse())?),
        )
        .into()),
        RenderStaticIncomingMessage::Error(error) => {
            bail!(trace_stack(error, intermediate_asset, intermediate_output_path).await?)
        }
    }
}

async fn static_error(
    path: FileSystemPathVc,
    error: anyhow::Error,
    operation: Option<NodeJsOperation>,
    fallback_page: DevHtmlAssetVc,
) -> Result<AssetContentVc> {
    let message = format!("{error:?}");
    let status = match operation {
        Some(operation) => Some(operation.wait_or_kill().await?),
        None => None,
    };

    let html_status = match status {
        Some(status) => format!("<h2>Exit status</h2><pre>{status}</pre>"),
        None => "<h3>No exit status</pre>".to_owned(),
    };

    let body = format!(
        "<script id=\"__NEXT_DATA__\" type=\"application/json\">{{ \"props\": {{}} }}</script>
    <div id=\"__next\">
        <h1>Error rendering page</h1>
        <h2>Message</h2>
        <pre>{message}</pre>
        {html_status}
    </div>",
    );

    let issue = RenderingIssue {
        context: path,
        message: StringVc::cell(format!("{error:?}")),
        status: status.and_then(|status| status.code()),
    };

    issue.cell().as_issue().emit();

    let html = fallback_page.with_body(body);

    Ok(html.content())
}

#[turbo_tasks::function]
pub fn create_node_evaluate_asset_context(project_path: FileSystemPathVc) -> AssetContextVc {
    ModuleAssetContextVc::new(
        TransitionsByNameVc::cell(Default::default()),
        EnvironmentVc::new(
            Value::new(ExecutionEnvironment::NodeJsBuildTime(
                NodeJsEnvironment::default().cell(),
            )),
            Value::new(EnvironmentIntention::Build),
        ),
        get_build_module_options_context(),
        get_build_resolve_options_context(project_path),
    )
    .as_asset_context()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[turbo_tasks::value(transparent, serialization = "custom")]
struct ProcessedCSS {
    css: String,
    map: Option<String>,
}

#[turbo_tasks::function]
async fn postcss(
    project_path: FileSystemPathVc,
    intermediate_output_path: FileSystemPathVc,
    css_path: FileSystemPathVc,
    css: String,
) -> Result<ProcessedCSSVc> {
    let context = create_node_evaluate_asset_context(project_path);
    let create_config_chunk = |config_asset: SourceAssetVc| {
        EcmascriptModuleAssetVc::new(
            config_asset.into(),
            context,
            Value::new(EcmascriptModuleAssetType::Ecmascript),
            EcmascriptInputTransformsVc::cell(vec![]),
            context.environment(),
        )
        .as_ecmascript_chunk_placeable()
    };
    let postcss_config_path = project_path.join("postcss.config.js").realpath();
    let tailwind_config_path = project_path.join("tailwind.config.js").realpath();
    let mut config_chunks = vec![];
    if matches!(
        &*postcss_config_path.get_type().await?,
        FileSystemEntryType::File
    ) {
        config_chunks.push(create_config_chunk(SourceAssetVc::new(postcss_config_path)));
    }
    if matches!(
        &*tailwind_config_path.get_type().await?,
        FileSystemEntryType::File
    ) {
        config_chunks.push(create_config_chunk(SourceAssetVc::new(
            tailwind_config_path,
        )));
    }

    let asset_path = project_path.join("postcss.js");
    let postcss_asset = context.process(
        next_asset(asset_path, "entry/config/postcss.js"),
        Value::new(ReferenceType::Entry(EntryReferenceSubType::Undefined)),
    );
    let css_fs_path = css_path.await?;
    let css_name = css_fs_path.file_name();
    let css_path = css_fs_path.path.to_owned();
    let output_css_path = intermediate_output_path.join(css_name);
    let output_file_name = output_css_path.await?.path.to_owned();
    let config_value = evaluate(
        project_path,
        postcss_asset,
        project_path,
        context,
        intermediate_output_path,
        Some(EcmascriptChunkPlaceablesVc::cell(config_chunks)),
        vec![css, css_path, output_file_name],
    )
    .await?;
    match &*config_value {
        JavaScriptValue::Value(val) => {
            let processed_css: ProcessedCSS = serde_json::from_reader(val.read())?;
            Ok(processed_css.cell())
        }
        JavaScriptValue::Stream(_) => {
            unimplemented!("Stream not supported now");
        }
    }
}
