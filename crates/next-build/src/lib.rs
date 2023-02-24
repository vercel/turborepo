#![feature(future_join)]
#![feature(min_specialization)]

mod turbo_tasks_viz;

use std::{collections::HashSet, net::SocketAddr, path::MAIN_SEPARATOR};

use anyhow::{anyhow, Result};
use next_core::{
    app_structure::find_app_structure, create_app_source, create_page_source,
    create_web_entry_source, env::load_env, manifest::DevManifestContentSource,
    next_config::load_next_config, next_image::NextImageContentSourceVc,
    pages_structure::find_pages_structure, router_source::NextRouterContentSourceVc,
    source_map::NextSourceMapTraceContentSourceVc,
};
use turbo_tasks::{
    CollectiblesSource, CompletionsVc, RawVc, TransientInstance, TransientValue, TurboTasks, Value,
};
use turbo_tasks_fs::{DiskFileSystemVc, FileSystem, FileSystemVc};
use turbo_tasks_memory::MemoryBackend;
use turbopack_core::{
    environment::ServerAddr,
    issue::{IssueReporter, IssueReporterVc, IssueVc},
    resolve::{parse::RequestVc, pattern::QueryMapVc},
};
use turbopack_dev_server::{
    introspect::IntrospectionSource,
    source::{
        combined::CombinedContentSourceVc, router::RouterContentSource,
        source_maps::SourceMapContentSourceVc, static_assets::StaticAssetsContentSourceVc,
        ContentSourceVc,
    },
};
use turbopack_node::execution_context::ExecutionContextVc;

#[derive(Clone)]
pub enum EntryRequest {
    Relative(String),
    Module(String, String),
}

async fn handle_issues<T: Into<RawVc> + CollectiblesSource + Copy>(
    source: T,
    issue_reporter: IssueReporterVc,
) -> Result<()> {
    let issues = IssueVc::peek_issues_with_path(source)
        .await?
        .strongly_consistent()
        .await?;

    let has_fatal = issue_reporter.report_issues(
        TransientInstance::new(issues.clone()),
        TransientValue::new(source.into()),
    );

    if *has_fatal.await? {
        Err(anyhow!("Fatal issue(s) occurred"))
    } else {
        Ok(())
    }
}

#[turbo_tasks::function]
async fn project_fs(project_dir: &str, issue_reporter: IssueReporterVc) -> Result<FileSystemVc> {
    let disk_fs = DiskFileSystemVc::new("project".to_string(), project_dir.to_string());
    handle_issues(disk_fs, issue_reporter).await?;
    disk_fs.await?.start_watching()?;
    Ok(disk_fs.into())
}

#[turbo_tasks::function]
async fn output_fs(project_dir: &str, issue_reporter: IssueReporterVc) -> Result<FileSystemVc> {
    let disk_fs = DiskFileSystemVc::new("output".to_string(), project_dir.to_string());
    handle_issues(disk_fs, issue_reporter).await?;
    disk_fs.await?.start_watching()?;
    Ok(disk_fs.into())
}

