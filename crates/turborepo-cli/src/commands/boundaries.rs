use std::collections::HashMap;

use dialoguer::{Confirm, Input};
use miette::{Report, SourceSpan};
use turbopath::AbsoluteSystemPath;
use turborepo_boundaries::{BoundariesChecker, BoundariesContext};
use turborepo_run::{boundaries::RunTurboJsonProvider, builder::RunBuilder};
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{color, BOLD_GREEN};

use crate::{cli, cli::BoundariesIgnore, commands::CommandBase};

pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    ignore: Option<BoundariesIgnore>,
    reason: Option<String>,
) -> Result<i32, cli::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let (run, _analytics) = RunBuilder::new(base.run_builder_input()?, None)?
        .do_not_validate_engine()
        .build(&handler, telemetry)
        .await?;

    let turbo_json_provider = RunTurboJsonProvider::new(run.turbo_json_loader());
    let root_boundaries_config = run
        .root_turbo_json()
        .boundaries
        .as_ref()
        .map(|spanned| spanned.as_inner());
    let ctx = BoundariesContext {
        repo_root: run.repo_root(),
        pkg_dep_graph: run.pkg_dep_graph(),
        turbo_json_provider: &turbo_json_provider,
        root_boundaries_config,
        filtered_pkgs: run.filtered_pkgs(),
    };
    let result = BoundariesChecker::check_boundaries(&ctx, true)?;

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
            BoundariesChecker::patch_file(path, file_patches)?;
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
