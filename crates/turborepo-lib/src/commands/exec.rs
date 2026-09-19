//! `turbo exec -- <command> [args...]`: run an arbitrary command with the
//! repository's managed toolchain on `PATH`.
//!
//! Activation already happened at startup (the shim prepends
//! `.turbo/tools/bin` to `PATH` and exports the manifest env), so the
//! command inherits exactly the environment turbo gives its tasks. This makes
//! `turbo exec -- uv sync` the spelling that is always correct in scripts and
//! docs, regardless of what the caller's shell has on `PATH`.

use std::{env, io, process::Command};

use camino::Utf8Path;
use miette::Diagnostic;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_tools::{ToolsDir, TOOLS_ROOT_ENV};
use turborepo_ui::{color, ColorConfig, BOLD, YELLOW};

use crate::{cli::INVOCATION_DIR_ENV_VAR, spawn_child};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("No command given.")]
    #[diagnostic(help("Usage: turbo exec -- <command> [args...]"))]
    MissingCommand,
    #[error("`{program}` was not found on PATH")]
    #[diagnostic(help(
        "turbo exec searches `.turbo/tools/bin` first, then your PATH. Run `turbo setup` to \
         install the toolchain the repository declares."
    ))]
    NotFound { program: String },
    #[error("Unable to run `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("Unable to wait for `{program}`: {source}")]
    Wait {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

/// Runs `command` and returns its exit code.
pub fn run(
    repo_root: &AbsoluteSystemPath,
    cwd_override: Option<&Utf8Path>,
    color_config: ColorConfig,
    command: &[String],
) -> Result<i32, Error> {
    let Some((program, args)) = command.split_first() else {
        return Err(Error::MissingCommand);
    };

    // Activation happened in the shim; it records the directory it applied so
    // this check stays correct when the tools live above the inferred root.
    let activated = env::var_os(TOOLS_ROOT_ENV).is_some();
    if !activated && !ToolsDir::new(repo_root).bin_dir().exists() {
        eprintln!(
            "{}",
            color!(
                color_config,
                YELLOW,
                "No managed toolchain found in {}. Running `{}` with your current PATH; run \
                 `turbo setup` to install the tools this repository declares.",
                ".turbo/tools",
                program
            )
        );
    }

    // Commands run where the user invoked turbo, not at the repository root:
    // `turbo exec -- uv sync` inside a package should act on that package.
    let cwd = invocation_dir(cwd_override)?;

    let mut process = Command::new(program);
    process.args(args).current_dir(cwd.as_std_path());

    let child = spawn_child(process).map_err(|source| match source.kind() {
        io::ErrorKind::NotFound => Error::NotFound {
            program: program.clone(),
        },
        _ => Error::Spawn {
            program: program.clone(),
            source,
        },
    })?;
    let status = child.wait().map_err(|source| Error::Wait {
        program: program.clone(),
        source,
    })?;

    Ok(status.code().unwrap_or_else(|| {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                eprintln!(
                    "{}",
                    color!(
                        color_config,
                        BOLD,
                        "`{}` terminated by signal {}",
                        program,
                        signal
                    )
                );
                return 128 + signal;
            }
        }
        1
    }))
}

/// The directory the user ran turbo from: an explicit `--cwd`, else the
/// invocation directory the shim recorded before handing off to the local
/// turbo (which runs at the repository root), else the process cwd.
fn invocation_dir(cwd_override: Option<&Utf8Path>) -> Result<AbsoluteSystemPathBuf, Error> {
    if let Some(cwd) = cwd_override {
        return Ok(AbsoluteSystemPathBuf::from_cwd(cwd)?);
    }
    if let Some(dir) = env::var_os(INVOCATION_DIR_ENV_VAR) {
        if let Ok(path) = AbsoluteSystemPathBuf::try_from(std::path::PathBuf::from(dir)) {
            return Ok(path);
        }
    }
    Ok(AbsoluteSystemPathBuf::cwd()?)
}
