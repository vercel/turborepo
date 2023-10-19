//! Module for summarizing and tracking a run.
//! We have two separate types of structs here: Trackers and Summaries.
//! A tracker tracks the live data and then gets turned into a summary for
//! displaying it We have this split because the tracker representation is not
//! exactly what we want to display to the user.
#[allow(dead_code)]
mod execution;
mod global_hash;
mod scm;
mod spaces;
mod task;

use std::{collections::HashSet, io, io::Write};

use chrono::{DateTime, Local};
pub use global_hash::GlobalHashSummary;
use itertools::Itertools;
use serde::Serialize;
use svix_ksuid::{Ksuid, KsuidLike};
use tabwriter::TabWriter;
use thiserror::Error;
use tracing::log::warn;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_api_client::{spaces::CreateSpaceRunPayload, APIAuth, APIClient};
use turborepo_env::EnvironmentVariableMap;
use turborepo_ui::{color, cprintln, cwriteln, BOLD, BOLD_CYAN, GREY, UI};

use self::execution::TaskTracker;
use super::task_id::TaskId;
use crate::{
    cli,
    opts::RunOpts,
    package_graph::{PackageGraph, WorkspaceName},
    run::summary::{
        execution::{ExecutionState, ExecutionSummary, ExecutionTracker},
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
    #[error("failed to parse environment variables")]
    EnvironmentVars(regex::Error),
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

// Can't reuse `cli::EnvMode` because the serialization
// is different (lowercase vs uppercase)
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvMode {
    Infer,
    Loose,
    Strict,
}

impl From<cli::EnvMode> for EnvMode {
    fn from(env_mode: cli::EnvMode) -> Self {
        match env_mode {
            cli::EnvMode::Infer => EnvMode::Infer,
            cli::EnvMode::Loose => EnvMode::Loose,
            cli::EnvMode::Strict => EnvMode::Strict,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary<'a> {
    id: Ksuid,
    version: String,
    turbo_version: &'static str,
    monorepo: bool,
    #[serde(rename = "globalCacheInputs")]
    global_hash_summary: GlobalHashSummary<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution: Option<ExecutionSummary<'a>>,
    packages: HashSet<WorkspaceName>,
    env_mode: EnvMode,
    framework_inference: bool,
    tasks: Vec<TaskSummary<'a>>,
    user: String,
    scm: SCMState,
    #[serde(skip)]
    repo_root: &'a AbsoluteSystemPath,
    #[serde(skip)]
    should_save: bool,
    #[serde(skip)]
    run_type: RunType,
    #[serde(skip)]
    spaces_client_handle: Option<SpacesClientHandle>,
}

/// We use this to track the run, so it's constructed before the run.
#[derive(Debug)]
pub struct RunTracker {
    scm: SCMState,
    version: &'static str,
    started_at: DateTime<Local>,
    execution_tracker: ExecutionTracker,
    spaces_client_handle: Option<SpacesClientHandle>,
    user: String,
}

impl RunTracker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        started_at: DateTime<Local>,
        synthesized_command: &str,
        package_inference_root: Option<&AnchoredSystemPath>,
        env_at_execution_start: &EnvironmentVariableMap,
        repo_root: &AbsoluteSystemPath,
        version: &'static str,
        spaces_id: Option<String>,
        spaces_api_client: APIClient,
        api_auth: Option<APIAuth>,
        user: String,
    ) -> Self {
        let scm = SCMState::get(env_at_execution_start, repo_root);

        let mut run_tracker = RunTracker {
            scm: scm.clone(),
            version,
            started_at,
            execution_tracker: ExecutionTracker::new(synthesized_command),
            spaces_client_handle: None,
            user: user.clone(),
        };

        if let Some(spaces_client) =
            SpacesClient::new(spaces_id.clone(), spaces_api_client, api_auth)
        {
            let payload = CreateSpaceRunPayload::new(
                started_at,
                synthesized_command,
                package_inference_root,
                scm.branch,
                scm.sha,
                version.to_string(),
                user,
            );
            run_tracker.spaces_client_handle = spaces_client.start(payload).ok();
        }

        run_tracker
    }

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(
        repo_root,
        package_inference_root,
        run_opts,
        packages,
        global_hash_summary
    ))]
    pub async fn to_summary<'a>(
        self,
        repo_root: &'a AbsoluteSystemPath,
        package_inference_root: Option<&'a AnchoredSystemPath>,
        exit_code: i32,
        end_time: DateTime<Local>,
        run_opts: &RunOpts<'a>,
        packages: HashSet<WorkspaceName>,
        global_hash_summary: GlobalHashSummary<'a>,
    ) -> Result<RunSummary<'a>, Error> {
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

        let execution_summary = self
            .execution_tracker
            .finish(package_inference_root, exit_code, self.started_at, end_time)
            .await?;

        Ok(RunSummary {
            id: Ksuid::new(None, None),
            version: RUN_SUMMARY_SCHEMA_VERSION.to_string(),
            turbo_version: self.version,
            packages,
            execution: Some(execution_summary),
            env_mode: run_opts.env_mode.into(),
            framework_inference: run_opts.framework_inference,
            tasks: vec![],
            global_hash_summary,
            scm: self.scm,
            user: self.user,
            monorepo: !single_package,
            repo_root,
            should_save,
            run_type,
            spaces_client_handle: self.spaces_client_handle,
        })
    }

    #[tracing::instrument(skip(pkg_dep_graph, ui))]
    #[allow(clippy::too_many_arguments)]
    pub async fn finish<'a>(
        self,
        exit_code: i32,
        pkg_dep_graph: &PackageGraph,
        ui: UI,
        repo_root: &'a AbsoluteSystemPath,
        package_inference_root: Option<&AnchoredSystemPath>,
        run_opts: &RunOpts<'a>,
        packages: HashSet<WorkspaceName>,
        global_hash_summary: GlobalHashSummary<'a>,
    ) -> Result<(), Error> {
        let end_time = Local::now();

        let run_summary: RunSummary = self
            .to_summary(
                repo_root,
                package_inference_root,
                exit_code,
                end_time,
                run_opts,
                packages,
                global_hash_summary,
            )
            .await?;

        run_summary
            .finish(end_time, exit_code, pkg_dep_graph, ui)
            .await
    }

    pub fn track_task(&self, task_id: TaskId<'static>) -> TaskTracker<()> {
        self.execution_tracker.task_tracker(task_id)
    }
}

