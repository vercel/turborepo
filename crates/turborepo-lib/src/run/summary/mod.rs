#[allow(dead_code)]
mod execution;
mod global_hash;
mod scm;
mod spaces;
mod task;

use chrono::Local;
use global_hash::GlobalHashSummary;
use serde::Serialize;
use svix_ksuid::{Ksuid, KsuidLike};
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_api_client::APIClient;
use turborepo_ci::Vendor;
use turborepo_env::EnvironmentVariableMap;
use turborepo_ui::UI;

use crate::{
    cli::EnvMode,
    opts::RunOpts,
    run::summary::{execution::ExecutionSummary, scm::SCMState, task::TaskSummary},
};

#[derive(Debug, Error)]
enum Error {
    #[error("Failed to write run summary {0}")]
    IO(#[from] std::io::Error),
    #[error("Failed to serialize run summary to JSON")]
    Serde(#[from] serde_json::Error),
}

// NOTE: When changing this, please ensure that the server side is updated to
// handle the new version on vercel.com this is required to ensure safe handling
// of env vars (unknown run summary versions will be ignored on the server)
const RUN_SUMMARY_SCHEMA_VERSION: &str = "1";

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

// Wrapper around the serializable RunSummary, with some extra information
// about the Run and references to other things that we need.
struct Meta<'a> {
    run_summary: RunSummary<'a>,
    repo_root: &'a AbsoluteSystemPath,
    single_package: bool,
    should_save: bool,
    run_type: RunType,
    synthesized_command: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunSummary<'a> {
    id: Ksuid,
    version: String,
    turbo_version: String,
    monorepo: bool,
    global_hash_summary: GlobalHashSummary,
    packages: Vec<String>,
    env_mode: EnvMode,
    framework_inference: bool,
    execution_summary: Option<ExecutionSummary>,
    tasks: Vec<TaskSummary<'a>>,
    user: Option<String>,
    scm: SCMState,
}

impl<'a> Meta<'a> {
    pub fn new_run_summary(
        start_at: chrono::DateTime<Local>,
        repo_root: &'a AbsoluteSystemPath,
        package_inference_root: &AnchoredSystemPath,
        turbo_version: &'static str,
        run_opts: RunOpts,
        packages: &[String],
        global_env_mode: EnvMode,
        env_at_execution_start: EnvironmentVariableMap,
        global_hash_summary: GlobalHashSummary,
        synthesized_command: String,
    ) -> Meta<'a> {
        let single_package = run_opts.single_package;
        let profile = run_opts.profile;
        let should_save = run_opts.summarize.flatten().is_some_and(|s| s);
        let space_id = &run_opts.experimental_space_id;

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

        Meta {
            run_summary: RunSummary {
                id: Ksuid::new(None, None),
                version: RUN_SUMMARY_SCHEMA_VERSION.to_string(),
                execution_summary: Some(execution_summary),
                turbo_version: turbo_version.to_string(),
                packages: packages.to_vec(),
                env_mode: global_env_mode,
                framework_inference: run_opts.framework_inference,
                tasks: vec![],
                global_hash_summary,
                scm: SCMState::get(&env_at_execution_start, repo_root),
                user: get_user(&env_at_execution_start),
                monorepo: !single_package,
            },
            run_type,
            repo_root,
            single_package,
            should_save,

            synthesized_command,
        }
    }

    fn close(&mut self, exit_code: u32, ui: UI) {
        if matches!(self.run_type, RunType::DryJson | RunType::DryText) {
            self.close_dry_run()
        }
    }

    fn close_dry_run(&mut self) -> Result<(), Error> {
        if matches!(self.run_type, RunType::DryJson) {
            let rendered = self.format_json();

            println!("{}", rendered);
            return Ok(());
        }

        self.format_and_print_text()
    }

    fn format_and_print_text(&mut self) -> Result<(), Error> {
        self.normalize();
    }

    fn normalize(&mut self) {
        // Remove execution summary for dry runs
        if matches!(self.run_type, RunType::DryJson) {
            self.run_summary.execution_summary = None;
        }

        // For single packages, we don't need the packages
        // and each task summary needs some cleaning
        if self.single_package {
            self.run_summary.packages = vec![];

            for task_summary in &mut self.run_summary.tasks {
                task_summary.clean_for_single_package();
            }
        }

        self.run_summary
            .tasks
            .sort_by(|a, b| a.task_id.cmp(&b.task_id));
    }

    fn get_path(&self) -> AbsoluteSystemPathBuf {
        let filename = format!("{}.json", self.run_summary.id);

        self.repo_root
            .join_components(&[".turbo", "runs", &filename])
    }

    fn save(&self) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(&self.run_summary)?;

        let summary_path = self.get_path();
        summary_path.ensure_dir()?;

        Ok(summary_path.create_with_contents(json)?)
    }
}
