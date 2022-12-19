use anyhow::{Context, Result};
use turbo_tasks::Value;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack::{resolve_options, resolve_options_context::ResolveOptionsContext};
use turbopack_core::resolve::{
    options::{ImportMapping, ImportMappingVc},
    parse::RequestVc,
    pattern::Pattern,
    resolve,
};
use turbopack_node::execution_context::ExecutionContextVc;

#[turbo_tasks::function]
pub async fn get_next_package(project_root: FileSystemPathVc) -> Result<FileSystemPathVc> {
    let result = resolve(
        project_root,
        RequestVc::parse(Value::new(Pattern::Constant(
            "next/package.json".to_string(),
        ))),
        resolve_options(
            project_root,
            ResolveOptionsContext {
                enable_node_modules: true,
                enable_node_native_modules: true,
                custom_conditions: vec!["development".to_string()],
                ..Default::default()
            }
            .cell(),
        ),
    );
    let assets = result.primary_assets().await?;
    let asset = assets.first().context("Next.js package not found")?;
    Ok(asset.path().parent())
}

#[turbo_tasks::function]
pub async fn get_postcss_package_mapping(
    execution_context: ExecutionContextVc,
) -> Result<ImportMappingVc> {
    let project_root = execution_context.await?.project_root;
    Ok(ImportMapping::Alternatives(vec![
        // Prefer the local installed version over the next.js version
        ImportMapping::PrimaryAlternative("postcss".to_string(), Some(project_root)).cell(),
        ImportMapping::PrimaryAlternative(
            "postcss".to_string(),
            Some(get_next_package(project_root)),
        )
        .cell(),
    ])
    .cell())
}
