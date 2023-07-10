use anyhow::{anyhow, Result};
use turbo_tasks::{TryJoinIterExt, Value};
use turbo_tasks_env::ProcessEnvVc;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack::ecmascript::EcmascriptModuleAssetVc;
use turbopack_cli_utils::runtime_entry::{RuntimeEntriesVc, RuntimeEntry};
use turbopack_core::{
    chunk::{ChunkableModuleVc, ChunkingContextVc},
    environment::{BrowserEnvironment, EnvironmentVc, ExecutionEnvironment},
    reference_type::{EntryReferenceSubType, ReferenceType},
    resolve::{origin::PlainResolveOriginVc, parse::RequestVc},
    source_asset::SourceAssetVc,
};
use turbopack_dev::{react_refresh::assert_can_resolve_react_refresh, DevChunkingContextVc};
use turbopack_dev_server::{
    html::DevHtmlAssetVc,
    source::{asset_graph::AssetGraphContentSourceVc, ContentSourceVc},
};
use turbopack_node::execution_context::ExecutionContextVc;

use crate::{
    contexts::{
        get_client_asset_context, get_client_compile_time_info, get_client_resolve_options_context,
        NodeEnvVc,
    },
    embed_js::embed_file_path,
};

#[turbo_tasks::function]
pub async fn create_web_entry_source(
    project_path: FileSystemPathVc,
    execution_context: ExecutionContextVc,
    entry_requests: Vec<RequestVc>,
    server_root: FileSystemPathVc,
    _env: ProcessEnvVc,
    eager_compile: bool,
    node_env: NodeEnvVc,
    browserslist_query: &str,
) -> Result<ContentSourceVc> {
    let env = EnvironmentVc::new(Value::new(ExecutionEnvironment::Browser(
        BrowserEnvironment {
            dom: true,
            web_worker: false,
            service_worker: false,
            browserslist_query: browserslist_query.to_owned(),
        }
        .into(),
    )));
    let compile_time_info = get_client_compile_time_info(env, node_env);
    let context =
        get_client_asset_context(project_path, execution_context, compile_time_info, node_env);
    let chunking_context =
        get_client_chunking_context(project_path, server_root, compile_time_info.environment());
    let entries = get_client_runtime_entries(project_path, node_env);

    let runtime_entries = entries.resolve_entries(context);

    let origin = PlainResolveOriginVc::new(context, project_path.join("_")).as_resolve_origin();
    let entries = entry_requests
        .into_iter()
        .map(|request| async move {
            let ty = Value::new(ReferenceType::Entry(EntryReferenceSubType::Web));
            Ok(origin
                .resolve_asset(request, origin.resolve_options(ty.clone()), ty)
                .primary_assets()
                .await?
                .first()
                .copied())
        })
        .try_join()
        .await?;

    let entries: Vec<_> = entries
        .into_iter()
        .flatten()
        .map(|module| async move {
            if let Some(ecmascript) = EcmascriptModuleAssetVc::resolve_from(module).await? {
                Ok((
                    ecmascript.into(),
                    chunking_context,
                    Some(runtime_entries.with_entry(ecmascript.into())),
                ))
            } else if let Some(chunkable) = ChunkableModuleVc::resolve_from(module).await? {
                // TODO this is missing runtime code, so it's probably broken and we should also
                // add an ecmascript chunk with the runtime code
                Ok((chunkable, chunking_context, None))
            } else {
                // TODO convert into a serve-able asset
                Err(anyhow!(
                    "Entry module is not chunkable, so it can't be used to bootstrap the \
                     application"
                ))
            }
        })
        .try_join()
        .await?;

    let entry_asset = DevHtmlAssetVc::new(server_root.join("index.html"), entries).into();

    let graph = if eager_compile {
        AssetGraphContentSourceVc::new_eager(server_root, entry_asset)
    } else {
        AssetGraphContentSourceVc::new_lazy(server_root, entry_asset)
    }
    .into();
    Ok(graph)
}

#[turbo_tasks::function]
pub fn get_client_chunking_context(
    project_path: FileSystemPathVc,
    server_root: FileSystemPathVc,
    environment: EnvironmentVc,
) -> ChunkingContextVc {
    DevChunkingContextVc::builder(
        project_path,
        server_root,
        server_root.join("/_chunks"),
        server_root.join("/_assets"),
        environment,
    )
    .hot_module_replacement()
    .build()
}

#[turbo_tasks::function]
pub async fn get_client_runtime_entries(
    project_path: FileSystemPathVc,
    node_env: NodeEnvVc,
) -> Result<RuntimeEntriesVc> {
    let resolve_options_context = get_client_resolve_options_context(project_path, node_env);

    let mut runtime_entries = Vec::new();

    let enable_react_refresh =
        assert_can_resolve_react_refresh(project_path, resolve_options_context)
            .await?
            .as_request();
    // It's important that React Refresh come before the regular bootstrap file,
    // because the bootstrap contains JSX which requires Refresh's global
    // functions to be available.
    if let Some(request) = enable_react_refresh {
        runtime_entries.push(RuntimeEntry::Request(request, project_path.join("_")).cell())
    };

    runtime_entries.push(
        RuntimeEntry::Source(SourceAssetVc::new(embed_file_path("entry/bootstrap.ts")).into())
            .cell(),
    );

    Ok(RuntimeEntriesVc::cell(runtime_entries))
}
