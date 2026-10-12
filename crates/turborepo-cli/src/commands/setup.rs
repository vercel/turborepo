//! Native local/frozen provisioning and read-only tools-only checks. No
//! execution.

use miette::Diagnostic;
use thiserror::Error;
use turborepo_repository::toolchain::ToolchainId;
use turborepo_setup::source_policy::OfficialSourcePolicy;
use turborepo_turbo_json::ToolPolicy;

use crate::cli::{Args, SetupArgs};

mod check;
mod effective;
mod provision;
mod root;
mod verify_system;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("`turbo setup` requires root futureFlags.experimentalSetup to be true")]
    #[diagnostic(
        code(turbo::setup::disabled),
        help(
            "Set \"futureFlags\": {{\"experimentalSetup\": true}} in the root turbo.json or \
             turbo.jsonc. Select the intended repository with --cwd=<root> if necessary."
        )
    )]
    Disabled,
    #[error("--update-lock cannot be used in frozen mode (inferred in CI)")]
    #[diagnostic(help("For an intentional lock refresh in CI, pass --no-frozen --update-lock."))]
    FrozenUpdateLock,
    #[error(
        "this setup mode is not implemented; use local or frozen --tools-only provisioning or \
         --check --tools-only readiness checks"
    )]
    #[diagnostic(
        code(turbo::setup::not_implemented),
        help(
            "Use the repository's documented toolchain and dependency installation steps for now. \
             No tools, dependencies, or locks were changed and no tasks were run. Plan and \
             dependency checks are not implemented."
        )
    )]
    NotImplemented,
    #[error("unsupported setup request: {0}")]
    Unsupported(&'static str),
    #[error(transparent)]
    SourcePolicy(#[from] turborepo_setup::source_policy::Error),
    #[error(transparent)]
    Activation(#[from] turborepo_setup::activation::Error),
    #[error(transparent)]
    Storage(#[from] turborepo_setup::lock::StorageError),
    #[error(transparent)]
    Lock(#[from] turborepo_setup::lock::Error),
    #[error(transparent)]
    Reconcile(#[from] turborepo_setup::lock::reconcile::Error),
    #[error(transparent)]
    Node(#[from] turborepo_setup::node_provision::Error),
    #[error(transparent)]
    Pnpm(#[from] turborepo_setup::pnpm_provision::Error),
    #[error(transparent)]
    SystemTool(#[from] turborepo_setup::system_tool::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Root(#[from] root::Error),
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
    ignore_scripts: bool,
    update_lock: bool,
}

impl SetupRequest {
    /// Lifecycle intent for a toolchain's dependency installer: CLI >
    /// repository policy > native default.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "consumed by dependency installers (TURBO-6229)")
    )]
    fn lifecycle(
        &self,
        repository: Option<turborepo_turbo_json::LifecycleScripts>,
    ) -> turborepo_setup::lifecycle::Request {
        use turborepo_setup::lifecycle::{Intent, Request};
        use turborepo_turbo_json::LifecycleScripts;
        Request::new(
            self.ignore_scripts,
            repository.map(|scripts| match scripts {
                LifecycleScripts::Native => Intent::Native,
                LifecycleScripts::Skip => Intent::Skip,
            }),
        )
    }

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
            ignore_scripts: args.ignore_scripts,
            update_lock: args.update_lock,
        })
    }
}

pub fn run(args: &Args, setup_args: &SetupArgs) -> Result<i32, Error> {
    let invocation = match args.cwd.as_deref() {
        Some(cwd) => turbopath::AbsoluteSystemPathBuf::from_cwd(cwd)?,
        None => turbopath::AbsoluteSystemPathBuf::cwd()?,
    };
    run_with_policy(args, setup_args, None, || {
        Ok(OfficialSourcePolicy::inspect(
            invocation.as_std_path(),
            None,
        )?)
    })
}

// Trusted internal fixture injection only: never a CLI flag, repo URL or env
// override.
fn run_with_policy(
    args: &Args,
    setup_args: &SetupArgs,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<i32, Error> {
    // Discovery reads files/Git metadata: no language-tool probes, environment
    // config pipeline, graph, package manager detection, or local CLI handoff.
    let discovery = root::Discovery::capture(args)?;
    tracing::debug!("setup root: {}", discovery.root_path());
    if !discovery.flags().experimental_setup {
        return Err(Error::Disabled);
    }

    let request = SetupRequest::new(setup_args, turborepo_ci::is_ci())?;
    let eligible = discovery.source_eligibility();
    let plan = effective::Plan::new(
        discovery.policy(),
        [
            (eligible.node(), ToolchainId::JAVASCRIPT),
            (eligible.cargo(), ToolchainId::RUST),
            (eligible.python(), ToolchainId::PYTHON),
            (eligible.go(), ToolchainId::GO),
        ]
        .into_iter()
        .filter_map(|(eligible, id)| eligible.then_some(id)),
        request.tools_only,
    );
    validate(&request, plan.tools_only())?;
    if args.test_run {
        return Err(Error::NotImplemented);
    }
    if plan.unsupported() {
        return Err(Error::Unsupported("non-JavaScript workspace setup"));
    }
    let javascript = plan
        .toolchain(&ToolchainId::JAVASCRIPT)
        .map_or(ToolPolicy::Skip, |toolchain| toolchain.tools.0);
    let snapshot =
        turborepo_setup::lock::Snapshot::capture(discovery.snapshot_root()?.as_std_path())?;
    // Report the effective decisions before any traffic or installation.
    for line in plan.report() {
        println!("{line}");
    }
    if javascript == ToolPolicy::Skip {
        // Never silently ignore committed selections for a skipped toolchain.
        if snapshot
            .previous_lock()
            .is_some_and(|lock| !lock.tools().is_empty())
        {
            return Err(Error::Unsupported(
                "turbo.lock selects tools that the setup policy skips; remove the skip or the \
                 locked tools",
            ));
        }
        println!("Nothing to prepare: every toolchain is skipped. No tasks run.");
        return Ok(0);
    }
    let resolution = if request.update_lock {
        turborepo_setup::lock::reconcile::Mode::Refresh
    } else {
        turborepo_setup::lock::reconcile::Mode::Local
    };
    match (request.mode, javascript) {
        (Mode::Check, ToolPolicy::VerifySystem) => verify_system::check(&discovery, snapshot),
        (_, ToolPolicy::VerifySystem) => verify_system::provision(
            &discovery,
            snapshot,
            request.lock,
            resolution,
            request.force,
        ),
        (Mode::Check, _) => check::run(&discovery, snapshot, preflight),
        _ => provision::run(
            &discovery,
            snapshot,
            request.lock,
            resolution,
            request.force,
            transports,
            preflight,
        ),
    }
}

#[cfg(test)]
fn validate_request(request: &SetupRequest) -> Result<(), Error> {
    validate(request, request.tools_only)
}

/// `tools_only` is the effective decision: `--tools-only` or a root policy
/// that skips every eligible toolchain's dependencies.
fn validate(request: &SetupRequest, tools_only: bool) -> Result<(), Error> {
    if request.mode == Mode::Check
        && tools_only
        && request.lock != LockMode::NoLock
        && !request.force
        && !request.update_lock
    {
        // CI/frozen/offline never change a check into resolution or repair.
        return Ok(());
    }
    // Force reinstalls existing locked selections, independently of local
    // Write normalization. Combined force/refresh remains fail-closed.
    if request.mode != Mode::Provision
        || request.lock == LockMode::NoLock
        || !tools_only
        || request.offline
        || (request.force && request.update_lock)
    {
        return Err(Error::NotImplemented);
    }
    Ok(())
}

#[cfg(all(
    test,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#[path = "setup/tests.rs"]
mod provisioning_tests;

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
    fn platform_qualification_rejects_unsupported_hosts_including_musl() {
        let arch = cfg!(any(target_arch = "x86_64", target_arch = "aarch64"));
        let host = cfg!(target_os = "macos") || cfg!(all(target_os = "linux", target_env = "gnu"));
        assert_eq!(provision::platform().is_ok(), arch && host);
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
    fn tools_only_check_accepts_ci_frozen_and_offline_without_resolution() {
        for ci in [false, true] {
            for flags in [
                vec!["--check", "--tools-only"],
                vec!["--check", "--tools-only", "--offline"],
                vec!["--check", "--tools-only", "--frozen"],
                vec!["--check", "--tools-only", "--no-frozen", "--offline"],
            ] {
                let request = request(&flags, ci).unwrap();
                assert_eq!(request.mode, Mode::Check);
                assert!(validate_request(&request).is_ok());
            }
        }
    }

    #[test]
    fn local_tools_only_requires_explicit_no_frozen_in_ci_and_rejects_future_controls() {
        for ci in [false, true] {
            for flags in [vec!["--tools-only"], vec!["--tools-only", "--no-frozen"]] {
                let request = request(&flags, ci).unwrap();
                assert!(validate_request(&request).is_ok());
                assert_eq!(
                    request.lock,
                    if ci && flags.len() == 1 {
                        LockMode::Frozen
                    } else {
                        LockMode::Write
                    }
                );
            }
            assert!(
                validate_request(
                    &request(&["--tools-only", "--no-frozen", "--update-lock"], ci).unwrap()
                )
                .is_ok()
            );
            for control in ["--no-lock", "--force", "--offline", "--plan"] {
                let request = request(&["--tools-only", "--no-frozen", control], ci).unwrap();
                if control == "--force" {
                    assert!(validate_request(&request).is_ok());
                } else {
                    assert!(matches!(
                        validate_request(&request),
                        Err(Error::NotImplemented)
                    ));
                }
            }
        }
    }

    #[test]
    fn ignore_scripts_overrides_repository_lifecycle_policy() {
        use turborepo_setup::lifecycle::{Intent, Origin};
        use turborepo_turbo_json::LifecycleScripts;
        let native = Some(LifecycleScripts::Native);
        let cli = request(&["--tools-only", "--ignore-scripts"], false).unwrap();
        assert!(validate_request(&cli).is_ok());
        let resolved = cli.lifecycle(native);
        assert_eq!(
            (resolved.intent, resolved.origin),
            (Intent::Skip, Origin::Cli)
        );
        let plain = request(&[], false).unwrap();
        let resolved = plain.lifecycle(Some(LifecycleScripts::Skip));
        assert_eq!(
            (resolved.intent, resolved.origin),
            (Intent::Skip, Origin::Repository)
        );
        assert_eq!(plain.lifecycle(None).origin, Origin::Default);
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
            assert!(matches!(
                validate_request(&request),
                Err(Error::NotImplemented)
            ));
        }
        let request =
            request(&["--force", "--offline", "--tools-only", "--no-lock"], true).unwrap();
        assert!(request.force && request.offline && request.tools_only);
        assert!(!request.update_lock);
    }
}
