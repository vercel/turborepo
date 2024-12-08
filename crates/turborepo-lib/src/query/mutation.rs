use std::sync::Arc;

use async_graphql::{InputObject, Object};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{sender::UISender, wui::query::Run};

use crate::{
    cli::Error,
    commands::{run::get_signal, CommandBase},
    get_version,
    run::builder::RunBuilder,
    signal::SignalHandler,
};

pub struct Mutation {
    repo_root: AbsoluteSystemPathBuf,
}

impl Mutation {
    pub fn new(repo_root: AbsoluteSystemPathBuf) -> Self {
        Self { repo_root }
    }
}

#[derive(InputObject)]
pub struct RunOptions {
    pub team_id: Option<String>,
    pub team_slug: Option<String>,
    pub token: Option<String>,
    pub remote_cache_read_only: Option<bool>,
    pub cache: Option<String>,
    pub summary: Option<bool>,
    pub parallel: Option<bool>,
    pub continue_on_error: Option<bool>,
    pub dry_run: Option<bool>,
    pub only: Option<bool>,
    pub pass_through_args: Option<Vec<String>>,
    pub env_mode: Option<String>,
    pub graph: Option<String>,
    pub root_turbo_json: Option<String>,
    pub log_order: Option<String>,
    pub log_prefix: Option<String>,
    pub workspaces: Option<Vec<String>>,
    pub filter: Option<Vec<String>>,
    pub affected_range: Option<GitRange>,
}

#[derive(InputObject)]
pub struct GitRange {
    pub base: Option<String>,
    pub head: Option<String>,
}

#[Object]
impl Mutation {
    /// Executes a run. This function is blocking and will return all the tasks
    /// outputs. Therefore, don't use this function with persistent tasks or
    /// with tasks that return a lot of data.
    async fn run_blocking(&self, tasks: Vec<String>, options: RunOptions) -> Result<Run, Error> {
        let base =
            CommandBase::from_query_options(tasks, self.repo_root.clone(), options, get_version())?;
        let signal = get_signal()?;
        let signal_handler = SignalHandler::new(signal);
        let telemetry = CommandEventBuilder::new("run");
        let run = Arc::new(
            RunBuilder::new(base)?
                .build(&signal_handler, telemetry)
                .await?,
        );

        let (sender, handle) = run.start_ui()?.unzip();

        run.run(sender.clone(), false).await?;

        let Some(UISender::Wui(sender)) = sender else {
            panic!("Web UI is required for runs started via the GraphQL API");
        };

        Ok(Run::new(sender.shared_state.clone()))
    }
}
