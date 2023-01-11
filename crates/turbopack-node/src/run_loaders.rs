use anyhow::Result;
use turbo_tasks::{debug::ValueDebug, primitives::JsonValueVc, Value};
use turbo_tasks_fs::{File, FileContent, FileSystemPathVc};
use turbopack_core::{
    asset::{AssetContent, AssetContentVc},
    context::AssetContextVc,
    virtual_asset::VirtualAssetVc,
};
use turbopack_ecmascript::{
    EcmascriptInputTransform, EcmascriptInputTransformsVc, EcmascriptModuleAssetType,
    EcmascriptModuleAssetVc,
};

use crate::{
    embed_js::embed_file,
    evaluate::{evaluate, JavaScriptValue},
    execution_context::{ExecutionContext, ExecutionContextVc},
};

#[turbo_tasks::value(transparent)]
pub struct Loaders(Vec<FileSystemPathVc>);

#[turbo_tasks::function]
pub async fn run_loaders(
    execution_context: ExecutionContextVc,
    transform_target: FileSystemPathVc,
    context: AssetContextVc,
    loaders: LoadersVc,
) -> Result<AssetContentVc> {
    let ExecutionContext {
        project_root,
        intermediate_output_path,
    } = *execution_context.await?;

    let loader_executor = EcmascriptModuleAssetVc::new(
        VirtualAssetVc::new(
            project_root.join("run-loaders.js"),
            AssetContent::File(embed_file("transforms/run-loaders.ts")).cell(),
        )
        .into(),
        context,
        Value::new(EcmascriptModuleAssetType::Typescript),
        EcmascriptInputTransformsVc::cell(vec![EcmascriptInputTransform::TypeScript]),
        context.environment(),
    );

    let mut loader_paths = vec![];
    for path in &*loaders.await? {
        loader_paths.push(path.await?.path.clone())
    }

    let result = evaluate(
        project_root,
        loader_executor.into(),
        project_root,
        transform_target,
        context,
        intermediate_output_path,
        None,
        vec![
            JsonValueVc::cell((&*transform_target.await?).path.clone().into()),
            JsonValueVc::cell(loader_paths.into()),
        ],
    );

    let JavaScriptValue::Value(val) = &*result.await? else {
        // An error happened, which has already been converted into an issue.
        return Ok(AssetContent::File(FileContent::NotFound.cell()).cell());
    };

    Ok(AssetContent::File(FileContent::Content(File::from(val.clone())).cell()).cell())
}
