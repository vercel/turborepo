use turborepo_signals::SignalHandler;
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    cli,
    commands::{run::get_signal, CommandBase},
    run::builder::RunBuilder,
};

pub async fn run(base: CommandBase, telemetry: CommandEventBuilder) -> Result<i32, cli::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run = RunBuilder::new(base)?
        .do_not_validate_engine()
        .build(&handler, telemetry)
        .await?;

    let result = run.check_boundaries().await?;

    result.emit(run.color_config());

    if result.is_ok() {
        Ok(0)
    } else {
        Ok(1)
    }
}
