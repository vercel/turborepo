#[allow(dead_code)]
mod execution;
mod global_hash;
mod scm;
mod spaces;
mod task;

use std::{collections::HashSet, io, io::Write};

use chrono::Local;
pub use global_hash::GlobalHashSummary;
use itertools::Itertools;
use serde::Serialize;
use svix_ksuid::{Ksuid, KsuidLike};
use tabwriter::TabWriter;
use thiserror::Error;
use tracing::log::warn;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_api_client::{spaces::CreateSpaceRunPayload, APIAuth, APIClient};
use turborepo_ci::Vendor;
use turborepo_env::EnvironmentVariableMap;
use turborepo_ui::{color, cprintln, cwriteln, BOLD, BOLD_CYAN, GREY, UI};

use crate::{
    cli::EnvMode,
    opts::RunOpts,
    package_graph::{PackageGraph, WorkspaceName},
    run::summary::{
        execution::{ExecutionState, ExecutionSummary},
        scm::SCMState,
        spaces::{SpaceRequest, SpacesClient, SpacesClientHandle},
        task::TaskSummary,
    },
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to write run summary {0}")]
    IO(#[from] io::Error),
    #[error("failed to serialize run summary to JSON")]
    Serde(#[from] serde_json::Error),
    #[error("missing workspace {0}")]
    MissingWorkspace(WorkspaceName),
    #[error("request took too long to resolve: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("failed to send spaces request: {0}")]
    SpacesRequest(#[from] turborepo_api_client::Error),
    #[error("failed to close spaces client")]
    SpacesClientClose(#[from] tokio::task::JoinError),
    #[error("failed to contact spaces client")]
    SpacesClientSend(#[from] tokio::sync::mpsc::error::SendError<SpaceRequest>),
}

// NOTE: When changing this, please ensure that the server side is updated to
// handle the new version on vercel.com this is required to ensure safe handling
// of env vars (unknown run summary versions will be ignored on the server)
const RUN_SUMMARY_SCHEMA_VERSION: &str = "1";

#[derive(Debug)]
enum RunType {
    Real,
    DryText,
    DryJson,
}

fn get_user(env_vars: &EnvironmentVariableMap) -> Option<String> {
    if turborepo_ci::is_ci() {
        return Vendor::get_info()
            .and_then(|vendor| vendor.username_env_var)
            .and_then(|username_env_var| env_vars.get(username_env_var).cloned());
    }

    None
}

// Wrapper around the serializable RunSummaryInner, with some extra information
// about the Run and references to other things that we need.
#[derive(Debug)]
pub struct RunSummary<'a> {
    inner: RunSummaryInner<'a>,
    repo_root: &'a AbsoluteSystemPath,
    package_inference_root: Option<&'a AnchoredSystemPath>,
    single_package: bool,
    should_save: bool,
    spaces_client_handle: Option<SpacesClientHandle>,
    run_type: RunType,
    synthesized_command: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryInner<'a> {
    id: Ksuid,
    version: String,
    turbo_version: String,
    monorepo: bool,
    #[serde(rename = "globalCacheInputs")]
    global_hash_summary: GlobalHashSummary<'a>,
    packages: HashSet<WorkspaceName>,
    env_mode: EnvMode,
    framework_inference: bool,
    #[serde(rename = "execution", skip_serializing_if = "Option::is_none")]
    execution_summary: Option<ExecutionSummary<'a>>,
    tasks: Vec<TaskSummary<'a>>,
    user: Option<String>,
    pub scm: SCMState,
}

// This is an exact copy of RunSummary, but the JSON tags are structured
// for rendering a single-package run of turbo. Notably, we want to always omit
// packages since there is no concept of packages in a single-workspace repo.
// This struct exists solely for the purpose of serializing to JSON and should
// not be used anywhere else.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NonMonorepoRunSummary<'a, 'b> {
    id: Ksuid,
    version: &'b str,
    turbo_version: &'b str,
    monorepo: bool,
    #[serde(rename = "globalCacheInputs")]
    global_hash_summary: &'b GlobalHashSummary<'a>,
    env_mode: EnvMode,
    framework_inference: bool,
    #[serde(rename = "execution", skip_serializing_if = "Option::is_none")]
    execution_summary: Option<&'b ExecutionSummary<'a>>,
    tasks: &'b [TaskSummary<'a>],
    user: Option<&'b str>,
    pub scm: &'b SCMState,
}

impl<'a, 'b> From<&'b RunSummaryInner<'a>> for NonMonorepoRunSummary<'a, 'b> {
    fn from(rsm: &'b RunSummaryInner<'a>) -> Self {
        NonMonorepoRunSummary {
            id: rsm.id,
            version: &rsm.version,
            turbo_version: &rsm.turbo_version,
            monorepo: rsm.monorepo,
            global_hash_summary: &rsm.global_hash_summary,
            env_mode: rsm.env_mode,
            framework_inference: rsm.framework_inference,
            execution_summary: rsm.execution_summary.as_ref(),
            tasks: &rsm.tasks,
            user: rsm.user.as_deref(),
            scm: &rsm.scm,
        }
    }
}

impl<'a> RunSummary<'a> {
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(
        repo_root,
        package_inference_root,
        run_opts,
        packages,
        env_at_execution_start,
        global_hash_summary,
        synthesized_command
    ))]
    pub async fn new(
        start_at: chrono::DateTime<Local>,
        repo_root: &'a AbsoluteSystemPath,
        package_inference_root: Option<&'a AnchoredSystemPath>,
        turbo_version: &'static str,
        spaces_api_client: APIClient,
        run_opts: &RunOpts<'a>,
        packages: HashSet<WorkspaceName>,
        env_at_execution_start: EnvironmentVariableMap,
        global_hash_summary: GlobalHashSummary<'a>,
        synthesized_command: String,
        api_auth: Option<APIAuth>,
    ) -> RunSummary<'a> {
        let single_package = run_opts.single_package;
        let should_save = run_opts.summarize.flatten().is_some_and(|s| s);

        let run_type = if run_opts.dry_run {
            if run_opts.dry_run_json {
                RunType::DryJson
            } else {
                RunType::DryText
            }
        } else {
            RunType::Real
        };

        let execution_summary = ExecutionSummary::new(
            synthesized_command.clone(),
            package_inference_root,
            start_at,
        );

        let mut rsm = RunSummary {
            inner: RunSummaryInner {
                id: Ksuid::new(None, None),
                version: RUN_SUMMARY_SCHEMA_VERSION.to_string(),
                execution_summary: Some(execution_summary),
                turbo_version: turbo_version.to_string(),
                packages,
                env_mode: run_opts.env_mode,
                framework_inference: run_opts.framework_inference,
                tasks: vec![],
                global_hash_summary,
                scm: SCMState::get(&env_at_execution_start, repo_root),
                user: get_user(&env_at_execution_start),
                monorepo: !single_package,
            },
            run_type,
            repo_root,
            package_inference_root,
            single_package,
            should_save,
            spaces_client_handle: None,

            synthesized_command,
        };

        if let Some(spaces_client) = SpacesClient::new(
            run_opts.experimental_space_id.clone(),
            spaces_api_client,
            api_auth,
        ) {
            let payload = CreateSpaceRunPayload::from(&rsm);
            if let Ok(handle) = spaces_client.start(payload).await {
                rsm.spaces_client_handle = Some(handle)
            }
        }

        rsm
    }

    #[tracing::instrument(skip(pkg_dep_graph, ui))]
    pub async fn close(
        &mut self,
        exit_code: u32,
        pkg_dep_graph: &PackageGraph,
        ui: UI,
    ) -> Result<(), Error> {
        if matches!(self.run_type, RunType::DryJson | RunType::DryText) {
            self.close_dry_run(pkg_dep_graph, ui)?;
        }

        let end_time = Local::now();

        if let Some(execution_summary) = &mut self.inner.execution_summary {
            execution_summary.exit_code = Some(exit_code);
            execution_summary.end_time = Some(end_time);
        }

        if self.should_save {
            if let Err(err) = self.save() {
                warn!("Error writing run summary: {}", err)
            }
        }

        if let Some(execution_summary) = &self.inner.execution_summary {
            let path = self.get_path();
            let failed_tasks = self.get_failed_tasks();
            execution_summary.print(ui, path, failed_tasks);
        }

        if let Some(spaces_client_handle) = self.spaces_client_handle.take() {
            self.send_to_space(spaces_client_handle, end_time, exit_code)
                .await?;
        }

        Ok(())
    }

    async fn send_to_space(
        &self,
        spaces_client_handle: SpacesClientHandle,
        ended_at: chrono::DateTime<Local>,
        exit_code: u32,
    ) -> Result<(), Error> {
        let spinner = turborepo_ui::start_spinner("...sending run summary...");

        spaces_client_handle.finish_run(exit_code, ended_at).await?;

        let result = spaces_client_handle.close().await;

        spinner.finish_and_clear();

        Self::print_errors(&result.errors);

        if let Some(run) = result.run {
            println!("Run: {}\n", run.url);
        }

        Ok(())
    }

    fn print_errors(errors: &[Error]) {
        if errors.is_empty() {
            return;
        }

        for error in errors {
            warn!("{}", error)
        }
    }

    fn close_dry_run(&mut self, pkg_dep_graph: &PackageGraph, ui: UI) -> Result<(), Error> {
        if matches!(self.run_type, RunType::DryJson) {
            let rendered = self.format_json()?;

            println!("{}", rendered);
            return Ok(());
        }

        self.format_and_print_text(pkg_dep_graph, ui)
    }

    fn format_and_print_text(&mut self, pkg_dep_graph: &PackageGraph, ui: UI) -> Result<(), Error> {
        self.normalize();

        if !self.single_package {
            println!("\n{}", color!(ui, BOLD_CYAN, "Packages in Scope"));
            let mut tab_writer = TabWriter::new(io::stdout());
            writeln!(tab_writer, "Name\tPath")?;
            for pkg in &self.inner.packages {
                if matches!(pkg, WorkspaceName::Root) {
                    continue;
                }
                let dir = pkg_dep_graph
                    .workspace_info(pkg)
                    .ok_or_else(|| Error::MissingWorkspace(pkg.clone()))?
                    .package_path();

                writeln!(tab_writer, "{}\t{}", pkg, dir)?;
                tab_writer.flush()?;
            }
        }

        let file_count = self.inner.global_hash_summary.global_file_hash_map.len();

        let mut tab_writer = TabWriter::new(io::stdout());
        cprintln!(ui, BOLD_CYAN, "\nGlobal Hash Inputs");
        cwriteln!(tab_writer, ui, GREY, "  Global Files\t=\t{}", file_count)?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  External Dependencies Hash\t=\t{}",
            self.inner.global_hash_summary.root_external_deps_hash
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global Cache Key\t=\t{}",
            self.inner.global_hash_summary.global_cache_key
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global .env Files considered\t=\t{}",
            self.inner.global_hash_summary.dot_env.len()
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global Env Vars\t=\t{}",
            self.inner
                .global_hash_summary
                .env_vars
                .specified
                .env
                .join(", ")
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global Env Vars Values\t=\t{}",
            self.inner
                .global_hash_summary
                .env_vars
                .configured
                .join(", ")
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Inferred Global Env Vars Values\t=\t{}",
            self.inner.global_hash_summary.env_vars.inferred.join(", ")
        )?;

        tab_writer.flush()?;

        for task in &self.inner.tasks {
            if self.single_package {
                cprintln!(ui, BOLD, "{}", task.task_id.task());
            } else {
                cprintln!(ui, BOLD, "{}", task.task_id);
            };

            let mut tab_writer = TabWriter::new(io::stdout());
            cwriteln!(tab_writer, ui, GREY, "  Task\t=\t{}", task.task_id)?;

            if let Some(package) = &task.package {
                cwriteln!(tab_writer, ui, GREY, "  Package\t=\t{}", package)?;
            }

            cwriteln!(tab_writer, ui, GREY, "  Command\t=\t{}", task.command)?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Outputs\t=\t{}",
                task.outputs.join(", ")
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Log File\t=\t{}",
                task.log_file_relative_path
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Dependencies\t=\t{}",
                task.dependencies.iter().join(", ")
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Dependents\t=\t{}",
                task.dependents.iter().join(", ")
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Inputs Files Considered\t=\t{}",
                task.expanded_inputs.len()
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  .env Files Considered\t=\t{}",
                task.dot_env.len()
            )?;

            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Env Vars\t=\t{}",
                task.env_vars.specified.env.join(", ")
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Env Vars Values\t=\t{}",
                task.env_vars.configured.join(", ")
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Inferred Env Vars Values\t=\t{}",
                task.env_vars.inferred.join(", ")
            )?;

            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Passed Through Env Vars\t=\t{}",
                task.env_vars.specified.pass_through_env.join(", ")
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Passed Through Env Vars Values\t=\t{}",
                task.env_vars.pass_through.join(", ")
            )?;

            // If there's an error, we can silently ignore it, we don't need to block the
            // entire print.
            if let Ok(task_definition_json) = serde_json::to_string(&task.resolved_task_definition)
            {
                cwriteln!(
                    tab_writer,
                    ui,
                    GREY,
                    "  Task Definition\t=\t{}",
                    task_definition_json
                )?;
            }
        }

        Ok(())
    }

    fn format_json(&mut self) -> Result<String, Error> {
        self.normalize();

        if self.single_package {
            let monorepo_rsm: NonMonorepoRunSummary<'a, '_> = (&self.inner).into();
            Ok(serde_json::to_string_pretty(&monorepo_rsm)?)
        } else {
            Ok(serde_json::to_string_pretty(&self.inner)?)
        }
    }

    fn normalize(&mut self) {
        // Remove execution summary for dry runs
        if matches!(self.run_type, RunType::DryJson) {
            self.inner.execution_summary = None;
        }

        // For single packages, we don't need the packages
        // and each task summary needs some cleaning
        if self.single_package {
            self.inner.packages.drain();

            for task_summary in &mut self.inner.tasks {
                task_summary.clean_for_single_package();
            }
        }

        self.inner.tasks.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    }

    fn get_path(&self) -> AbsoluteSystemPathBuf {
        let filename = format!("{}.json", self.inner.id);

        self.repo_root
            .join_components(&[".turbo", "runs", &filename])
    }

    fn get_failed_tasks(&self) -> Vec<&TaskSummary<'a>> {
        self.inner
            .tasks
            .iter()
            .filter(|task| matches!(task.execution.state, ExecutionState::BuildFailed { .. }))
            .collect()
    }

    fn save(&mut self) -> Result<(), Error> {
        let json = self.format_json()?;

        let summary_path = self.get_path();
        summary_path.ensure_dir()?;

        Ok(summary_path.create_with_contents(json)?)
    }
}
