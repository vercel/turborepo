//! `turbo exec` dispatch. Managed activation is not implemented yet, so this
//! never starts the requested program and never falls back to the host PATH.

use miette::Diagnostic;
use thiserror::Error;
use turbopath::AbsoluteSystemPath;
use turborepo_turbo_json::RawTurboJson;

use crate::cli::{Args, resolve_configuration_from_args};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("`turbo exec` requires root futureFlags.experimentalSetup to be true")]
    #[diagnostic(
        code(turbo::exec::disabled),
        help(
            "Set \"futureFlags\": {{\"experimentalSetup\": true}} in the root turbo.json or \
             turbo.jsonc. No command was run. To run a task named exec, use `turbo run exec`."
        )
    )]
    Disabled,
    #[error("`turbo exec` requires managed setup, which this turbo cannot activate yet")]
    #[diagnostic(
        code(turbo::exec::managed_setup_required),
        help(
            "No command was run. `turbo exec` never falls back to tools on the host PATH; run the \
             command directly for now."
        )
    )]
    ManagedSetupRequired,
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJson(#[from] turborepo_turbo_json::Error),
}

/// `repo_root` is the root `turbo run` would use for this invocation.
pub fn run(repo_root: &AbsoluteSystemPath, args: &Args) -> Result<i32, Error> {
    let config = resolve_configuration_from_args(repo_root, args)?;
    let root_turbo_json = config.root_turbo_json_path(repo_root)?;
    let enabled = RawTurboJson::read(repo_root, &root_turbo_json, true)?
        .and_then(|raw| raw.future_flags)
        .is_some_and(|flags| flags.as_inner().experimental_setup);
    if !enabled {
        return Err(Error::Disabled);
    }
    Err(Error::ManagedSetupRequired)
}
