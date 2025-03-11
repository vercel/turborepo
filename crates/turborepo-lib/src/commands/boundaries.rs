use std::collections::HashSet;

use dialoguer::{Confirm, Input};
use miette::Report;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{color, BOLD_GREEN};

use crate::{cli, cli::BoundariesIgnore, commands::CommandBase, run::builder::RunBuilder};

pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    ignore: Option<BoundariesIgnore>,
    reason: Option<String>,
) -> Result<i32, cli::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run = RunBuilder::new(base)?
        .do_not_validate_engine()
        .build(&handler, telemetry)
        .await?;

    let result = run.check_boundaries().await?;

    if let Some(ignore) = ignore {
        let mut seen_locations = HashSet::new();
        for diagnostic in &result.diagnostics {
            let Some((path, span)) = diagnostic.path_and_span() else {
                continue;
            };
            if seen_locations.contains(&(path, span)) {
                continue;
            }
            seen_locations.insert((path, span));

            let short_path = match run.repo_root().anchor(path) {
                Ok(path) => path.to_string(),
                Err(_) => path.to_string(),
            };

            let reason = match ignore {
                BoundariesIgnore::All => Some(reason.clone().unwrap_or_else(|| {
                    "automatically added by `turbo boundaries --ignore=all`".to_string()
                })),
                BoundariesIgnore::Prompt => {
                    print!("\x1B[2J\x1B[1;1H");
                    println!("{:?}", Report::new(diagnostic.clone()));
                    let prompt = format!("Add @boundaries-ignore to {}?", short_path);
                    if Confirm::new()
                        .with_prompt(prompt)
                        .default(false)
                        .interact()?
                    {
                        if let Some(reason) = reason.clone() {
                            Some(reason)
                        } else {
                            Some(
                                Input::new()
                                    .with_prompt("Reason for ignoring this error")
                                    .interact_text()?,
                            )
                        }
                    } else {
                        None
                    }
                }
            };

            if let Some(reason) = reason {
                println!(
                    "{} {}",
                    color!(run.color_config(), BOLD_GREEN, "patching"),
                    short_path
                );
                run.add_ignore(path, span, reason)?;
            }
        }
    } else {
        result.emit(run.color_config());
    }

    if result.is_ok() {
        Ok(0)
    } else {
        Ok(1)
    }
}
