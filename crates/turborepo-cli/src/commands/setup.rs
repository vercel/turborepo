//! Native local/frozen/no-lock provisioning, read-only checks and tools-only
//! plans. No execution.

use miette::Diagnostic;
use thiserror::Error;
use turborepo_repository::toolchain::ToolchainId;
use turborepo_setup::source_policy::OfficialSourcePolicy;
use turborepo_turbo_json::ToolPolicy;

use crate::cli::{Args, SetupArgs};

#[cfg(target_os = "macos")]
mod acl;
pub(crate) mod authorization;
mod check;
mod cleanup;
#[cfg(windows)]
mod dacl;
mod effective;
mod lock_diagnostics;
mod no_lock;
mod offline;
mod plan;
mod provision;
mod root;
pub(crate) mod trust;
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
        "this setup mode is not implemented; use local or frozen provisioning, --plan \
         --tools-only reports or --check readiness checks"
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
    #[error(
        "offline setup is missing verified cached artifacts; nothing was installed:\n{0}\nRun \
         turbo setup --tools-only without --offline to download and verify them."
    )]
    OfflineMissing(String),
    #[error("--no-lock needs a Node declaration in the repository root")]
    #[diagnostic(
        code(turbo::setup::no_lock_node),
        help(
            "Declare Node in .nvmrc, .node-version, or package.json (devEngines.runtime or \
             engines.node). --no-lock provisions only Node and an optional exact pnpm. No tools, \
             dependencies, or locks were changed."
        )
    )]
    NoLockNode,
    #[error(
        "--no-lock provisions only Node and pnpm, but the repository declares {0}; clone-local \
         setup for it is not supported yet"
    )]
    #[diagnostic(
        code(turbo::setup::no_lock_tool),
        help("No tools, dependencies, or locks were changed.")
    )]
    NoLockTool(String),
    #[error(transparent)]
    SourcePolicy(#[from] turborepo_setup::source_policy::Error),
    #[error(transparent)]
    Mirror(#[from] turborepo_setup::mirror::Error),
    #[error(transparent)]
    Activation(#[from] turborepo_setup::activation::Error),
    #[error(transparent)]
    Offline(#[from] offline::Error),
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
    Uv(#[from] turborepo_setup::uv_provision::Error),
    #[error(transparent)]
    UvResolution(#[from] turborepo_setup::uv_resolution::Error),
    #[error(transparent)]
    Rustup(#[from] turborepo_setup::rustup_provision::Error),
    #[error(transparent)]
    Python(#[from] turborepo_setup::python_provision::Error),
    #[error(transparent)]
    PythonResolution(#[from] turborepo_setup::python_resolution::Error),
    #[error(transparent)]
    Rust(#[from] turborepo_setup::rust_provision::Error),
    #[error(transparent)]
    Artifact(#[from] turborepo_setup::artifact_provision::Error),
    #[error(
        "setup.additionalTools changed for {0:?}; run `turbo setup` locally and commit the \
         updated turbo.lock"
    )]
    AdditionalToolDrift(std::collections::BTreeSet<String>),
    #[error(transparent)]
    Go(#[from] turborepo_setup::go_provision::Error),
    #[error(transparent)]
    GoResolution(#[from] turborepo_setup::go_resolution::Error),
    #[error(transparent)]
    Bun(#[from] turborepo_setup::bun_provision::Error),
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
    #[cfg(windows)]
    #[error(transparent)]
    Signal(#[from] turborepo_signals::listeners::Error),
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
    #[cfg(windows)]
    exit_on_relayed_ctrl_c()?;
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

/// Setup runs before turbo's signal handler and has no graceful phase, so a
/// console Ctrl+C ends it through the default handler. The npm wrapper relays
/// Ctrl+C over a pipe instead under a package manager terminal; end setup the
/// same way, so locks release with the process and the transaction resumes.
#[cfg(windows)]
fn exit_on_relayed_ctrl_c() -> Result<(), Error> {
    const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;
    if let Some(mut relay) = turborepo_signals::listeners::wrapper_ctrl_c()? {
        std::thread::spawn(move || {
            if let Some(Some(_)) = relay.blocking_recv() {
                crate::process_exit::exit(STATUS_CONTROL_C_EXIT as i32);
            }
        });
    }
    Ok(())
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
        return Err(Error::Unsupported("verify-system non-JavaScript tools"));
    }
    if plan.explicit_dependencies() {
        return Err(Error::NotImplemented);
    }
    let skipped = |id: &ToolchainId| {
        plan.toolchain(id)
            .is_some_and(|t| t.tools.0 == ToolPolicy::Skip)
    };
    let javascript = plan
        .toolchain(&ToolchainId::JAVASCRIPT)
        .map_or(ToolPolicy::Skip, |t| t.tools.0);
    // Preserve current workspace gating, with explicit policy skips disabling
    // interpretation of that ecosystem's declarations.
    let snapshot = turborepo_setup::lock::Snapshot::capture_with(
        discovery.snapshot_root()?.as_std_path(),
        turborepo_setup::lock::Ecosystems {
            python: eligible.python() && !skipped(&ToolchainId::PYTHON),
            rust: eligible.cargo() && !skipped(&ToolchainId::RUST),
            go: eligible.go() && !skipped(&ToolchainId::GO),
        },
    )?;
    if snapshot.previous_lock().is_some_and(|lock| {
        lock.tools().values().any(|tool| {
            let id = match tool.adapter.as_str() {
                "node" | "npm" | "pnpm" | "yarn" => Some(ToolchainId::JAVASCRIPT),
                "rust" | "rustup" => Some(ToolchainId::RUST),
                "uv" | "python" => Some(ToolchainId::PYTHON),
                "go" => Some(ToolchainId::GO),
                _ => None,
            };
            id.is_some_and(|id| skipped(&id))
        })
    }) {
        return Err(Error::Unsupported(
            "turbo.lock selects tools that the setup policy skips; remove the skip or the locked \
             tools",
        ));
    }
    for line in plan.report() {
        println!("{line}");
    }
    if javascript == ToolPolicy::Skip {
        // The current snapshot has no JavaScript enable switch. Never provision
        // JavaScript against an explicit skip while preparing a native cohort.
        if plan.all_tools_skipped() {
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
        if snapshot
            .declarations()
            .keys()
            .any(|id| matches!(id.as_str(), "node" | "npm" | "pnpm" | "yarn"))
        {
            return Err(Error::Unsupported(
                "JavaScript skip with declared JavaScript tools and managed native tools",
            ));
        }
    }
    // Never silently: version-manager files are reported, not read as requests.
    if let Some(report) = turborepo_setup::version_managers::report(
        &snapshot.ignored_declarations(),
        snapshot.declarations(),
    ) {
        eprintln!("{report}");
    }
    let resolution = if request.update_lock {
        turborepo_setup::lock::reconcile::Mode::Refresh
    } else {
        turborepo_setup::lock::reconcile::Mode::Local
    };
    if javascript == ToolPolicy::VerifySystem {
        return match request.mode {
            Mode::Check => verify_system::check(&discovery, snapshot),
            Mode::Plan => Err(Error::Unsupported("verify-system setup plans")),
            Mode::Provision => verify_system::provision(
                &discovery,
                snapshot,
                request.lock,
                resolution,
                request.force,
            ),
        };
    }
    if request.mode == Mode::Check {
        return check::run(&discovery, snapshot, request.tools_only, preflight);
    }
    if request.mode == Mode::Plan {
        println!(
            "{}",
            plan::report(&discovery, snapshot, &request, transports, preflight)?
        );
        return Ok(0);
    }
    let code = if request.offline {
        offline::run(&discovery, snapshot, &request, transports, preflight)
    } else if request.lock == LockMode::NoLock {
        no_lock::run(&discovery, snapshot, transports, preflight)
    } else {
        provision::run(&discovery, snapshot, &request, transports, preflight)
    }?;
    println!(
        "Dependencies skipped ({}). No tasks run.",
        if request.tools_only {
            "--tools-only"
        } else {
            "dependency installation is not supported yet"
        }
    );
    Ok(code)
}

/// `turbo setup --frozen --tools-only` against loopback fixture artifacts,
/// publishing trust evidence under `user_base`, so binary tests can exercise
/// real managed activation. Fixture injection only, like `run_with_policy`.
#[cfg(feature = "test-support")]
pub fn frozen_tools_for_tests(
    invocation: &std::path::Path,
    user_base: &std::path::Path,
    origin: &str,
    policy: impl Fn() -> Result<OfficialSourcePolicy, turborepo_setup::source_policy::Error>,
) -> Result<i32, String> {
    use turborepo_setup::{
        node_provision::NodeTransport, pnpm_provision::PnpmTransport,
        registry_resolution::RegistryTransport,
    };
    let words = ["turbo", "setup", "--frozen", "--tools-only", "--cwd"]
        .map(std::ffi::OsString::from)
        .into_iter()
        .chain([invocation.as_os_str().to_owned()])
        .collect();
    let args = Args::parse_args(words)?;
    let Some(crate::cli::Command::Setup { setup_args }) = &args.command else {
        unreachable!("the words select setup");
    };
    let transports = provision::Transports {
        trust_base: Some(user_base.to_owned()),
        registry: RegistryTransport::loopback_http_for_tests(origin).map_err(|e| e.to_string())?,
        node: NodeTransport::loopback_http_for_tests(origin).map_err(|e| e.to_string())?,
        pnpm: PnpmTransport::loopback_http_for_tests(origin).map_err(|e| e.to_string())?,
        uv: turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(origin)
            .map_err(|e| e.to_string())?,
        rustup: turborepo_setup::rustup_provision::RustupTransport::loopback_http_for_tests(origin)
            .map_err(|e| e.to_string())?,
        python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(
            origin,
        )
        .map_err(|e| e.to_string())?,
        artifact: None,
        go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(origin)
            .map_err(|e| e.to_string())?,
        bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(origin)
            .map_err(|e| e.to_string())?,
    };
    run_with_policy(&args, setup_args, Some(transports), || Ok(policy()?))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
fn validate_request(request: &SetupRequest) -> Result<(), Error> {
    validate(request, request.tools_only)
}

fn validate(request: &SetupRequest, tools_only: bool) -> Result<(), Error> {
    // Plans report locked reinstalls or proposed resolution, never both.
    let read_only = match request.mode {
        Mode::Check => !request.force && !request.update_lock,
        Mode::Plan => tools_only && !(request.force && request.update_lock),
        Mode::Provision => false,
    };
    if read_only && request.lock != LockMode::NoLock {
        // CI/frozen/offline never change read-only modes into resolution or repair.
        return Ok(());
    }
    // Force reinstalls existing locked selections, independently of local
    // Write normalization. Combined force/refresh remains fail-closed. Until
    // dependency installers exist, plain setup provisions and checks exactly
    // the tools --tools-only does and reports dependencies as skipped.
    if request.mode != Mode::Provision
        || (request.lock == LockMode::NoLock
            && (request.force || request.update_lock || request.offline))
        || (request.force && request.update_lock)
    {
        return Err(Error::NotImplemented);
    }
    // Offline reinstalls only verified local bytes; resolution needs the network.
    if request.offline && request.update_lock {
        return Err(Error::Unsupported(
            "--offline cannot resolve --update-lock; run turbo setup --tools-only --no-frozen \
             --update-lock without --offline",
        ));
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

#[cfg(all(test, windows, target_arch = "x86_64"))]
#[path = "setup/windows_tests.rs"]
mod windows_provisioning_tests;

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
        let windows = cfg!(all(windows, target_arch = "x86_64"));
        let platform = provision::platform();
        assert_eq!(platform.is_ok(), (arch && host) || windows);
        if cfg!(target_os = "linux") && arch && matches!(libc, Ok(Some(Libc::Musl))) {
            assert!(matches!(platform, Err(Error::MuslHost)));
        }
        if cfg!(all(windows, target_arch = "aarch64")) {
            assert!(
                platform.is_err_and(|e| e.to_string().contains("not qualified for Windows arm64"))
            );
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
    fn checks_accept_ci_frozen_and_offline_without_resolution() {
        for ci in [false, true] {
            for flags in [
                vec!["--check", "--tools-only"],
                vec!["--check", "--tools-only", "--offline"],
                vec!["--check", "--tools-only", "--frozen"],
                vec!["--check", "--tools-only", "--no-frozen", "--offline"],
                vec!["--check"],
                vec!["--check", "--offline"],
                vec!["--check", "--frozen"],
                vec!["--check", "--no-frozen", "--offline"],
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
            assert!(matches!(
                validate_request(
                    &request(
                        &["--tools-only", "--no-frozen", "--update-lock", "--offline"],
                        ci
                    )
                    .unwrap()
                ),
                Err(Error::Unsupported(_))
            ));
            for control in ["--no-lock", "--force", "--offline", "--plan"] {
                let request = request(&["--tools-only", "--no-frozen", control], ci).unwrap();
                if matches!(control, "--force" | "--no-lock" | "--plan" | "--offline") {
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
    fn plain_setup_provisions_and_checks_like_tools_only_but_plan_does_not() {
        for ci in [false, true] {
            for flags in [
                vec![],
                vec!["--frozen"],
                vec!["--no-frozen"],
                vec!["--force"],
                vec!["--no-lock"],
                vec!["--offline"],
                vec!["--offline", "--force"],
                vec!["--no-frozen", "--update-lock"],
            ] {
                let request = request(&flags, ci).unwrap();
                assert!(!request.tools_only);
                assert!(validate_request(&request).is_ok(), "{flags:?}");
            }
        }
        for (flags, mode) in [
            (vec!["--plan", "--force", "--update-lock"], Mode::Plan),
            (vec!["--no-lock", "--force"], Mode::Provision),
            (vec!["--no-lock", "--offline"], Mode::Provision),
            (vec!["--no-lock", "--force", "--offline"], Mode::Provision),
        ] {
            let request = request(&flags, false).unwrap();
            assert_eq!(request.mode, mode);
            assert!(matches!(
                validate_request(&request),
                Err(Error::NotImplemented)
            ));
        }
        let no_lock =
            request(&["--force", "--offline", "--tools-only", "--no-lock"], true).unwrap();
        assert!(no_lock.force && no_lock.offline && no_lock.tools_only);
        assert!(!no_lock.update_lock);
        assert!(matches!(
            validate_request(&no_lock),
            Err(Error::NotImplemented)
        ));
        for ci in [false, true] {
            let refresh = request(&["--offline", "--no-frozen", "--update-lock"], ci).unwrap();
            assert!(matches!(
                validate_request(&refresh),
                Err(Error::Unsupported(message)) if message.starts_with("--offline cannot resolve --update-lock")
            ));
        }
    }
}
