//! Module for summarizing and tracking a run.
//! We have two separate types of structs here: Trackers and Summaries.
//! A tracker tracks the live data and then gets turned into a summary for
//! displaying it We have this split because the tracker representation is not
//! exactly what we want to display to the user.
#[allow(dead_code)]
mod duration;
mod execution;
mod global_hash;
mod scm;
mod spaces;
mod task;
mod task_factory;
use std::{collections::HashSet, io, io::Write};

use chrono::{DateTime, Local};
pub use duration::TurboDuration;
pub use execution::{TaskExecutionSummary, TaskTracker};
pub use global_hash::GlobalHashSummary;
use itertools::Itertools;
use serde::Serialize;
pub use spaces::{SpacesTaskClient, SpacesTaskInformation};
use svix_ksuid::{Ksuid, KsuidLike};
use tabwriter::TabWriter;
use thiserror::Error;
use tracing::{error, log::warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_api_client::{spaces::CreateSpaceRunPayload, APIAuth, APIClient};
use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_scm::SCM;
use turborepo_ui::{color, cprintln, cwriteln, ColorConfig, BOLD, BOLD_CYAN, GREY};

use self::{
    execution::TaskState, task::SinglePackageTaskSummary, task_factory::TaskSummaryFactory,
};
use super::task_id::TaskId;
use crate::{
    cli,
    cli::{DryRunMode, EnvMode},
    engine::Engine,
    opts::RunOpts,
    run::summary::{
        execution::{ExecutionSummary, ExecutionTracker},
        scm::SCMState,
        spaces::{SpaceRequest, SpacesClient, SpacesClientHandle},
        task::TaskSummary,
    },
    task_hash::TaskHashTracker,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to write run summary {0}")]
    IO(#[from] io::Error),
    #[error("failed to serialize run summary to JSON")]
    Serde(#[from] serde_json::Error),
    #[error("missing workspace {0}")]
    MissingWorkspace(PackageName),
    #[error("request took too long to resolve: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("failed to send spaces request: {0}")]
    SpacesRequest(#[from] turborepo_api_client::Error),
    #[error("failed to close spaces client")]
    SpacesClientClose(#[from] tokio::task::JoinError),
    #[error("failed to contact spaces client")]
    SpacesClientSend(#[from] tokio::sync::mpsc::error::SendError<SpaceRequest>),
    #[error("failed to parse environment variables")]
    Env(#[source] turborepo_env::Error),
    #[error("failed to construct task summary: {0}")]
    TaskSummary(#[from] task_factory::Error),
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
    packages: Vec<&'a PackageName>,
    env_mode: EnvMode,
    framework_inference: bool,
    tasks: Vec<TaskSummary>,
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
    synthesized_command: String,
}

impl RunTracker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        started_at: DateTime<Local>,
        synthesized_command: String,
        package_inference_root: Option<&AnchoredSystemPath>,
        env_at_execution_start: &EnvironmentVariableMap,
        repo_root: &AbsoluteSystemPath,
        version: &'static str,
        spaces_id: Option<String>,
        spaces_api_client: APIClient,
        api_auth: Option<APIAuth>,
        user: String,
        scm: &SCM,
    ) -> Self {
        let scm = SCMState::get(env_at_execution_start, scm, repo_root);

        let spaces_client_handle =
            SpacesClient::new(spaces_id.clone(), spaces_api_client, api_auth).and_then(
                |spaces_client| {
                    let payload = CreateSpaceRunPayload::new(
                        started_at,
                        synthesized_command.clone(),
                        package_inference_root,
                        scm.branch.clone(),
                        scm.sha.clone(),
                        version.to_string(),
                        user.clone(),
                    );
                    spaces_client.start(payload).ok()
                },
            );

        RunTracker {
            scm,
            version,
            started_at,
            execution_tracker: ExecutionTracker::new(),
            user,
            synthesized_command,
            spaces_client_handle,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(
        repo_root,
        package_inference_root,
        run_opts,
        packages,
        global_hash_summary,
        task_factory,
    ))]
    pub async fn to_summary<'a>(
        self,
        repo_root: &'a AbsoluteSystemPath,
        package_inference_root: Option<&'a AnchoredSystemPath>,
        exit_code: i32,
        end_time: DateTime<Local>,
        run_opts: &'a RunOpts,
        packages: &'a HashSet<PackageName>,
        global_hash_summary: GlobalHashSummary<'a>,
        global_env_mode: EnvMode,
        task_factory: TaskSummaryFactory<'a>,
    ) -> Result<RunSummary<'a>, Error> {
        let single_package = run_opts.single_package;
        let should_save = run_opts.summarize.flatten().is_some_and(|s| s);

        let run_type = match run_opts.dry_run {
            None => RunType::Real,
            Some(DryRunMode::Json) => RunType::DryJson,
            Some(DryRunMode::Text) => RunType::DryText,
        };

        let summary_state = self.execution_tracker.finish().await?;

        let tasks = summary_state
            .tasks
            .iter()
            .cloned()
            .map(|TaskState { task_id, execution }| task_factory.task_summary(task_id, execution))
            .collect::<Result<Vec<_>, task_factory::Error>>()?;
        let execution_summary = ExecutionSummary::new(
            self.synthesized_command.clone(),
            summary_state,
            package_inference_root,
            exit_code,
            self.started_at,
            end_time,
        );

        Ok(RunSummary {
            id: Ksuid::new(None, None),
            version: RUN_SUMMARY_SCHEMA_VERSION.to_string(),
            turbo_version: self.version,
            packages: packages.iter().sorted().collect(),
            execution: Some(execution_summary),
            env_mode: global_env_mode,
            framework_inference: run_opts.framework_inference,
            tasks,
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

    #[tracing::instrument(skip(
        pkg_dep_graph,
        ui,
        run_opts,
        packages,
        global_hash_summary,
        engine,
        hash_tracker,
        env_at_execution_start
    ))]
    #[allow(clippy::too_many_arguments)]
    pub async fn finish<'a>(
        self,
        exit_code: i32,
        pkg_dep_graph: &PackageGraph,
        ui: ColorConfig,
        repo_root: &'a AbsoluteSystemPath,
        package_inference_root: Option<&AnchoredSystemPath>,
        run_opts: &'a RunOpts,
        packages: &'a HashSet<PackageName>,
        global_hash_summary: GlobalHashSummary<'a>,
        global_env_mode: cli::EnvMode,
        engine: &'a Engine,
        hash_tracker: TaskHashTracker,
        env_at_execution_start: &'a EnvironmentVariableMap,
        is_watch: bool,
    ) -> Result<(), Error> {
        let end_time = Local::now();

        let task_factory = TaskSummaryFactory::new(
            pkg_dep_graph,
            engine,
            hash_tracker,
            env_at_execution_start,
            run_opts,
            global_env_mode,
        );

        let run_summary: RunSummary = self
            .to_summary(
                repo_root,
                package_inference_root,
                exit_code,
                end_time,
                run_opts,
                packages,
                global_hash_summary,
                global_env_mode,
                task_factory,
            )
            .await?;

        run_summary
            .finish(end_time, exit_code, pkg_dep_graph, ui, is_watch)
            .await
    }

    pub fn track_task(&self, task_id: TaskId<'static>) -> TaskTracker<()> {
        self.execution_tracker.task_tracker(task_id)
    }

    pub fn spaces_enabled(&self) -> bool {
        self.spaces_client_handle.is_some()
    }

    pub fn spaces_task_client(&self) -> Option<SpacesTaskClient> {
        self.spaces_client_handle
            .as_ref()
            .map(|handle| handle.task_client())
    }
}

// This is an exact copy of RunSummary, but the JSON tags are structured
// for rendering a single-package run of turbo. Notably, we want to always omit
// packages since there is no concept of packages in a single-workspace repo.
// This struct exists solely for the purpose of serializing to JSON and should
// not be used anywhere else.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SinglePackageRunSummary<'a> {
    id: Ksuid,
    version: &'a str,
    turbo_version: &'a str,
    monorepo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution: Option<&'a ExecutionSummary<'a>>,
    #[serde(rename = "globalCacheInputs")]
    global_hash_summary: &'a GlobalHashSummary<'a>,
    env_mode: EnvMode,
    framework_inference: bool,
    tasks: Vec<SinglePackageTaskSummary>,
    user: &'a str,
    pub scm: &'a SCMState,
}

impl<'a> From<&'a RunSummary<'a>> for SinglePackageRunSummary<'a> {
    fn from(run_summary: &'a RunSummary<'a>) -> Self {
        let mut tasks = run_summary
            .tasks
            .iter()
            .cloned()
            .map(SinglePackageTaskSummary::from)
            .collect::<Vec<_>>();
        tasks.sort_by(|t1, t2| t1.task_id.cmp(&t2.task_id));
        SinglePackageRunSummary {
            id: run_summary.id,
            version: &run_summary.version,
            turbo_version: run_summary.turbo_version,
            monorepo: run_summary.monorepo,
            execution: run_summary.execution.as_ref(),
            global_hash_summary: &run_summary.global_hash_summary,
            env_mode: run_summary.env_mode,
            framework_inference: run_summary.framework_inference,
            tasks,
            user: &run_summary.user,
            scm: &run_summary.scm,
        }
    }
}

impl<'a> RunSummary<'a> {
    #[tracing::instrument(skip(self, pkg_dep_graph, ui))]
    async fn finish(
        mut self,
        end_time: DateTime<Local>,
        exit_code: i32,
        pkg_dep_graph: &PackageGraph,
        ui: ColorConfig,
        is_watch: bool,
    ) -> Result<(), Error> {
        if matches!(self.run_type, RunType::DryJson | RunType::DryText) {
            return self.close_dry_run(pkg_dep_graph, ui);
        }

        if self.should_save {
            if let Err(err) = self.save() {
                warn!("Error writing run summary: {}", err)
            }
        }

        if !is_watch {
            if let Some(execution) = &self.execution {
                let path = self.get_path();
                let failed_tasks = self.get_failed_tasks();
                execution.print(ui, path, failed_tasks);
            }
        }

        if let Some(spaces_client_handle) = self.spaces_client_handle.take() {
            self.send_to_space(spaces_client_handle, end_time, exit_code)
                .await;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn send_to_space(
        &self,
        spaces_client_handle: SpacesClientHandle,
        ended_at: DateTime<Local>,
        exit_code: i32,
    ) {
        let spinner = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            turborepo_ui::start_spinner("...sending run summary...");
        });

        // We log the error here but don't fail because
        // failing to send the space shouldn't fail the run.
        if let Err(err) = spaces_client_handle.finish_run(exit_code, ended_at).await {
            warn!("Error sending to space: {}", err);
        };

        let result = spaces_client_handle.close().await;

        spinner.abort();

        Self::print_errors(&result.errors);

        if let Some(run) = result.run {
            println!("Run: {}\n", run.url);
        }
    }

    fn print_errors(errors: &[Error]) {
        if errors.is_empty() {
            return;
        }

        for error in errors {
            warn!("{}", error)
        }
    }

    fn close_dry_run(
        &mut self,
        pkg_dep_graph: &PackageGraph,
        ui: ColorConfig,
    ) -> Result<(), Error> {
        if matches!(self.run_type, RunType::DryJson) {
            let rendered = self.format_json()?;

            println!("{}", rendered);
            return Ok(());
        }

        self.format_and_print_text(pkg_dep_graph, ui)
    }

    fn format_and_print_text(
        &mut self,
        pkg_dep_graph: &PackageGraph,
        ui: ColorConfig,
    ) -> Result<(), Error> {
        self.normalize();

        if self.monorepo {
            println!("\n{}", color!(ui, BOLD_CYAN, "Packages in Scope"));
            let mut tab_writer = TabWriter::new(io::stdout()).minwidth(0).padding(1);
            writeln!(tab_writer, "Name\tPath\t")?;
            for pkg in &self.packages {
                if matches!(pkg, PackageName::Root) {
                    continue;
                }
                let dir = pkg_dep_graph
                    .package_info(pkg)
                    .ok_or_else(|| Error::MissingWorkspace((*pkg).to_owned()))?
                    .package_path();

                writeln!(tab_writer, "{}\t{}", pkg, dir)?;
            }
            tab_writer.flush()?;
        }

        let file_count = self.global_hash_summary.files.len();

        let mut tab_writer = TabWriter::new(io::stdout()).minwidth(0).padding(1);
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
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global Passed Through Env Vars\t=\t{}",
            self.global_hash_summary
                .environment_variables
                .specified
                .pass_through_env
                .unwrap_or_default()
                .join(", ")
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Global Passed Through Env Vars Values\t=\t{}",
            self.global_hash_summary
                .environment_variables
                .pass_through
                .as_deref()
                .unwrap_or_default()
                .join(", ")
        )?;
        cwriteln!(
            tab_writer,
            ui,
            GREY,
            "  Engines Values\t=\t{}",
            self.global_hash_summary
                .engines
                .as_ref()
                .map(|engines| engines
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .join(", "))
                .unwrap_or_default()
        )?;

        tab_writer.flush()?;
        println!();
        cprintln!(ui, BOLD_CYAN, "Tasks to Run");

        for task in &self.tasks {
            if self.monorepo {
                cprintln!(ui, BOLD, "{}", task.task_id);
            } else {
                cprintln!(ui, BOLD, "{}", task.task_id.task());
            };

            let mut tab_writer = TabWriter::new(io::stdout()).padding(1).minwidth(0);
            cwriteln!(tab_writer, ui, GREY, "  Task\t=\t{}", task.task)?;
            if self.monorepo {
                cwriteln!(tab_writer, ui, GREY, "  Package\t=\t{}", &task.package)?;
            }
            cwriteln!(tab_writer, ui, GREY, "  Hash\t=\t{}", &task.shared.hash)?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Cached (Local)\t=\t{}",
                &task.shared.cache.local
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Cached (Remote)\t=\t{}",
                &task.shared.cache.remote
            )?;

            if self.monorepo {
                if let Some(directory) = &task.shared.directory {
                    cwriteln!(tab_writer, ui, GREY, "  Directory\t=\t{}", directory)?;
                }
            }
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Command\t=\t{}",
                task.shared.command
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Outputs\t=\t{}",
                task.shared
                    .outputs
                    .as_ref()
                    .map_or_else(String::new, |outputs| outputs.join(", "))
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Log File\t=\t{}",
                task.shared.log_file
            )?;

            let dependencies = if !self.monorepo {
                task.shared
                    .dependencies
                    .iter()
                    .map(|dep| dep.task())
                    .join(", ")
            } else {
                task.shared.dependencies.iter().join(", ")
            };

            cwriteln!(tab_writer, ui, GREY, "  Dependencies\t=\t{}", dependencies)?;

            let dependents = if !self.monorepo {
                task.shared
                    .dependents
                    .iter()
                    .map(|dep| dep.task())
                    .join(", ")
            } else {
                task.shared.dependents.iter().join(", ")
            };

            cwriteln!(tab_writer, ui, GREY, "  Dependents\t=\t{}", dependents)?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Inputs Files Considered\t=\t{}",
                task.shared.inputs.len()
            )?;

            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Env Vars\t=\t{}",
                task.shared.environment_variables.specified.env.join(", ")
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Env Vars Values\t=\t{}",
                task.shared.environment_variables.configured.join(", ")
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Inferred Env Vars Values\t=\t{}",
                task.shared.environment_variables.inferred.join(", ")
            )?;

            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Passed Through Env Vars\t=\t{}",
                task.shared
                    .environment_variables
                    .specified
                    .pass_through_env
                    .as_ref()
                    .map_or_else(String::new, |pass_through_env| pass_through_env.join(", "))
            )?;
            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Passed Through Env Vars Values\t=\t{}",
                task.shared
                    .environment_variables
                    .pass_through
                    .as_ref()
                    .map_or_else(String::new, |vars| vars.join(", "))
            )?;

            // If there's an error, we can silently ignore it, we don't need to block the
            // entire print.
            if let Ok(task_definition_json) =
                serde_json::to_string(&task.shared.resolved_task_definition)
            {
                cwriteln!(
                    tab_writer,
                    ui,
                    GREY,
                    "  Resolved Task Definition\t=\t{}",
                    task_definition_json
                )?;
            }

            cwriteln!(
                tab_writer,
                ui,
                GREY,
                "  Framework\t=\t{}",
                task.shared.framework
            )?;

            tab_writer.flush()?;
        }

        Ok(())
    }

    fn format_json(&mut self) -> Result<String, Error> {
        self.normalize();

        let mut rendered_json = if self.monorepo {
            serde_json::to_string_pretty(&self)
        } else {
            // Deref coercion used to get an immutable reference from the mutable reference.
            let monorepo_rsm = SinglePackageRunSummary::from(&*self);
            serde_json::to_string_pretty(&monorepo_rsm)
        }?;
        // Go produces an extra newline at the end of the JSON
        rendered_json.push('\n');
        Ok(rendered_json)
    }

    fn normalize(&mut self) {
        // Remove execution summary for dry runs
        if matches!(self.run_type, RunType::DryJson) {
            self.execution = None;
        }

        // For single packages, we don't need the packages
        // and each task summary needs some cleaning
        if !self.monorepo {
            self.packages.clear();
        }

        self.tasks.sort_by(|a, b| a.task_id.cmp(&b.task_id));

        // Sort dependencies
        for task in &mut self.tasks {
            task.shared.dependencies.sort();
            task.shared.dependents.sort();
        }
    }

    fn get_path(&self) -> AbsoluteSystemPathBuf {
        let filename = format!("{}.json", self.id);

        self.repo_root
            .join_components(&[".turbo", "runs", &filename])
    }

    fn get_failed_tasks(&self) -> Vec<&TaskSummary> {
        self.tasks
            .iter()
            .filter(|task| {
                task.shared
                    .execution
                    .as_ref()
                    .map_or(false, |e| e.is_failure())
            })
            .collect()
    }

    fn save(&mut self) -> Result<(), Error> {
        let json = self.format_json()?;

        let summary_path = self.get_path();
        summary_path.ensure_dir()?;

        Ok(summary_path.create_with_contents(json)?)
    }
}
