use std::{
    env::current_dir,
    path::{PathBuf, MAIN_SEPARATOR},
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use turbo_tasks::{
    primitives::StringVc, NothingVc, TransientInstance, TryJoinIterExt, TurboTasks, Value,
};
use turbo_tasks_fs::FileSystem;
use turbo_tasks_memory::MemoryBackend;
use turbopack::ecmascript::EcmascriptModuleAssetVc;
use turbopack_build::BuildChunkingContextVc;
use turbopack_cli_utils::issue::{ConsoleUiVc, LogOptions};
use turbopack_core::{
    asset::{Asset, AssetsVc},
    chunk::{
        ChunkableModule, ChunkableModuleVc, ChunkingContext, ChunkingContextVc, EvaluatableAssetsVc,
    },
    context::AssetContext,
    environment::{BrowserEnvironment, EnvironmentVc, ExecutionEnvironment},
    issue::{handle_issues, IssueReporterVc, IssueSeverity},
    reference::all_assets_from_entry,
    reference_type::{EntryReferenceSubType, ReferenceType},
    resolve::{origin::PlainResolveOriginVc, parse::RequestVc, pattern::QueryMapVc},
};
use turbopack_env::dotenv::load_env;
use turbopack_node::execution_context::ExecutionContextVc;

use crate::{
    arguments::BuildArguments,
    contexts::{get_client_asset_context, get_client_compile_time_info, NodeEnv},
    util::{
        normalize_dirs, normalize_entries, output_fs, project_fs, EntryRequest, EntryRequestVc,
        EntryRequestsVc, NormalizedDirs,
    },
};

pub fn register() {
    turbopack::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}

pub struct TurbopackBuildBuilder {
    turbo_tasks: Arc<TurboTasks<MemoryBackend>>,
    project_dir: String,
    root_dir: String,
    entry_requests: Vec<EntryRequest>,
    browserslist_query: String,
    log_level: IssueSeverity,
    show_all: bool,
    log_detail: bool,
}

impl TurbopackBuildBuilder {
    pub fn new(
        turbo_tasks: Arc<TurboTasks<MemoryBackend>>,
        project_dir: String,
        root_dir: String,
    ) -> Self {
        TurbopackBuildBuilder {
            turbo_tasks,
            project_dir,
            root_dir,
            entry_requests: vec![],
            browserslist_query: "chrome 64, edge 79, firefox 67, opera 51, safari 12".to_owned(),
            log_level: IssueSeverity::Warning,
            show_all: false,
            log_detail: false,
        }
    }

    pub fn entry_request(mut self, entry_asset_path: EntryRequest) -> Self {
        self.entry_requests.push(entry_asset_path);
        self
    }

    pub fn browserslist_query(mut self, browserslist_query: String) -> Self {
        self.browserslist_query = browserslist_query;
        self
    }

    pub fn log_level(mut self, log_level: IssueSeverity) -> Self {
        self.log_level = log_level;
        self
    }

    pub fn show_all(mut self, show_all: bool) -> Self {
        self.show_all = show_all;
        self
    }

    pub fn log_detail(mut self, log_detail: bool) -> Self {
        self.log_detail = log_detail;
        self
    }

    pub async fn build(self) -> Result<()> {
        let task = self.turbo_tasks.spawn_once_task(async move {
            let build_result = build_internal(
                StringVc::cell(self.project_dir.clone()),
                StringVc::cell(self.root_dir),
                EntryRequestsVc::cell(
                    self.entry_requests
                        .iter()
                        .cloned()
                        .map(EntryRequestVc::cell)
                        .collect(),
                ),
                StringVc::cell(self.browserslist_query),
            );

            // Await the result to propagate any errors.
            build_result.await?;

            let issue_reporter: IssueReporterVc =
                ConsoleUiVc::new(TransientInstance::new(LogOptions {
                    project_dir: PathBuf::from(self.project_dir),
                    current_dir: current_dir().unwrap(),
                    show_all: self.show_all,
                    log_detail: self.log_detail,
                    log_level: self.log_level,
                }))
                .into();

            handle_issues(build_result, issue_reporter, &None, &None).await?;

            Ok(NothingVc::new().into())
        });

        self.turbo_tasks.wait_task_completion(task, true).await?;

        Ok(())
    }
}

#[turbo_tasks::function]
async fn build_internal(
    project_dir: StringVc,
    root_dir: StringVc,
    entry_requests: EntryRequestsVc,
    browserslist_query: StringVc,
) -> Result<NothingVc> {
    let project_dir = &*project_dir.await?;
    let root_dir = &*root_dir.await?;
    let browserslist_query = &*browserslist_query.await?;

    let env = EnvironmentVc::new(Value::new(ExecutionEnvironment::Browser(
        BrowserEnvironment {
            dom: true,
            web_worker: false,
            service_worker: false,
            browserslist_query: browserslist_query.to_owned(),
        }
        .into(),
    )));
    let output_fs = output_fs(project_dir);
    let project_fs = project_fs(root_dir);
    let project_relative = project_dir.strip_prefix(root_dir).unwrap();
    let project_relative = project_relative
        .strip_prefix(MAIN_SEPARATOR)
        .unwrap_or(project_relative)
        .replace(MAIN_SEPARATOR, "/");
    let project_path = project_fs.root().join(&project_relative);
    let build_output_root = output_fs.root().join("dist");

    let chunking_context: ChunkingContextVc = BuildChunkingContextVc::builder(
        project_path,
        build_output_root,
        build_output_root,
        build_output_root,
        env,
    )
    .build()
    .into();

    let node_env = NodeEnv::Production.cell();
    // TODO: allow node environment via cli
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
    let execution_context =
        ExecutionContextVc::new(project_path, chunking_context, load_env(project_path));
    let context =
        get_client_asset_context(project_path, execution_context, compile_time_info, node_env);

    let entry_requests = (*entry_requests
        .await?
        .iter()
        .cloned()
        .map(|r| async move {
            Ok(match &*r.await? {
                EntryRequest::Relative(p) => {
                    RequestVc::relative(Value::new(p.clone().into()), false)
                }
                EntryRequest::Module(m, p) => {
                    RequestVc::module(m.clone(), Value::new(p.clone().into()), QueryMapVc::none())
                }
            })
        })
        .try_join()
        .await?)
        .to_vec();

    let origin = PlainResolveOriginVc::new(context, output_fs.root().join("_")).as_resolve_origin();

    let entries = entry_requests
        .into_iter()
        .map(|request_vc| async move {
            let ty = Value::new(ReferenceType::Entry(EntryReferenceSubType::Undefined));
            let request = request_vc.await?;
            Ok(*origin
                .resolve_asset(request_vc, origin.resolve_options(ty.clone()), ty)
                .primary_assets()
                .await?
                .first()
                .with_context(|| {
                    format!(
                        "Unable to resolve entry {} from directory {}.",
                        request.request().unwrap(),
                        project_dir
                    )
                })?)
        })
        .try_join()
        .await?;

    let modules = entries.into_iter().map(|entry| {
        context.process(
            entry,
            Value::new(ReferenceType::Entry(EntryReferenceSubType::Undefined)),
        )
    });

    let entry_chunk_groups = modules
        .map(|entry_module| async move {
            Ok(
                if let Some(ecmascript) =
                    EcmascriptModuleAssetVc::resolve_from(entry_module).await?
                {
                    AssetsVc::cell(vec![BuildChunkingContextVc::resolve_from(chunking_context)
                        .await?
                        .unwrap()
                        .generate_entry_chunk(
                            build_output_root
                                .join(
                                    entry_module
                                        .ident()
                                        .path()
                                        .file_stem()
                                        .await?
                                        .as_deref()
                                        .unwrap(),
                                )
                                .with_extension("entry.js"),
                            ecmascript.into(),
                            EvaluatableAssetsVc::one(ecmascript.into()),
                        )])
                } else if let Some(chunkable) =
                    ChunkableModuleVc::resolve_from(entry_module).await?
                {
                    chunking_context.chunk_group(chunkable.as_root_chunk(chunking_context))
                } else {
                    // TODO convert into a serve-able asset
                    bail!(
                        "Entry module is not chunkable, so it can't be used to bootstrap the \
                         application"
                    )
                },
            )
        })
        .try_join()
        .await?;

    for chunk_group in entry_chunk_groups {
        for entry in &*chunk_group.await? {
            for asset in &*all_assets_from_entry(entry.to_owned()).await? {
                asset.content().write(asset.ident().path()).await?;
            }
        }
    }

    Ok(NothingVc::new())
}

pub async fn build(args: &BuildArguments) -> Result<()> {
    let NormalizedDirs {
        project_dir,
        root_dir,
    } = normalize_dirs(&args.common.dir, &args.common.root)?;

    let tt = TurboTasks::new(MemoryBackend::new(
        args.common
            .memory_limit
            .map_or(usize::MAX, |l| l * 1024 * 1024),
    ));

    let mut builder = TurbopackBuildBuilder::new(tt, project_dir, root_dir)
        .log_detail(args.common.log_detail)
        .show_all(args.common.show_all)
        .log_level(
            args.common
                .log_level
                .map_or_else(|| IssueSeverity::Warning, |l| l.0),
        );

    for entry in normalize_entries(&args.common.entries) {
        builder = builder.entry_request(EntryRequest::Relative(entry));
    }

    builder.build().await?;

    Ok(())
}
