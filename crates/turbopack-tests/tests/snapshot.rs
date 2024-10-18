#![cfg(test)]

mod util;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::PathBuf,
};

use anyhow::{bail, Context, Result};
use dunce::canonicalize;
use serde::Deserialize;
use serde_json::json;
use turbo_tasks::{ReadRef, TryJoinIterExt, TurboTasks, Value, ValueToString, Vc};
use turbo_tasks_env::DotenvProcessEnv;
use turbo_tasks_fs::{
    json::parse_json_with_source_context, util::sys_to_unix, DiskFileSystem, FileSystem,
    FileSystemPath,
};
use turbo_tasks_memory::MemoryBackend;
use turbopack::{
    ecmascript::{EcmascriptInputTransform, EcmascriptModuleAsset, TreeShakingMode},
    module_options::{
        JsxTransformOptions, ModuleOptionsContext, ModuleRule, ModuleRuleCondition,
        ModuleRuleEffect,
    },
    ModuleAssetContext,
};
use turbopack_browser::BrowserChunkingContext;
use turbopack_core::{
    asset::Asset,
    chunk::{
        availability_info::AvailabilityInfo, ChunkableModule, ChunkingContext, ChunkingContextExt,
        EvaluatableAssetExt, EvaluatableAssets, MinifyType,
    },
    compile_time_defines,
    compile_time_info::CompileTimeInfo,
    condition::ContextCondition,
    context::AssetContext,
    environment::{BrowserEnvironment, Environment, ExecutionEnvironment, NodeJsEnvironment},
    file_source::FileSource,
    free_var_references,
    issue::{Issue, IssueDescriptionExt},
    module::Module,
    output::OutputAsset,
    reference_type::{EntryReferenceSubType, ReferenceType},
    source::Source,
};
use turbopack_ecmascript_plugins::transform::{
    emotion::{EmotionTransformConfig, EmotionTransformer},
    styled_components::{StyledComponentsTransformConfig, StyledComponentsTransformer},
};
use turbopack_ecmascript_runtime::RuntimeType;
use turbopack_env::ProcessEnvAsset;
use turbopack_nodejs::NodeJsChunkingContext;
use turbopack_resolve::resolve_options_context::ResolveOptionsContext;
use turbopack_test_utils::snapshot::{diff, expected, matches_expected, snapshot_issues};

use crate::util::REPO_ROOT;

fn register() {
    turbo_tasks::register();
    turbo_tasks_env::register();
    turbo_tasks_fs::register();
    turbopack::register();
    turbopack_nodejs::register();
    turbopack_browser::register();
    turbopack_env::register();
    turbopack_ecmascript_plugins::register();
    turbopack_ecmascript_runtime::register();
    turbopack_resolve::register();
    include!(concat!(env!("OUT_DIR"), "/register_test_snapshot.rs"));
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotOptions {
    #[serde(default = "default_browserslist")]
    browserslist: String,
    #[serde(default = "default_entry")]
    entry: String,
    #[serde(default)]
    minify_type: MinifyType,
    #[serde(default)]
    runtime: Runtime,
    #[serde(default = "default_runtime_type")]
    runtime_type: RuntimeType,
    #[serde(default)]
    environment: SnapshotEnvironment,
    #[serde(default)]
    use_swc_css: bool,
    #[serde(default)]
    tree_shaking_mode: Option<TreeShakingMode>,
}

#[derive(Debug, Deserialize, Default)]
enum Runtime {
    #[default]
    Dev,
    Build,
}

#[derive(Debug, Deserialize, Default)]
enum SnapshotEnvironment {
    #[default]
    Browser,
    NodeJs,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        SnapshotOptions {
            browserslist: default_browserslist(),
            entry: default_entry(),
            minify_type: Default::default(),
            runtime: Default::default(),
            runtime_type: default_runtime_type(),
            environment: Default::default(),
            use_swc_css: Default::default(),
            tree_shaking_mode: Default::default(),
        }
    }
}

fn default_browserslist() -> String {
    // Use a specific version to avoid churn in transform over time as the
    // preset_env crate data changes
    "Chrome 102".to_owned()
}

fn default_entry() -> String {
    "input/index.js".to_owned()
}

fn default_runtime_type() -> RuntimeType {
    // We don't want all snapshot tests to also include the runtime every time,
    // as this would be a lot of extra noise whenever we make a single change to
    // the runtime. Instead, we only include the runtime in snapshots that
    // specifically request it via "runtime": "Default".
    RuntimeType::Dummy
}

