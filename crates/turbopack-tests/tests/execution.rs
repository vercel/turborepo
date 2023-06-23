#![cfg(test)]

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use dunce::canonicalize;
use futures::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use turbo_tasks::{debug::ValueDebug, CompletionVc, TryJoinIterExt, TurboTasks, Value};
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
        JsxTransformOptionsVc, ModuleOptionsContext, TypescriptTransformOptionsVc,
    },
    resolve_options_context::ResolveOptionsContext,
    transition::TransitionsByNameVc,
    ModuleAssetContextVc,
};
use turbopack_core::{
    asset::Asset,
    chunk::{EvaluatableAssetVc, EvaluatableAssetsVc},
    compile_time_defines,
    compile_time_info::CompileTimeInfo,
    context::{AssetContext, AssetContextVc},
    environment::{EnvironmentIntention, EnvironmentVc, ExecutionEnvironment, NodeJsEnvironment},
    issue::{IssueVc, IssuesVc},
    reference_type::{EntryReferenceSubType, ReferenceType},
    source_asset::SourceAssetVc,
};
use turbopack_dev::DevChunkingContextVc;
use turbopack_ecmascript_plugins::transform::styled_jsx::StyledJsxTransformer;
use turbopack_node::evaluate::evaluate;
use turbopack_test_utils::snapshot::snapshot_issues;

