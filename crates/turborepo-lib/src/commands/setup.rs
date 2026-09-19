//! `turbo setup`: install the toolchain a repository declares into
//! `.turbo/tools`.

use std::io;

use miette::Diagnostic;
use thiserror::Error;
use turborepo_gitignore::ensure_turbo_is_gitignored;
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_tools::{
    declared::{self, Declaration},
    http::Downloader,
    sources::Sources,
    InstallOutcome, InstallStatus, Installer, Reporter, ToolsDir,
};
use turborepo_ui::{color, ColorConfig, BOLD, BOLD_GREEN, BOLD_RED, GREY, YELLOW};

use super::CommandBase;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(code(turbo::setup::tools))]
    Tools(#[from] turborepo_tools::Error),
    #[error("Unable to add `.turbo` to .gitignore: {0}")]
    Gitignore(#[source] io::Error),
}

/// Prints progress through turbo's color configuration.
struct PrintReporter {
    color_config: ColorConfig,
}

impl Reporter for PrintReporter {
    fn step(&self, message: &str) {
        println!("{}", color!(self.color_config, BOLD, "{}", message));
    }

    fn detail(&self, message: &str) {
        println!("  {}", color!(self.color_config, GREY, "{}", message));
    }
}

/// Runs the command. Returns the process exit code: `--check` exits with 1
/// when anything is missing or outdated.
pub async fn run(
    base: CommandBase,
    check: bool,
    force: bool,
    telemetry: CommandEventBuilder,
) -> Result<i32, Error> {
    telemetry.track_arg_usage("check", check);
    telemetry.track_arg_usage("force", force);

    let declarations = declared::discover(&base.repo_root)?;
    if declarations.is_empty() {
        return Err(turborepo_tools::Error::NoDeclarations.into());
    }
    for declaration in &declarations {
        telemetry.track_arg_usage(&format!("tool:{}", declaration.tool()), true);
    }

    let color_config = base.color_config;
    let reporter = PrintReporter { color_config };
    let http = Downloader::new()?;
    let mut installer = Installer::new(&base.repo_root, &reporter, Sources::from_env(), http)?;
    let tools_dir = ToolsDir::new(&base.repo_root);
    let tools_display = format!(
        ".turbo/tools ({})",
        color!(color_config, GREY, "{}", tools_dir.root())
    );

    println!(
        "{}",
        color!(
            color_config,
            BOLD,
            "Declared toolchain ({} tool{}):",
            declarations.len(),
            if declarations.len() == 1 { "" } else { "s" }
        )
    );
    for declaration in &declarations {
        print_declaration(color_config, declaration);
    }
    println!();

    if check {
        let outcomes = installer.check(&declarations).await?;
        print_outcomes(color_config, &outcomes);
        let satisfied = outcomes.iter().all(|outcome| outcome.status.is_satisfied());
        println!();
        if satisfied {
            println!(
                "{}",
                color!(
                    color_config,
                    BOLD_GREEN,
                    "All declared tools are installed in {tools_display}."
                )
            );
            return Ok(0);
        }
        println!(
            "{}",
            color!(
                color_config,
                BOLD_RED,
                "Some declared tools are missing from {tools_display}. Run `turbo setup` to \
                 install them."
            )
        );
        return Ok(1);
    }

    // The tools directory lives inside the repository and must never be
    // committed; `.turbo` is already the convention for turbo's local state.
    ensure_turbo_is_gitignored(&base.repo_root).map_err(Error::Gitignore)?;

    let outcomes = installer.install(&declarations, force).await?;
    println!();
    print_outcomes(color_config, &outcomes);
    let installed = outcomes
        .iter()
        .filter(|outcome| outcome.status == InstallStatus::Installed)
        .count();
    println!();
    println!(
        "{}",
        color!(
            color_config,
            BOLD_GREEN,
            "{} tool{} installed into {tools_display}.",
            installed,
            if installed == 1 { "" } else { "s" }
        )
    );
    println!(
        "{}",
        color!(
            color_config,
            GREY,
            "turbo prepends .turbo/tools/bin to PATH for every task it runs. To use the tools in \
             your own shell, add that directory to PATH."
        )
    );
    Ok(0)
}

fn print_declaration(color_config: ColorConfig, declaration: &Declaration) {
    println!(
        "  {} {} {}",
        color!(color_config, BOLD, "{}", declaration.tool()),
        declaration.requested(),
        color!(color_config, GREY, "(from {})", declaration.source())
    );
}

fn print_outcomes(color_config: ColorConfig, outcomes: &[InstallOutcome]) {
    for outcome in outcomes {
        let status = match &outcome.status {
            InstallStatus::Installed | InstallStatus::UpToDate => {
                color!(color_config, BOLD_GREEN, "{}", outcome.status)
            }
            InstallStatus::Missing => color!(color_config, BOLD_RED, "{}", outcome.status),
            InstallStatus::Outdated { .. } => color!(color_config, YELLOW, "{}", outcome.status),
        };
        println!(
            "  {} {} {}",
            color!(color_config, BOLD, "{}", outcome.tool),
            outcome.version,
            status
        );
    }
}
