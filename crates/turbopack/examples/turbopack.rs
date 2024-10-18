#![feature(trivial_bounds)]

use std::{
    collections::HashMap,
    env::current_dir,
    time::{Duration, Instant},
};

use anyhow::Result;
use tokio::{spawn, time::sleep};
use turbo_tasks::{util::FormatDuration, TurboTasks, UpdateInfo, Value, Vc};
use turbo_tasks_fs::{DiskFileSystem, FileSystem};
use turbo_tasks_memory::MemoryBackend;
use turbopack::{emit_with_completion, rebase::RebasedAsset, register};
use turbopack_core::{
    compile_time_info::CompileTimeInfo,
    context::AssetContext,
    environment::{Environment, ExecutionEnvironment, NodeJsEnvironment},
    file_source::FileSource,
    PROJECT_FILESYSTEM_NAME,
};
use turbopack_resolve::resolve_options_context::ResolveOptionsContext;

#[tokio::main]
async fn main() -> Result<()> {
    register();

    let tt = TurboTasks::new(MemoryBackend::default());
    let start = Instant::now();

    let task = tt.spawn_root_task(|| {
        Box::pin(async {
            let root = current_dir().unwrap().to_str().unwrap().to_string();
            let disk_fs = DiskFileSystem::new(PROJECT_FILESYSTEM_NAME.to_string(), root, vec![]);
            disk_fs.await?.start_watching()?;

            // Smart Pointer cast
            let fs: Vc<Box<dyn FileSystem>> = Vc::upcast(disk_fs);
            let input = fs.root().join("demo".to_string());
            let output = fs.root().join("out".to_string());
            let entry = fs.root().join("demo/index.js".to_string());

            let source = FileSource::new(entry);
            let module_asset_context = turbopack::ModuleAssetContext::new(
                Vc::cell(HashMap::new()),
                CompileTimeInfo::new(Environment::new(Value::new(
                    ExecutionEnvironment::NodeJsLambda(NodeJsEnvironment::default().into()),
                ))),
                Default::default(),
                ResolveOptionsContext {
                    enable_typescript: true,
                    enable_react: true,
                    enable_node_modules: Some(fs.root()),
                    custom_conditions: vec!["development".to_string()],
                    ..Default::default()
                }
                .cell(),
                Vc::cell("default".to_string()),
            );
            let module = module_asset_context
                .process(
                    Vc::upcast(source),
                    Value::new(turbopack_core::reference_type::ReferenceType::Undefined),
                )
                .module();
            let rebased = RebasedAsset::new(module, input, output);
            emit_with_completion(Vc::upcast(rebased), output).await?;

            Ok::<Vc<()>, _>(Default::default())
        })
    });
    spawn({
        let tt = tt.clone();
        async move {
            tt.wait_task_completion(task, true).await.unwrap();
            println!("done in {}", FormatDuration(start.elapsed()));

            loop {
                let UpdateInfo {
                    duration, tasks, ..
                } = tt
                    .get_or_wait_aggregated_update_info(Duration::from_millis(100))
                    .await;
                println!("updated {} tasks in {}", tasks, FormatDuration(duration));
            }
        }
    })
    .await
    .unwrap();

    loop {
        sleep(Duration::from_secs(10)).await;
    }
}
