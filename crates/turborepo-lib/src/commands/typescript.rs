use camino::Utf8PathBuf;
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{cli, commands::CommandBase};

pub fn run_typescript(
    config: Option<Utf8PathBuf>,
    base: &CommandBase,
    telemetry: CommandEventBuilder,
) -> Result<(), cli::Error> {
    // For now, just return success
    Ok(())
}
