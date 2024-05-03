use camino::Utf8PathBuf;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    commands::{run::get_signal, CommandBase},
    run::{builder::RunBuilder, scip::Error},
    signal::SignalHandler,
};

pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    output_path: Option<Utf8PathBuf>,
) -> Result<i32, Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let output_file = match output_path {
        Some(path) => AbsoluteSystemPathBuf::from_unknown(base.cwd(), path),
        None => base.cwd().join_component("out.scip"),
    };
    let run = RunBuilder::new(base)?.build(&handler, telemetry).await?;

    run.emit_scip(&output_file)?;

    Ok(0)
}
