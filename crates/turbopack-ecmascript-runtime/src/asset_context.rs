use turbo_tasks::Vc;
use turbopack::{
    module_options::{ModuleOptionsContext, TypescriptTransformOptions},
    resolve_options_context::ResolveOptionsContext,
    transition::TransitionsByName,
    ModuleAssetContext,
};
use turbopack_core::{
    compile_time_info::CompileTimeInfo, context::AssetContext, environment::Environment,
};

/// Returns the runtime asset context to use to process runtime code assets.
pub fn get_runtime_asset_context(environment: Vc<Environment>) -> Vc<Box<dyn AssetContext>> {
    let resolve_options_context = ResolveOptionsContext::default();
    let module_options_context = ModuleOptionsContext {
        enable_typescript_transform: Some(TypescriptTransformOptions::default().cell()),
        ..Default::default()
    }
    .cell();
    let compile_time_info = CompileTimeInfo::builder(environment).cell();

    let context: Vc<Box<dyn AssetContext>> = Vc::upcast(ModuleAssetContext::new(
        Vc::cell(Default::default()),
        compile_time_info,
        module_options_context,
        resolve_options_context,
    ));

    context
}
