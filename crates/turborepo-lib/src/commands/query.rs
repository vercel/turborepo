use async_graphql::{EmptyMutation, EmptySubscription, Schema};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    cli::Command,
    commands::{run::get_signal, CommandBase},
    query::{Error, Query},
    run::builder::RunBuilder,
    signal::SignalHandler,
};

pub async fn run(
    mut base: CommandBase,
    telemetry: CommandEventBuilder,
    query: String,
) -> Result<i32, Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    // We fake a run command, so we can construct a `Run` type
    base.args_mut().command = Some(Command::Run {
        run_args: Box::default(),
        execution_args: Box::default(),
    });

    let run_builder = RunBuilder::new(base)?;
    let run = run_builder.build(&handler, telemetry).await?;

    let schema = Schema::new(Query::new(run), EmptyMutation, EmptySubscription);

    let result = schema.execute(query).await;
    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(0)
}