#[allow(clippy::too_many_arguments)]
#[turbo_tasks::function]
pub async fn source(
    root_dir: String,
    project_dir: String,
    entry_requests: TransientInstance<Vec<EntryRequest>>,
    eager_compile: bool,
    turbo_tasks: TransientInstance<TurboTasks<MemoryBackend>>,
    issue_reporter: IssueReporterVc,
    browserslist_query: String,
    server_addr: TransientInstance<SocketAddr>,
    server_root_fs: FileSystemVc,
) -> Result<ContentSourceVc> {
    let output_fs = output_fs(&project_dir, issue_reporter);
    let fs = project_fs(&root_dir, issue_reporter);
    let server_root = server_root_fs.root();
    let project_relative = project_dir.strip_prefix(&root_dir).unwrap();
    let project_relative = project_relative
        .strip_prefix(MAIN_SEPARATOR)
        .unwrap_or(project_relative)
        .replace(MAIN_SEPARATOR, "/");
    let project_path = fs.root().join(&project_relative);

    let env = load_env(project_path);
    let build_output_root = output_fs.root().join(".next/build");

    let execution_context = ExecutionContextVc::new(project_path, build_output_root, env);

    let next_config = load_next_config(execution_context.join("next_config"));

    let output_root = output_fs.root().join(".next/server");
    let server_addr = ServerAddr::new(*server_addr).cell();

    let entry_requests = entry_requests
        .iter()
        .map(|r| match r {
            EntryRequest::Relative(p) => RequestVc::relative(Value::new(p.clone().into()), false),
            EntryRequest::Module(m, p) => {
                RequestVc::module(m.clone(), Value::new(p.clone().into()), QueryMapVc::none())
            }
        })
        .collect();

    let web_source = create_web_entry_source(
        project_path,
        execution_context,
        entry_requests,
        server_root,
        env,
        eager_compile,
        &browserslist_query,
        next_config,
    );
    let pages_structure = find_pages_structure(project_path, server_root, next_config);
    let page_source = create_page_source(
        pages_structure,
        project_path,
        execution_context,
        output_root.join("pages"),
        server_root,
        env,
        &browserslist_query,
        next_config,
        server_addr,
    );
    let app_structure = find_app_structure(project_path, server_root, next_config);
    let app_source = create_app_source(
        app_structure,
        project_path,
        execution_context,
        output_root.join("app"),
        server_root,
        env,
        &browserslist_query,
        next_config,
        server_addr,
    );
    let viz = turbo_tasks_viz::TurboTasksSource {
        turbo_tasks: turbo_tasks.into(),
    }
    .cell()
    .into();
    let static_source =
        StaticAssetsContentSourceVc::new(String::new(), project_path.join("public")).into();
    let manifest_source = DevManifestContentSource {
        page_roots: vec![app_source, page_source],
        next_config,
    }
    .cell()
    .into();
    let main_source = CombinedContentSourceVc::new(vec![
        manifest_source,
        static_source,
        app_source,
        page_source,
        web_source,
    ]);
    let introspect = IntrospectionSource {
        roots: HashSet::from([main_source.into()]),
    }
    .cell()
    .into();
    let main_source = main_source.into();
    let source_maps = SourceMapContentSourceVc::new(main_source).into();
    let source_map_trace = NextSourceMapTraceContentSourceVc::new(main_source).into();
    let img_source = NextImageContentSourceVc::new(
        CombinedContentSourceVc::new(vec![static_source, page_source]).into(),
    )
    .into();
    let router_source = NextRouterContentSourceVc::new(
        main_source,
        execution_context,
        next_config,
        server_addr,
        CompletionsVc::cell(vec![
            app_structure.routes_changed(),
            pages_structure.routes_changed(),
        ])
        .all(),
    )
    .into();
    let source = RouterContentSource {
        routes: vec![
            ("__turbopack__/".to_string(), introspect),
            ("__turbo_tasks__/".to_string(), viz),
            (
                "__nextjs_original-stack-frame".to_string(),
                source_map_trace,
            ),
            // TODO: Load path from next.config.js
            ("_next/image".to_string(), img_source),
            ("__turbopack_sourcemap__/".to_string(), source_maps),
        ],
        fallback: router_source,
    }
    .cell()
    .into();

    handle_issues(server_root_fs, issue_reporter).await?;
    handle_issues(web_source, issue_reporter).await?;
    handle_issues(page_source, issue_reporter).await?;

    Ok(source)
}

pub fn register() {
    next_core::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}

pub trait IssueReporterProvider: Send + Sync + 'static {
    fn get_issue_reporter(&self) -> IssueReporterVc;
}

impl<T> IssueReporterProvider for T
where
    T: Fn() -> IssueReporterVc + Send + Sync + Clone + 'static,
{
    fn get_issue_reporter(&self) -> IssueReporterVc {
        self()
    }
}
