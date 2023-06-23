#![cfg(test)]

use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use dunce::canonicalize;
use once_cell::sync::Lazy;
use serde::Deserialize;
use turbo_tasks::{CompletionVc, NothingVc, TurboTasks, Value};
use turbo_tasks_env::CommandLineProcessEnvVc;
use turbo_tasks_fs::{
    json::parse_json_with_source_context, util::sys_to_unix, DiskFileSystemVc, FileSystem,
    FileSystemPathVc,
};
use turbo_tasks_memory::MemoryBackend;
use turbopack::{
    condition::ContextCondition,
    ecmascript::TransformPluginVc,
    module_options::{
        CustomEcmascriptTransformPlugins, CustomEcmascriptTransformPluginsVc, JsxTransformOptions,
        JsxTransformOptionsVc, ModuleOptionsContext,
    },
    resolve_options_context::ResolveOptionsContext,
    transition::TransitionsByNameVc,
    ModuleAssetContextVc,
};
use turbopack_build::BuildChunkingContextVc;
use turbopack_core::{
    asset::Asset,
    chunk::{ChunkingContextVc, EvaluatableAssetVc},
    compile_time_defines,
    compile_time_info::CompileTimeInfo,
    context::{AssetContext, AssetContextVc},
    environment::{EnvironmentIntention, EnvironmentVc, ExecutionEnvironment, NodeJsEnvironment},
    reference_type::{EntryReferenceSubType, ReferenceType},
    source_asset::SourceAssetVc,
};
use turbopack_dev::DevChunkingContextVc;
use turbopack_ecmascript_plugins::transform::{
    emotion::{EmotionTransformConfig, EmotionTransformer},
    styled_components::{StyledComponentsTransformConfig, StyledComponentsTransformer},
};
use turbopack_node::evaluate::evaluate;

fn register() {
    turbo_tasks::register();
    turbo_tasks_env::register();
    turbo_tasks_fs::register();
    turbopack::register();
    turbopack_build::register();
    turbopack_dev::register();
    turbopack_env::register();
    turbopack_ecmascript_plugins::register();
    turbopack_ecmascript_runtime::register();
    include!(concat!(env!("OUT_DIR"), "/register_test_execution.rs"));
}

static WORKSPACE_ROOT: Lazy<String> = Lazy::new(|| {
    let package_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    canonicalize(package_root)
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
});

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionOptions {
    #[serde(default = "default_entry")]
    entry: String,
    #[serde(default)]
    runtime: Runtime,
}

#[derive(Debug, Deserialize, Default)]
enum Runtime {
    #[default]
    Dev,
    Build,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        ExecutionOptions {
            entry: default_entry(),
            runtime: Default::default(),
        }
    }
}

fn default_entry() -> String {
    "input/index.js".to_owned()
}

#[testing::fixture("tests/execution/*/*/")]
fn test(resource: PathBuf) {
    let resource = canonicalize(resource).unwrap();
    // Separating this into a different function fixes my IDE's types for some
    // reason...
    run(resource).unwrap();
}

#[tokio::main(flavor = "current_thread")]
async fn run(resource: PathBuf) -> Result<()> {
    register();

    let tt = TurboTasks::new(MemoryBackend::default());
    let task = tt.spawn_once_task(async move {
        run_test(resource.to_str().unwrap());
        Ok(NothingVc::new().into())
    });
    tt.wait_task_completion(task, true).await?;

    Ok(())
}

