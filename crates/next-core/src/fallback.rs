use std::collections::HashMap;

use anyhow::{bail, Result};
use indoc::formatdoc;
use turbo_tasks::{primitives::StringVc, Value};
use turbo_tasks_env::ProcessEnvVc;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack::{
    ecmascript::EcmascriptModuleAssetVc, transition::TransitionsByNameVc, ModuleAssetContextVc,
};
use turbopack_core::{
    asset::{Asset, AssetContentVc, AssetVc},
    chunk::ChunkGroupVc,
    context::AssetContextVc,
    environment::EnvironmentVc,
    reference::AssetReferencesVc,
    resolve::{options::ImportMap, origin::PlainResolveOriginVc},
};
use turbopack_dev_server::html::DevHtmlAssetVc;
use turbopack_node::{
    execution_context::ExecutionContextVc,
    html_error::{FallbackPageAsset, FallbackPageAssetVc},
};

use crate::{
    next_client::context::{
        get_client_chunking_context, get_client_module_options_context,
        get_client_resolve_options_context, get_client_runtime_entries, ClientContextType,
    },
    next_config::NextConfigVc,
    next_import_map::{insert_alias_option, insert_next_shared_aliases},
    runtime::resolve_runtime_request,
};

#[turbo_tasks::function]
pub async fn get_fallback_page(
    project_path: FileSystemPathVc,
    execution_context: ExecutionContextVc,
    dev_server_root: FileSystemPathVc,
    assets_root: FileSystemPathVc,
    env: ProcessEnvVc,
    client_environment: EnvironmentVc,
    next_config: NextConfigVc,
) -> Result<FallbackPageAssetVc> {
    let ty = Value::new(ClientContextType::Fallback);
    let resolve_options_context = get_client_resolve_options_context(project_path, ty, next_config);
    let module_options_context = get_client_module_options_context(
        project_path,
        execution_context,
        client_environment,
        ty,
        next_config,
    );
    let chunking_context = get_client_chunking_context(
        project_path,
        dev_server_root,
        assets_root,
        client_environment,
        ty,
    );
    let entries = get_client_runtime_entries(project_path, env, ty, next_config);

    let mut import_map = ImportMap::empty();
    insert_next_shared_aliases(&mut import_map, project_path).await?;
    insert_alias_option(
        &mut import_map,
        project_path,
        next_config.resolve_alias_options(),
        ["browser"],
    )
    .await?;

    let context: AssetContextVc = ModuleAssetContextVc::new(
        TransitionsByNameVc::cell(HashMap::new()),
        client_environment,
        module_options_context,
        resolve_options_context.with_extended_import_map(import_map.cell()),
    )
    .into();

    let runtime_entries = entries.resolve_entries(context);

    let fallback_chunk = resolve_runtime_request(
        PlainResolveOriginVc::new(context, project_path).into(),
        "entry/fallback",
    );

    let module = if let Some(module) =
        EcmascriptModuleAssetVc::resolve_from(fallback_chunk.as_asset()).await?
    {
        module
    } else {
        bail!("fallback runtime entry is not an ecmascript module");
    };

    let chunk = module.as_evaluated_chunk(chunking_context, Some(runtime_entries));

    Ok(FallbackAsset {
        html_asset: DevHtmlAssetVc::new(
            dev_server_root,
            assets_root.join("fallback.html"),
            vec![ChunkGroupVc::from_chunk(chunk)],
        ),
        config: next_config,
    }
    .cell()
    .into())
}

#[turbo_tasks::value]
struct FallbackAsset {
    html_asset: DevHtmlAssetVc,
    config: NextConfigVc,
}

#[turbo_tasks::value_impl]
impl FallbackPageAsset for FallbackAsset {
    #[turbo_tasks::function]
    async fn with_error(&self, exit_code: Option<i32>, error: StringVc) -> Result<AssetVc> {
        let error = error.await?;

        let html_status = match exit_code {
            Some(exit_code) => format!("<h2>Exit status</h2><pre>{exit_code}</pre>"),
            None => "<h3>No exit status</pre>".to_owned(),
        };

        let body = formatdoc! {r#"
            <script id="__NEXT_DATA__" type="application/json">
                {{
                    "props": {{}},
                    "assetPrefix": {assetPrefix}
                }}
            </script>
            <div id="__next">
                <h1>Error rendering page</h1>
                <h2>Message</h2>
                <pre>{error}</pre>
                {html_status}
            </div>
        "#, assetPrefix = serde_json::to_string(&self.config.asset_prefix().await?.as_deref())? };

        Ok(self.html_asset.with_body(body).into())
    }
}

#[turbo_tasks::value_impl]
impl Asset for FallbackAsset {
    #[turbo_tasks::function]
    fn path(&self) -> FileSystemPathVc {
        self.html_asset.path()
    }

    #[turbo_tasks::function]
    fn content(&self) -> AssetContentVc {
        unreachable!("fallback asset has no content")
    }

    #[turbo_tasks::function]
    fn references(&self) -> AssetReferencesVc {
        self.html_asset.references()
    }
}