#[turbo_tasks::value]
#[derive(Debug)]
#[serde(rename_all = "camelCase")]
struct JsResult {
    uncaught_exceptions: Vec<String>,
    unhandled_rejections: Vec<String>,
    #[turbo_tasks(trace_ignore)]
    jest_result: JestRunResult,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct JestRunResult {
    test_results: Vec<JestTestResult>,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct JestTestResult {
    test_path: Vec<String>,
    errors: Vec<String>,
}

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

#[testing::fixture("tests/execution/tests/*/*/*")]
fn test(resource: PathBuf) {
    let resource = canonicalize(resource).unwrap();
    if resource.ends_with("__skipped__") {
        // "Skip" directories named `__skipped__`, which include test directories to
        // skip. These tests are not considered truly skipped by `cargo test`, but they
        // are not run.
        return;
    }

    let messages = get_messages(run(resource).unwrap());
    if !messages.is_empty() {
        panic!(
            "Failed with error(s) in the following test(s):\n\n{}",
            messages.join("\n\n--\n")
        )
    }
}

#[testing::fixture("tests/execution/tests/*/*/__skipped__/*/input")]
#[should_panic]
fn test_skipped_fails(resource: PathBuf) {
    let resource = resource.parent().unwrap().to_path_buf();
    let JsResult {
        // Ignore uncaught exceptions for skipped tests.
        uncaught_exceptions: _,
        unhandled_rejections: _,
        jest_result,
    } = run(resource).unwrap();

    // Assert that this skipped test itself has at least one browser test which
    // fails.
    assert!(
        // Skipped tests sometimes have errors (e.g. unsupported syntax) that prevent tests from
        // running at all. Allow them to have empty results.
        jest_result.test_results.is_empty()
            || jest_result
                .test_results
                .into_iter()
                .any(|r| !r.errors.is_empty()),
    );
}

fn get_messages(js_results: JsResult) -> Vec<String> {
    let mut messages = vec![];

    if js_results.jest_result.test_results.is_empty() {
        messages.push("No tests were run.".to_string());
    }

    for test_result in js_results.jest_result.test_results {
        // It's possible to fail multiple tests across these tests,
        // so collect them and fail the respective test in Rust with
        // an aggregate message.
        if !test_result.errors.is_empty() {
            messages.push(format!(
                "\"{}\":\n{}",
                test_result.test_path[1..].join(" > "),
                test_result.errors.join("\n")
            ));
        }
    }

    for uncaught_exception in js_results.uncaught_exceptions {
        messages.push(format!("Uncaught exception: {}", uncaught_exception));
    }

    for unhandled_rejection in js_results.unhandled_rejections {
        messages.push(format!("Unhandled rejection: {}", unhandled_rejection));
    }

    messages
}

#[tokio::main(flavor = "current_thread")]
async fn run(resource: PathBuf) -> Result<JsResult> {
    register();

    let tt = TurboTasks::new(MemoryBackend::default());
    Ok(tt
        .run_once(async move { run_test(resource.to_str().unwrap()).await })
        .await?
        .1)
}

async fn run_test(resource: &str) -> Result<(FileSystemPathVc, JsResult)> {
    let test_path = Path::new(resource);
    assert!(test_path.exists(), "{} does not exist", resource);
    assert!(
        test_path.is_dir(),
        "{} is not a directory. Execution tests must be directories.",
        test_path.to_str().unwrap()
    );

    let root_fs = DiskFileSystemVc::new("workspace".to_string(), WORKSPACE_ROOT.clone());
    let project_fs = DiskFileSystemVc::new("project".to_string(), WORKSPACE_ROOT.clone());
    let project_root = project_fs.root();

    let relative_path = test_path.strip_prefix(&*WORKSPACE_ROOT)?;
    let relative_path = sys_to_unix(relative_path.to_str().unwrap());
    let path = root_fs.root().join(&relative_path);
    let project_path = project_root.join(&relative_path);
    let tests_path = project_fs.root().join("crates/turbopack-tests");

    let test_entry_asset = project_path.join("input/index.js");
    let jest_runtime_asset = tests_path.join("js/jest-runtime.ts");
    let jest_entry_asset = tests_path.join("js/jest-entry.ts");
    let entry_paths = vec![jest_runtime_asset, test_entry_asset];

    let env = EnvironmentVc::new(
        Value::new(ExecutionEnvironment::NodeJsBuildTime(
            NodeJsEnvironment::default().into(),
        )),
        Value::new(EnvironmentIntention::Client),
    );

    let compile_time_info = CompileTimeInfo::builder(env)
        .defines(compile_time_defines!(process.env.NODE_ENV = "development",).cell())
        .cell();

    let context: AssetContextVc = ModuleAssetContextVc::new(
        TransitionsByNameVc::cell(HashMap::new()),
        compile_time_info,
        ModuleOptionsContext {
            enable_jsx: Some(JsxTransformOptionsVc::cell(JsxTransformOptions {
                development: true,
                ..Default::default()
            })),
            enable_typescript_transform: Some(TypescriptTransformOptionsVc::default()),
            preset_env_versions: Some(env),
            rules: vec![(
                ContextCondition::InDirectory("node_modules".to_string()),
                ModuleOptionsContext {
                    ..Default::default()
                }
                .cell(),
            )],
            custom_ecma_transform_plugins: Some(CustomEcmascriptTransformPluginsVc::cell(
                CustomEcmascriptTransformPlugins {
                    source_transforms: vec![TransformPluginVc::cell(
                        Box::<StyledJsxTransformer>::default(),
                    )],
                    output_transforms: vec![],
                },
            )),
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
    let chunking_context = DevChunkingContextVc::builder(
        project_root,
        chunk_root_path,
        chunk_root_path,
        static_root_path,
        env,
    )
    .build();

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

    let captured_issues = IssueVc::peek_issues_with_path(path)
        .await?
        .strongly_consistent()
        .await?;

    println!(
        "issue len {}",
        captured_issues
            .iter_with_shortest_path()
            .collect::<Vec<_>>()
            .len()
    );
    let plain_issues = captured_issues
        .iter_with_shortest_path()
        .map(|(issue_vc, path)| async move {
            // issue_vc.into_plain(path).await?.description
            Ok((
                issue_vc.into_plain(path).await?,
                issue_vc.into_plain(path).dbg().await?,
            ))
        })
        .try_join()
        .await?;

    snapshot_issues(
        plain_issues.into_iter(),
        path.join("issues"),
        &WORKSPACE_ROOT,
    )
    .await
    .context("Unable to handle issues")?;

    let res = evaluate(
        entry,
        chunk_root_path,
        CommandLineProcessEnvVc::new().into(),
        modules.first().unwrap().ident(),
        context,
        chunking_context,
        Some(EvaluatableAssetsVc::many(modules)),
        vec![],
        CompletionVc::immutable(),
        false,
    )
    .await?;

    let mut read = res.read();
    let bytes = read
        .next()
        .await
        .context("test node result did not emit anything")??;

    Ok((path, parse_json_with_source_context(bytes.to_str()?)?))
}