#[testing::fixture("tests/snapshot/*/*/", exclude("node_modules"))]
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
        let out = run_test(resource.to_str().unwrap().to_string());
        let _ = out.resolve_strongly_consistent().await?;
        let captured_issues = out.peek_issues_with_path().await?;

        let plain_issues = captured_issues
            .iter_with_shortest_path()
            .map(|(issue_vc, path)| async move { issue_vc.into_plain(path).await })
            .try_join()
            .await?;

        snapshot_issues(plain_issues, out.join("issues".to_string()), &REPO_ROOT)
            .await
            .context("Unable to handle issues")?;
        Ok(Vc::<()>::default())
    });
    tt.wait_task_completion(task, true).await?;

    Ok(())
}

#[turbo_tasks::function]
async fn run_test(resource: String) -> Result<Vc<FileSystemPath>> {
    let test_path = canonicalize(&resource)?;
    assert!(test_path.exists(), "{} does not exist", resource);
    assert!(
        test_path.is_dir(),
        "{} is not a directory. Snapshot tests must be directories.",
        test_path.to_str().unwrap()
    );

    let options_file = fs::read_to_string(test_path.join("options.json"));
    let options = match options_file {
        Err(_) => SnapshotOptions::default(),
        Ok(options_str) => parse_json_with_source_context(&options_str).unwrap(),
    };
    let root_fs = DiskFileSystem::new("workspace".to_string(), REPO_ROOT.clone(), vec![]);
    let project_fs = DiskFileSystem::new("project".to_string(), REPO_ROOT.clone(), vec![]);
    let project_root = project_fs.root();

    let relative_path = test_path.strip_prefix(&*REPO_ROOT)?;
    let relative_path = sys_to_unix(relative_path.to_str().unwrap());
    let path = root_fs.root().join(relative_path.to_string());
    let project_path = project_root.join(relative_path.to_string());

    let entry_asset = project_path.join(options.entry);

    let env = Environment::new(Value::new(match options.environment {
        SnapshotEnvironment::Browser => {
            ExecutionEnvironment::Browser(
                // TODO: load more from options.json
                BrowserEnvironment {
                    dom: true,
                    web_worker: false,
                    service_worker: false,
                    browserslist_query: options.browserslist.to_owned(),
                }
                .into(),
            )
        }
        SnapshotEnvironment::NodeJs => {
            ExecutionEnvironment::NodeJsBuildTime(
                // TODO: load more from options.json
                NodeJsEnvironment::default().into(),
            )
        }
    }));

    let defines = compile_time_defines!(
        process.turbopack = true,
        process.env.TURBOPACK = true,
        process.env.NODE_ENV = "development",
        DEFINED_VALUE = "value",
        DEFINED_TRUE = true,
        A.VERY.LONG.DEFINED.VALUE = json!({ "test": true }),
    );

    let compile_time_info = CompileTimeInfo::builder(env)
        .defines(defines.clone().cell())
        .free_var_references(free_var_references!(..defines.into_iter()).cell())
        .cell();

    let conditions = ModuleRuleCondition::any(vec![
        ModuleRuleCondition::ResourcePathEndsWith(".js".to_string()),
        ModuleRuleCondition::ResourcePathEndsWith(".jsx".to_string()),
        ModuleRuleCondition::ResourcePathEndsWith(".ts".to_string()),
        ModuleRuleCondition::ResourcePathEndsWith(".tsx".to_string()),
    ]);

    let custom_rules = ModuleRule::new(
        conditions,
        vec![ModuleRuleEffect::ExtendEcmascriptTransforms {
            prepend: Vc::cell(vec![
                EcmascriptInputTransform::Plugin(Vc::cell(Box::new(
                    EmotionTransformer::new(&EmotionTransformConfig::default())
                        .expect("Should be able to create emotion transformer"),
                ) as _)),
                EcmascriptInputTransform::Plugin(Vc::cell(Box::new(
                    StyledComponentsTransformer::new(&StyledComponentsTransformConfig::default()),
                ) as _)),
            ]),
            append: Vc::cell(vec![]),
        }],
    );
    let asset_context: Vc<Box<dyn AssetContext>> = Vc::upcast(ModuleAssetContext::new(
        Vc::cell(HashMap::new()),
        compile_time_info,
        ModuleOptionsContext {
            enable_jsx: Some(JsxTransformOptions::cell(JsxTransformOptions {
                development: true,
                ..Default::default()
            })),
            preset_env_versions: Some(env),
            ignore_dynamic_requests: true,
            use_swc_css: options.use_swc_css,
            rules: vec![(
                ContextCondition::InDirectory("node_modules".to_string()),
                ModuleOptionsContext {
                    use_swc_css: options.use_swc_css,
                    ..Default::default()
                }
                .cell(),
            )],
            custom_rules: vec![custom_rules],
            tree_shaking_mode: options.tree_shaking_mode,
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
        Vc::cell("test".to_string()),
    ));

    let runtime_entries = maybe_load_env(asset_context, project_path)
        .await?
        .map(|asset| EvaluatableAssets::one(asset.to_evaluatable(asset_context)));

    let chunk_root_path = path.join("output".to_string());
    let static_root_path = path.join("static".to_string());

    let chunking_context: Vc<Box<dyn ChunkingContext>> = match options.runtime {
        Runtime::Dev => Vc::upcast(
            BrowserChunkingContext::builder(
                project_root,
                path,
                path,
                chunk_root_path,
                static_root_path,
                env,
                RuntimeType::Development,
            )
            .runtime_type(options.runtime_type)
            .build(),
        ),
        Runtime::Build => Vc::upcast(
            NodeJsChunkingContext::builder(
                project_root,
                path,
                path,
                chunk_root_path,
                static_root_path,
                env,
                RuntimeType::Production,
            )
            .minify_type(options.minify_type)
            .runtime_type(options.runtime_type)
            .build(),
        ),
    };

    let expected_paths = expected(chunk_root_path)
        .await?
        .union(&expected(static_root_path).await?)
        .copied()
        .collect();

    let entry_module = asset_context
        .process(
            Vc::upcast(FileSource::new(entry_asset)),
            Value::new(ReferenceType::Entry(EntryReferenceSubType::Undefined)),
        )
        .module();

    let chunks = if let Some(ecmascript) =
        Vc::try_resolve_downcast_type::<EcmascriptModuleAsset>(entry_module).await?
    {
        // TODO: Load runtime entries from snapshots
        match options.runtime {
            Runtime::Dev => chunking_context.evaluated_chunk_group_assets(
                ecmascript.ident(),
                runtime_entries
                    .unwrap_or_else(EvaluatableAssets::empty)
                    .with_entry(Vc::upcast(ecmascript)),
                Value::new(AvailabilityInfo::Root),
            ),
            Runtime::Build => {
                Vc::cell(vec![
                    Vc::try_resolve_downcast_type::<NodeJsChunkingContext>(chunking_context)
                        .await?
                        .unwrap()
                        .entry_chunk_group(
                            // `expected` expects a completely flat output directory.
                            chunk_root_path
                                .join(
                                    entry_module
                                        .ident()
                                        .path()
                                        .file_stem()
                                        .await?
                                        .as_deref()
                                        .unwrap()
                                        .to_string(),
                                )
                                .with_extension("entry.js".to_string()),
                            Vc::upcast(ecmascript),
                            runtime_entries
                                .unwrap_or_else(EvaluatableAssets::empty)
                                .with_entry(Vc::upcast(ecmascript)),
                            Value::new(AvailabilityInfo::Root),
                        )
                        .await?
                        .asset,
                ])
            }
        }
    } else if let Some(chunkable) =
        Vc::try_resolve_downcast::<Box<dyn ChunkableModule>>(entry_module).await?
    {
        chunking_context.root_chunk_group_assets(chunkable)
    } else {
        // TODO convert into a serve-able asset
        bail!("Entry module is not chunkable, so it can't be used to bootstrap the application")
    };

    let mut seen = HashSet::new();
    let mut queue: VecDeque<_> = chunks.await?.iter().copied().collect();

    let output_path = path.await?;
    while let Some(asset) = queue.pop_front() {
        walk_asset(asset, &output_path, &mut seen, &mut queue)
            .await
            .context(format!(
                "Failed to walk asset {}",
                asset
                    .ident()
                    .to_string()
                    .await
                    .context("to_string failed")?
            ))?;
    }

    matches_expected(expected_paths, seen)
        .await
        .context("Actual assets doesn't match with expected assets")?;

    Ok(path)
}

async fn walk_asset(
    asset: Vc<Box<dyn OutputAsset>>,
    output_path: &ReadRef<FileSystemPath>,
    seen: &mut HashSet<Vc<FileSystemPath>>,
    queue: &mut VecDeque<Vc<Box<dyn OutputAsset>>>,
) -> Result<()> {
    let path = asset.ident().path().resolve().await?;

    if !seen.insert(path) {
        return Ok(());
    }

    if path.await?.is_inside_ref(output_path) {
        // Only consider assets that should be written to disk.
        diff(path, asset.content()).await?;
    }

    queue.extend(
        asset
            .references()
            .await?
            .iter()
            .copied()
            .map(|asset| async move {
                Ok(Vc::try_resolve_downcast::<Box<dyn OutputAsset>>(asset).await?)
            })
            .try_join()
            .await?
            .into_iter()
            .flatten(),
    );

    Ok(())
}

async fn maybe_load_env(
    _context: Vc<Box<dyn AssetContext>>,
    path: Vc<FileSystemPath>,
) -> Result<Option<Vc<Box<dyn Source>>>> {
    let dotenv_path = path.join("input/.env".to_string());

    if !dotenv_path.read().await?.is_content() {
        return Ok(None);
    }

    let env = DotenvProcessEnv::new(None, dotenv_path);
    let asset = ProcessEnvAsset::new(dotenv_path, Vc::upcast(env));
    Ok(Some(Vc::upcast(asset)))
}
