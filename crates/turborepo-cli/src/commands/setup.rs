//! Native setup entry point. Discovery and provisioning land separately.

use miette::Diagnostic;
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_turbo_json::RawTurboJson;

use crate::cli::{Args, SetupArgs};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("`turbo setup` requires root futureFlags.experimentalSetup to be true")]
    #[diagnostic(
        code(turbo::setup::disabled),
        help(
            "Set \"futureFlags\": {{\"experimentalSetup\": true}} in the root turbo.json or \
             turbo.jsonc. For now, run from the repository root or select it with --cwd; \
             automatic setup root discovery is not implemented yet."
        )
    )]
    Disabled,
    #[error("--update-lock cannot be used in frozen mode (inferred in CI)")]
    #[diagnostic(help("For an intentional lock refresh in CI, pass --no-frozen --update-lock."))]
    FrozenUpdateLock,
    #[error("`turbo setup` provisioning is not implemented yet in this version")]
    #[diagnostic(
        code(turbo::setup::not_implemented),
        help(
            "Use the repository's documented toolchain and dependency installation steps for now. \
             No tools, dependencies, or locks were changed and no tasks were run. Plan and check \
             are also not implemented; see turbo setup --help for the experimental command \
             surface."
        )
    )]
    NotImplemented,
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] turborepo_config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJson(#[from] turborepo_turbo_json::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Provision,
    Plan,
    Check,
}

#[derive(Debug, PartialEq, Eq)]
enum LockMode {
    Write,
    Frozen,
    NoLock,
}

/// A typed, normalized request, not task execution options.
#[derive(Debug, PartialEq, Eq)]
struct SetupRequest {
    mode: Mode,
    lock: LockMode,
    force: bool,
    offline: bool,
    tools_only: bool,
    update_lock: bool,
}

impl SetupRequest {
    fn new(args: &SetupArgs, is_ci: bool) -> Result<Self, Error> {
        // Explicit conflicts are enforced by the argument parser. --no-lock
        // overrides only the inferred CI default, not explicit --frozen.
        let lock = if args.no_lock {
            LockMode::NoLock
        } else if args.frozen || (is_ci && !args.no_frozen) {
            LockMode::Frozen
        } else {
            LockMode::Write
        };
        if args.update_lock && lock == LockMode::Frozen {
            return Err(Error::FrozenUpdateLock);
        }
        Ok(Self {
            mode: if args.plan {
                Mode::Plan
            } else if args.check {
                Mode::Check
            } else {
                Mode::Provision
            },
            lock,
            force: args.force,
            offline: args.offline,
            tools_only: args.tools_only,
            update_lock: args.update_lock,
        })
    }
}

pub fn run(args: &Args, setup_args: &SetupArgs) -> Result<i32, Error> {
    // Deliberately do not infer a JS repository, resolve configuration from the
    // environment, build a package graph, or require a package manager. Root
    // inference is a follow-up; the current root is cwd/--cwd only.
    let root = match args.cwd.as_deref() {
        Some(cwd) => AbsoluteSystemPathBuf::from_cwd(cwd)?,
        None => AbsoluteSystemPathBuf::cwd()?,
    };
    let config_path = match args.root_turbo_json.as_deref() {
        Some(path) => AbsoluteSystemPathBuf::from_cwd(path)?,
        None => turborepo_config::resolve_turbo_config_path(&root)?,
    };
    let enabled = RawTurboJson::read(&root, &config_path, true)?
        .and_then(|raw| raw.future_flags)
        .is_some_and(|flags| flags.as_inner().experimental_setup);
    if !enabled {
        return Err(Error::Disabled);
    }

    execute(SetupRequest::new(setup_args, turborepo_ci::is_ci())?)
}

fn execute(request: SetupRequest) -> Result<i32, Error> {
    tracing::debug!(
        ?request,
        "setup request accepted; provisioning is not implemented"
    );
    // No success until provisioning exists, even for plan/check or --__test-run.
    Err(Error::NotImplemented)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::cli::Command;

    fn request(flags: &[&str], ci: bool) -> Result<SetupRequest, Error> {
        let words = ["turbo", "setup"].into_iter().chain(flags.iter().copied());
        let args = Args::parse_args(words.map(OsString::from).collect()).unwrap();
        let Some(Command::Setup { setup_args }) = args.command else {
            panic!("expected setup");
        };
        SetupRequest::new(&setup_args, ci)
    }

    #[test]
    fn lock_precedence_and_ci_refresh() {
        assert_eq!(request(&[], false).unwrap().lock, LockMode::Write);
        assert_eq!(request(&[], true).unwrap().lock, LockMode::Frozen);
        assert_eq!(
            request(&["--frozen"], false).unwrap().lock,
            LockMode::Frozen
        );
        for ci in [true, false] {
            assert_eq!(request(&["--no-lock"], ci).unwrap().lock, LockMode::NoLock);
            assert_eq!(request(&["--no-frozen"], ci).unwrap().lock, LockMode::Write);
            assert!(
                request(&["--update-lock", "--no-frozen"], ci)
                    .unwrap()
                    .update_lock
            );
        }
        assert!(matches!(
            request(&["--update-lock"], true),
            Err(Error::FrozenUpdateLock)
        ));
        assert!(matches!(
            request(&["--plan", "--update-lock"], true),
            Err(Error::FrozenUpdateLock)
        ));
        assert!(request(&["--update-lock"], false).unwrap().update_lock);
    }

    #[test]
    fn modes_and_controls_reach_the_empty_executor() {
        for (flags, mode) in [
            (vec![], Mode::Provision),
            (vec!["--plan", "--force", "--update-lock"], Mode::Plan),
            (vec!["--check"], Mode::Check),
        ] {
            let request = request(&flags, false).unwrap();
            assert_eq!(request.mode, mode);
            assert!(matches!(execute(request), Err(Error::NotImplemented)));
        }
        let request =
            request(&["--force", "--offline", "--tools-only", "--no-lock"], true).unwrap();
        assert!(request.force && request.offline && request.tools_only);
        assert!(!request.update_lock);
    }
}