// This is an exact copy of RunSummary, but the JSON tags are structured
// for rendering a single-package run of turbo. Notably, we want to always omit
// packages since there is no concept of packages in a single-workspace repo.
// This struct exists solely for the purpose of serializing to JSON and should
// not be used anywhere else.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SinglePackageRunSummary<'a, 'b> {
    id: Ksuid,
    version: &'b str,
    turbo_version: &'b str,
    monorepo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution: Option<&'b ExecutionSummary<'a>>,
    #[serde(rename = "globalCacheInputs")]
    global_hash_summary: &'b GlobalHashSummary<'a>,
    env_mode: EnvMode,
    framework_inference: bool,
    tasks: &'b [TaskSummary<'a>],
    user: &'b str,
    pub scm: &'b SCMState,
}

impl<'a, 'b> From<&'b RunSummary<'a>> for SinglePackageRunSummary<'a, 'b> {
    fn from(run_summary: &'b RunSummary<'a>) -> Self {
        SinglePackageRunSummary {
            id: run_summary.id,
            version: &run_summary.version,
            turbo_version: run_summary.turbo_version,
            monorepo: run_summary.monorepo,
            execution: run_summary.execution.as_ref(),
            global_hash_summary: &run_summary.global_hash_summary,
            env_mode: run_summary.env_mode,
            framework_inference: run_summary.framework_inference,
            tasks: &run_summary.tasks,
            user: &run_summary.user,
            scm: &run_summary.scm,
        }
    }
}

