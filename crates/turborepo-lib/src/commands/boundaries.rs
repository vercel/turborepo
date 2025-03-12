use std::collections::HashMap;

use dialoguer::{Confirm, Input};
use miette::{Report, SourceSpan};
use turbopath::AbsoluteSystemPath;
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

    let result = run.check_boundaries(true).await?;

    if let Some(ignore) = ignore {
        let mut patches: HashMap<&AbsoluteSystemPath, Vec<(SourceSpan, String)>> = HashMap::new();
        for diagnostic in &result.diagnostics {
            let Some((path, span)) = diagnostic.path_and_span() else {
                continue;
            };

            let reason = match ignore {
                BoundariesIgnore::All => Some(reason.clone().unwrap_or_else(|| {
                    "automatically added by `turbo boundaries --ignore=all`".to_string()
                })),
                BoundariesIgnore::Prompt => {
                    print!("{esc}c", esc = 27 as char);
                    println!();
                    println!();
                    println!("{:?}", Report::new(diagnostic.clone()));
                    let prompt = format!(
                        "Ignore this error by adding a {} comment?",
                        color!(run.color_config(), BOLD_GREEN, "@boundaries-ignore"),
                    );
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
                patches.entry(path).or_default().push((span, reason));
            }
        }

        for (path, file_patches) in patches {
            let short_path = match run.repo_root().anchor(path) {
                Ok(path) => path.to_string(),
                Err(_) => path.to_string(),
            };
            println!(
                "{} {}",
                color!(run.color_config(), BOLD_GREEN, "patching"),
                short_path
            );
            run.patch_file(path, file_patches)?;
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