#[turbo_tasks::function]
async fn run_test(resource: &str) -> Result<FileSystemPathVc> {
    let test_path = Path::new(resource);
    assert!(test_path.exists(), "{} does not exist", resource);
    assert!(
        test_path.is_dir(),
        "{} is not a directory. Snapshot tests must be directories.",
        test_path.to_str().unwrap()
    );

    let options_file = fs::read_to_string(test_path.join("options.json"));
    let options = match options_file {
        Err(_) => ExecutionOptions::default(),
        Ok(options_str) => parse_json_with_source_context(&options_str).unwrap(),
    };
    let root_fs = DiskFileSystemVc::new("workspace".to_string(), WORKSPACE_ROOT.clone());
    let project_fs = DiskFileSystemVc::new("project".to_string(), WORKSPACE_ROOT.clone());
    let project_root = project_fs.root();

    let relative_path = test_path.strip_prefix(&*WORKSPACE_ROOT)?;
    let relative_path = sys_to_unix(relative_path.to_str().unwrap());
    let path = root_fs.root().join(&relative_path);
    let project_path = project_root.join(&relative_path);
    let tests_path = project_fs.root().join("crates/turbopack-tests");

    let test_entry_asset = project_path.join(&options.entry);
    let jest_runtime_asset = tests_path.join("js/jest-runtime.ts");
    let jest_entry_asset = tests_path.join("js/jest-entry.ts");
    println!("lines {:?}", jest_entry_asset.read().await?.lines());
    let entry_paths = vec![jest_runtime_asset, test_entry_asset];

    let env = EnvironmentVc::new(
        Value::new(ExecutionEnvironment::NodeJsBuildTime(
            // TODO: load more from options.json
            NodeJsEnvironment::default().into(),
        )),
        Value::new(EnvironmentIntention::Client),
    );
    let compile_time_info = CompileTimeInfo::builder(env)
        .defines(
            compile_time_defines!(
                process.env.NODE_ENV = "development",
                DEFINED_VALUE = "value",
                DEFINED_TRUE = true,
                A.VERY.LONG.DEFINED.VALUE = "value",
            )
            .cell(),
        )
        .cell();

    let custom_ecma_transform_plugins = Some(CustomEcmascriptTransformPluginsVc::cell(
        CustomEcmascriptTransformPlugins {
            source_transforms: vec![
                TransformPluginVc::cell(Box::new(
                    EmotionTransformer::new(&EmotionTransformConfig {
                        sourcemap: Some(false),
                        ..Default::default()
                    })
                    .expect("Should be able to create emotion transformer"),
                )),
                TransformPluginVc::cell(Box::new(StyledComponentsTransformer::new(
                    &StyledComponentsTransformConfig::default(),
                ))),
            ],
            output_transforms: vec![],
        },
    ));
    let context: AssetContextVc = ModuleAssetContextVc::new(
        TransitionsByNameVc::cell(HashMap::new()),
        compile_time_info,
        ModuleOptionsContext {
            enable_jsx: Some(JsxTransformOptionsVc::cell(JsxTransformOptions {
                development: true,
                ..Default::default()
            })),
            preset_env_versions: Some(env),
            rules: vec![(
                ContextCondition::InDirectory("node_modules".to_string()),
                ModuleOptionsContext {
                    ..Default::default()
                }
                .cell(),
            )],
            custom_ecma_transform_plugins,
            ..Default::default()
        }
        .into(),
        ResolveOptionsContext {
            enable_typescript: true,
            enable_react: true,
            enable_node_modules: Some(project_root),
            custom_conditions: vec!["development".to_string()],
            rules: vec![(
                ContextCondition::InDirectory("node_modules".to_string()),
                ResolveOptionsContext {
                    enable_node_modules: Some(project_root),
                    custom_conditions: vec!["development".to_string()],
                    ..Default::default()
                }
                .cell(),
            )],
            ..Default::default()
        }
        .cell(),
    )
    .into();

    let chunk_root_path = path.join("output");
    let static_root_path = path.join("static");
    let chunking_context: ChunkingContextVc = match options.runtime {
        Runtime::Dev => DevChunkingContextVc::builder(
            project_root,
            path,
            chunk_root_path,
            static_root_path,
            env,
        )
        .build(),
        Runtime::Build => BuildChunkingContextVc::builder(
            project_root,
            path,
            chunk_root_path,
            static_root_path,
            env,
        )
        .build()
        .into(),
    };

    let modules: Vec<EvaluatableAssetVc> = entry_paths
        .into_iter()
        .map(SourceAssetVc::new)
        .map(|p| {
            EvaluatableAssetVc::from_asset(
                context.process(
                    p.into(),
                    Value::new(ReferenceType::Entry(EntryReferenceSubType::Undefined)),
                ),
                context,
            )
        })
        .collect();

    let entry = context.process(
        SourceAssetVc::new(jest_entry_asset).into(),
        Value::new(ReferenceType::Entry(EntryReferenceSubType::Undefined)),
    );
    println!("BEFORE EVALUATE");
    let res = evaluate(
        entry,
        chunk_root_path,
        CommandLineProcessEnvVc::new().into(),
        modules.first().unwrap().ident(),
        context,
        chunking_context,
        None,
        // Some(EvaluatableAssetsVc::many(modules)),
        vec![],
        CompletionVc::immutable(),
        false,
    )
    .await?;
    println!("AFTER EVALUATE");

    println!("BEFORE READ");
    let turbo_tasks_bytes::stream::SingleValue::Single(val) =
        res.try_into_single().await.context("try")? else {
            return Err(anyhow!("oh no"));
        };
    println!("AFTER READ {}", val.to_str()?);

    Ok(path)
}