impl<'a> RunSummary<'a> {
    async fn finish(
        mut self,
        end_time: DateTime<Local>,
        exit_code: i32,
        pkg_dep_graph: &PackageGraph,
        ui: UI,
    ) -> Result<(), Error> {
        if matches!(self.run_type, RunType::DryJson | RunType::DryText) {
            self.close_dry_run(pkg_dep_graph, ui)?;
        }

        if self.should_save {
            if let Err(err) = self.save() {
                warn!("Error writing run summary: {}", err)
            }
        }

        if let Some(execution) = &self.execution {
            let path = self.get_path();
            let failed_tasks = self.get_failed_tasks();
            execution.print(ui, path, failed_tasks);
        }

        if let Some(spaces_client_handle) = self.spaces_client_handle.take() {
            println!("Sending to space");
            self.send_to_space(spaces_client_handle, end_time, exit_code)
                .await?;
        }

        Ok(())
    }

    async fn send_to_space(
        &self,
        spaces_client_handle: SpacesClientHandle,
        ended_at: DateTime<Local>,
        exit_code: i32,
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

        if self.monorepo {
            println!("\n{}", color!(ui, BOLD_CYAN, "Packages in Scope"));
            let mut tab_writer = TabWriter::new(io::stdout());
            writeln!(tab_writer, "Name\tPath")?;
            for pkg in &self.packages {
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

        let file_count = self.global_hash_summary.files.len();

        let mut tab_writer = TabWriter::new(io::stdout());
        cprintln!(ui, BOLD_CYAN, "\nGlobal Hash Inputs");
        cwriteln!(tab_writer, ui, GREY, "  Global Files\t=\t{}", file_count)?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  External Dependencies Hash\t=\t{}",
            self.global_hash_summary.hash_of_external_dependencies
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global Cache Key\t=\t{}",
            self.global_hash_summary.root_key
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global .env Files considered\t=\t{}",
            self.global_hash_summary
                .global_dot_env
                .unwrap_or_default()
                .len()
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global Env Vars\t=\t{}",
            self.global_hash_summary
                .environment_variables
                .specified
                .env
                .join(", ")
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global Env Vars Values\t=\t{}",
            self.global_hash_summary
                .environment_variables
                .configured
                .as_deref()
                .unwrap_or_default()
                .join(", ")
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Inferred Global Env Vars Values\t=\t{}",
            self.global_hash_summary
                .environment_variables
                .inferred
                .as_deref()
                .unwrap_or_default()
                .join(", ")
        )?;

        tab_writer.flush()?;

        for task in &self.tasks {
            if self.monorepo {
                cprintln!(ui, BOLD, "{}", task.task_id);
            } else {
                cprintln!(ui, BOLD, "{}", task.task_id.task());
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

        if self.monorepo {
            Ok(serde_json::to_string_pretty(&self)?)
        } else {
            let monorepo_rsm: SinglePackageRunSummary<'a, '_> = (&*self).into();
            Ok(serde_json::to_string_pretty(&monorepo_rsm)?)
        }
    }

    fn normalize(&mut self) {
        // Remove execution summary for dry runs
        if matches!(self.run_type, RunType::DryJson) {
            self.execution = None;
        }

        // For single packages, we don't need the packages
        // and each task summary needs some cleaning
        if !self.monorepo {
            self.packages.drain();

            for task_summary in &mut self.tasks {
                task_summary.clean_for_single_package();
            }
        }

        self.tasks.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    }

    fn get_path(&self) -> AbsoluteSystemPathBuf {
        let filename = format!("{}.json", self.id);

        self.repo_root
            .join_components(&[".turbo", "runs", &filename])
    }

    fn get_failed_tasks(&self) -> Vec<&TaskSummary<'a>> {
        self.tasks
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
