//! Native local/frozen/no-lock provisioning, then locked dependency setup
//! with the managed tools, plus read-only checks/plans. Never runs Turborepo
//! tasks.

use std::time::Instant;

use miette::Diagnostic;
use thiserror::Error;
use turborepo_repository::toolchain::ToolchainId;
use turborepo_setup::source_policy::OfficialSourcePolicy;
use turborepo_turbo_json::ToolPolicy;
use turborepo_ui::ColorConfig;

use crate::cli::{Args, SetupArgs};

#[cfg(target_os = "macos")]
mod acl;
pub(crate) mod authorization;
mod check;
mod cleanup;
#[cfg(windows)]
mod dacl;
mod dependencies;
pub(crate) mod effective;
mod lock_diagnostics;
#[cfg(windows)]
mod msvc;
mod no_lock;
mod offline;
mod plan;
mod provision;
mod root;
mod telemetry;
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
        "this setup mode is not implemented; use local or frozen provisioning, --plan reports or \
         --check readiness checks"
    )]
    #[diagnostic(
        code(turbo::setup::not_implemented),
        help(
            "Use the repository's documented toolchain and dependency installation steps for now. \
             No tools, dependencies, or locks were changed and no tasks were run."
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
            "turbo reads the ELF interpreter of /bin/sh, then the libc.so.6 beside a glibc loader \
             or the release a musl loader embeds, without running them. Managed setup requires a \
             glibc host with those files; otherwise install the tools yourself and use \
             verify-system."
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
             engines.node). --no-lock provisions only Node and an optional exact npm or pnpm. No \
             tools, dependencies, or locks were changed."
        )
    )]
    NoLockNode,
    #[error(
        "--no-lock provisions only Node with npm or pnpm, but the repository declares {0}; \
         clone-local setup for it is not supported yet"
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
    Dependencies(#[from] dependencies::Error),
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
    #[cfg(windows)]
    exit_on_relayed_ctrl_c()?;
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
#[cfg(all(
    test,
    any(
        all(
            any(target_arch = "x86_64", target_arch = "aarch64"),
            any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
        ),
        all(windows, target_arch = "x86_64")
    )
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
    // Rust, Python and Go have managed setup adapters; a skip policy ignores
    // their declarations. Managed run derives the same ecosystems.
    let ecosystems = effective::ecosystems(&discovery.flags(), discovery.policy());
    if plan.unsupported() {
        return Err(Error::Unsupported("verify-system non-JavaScript tools"));
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
        // Python declarations are captured only when experimentalPythonWorkspaces
        // is on; otherwise uv.toml/pyproject.toml are neither read nor locked.
        ecosystems,
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
    // Managed Rust's Windows linker is an external prerequisite: reported,
    // never installed, and never a failure.
    #[cfg(windows)]
    if plan
        .toolchain(&ToolchainId::RUST)
        .is_some_and(|t| t.tools.0 != ToolPolicy::Skip)
        && snapshot.declarations().contains_key("rust")
    {
        msvc::report();
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
    // Decided before any download, so a missing ecosystem lockfile fails first.
    let dependencies = dependencies::Dependencies::select(&discovery, &plan, &snapshot, &request)?;
    let ecosystems = snapshot.ecosystems();
    let resolution = if request.update_lock {
        turborepo_setup::lock::reconcile::Mode::Refresh
    } else {
        turborepo_setup::lock::reconcile::Mode::Local
    };
    let trust_base = transports
        .as_ref()
        .map(provision::Transports::trust_base)
        .transpose()?;
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
        return check::run(&discovery, snapshot, &dependencies, preflight);
    }
    if request.mode == Mode::Plan {
        println!(
            "{}",
            plan::report(
                &discovery,
                snapshot,
                &request,
                &dependencies,
                transports,
                preflight
            )?
        );
        return Ok(0);
    }
    // Offline setup reuses the trusted installation or verified cached bytes;
    // enabled dependencies then install offline from the native caches.
    // Clone-local --no-lock runs only when every dependency is skipped
    // (`validate`).
    let provisioned = if request.offline {
        offline::run(&discovery, snapshot, &request, transports, preflight)
    } else if request.lock == LockMode::NoLock {
        no_lock::run(&discovery, snapshot, transports, preflight)
    } else {
        provision::run(&discovery, snapshot, &request, transports, preflight)
    }?;
    report.downloaded_bytes = provisioned.downloaded_bytes;
    report.tools_installed = provisioned.installed;
    report.tools_reused = provisioned.reused;
    let root = discovery.snapshot_root()?.as_std_path();
    Ok(dependencies.install(root, ecosystems, trust_base)?)
}

/// Whether setup provisions for this host. On Windows only a confirmed x64
/// machine qualifies: arm64 hosts, including x64 turbo under emulation, and
/// unidentified hosts are refused.
pub(crate) fn platform_supported() -> bool {
    provision::platform().is_ok()
}

/// Records `root`'s current installation as trusted under `user_base`, or the
/// real user state directory, exactly as setup publishes it, so binary tests
/// can drive the shell hook. Returns the record's path. Fixture injection only.
#[cfg(feature = "test-support")]
pub fn record_trust_for_tests(
    root: &std::path::Path,
    user_base: Option<&std::path::Path>,
) -> Result<std::path::PathBuf, String> {
    let base = match user_base {
        Some(base) => base.to_owned(),
        None => dirs_next::data_local_dir().ok_or("cannot locate the user state directory")?,
    };
    let store = turborepo_tool_install::Store::open(root).map_err(|e| e.to_string())?;
    let trust = trust::TrustStore::new(&base, root).map_err(|e| e.to_string())?;
    trust.record(&store).map_err(|e| e.to_string())?;
    Ok(trust.path())
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
    setup_for_tests(
        invocation,
        user_base,
        origin,
        &["--frozen", "--tools-only"],
        policy,
    )
}

/// `turbo setup <flags>` from `invocation`, like `frozen_tools_for_tests`, so
/// binary tests can also create `turbo.lock` from loopback release metadata.
#[cfg(feature = "test-support")]
pub fn setup_for_tests(
    invocation: &std::path::Path,
    user_base: &std::path::Path,
    origin: &str,
    flags: &[&str],
    policy: impl Fn() -> Result<OfficialSourcePolicy, turborepo_setup::source_policy::Error>,
) -> Result<i32, String> {
    use turborepo_setup::{
        node_provision::NodeTransport, pnpm_provision::PnpmTransport,
        registry_resolution::RegistryTransport,
    };
    let words = ["turbo", "setup"]
        .iter()
        .chain(flags)
        .chain(&["--cwd"])
        .map(std::ffi::OsString::from)
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
    // Like run_with_policy, never telemetry.
    run_reporting(
        &args,
        setup_args,
        Some(transports),
        &mut telemetry::Report::default(),
        || Ok(policy()?),
    )
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
        Mode::Plan => !(request.force && request.update_lock),
        Mode::Provision => false,
    };
    if read_only && request.lock != LockMode::NoLock {
        // CI/frozen/offline never change read-only modes into resolution or repair.
        return Ok(());
    }
    // Force reinstalls existing locked selections, independently of local
    // Write normalization. Combined force/refresh remains fail-closed.
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
    // Installers run with the activation that turbo.lock records. Offline,
    // they use their native offline modes (`dependencies`).
    if !tools_only && request.lock == LockMode::NoLock {
        return Err(Error::Unsupported(
            "--no-lock cannot install locked dependencies, because installers run with the \
             managed tools that turbo.lock records; add --tools-only",
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
        // An x64 build qualifies only on an x64 host, not under arm64 emulation.
        let windows = cfg!(all(windows, target_arch = "x86_64"))
            && provision::windows_host_machine() == Some(0x8664);
        let platform = provision::platform();
        assert_eq!(platform.is_ok(), (arch && host) || windows);
        if cfg!(target_os = "linux") && arch && matches!(libc, Ok(Some(Libc::Musl { .. }))) {
            assert!(matches!(platform, Err(Error::MuslHost)));
        }
        if cfg!(windows) && provision::windows_host_machine() == Some(0xAA64) {
            assert!(
                platform.is_err_and(|e| e.to_string().contains("not qualified for Windows arm64"))
            );
        }
    }

    #[test]
    fn windows_x64_builds_qualify_only_on_an_identified_x64_host() {
        assert!(matches!(
            provision::windows_x64_platform(Some(0x8664)),
            Ok(turborepo_setup::lock::Platform::WindowsX64)
        ));
        let arm64 = provision::windows_x64_platform(Some(0xAA64)).unwrap_err();
        assert!(
            arm64
                .to_string()
                .contains("including x64 turbo under emulation; use WSL")
        );
        // x86, unknown, and an unanswered host query all fail closed.
        for host in [Some(0x014C), Some(0), None] {
            assert!(provision::windows_x64_platform(host).is_err(), "{host:?}");
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
    fn plain_setup_plans_and_checks_include_locked_dependencies() {
        for ci in [false, true] {
            for flags in [
                vec![],
                vec!["--frozen"],
                vec!["--no-frozen"],
                vec!["--force"],
                vec!["--no-frozen", "--update-lock"],
                vec!["--plan"],
                vec!["--check"],
                vec!["--check", "--offline"],
                // Native offline installs (`dependencies`).
                vec!["--offline"],
                vec!["--frozen", "--offline"],
                vec!["--force", "--offline"],
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
        // --no-lock (#14486) cannot install locked dependencies; with
        // dependencies skipped it still provisions.
        let no_lock = request(&["--no-lock"], false).unwrap();
        assert_eq!(no_lock.mode, Mode::Provision);
        assert!(matches!(
            validate_request(&no_lock),
            Err(Error::Unsupported(_))
        ));
        assert!(validate(&no_lock, true).is_ok());
    }
}
