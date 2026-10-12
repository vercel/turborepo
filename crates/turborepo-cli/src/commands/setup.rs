//! Native local/frozen provisioning and read-only tools-only checks/plans.
//! No execution.

use std::time::Instant;

use miette::Diagnostic;
use thiserror::Error;
use turborepo_setup::source_policy::OfficialSourcePolicy;
use turborepo_ui::ColorConfig;

use crate::cli::{Args, SetupArgs};

mod check;
mod lock_diagnostics;
mod no_lock;
mod plan;
mod provision;
mod root;
mod telemetry;

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
        "this setup mode is not implemented; use local or frozen provisioning, --plan \
         --tools-only reports or --check --tools-only readiness checks"
    )]
    #[diagnostic(
        code(turbo::setup::not_implemented),
        help(
            "Use the repository's documented toolchain and dependency installation steps for now. \
             No tools, dependencies, or locks were changed and no tasks were run. Dependency \
             plans and checks are not implemented."
        )
    )]
    NotImplemented,
    #[error("unsupported setup request: {0}")]
    Unsupported(&'static str),
    #[error("managed tools are not supported on musl Linux hosts such as Alpine")]
    #[diagnostic(
        code(turbo::setup::musl_host),
        help(
            "Managed setup installs official glibc builds. On musl hosts, install the \
             repository's tools in the image and use verify-system for them."
        )
    )]
    MuslHost,
    #[error("cannot classify this Linux host's C library: {0}")]
    #[diagnostic(
        code(turbo::setup::unclassified_libc),
        help(
            "turbo reads the ELF interpreter of /bin/sh and the libc.so.6 beside a glibc loader \
             without running them. Managed setup requires a glibc host with those files; \
             otherwise install the tools yourself and use verify-system."
        )
    )]
    UnclassifiedLibc(turborepo_platform::LibcError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LockDiagnostics(#[from] lock_diagnostics::Error),
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
    Registry(#[from] turborepo_setup::pnpm_provision::Error),
    #[error(transparent)]
    Native(#[from] turborepo_setup::native_baseline::Error),
    #[error(transparent)]
    Ephemeral(#[from] turborepo_setup::ephemeral_reconcile::Error),
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

/// The CLI entry point: setup, then standard telemetry about its outcome. Only
/// after the outcome is final, so telemetry cannot affect it.
pub fn run_with_telemetry(
    args: &Args,
    setup_args: &SetupArgs,
    color_config: ColorConfig,
) -> Result<i32, Error> {
    let started = Instant::now();
    let mut report = telemetry::Report::default();
    let result = run_observed(args, setup_args, &mut report);
    telemetry::record(&report, &result, started.elapsed(), color_config);
    result
}

/// Production setup without telemetry, so tests never initialize it.
#[cfg(test)]
pub fn run(args: &Args, setup_args: &SetupArgs) -> Result<i32, Error> {
    run_observed(args, setup_args, &mut telemetry::Report::default())
}

fn run_observed(
    args: &Args,
    setup_args: &SetupArgs,
    report: &mut telemetry::Report,
) -> Result<i32, Error> {
    let invocation = match args.cwd.as_deref() {
        Some(cwd) => turbopath::AbsoluteSystemPathBuf::from_cwd(cwd)?,
        None => turbopath::AbsoluteSystemPathBuf::cwd()?,
    };
    run_reporting(args, setup_args, None, report, || {
        Ok(OfficialSourcePolicy::inspect(
            invocation.as_std_path(),
            None,
        )?)
    })
}

// Trusted internal fixture injection only: never a CLI flag, repo URL or env
// override.
#[cfg(all(
    test,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
fn run_with_policy(
    args: &Args,
    setup_args: &SetupArgs,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<i32, Error> {
    run_reporting(
        args,
        setup_args,
        transports,
        &mut telemetry::Report::default(),
        preflight,
    )
}

fn run_reporting(
    args: &Args,
    setup_args: &SetupArgs,
    transports: Option<provision::Transports>,
    report: &mut telemetry::Report,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<i32, Error> {
    // Discovery reads files/Git metadata: no language-tool probes, environment
    // config pipeline, graph, package manager detection, or local CLI handoff.
    let discovery = root::Discovery::capture(args)?;
    tracing::debug!("setup root: {}", discovery.root_path());
    if !discovery.flags().experimental_setup {
        return Err(Error::Disabled);
    }
    report.gate_passed();

    let request = SetupRequest::new(setup_args, turborepo_ci::is_ci())?;
    report.request(&request);
    validate_request(&request)?;
    if args.test_run {
        return Err(Error::NotImplemented);
    }
    let eligible = discovery.source_eligibility();
    if eligible.cargo() || eligible.python() || eligible.go() {
        return Err(Error::Unsupported("non-JavaScript workspace setup"));
    }
    let snapshot =
        turborepo_setup::lock::Snapshot::capture(discovery.snapshot_root()?.as_std_path())?;
    // Never silently: version-manager files are reported, not read as requests.
    if let Some(report) = turborepo_setup::version_managers::report(
        &snapshot.ignored_declarations(),
        snapshot.declarations(),
    ) {
        eprintln!("{report}");
    }
    let provisioned = match request.mode {
        Mode::Check => return check::run(&discovery, snapshot, preflight),
        Mode::Plan => {
            println!(
                "{}",
                plan::report(&discovery, snapshot, &request, transports, preflight)?
            );
            return Ok(0);
        }
        // Clone-local --no-lock reports its own skipped dependencies.
        Mode::Provision if request.lock == LockMode::NoLock => {
            return no_lock::run(&discovery, snapshot, transports, preflight);
        }
        Mode::Provision => provision::run(
            &discovery,
            snapshot,
            request.lock,
            if request.update_lock {
                turborepo_setup::lock::reconcile::Mode::Refresh
            } else {
                turborepo_setup::lock::reconcile::Mode::Local
            },
            request.force,
            transports,
            preflight,
        )?,
    };
    report.downloaded_bytes = provisioned.downloaded_bytes;
    report.tools_installed = provisioned.installed;
    report.tools_reused = provisioned.reused;
    // Never claim dependencies are ready: they are skipped until dependency
    // policy and installers exist, even without --tools-only.
    println!(
        "Dependencies skipped ({}). No tasks run; managed activation is not enabled.",
        if request.tools_only {
            "--tools-only"
        } else {
            "dependency installation is not supported yet"
        }
    );
    Ok(0)
}

fn validate_request(request: &SetupRequest) -> Result<(), Error> {
    // Plans report locked reinstalls or proposed resolution, never both.
    let read_only = match request.mode {
        Mode::Check => !request.force && !request.update_lock,
        Mode::Plan => !(request.force && request.update_lock),
        Mode::Provision => false,
    };
    if read_only && request.tools_only && request.lock != LockMode::NoLock {
        // CI/frozen/offline never change read-only modes into resolution or repair.
        return Ok(());
    }
    // Force reinstalls existing locked selections, independently of local
    // Write normalization. Combined force/refresh remains fail-closed. Until
    // dependency installers exist, plain setup provisions exactly the tools
    // --tools-only does and reports dependencies as skipped.
    if request.mode != Mode::Provision
        || (request.lock == LockMode::NoLock && (request.force || request.update_lock))
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
    fn platform_qualification_follows_the_host_libc_not_the_build_target() {
        use turborepo_platform::Libc;
        let arch = cfg!(any(target_arch = "x86_64", target_arch = "aarch64"));
        let libc = turborepo_platform::host_libc();
        let host = cfg!(target_os = "macos") || matches!(libc, Ok(Some(Libc::Glibc { .. })));
        let platform = provision::platform();
        assert_eq!(platform.is_ok(), arch && host);
        if cfg!(target_os = "linux") && arch && matches!(libc, Ok(Some(Libc::Musl))) {
            assert!(matches!(platform, Err(Error::MuslHost)));
        }
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
    fn locked_plan_controls_remain_readonly_with_ci_frozen_offline_and_force() {
        for ci in [false, true] {
            for control in [None, Some("--frozen"), Some("--no-frozen")] {
                let mut flags = vec!["--plan", "--tools-only", "--offline", "--force"];
                flags.extend(control);
                let request = request(&flags, ci).unwrap();
                assert_eq!(request.mode, Mode::Plan);
                assert!(request.force && request.offline);
                assert!(!request.update_lock);
                assert!(validate_request(&request).is_ok());
            }
        }
    }

    #[test]
    fn plans_admit_refresh_proposals_but_never_with_force() {
        for ci in [false, true] {
            let mut flags = vec!["--plan", "--tools-only", "--no-frozen", "--update-lock"];
            assert!(validate_request(&request(&flags, ci).unwrap()).is_ok());
            flags.push("--force");
            assert!(matches!(
                validate_request(&request(&flags, ci).unwrap()),
                Err(Error::NotImplemented)
            ));
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
                if matches!(control, "--force" | "--no-lock" | "--plan") {
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
    fn plain_setup_provisions_like_tools_only_but_plan_and_dependency_checks_do_not() {
        for ci in [false, true] {
            for flags in [
                vec![],
                vec!["--frozen"],
                vec!["--no-frozen"],
                vec!["--force"],
                vec!["--no-frozen", "--update-lock"],
            ] {
                let request = request(&flags, ci).unwrap();
                assert!(!request.tools_only);
                assert!(validate_request(&request).is_ok(), "{flags:?}");
            }
        }
        for (flags, mode) in [
            (vec!["--plan", "--force", "--update-lock"], Mode::Plan),
            (vec!["--check"], Mode::Check),
            (vec!["--offline"], Mode::Provision),
            (vec!["--no-lock"], Mode::Provision),
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
