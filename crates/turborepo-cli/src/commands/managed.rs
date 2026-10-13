//! Exact-root strict activation shared by `turbo run`, `turbo exec` and the
//! commands that discover the repository. No provisioning, HTTP or ancestor
//! inventory search. Only [`activate_probes`] changes the process
//! environment, after selection.

use std::{collections::BTreeSet, ffi::OsString, path::Path, sync::Arc};

use turborepo_run::{EnvironmentVariableMap, ManagedEnvironment, ManagedRun};
use turborepo_setup::{
    activation::External,
    execution_identity::{ExecutionContext, ExecutionSnapshot, Libc, SelectedInstallation},
    lock::{self, Ecosystems},
    system_tool,
};
use turborepo_shim::capabilities::Requirement;
use turborepo_tool_install::Store;
use turborepo_turbo_json::{FutureFlags, RawTurboJson, SetupPolicy};

use crate::commands::setup::{
    authorization::{self, Authorization},
    effective,
};

pub(crate) fn select(
    base: &crate::commands::CommandBase,
) -> Result<Option<Arc<ManagedRun>>, turborepo_run::Error> {
    select_with(base, true)
}

/// Validates managed state like `select`, before local CLI handoff or
/// telemetry, but leaves the verify-system probes to the run itself.
pub(crate) fn validate(base: &crate::commands::CommandBase) -> Result<bool, turborepo_run::Error> {
    Ok(requirement(base)?.is_some())
}

/// [`validate`], reporting what a repository-local CLI must support to run
/// the validated selection.
pub(crate) fn requirement(
    base: &crate::commands::CommandBase,
) -> Result<Option<Requirement>, turborepo_run::Error> {
    Ok(select_with(base, false)?.map(|run| peer_requirement(run.snapshot())))
}

/// The selection's lock schema, adapters, verify-system tools, and whether a
/// `turbo setup --no-lock` record selected it, named as CLIs advertise them.
fn peer_requirement(snapshot: &ExecutionSnapshot) -> Requirement {
    let features = snapshot.tools().values().flat_map(|tool| {
        let external = matches!(tool.installation, SelectedInstallation::VerifySystem { .. });
        [
            Some(tool.adapter.as_str()),
            external.then_some("verify-system"),
        ]
    });
    let clone_local = snapshot.clone_local().then_some("clone-local");
    Requirement::new(
        lock::SCHEMA_VERSION,
        features.chain([clone_local]).flatten(),
    )
}

/// Whether repository discovery may run a tool that setup manages. Under their
/// workspace flags, Cargo, uv and Go discovery run their toolchains on demand
/// (`cargo metadata`, `uv workspace metadata`, `go env`), so selection comes
/// before any discovery. JavaScript discovery runs no tool, so a
/// JavaScript-only repository needs no managed selection.
pub(crate) fn discovery_probes_tools(flags: &FutureFlags) -> bool {
    flags.experimental_cargo_workspaces
        || flags.experimental_python_workspaces
        || flags.experimental_go_workspaces
}

/// [`select`] for a command that discovers the repository without running
/// tasks (`turbo ls`, `turbo query`, `turbo boundaries`, `turbo devtools` and
/// the task listing): `None` unless its discovery may probe a tool.
pub(crate) fn select_for_discovery(
    base: &crate::commands::CommandBase,
) -> Result<Option<Arc<ManagedRun>>, turborepo_run::Error> {
    if discovery_probes_tools(&base.opts.future_flags) {
        select(base)
    } else {
        Ok(None)
    }
}

/// [`select_for_discovery`], activated for turbo's own probes before any
/// discovery exactly like `turbo run`. Missing, stale or damaged setup fails
/// with `turbo setup` guidance instead of probing tools from the host PATH.
pub(crate) fn activate_for_discovery(
    base: &crate::commands::CommandBase,
) -> Result<Option<Arc<ManagedRun>>, turborepo_run::Error> {
    let managed = select_for_discovery(base)?;
    if let Some(tools) = &managed {
        activate_probes(tools)?;
    }
    Ok(managed)
}

fn select_with(
    base: &crate::commands::CommandBase,
    external: bool,
) -> Result<Option<Arc<ManagedRun>>, turborepo_run::Error> {
    // Every run checks for managed setup, so an unmanaged repository must not
    // depend on the user state directory.
    let user_base = || {
        dirs_next::data_local_dir().ok_or_else(|| "cannot locate setup user state directory".into())
    };
    ManagedTools::inspect_with(
        base.repo_root.as_std_path(),
        base.opts.repo_opts.root_turbo_json_path.as_std_path(),
        user_base,
        context,
        external,
    )
    .map_err(turborepo_run::Error::ManagedTools)
}

pub(crate) struct ManagedTools {
    authorization: Authorization,
    inherited_path: Option<OsString>,
    identity: String,
    ci: bool,
    clone_local: bool,
}

impl ManagedTools {
    #[cfg(test)]
    pub(crate) fn inspect(
        root: &Path,
        config: &Path,
        user_base: &Path,
        context: impl FnOnce() -> Result<ExecutionContext, String>,
    ) -> Result<Option<Arc<ManagedRun>>, String> {
        Self::inspect_with(root, config, || Ok(user_base.to_owned()), context, true)
    }

    /// A root turbo.lock, or without one the sealed selection of this clone's
    /// `turbo setup --no-lock` generation, selects managed mode. Either way
    /// activation is equally strict and never downloads.
    fn inspect_with(
        root: &Path,
        config: &Path,
        user_base: impl FnOnce() -> Result<std::path::PathBuf, String>,
        context: impl FnOnce() -> Result<ExecutionContext, String>,
        external: bool,
    ) -> Result<Option<Arc<ManagedRun>>, String> {
        let clone_local = match std::fs::symlink_metadata(root.join("turbo.lock")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Lock-only leftovers and ordinary repositories stay unmanaged.
                if !Store::selects_record(root).map_err(|e| {
                    format!("cannot read managed tools; run turbo setup --no-lock: {e}")
                })? {
                    return Ok(None);
                }
                true
            }
            Err(error) => return Err(format!("cannot read turbo.lock; run turbo setup: {error}")),
            Ok(_) => false,
        };
        let canonical = root.canonicalize().map_err(|e| e.to_string())?;
        if config
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .as_deref()
            != Some(&canonical)
            || !matches!(
                config.file_name().and_then(|n| n.to_str()),
                Some("turbo.json" | "turbo.jsonc")
            )
        {
            return Err(
                "managed run requires a root turbo.json or turbo.jsonc; run turbo setup".into(),
            );
        }
        let user_base = user_base()?;
        let ci = turborepo_ci::is_ci();
        let explain = |e: authorization::Error| guidance(&e, ci, clone_local);
        // `--no-lock` setup is Node/npm/pnpm only, so a clone-local selection
        // uses the default ecosystems; a lock uses exactly setup's.
        let authorization = if clone_local {
            Authorization::inspect_clone_local(&canonical, &user_base, context()?)
        } else {
            // The root configuration holds setup policy and additional-tool
            // declarations; report its errors like other invalid declarations.
            let ecosystems = ecosystems(&canonical, config).map_err(|e| {
                format!("tool declarations are invalid; fix them, then run turbo setup: {e}")
            })?;
            Authorization::inspect(&canonical, &user_base, context()?, ecosystems)
        }
        .map_err(explain)?;
        let (snapshot, tools) = authorization
            .with_current(|p| (p.snapshot().clone(), p.external().to_vec()))
            .map_err(explain)?;
        let identity = snapshot.fingerprint().to_owned();
        let inherited_path = std::env::var_os("PATH");
        // Once per run: a changed system tool fails here rather than running
        // under the locked identity. Tasks then find it on the same PATH.
        let notices = if external {
            verify_external(&tools, inherited_path.as_deref(), &canonical)?
        } else {
            Vec::new()
        };
        let tools = Arc::new(Self {
            authorization,
            inherited_path,
            identity,
            ci,
            clone_local,
        });
        tools.revalidate()?;
        Ok(Some(Arc::new(
            ManagedRun::new(snapshot, tools)?.with_notices(notices),
        )))
    }
}

fn guidance(error: &authorization::Error, ci: bool, clone_local: bool) -> String {
    if clone_local {
        error.no_lock_guidance()
    } else {
        error.guidance(ci)
    }
}

/// Verifies each locked verify-system tool at its exact version on `path`,
/// exactly as `turbo setup --check` does, and reports where it was found.
fn verify_external(
    tools: &[External],
    path: Option<&std::ffi::OsStr>,
    root: &Path,
) -> Result<Vec<String>, String> {
    let mut notices = Vec::new();
    for tool in tools {
        let verified =
            system_tool::verify_tool(&tool.id, &tool.version, &tool.executables, path, root)
                .map_err(|e| format!("verify-system {}: {e}", tool.id))?;
        notices.extend(verified.into_iter().map(|tool| {
            let path = tool.path.display();
            format!(
                "{} {} from PATH ({path}, verify-system)",
                tool.id, tool.version
            )
        }));
    }
    Ok(notices)
}

/// Setup's ecosystems from the root configuration's flags and `setup` policy.
/// The snapshot's CAS covers the configuration, so a later edit is drift.
fn ecosystems(root: &Path, config: &Path) -> Result<Ecosystems, String> {
    let path = |path| turbopath::AbsoluteSystemPath::from_std_path(path).map_err(|e| e.to_string());
    let raw = RawTurboJson::read(path(root)?, path(config)?, true)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let flags = raw.future_flags.map(|f| *f.as_inner()).unwrap_or_default();
    let policy = SetupPolicy::from_raw(raw.setup.as_ref(), &flags).map_err(|e| e.to_string())?;
    Ok(effective::ecosystems(&flags, &policy))
}

/// turbo's own toolchain probes (`cargo metadata`, `rustc`, uv and `go env`)
/// and the Cargo home read for config hashing all come from turbo's process
/// environment, so apply the freshly validated managed PATH and runtime
/// variables to it before the run builder discovers the repository. Tasks
/// still receive the same overlay from each later revalidation, and every
/// in-repository executable other than the managed bin stays untrusted.
pub(crate) fn activate_probes(tools: &ManagedRun) -> Result<(), turborepo_run::Error> {
    let overrides = tools
        .revalidate()
        .map_err(turborepo_run::Error::ManagedTools)?;
    if let Some(bin) = overrides
        .get("PATH")
        .and_then(|path| std::env::split_paths(path).next())
    {
        turborepo_repository::uv::trust_managed_bin(bin);
    }
    for (key, value) in overrides.iter() {
        // SAFETY: like the shim warning's removal in `run`, this happens once,
        // before the run builder starts discovery threads or task processes.
        unsafe { std::env::set_var(key, value) };
    }
    Ok(())
}

impl ManagedEnvironment for ManagedTools {
    fn revalidate(&self) -> Result<EnvironmentVariableMap, String> {
        self.authorization
            .with_current(|plan| {
                let mut overrides = EnvironmentVariableMap::default();
                // verify-system tools alone keep the caller's PATH unchanged.
                if plan.path_prepend().is_some() {
                    let path = plan
                        .path(self.inherited_path.as_deref())
                        .map_err(|e| e.to_string())?;
                    overrides.insert(
                        "PATH".into(),
                        path.into_string().map_err(|_| {
                            "managed PATH is not UTF-8; run from a UTF-8 repository path".to_owned()
                        })?,
                    );
                }
                for (key, value) in plan.runtime_env() {
                    overrides.insert(
                        key.clone(),
                        value
                            .clone()
                            .into_string()
                            .map_err(|_| "managed runtime variable is not UTF-8".to_owned())?,
                    );
                }
                Ok(overrides)
            })
            .map_err(|e| guidance(&e, self.ci, self.clone_local))?
    }

    fn cache_identity(&self) -> &str {
        &self.identity
    }
}

/// Conservative native execution identity. No readiness-only/unprobed labels.
/// These initial adapters are supported only on native macOS, glibc Linux and
/// Windows x64. Windows arm64 is unqualified and never borrows x64 artifacts.
/// musl Linux gets its own identity, and the activation plan qualifies it only
/// for locks whose every tool is verify-system.
pub(crate) fn context() -> Result<ExecutionContext, String> {
    if !cfg!(any(
        target_os = "macos",
        target_os = "linux",
        all(windows, target_arch = "x86_64")
    )) {
        return Err("managed run is unsupported on this host".into());
    }
    let libc = host_libc(turborepo_platform::host_libc())?;
    let mut cpu = BTreeSet::from(["baseline".into()]);
    #[cfg(target_arch = "x86_64")]
    for (name, detected) in [
        ("avx", std::is_x86_feature_detected!("avx")),
        ("avx2", std::is_x86_feature_detected!("avx2")),
        ("avx512f", std::is_x86_feature_detected!("avx512f")),
        ("sse4.2", std::is_x86_feature_detected!("sse4.2")),
    ] {
        if detected {
            cpu.insert(name.into());
        }
    }
    #[cfg(target_arch = "aarch64")]
    for (name, detected) in [
        ("neon", std::arch::is_aarch64_feature_detected!("neon")),
        ("aes", std::arch::is_aarch64_feature_detected!("aes")),
        ("sve", std::arch::is_aarch64_feature_detected!("sve")),
    ] {
        if detected {
            cpu.insert(name.into());
        }
    }
    use sysinfo::SystemExt;
    let system = sysinfo::System::new();
    let release = system
        .kernel_version()
        .ok_or("cannot qualify managed host kernel")?;
    ExecutionContext::new(
        turborepo_platform::Platform::current(),
        libc,
        format!(
            "{}-{}-{release}",
            std::env::consts::ARCH,
            std::env::consts::OS
        ),
        cpu,
    )
    .map_err(|e| e.to_string())
}

/// The host's C library, not the one turbo was built against: released Linux
/// binaries are static musl builds, and they select glibc artifacts on glibc
/// hosts exactly like setup does. The glibc or musl release is the ABI label,
/// so task caches never cross C libraries or their releases.
fn host_libc(
    host: Result<Option<turborepo_platform::Libc>, turborepo_platform::LibcError>,
) -> Result<Libc, String> {
    match host {
        Ok(None) => Ok(Libc::None),
        Ok(Some(turborepo_platform::Libc::Glibc { version })) => Ok(Libc::Gnu { abi: version }),
        Ok(Some(turborepo_platform::Libc::Musl { version })) => Ok(Libc::Musl { abi: version }),
        Err(error) => Err(format!(
            "cannot classify this Linux host's C library ({error}); managed run requires a glibc \
             host, or a musl host whose tools are all verify-system"
        )),
    }
}

#[cfg(test)]
mod discovery_tests {
    use turborepo_turbo_json::FutureFlags;

    #[test]
    fn only_cargo_python_or_go_discovery_needs_managed_tools() {
        let javascript = FutureFlags::default();
        assert!(!super::discovery_probes_tools(&javascript));
        for enable in [
            |f: &mut FutureFlags| f.experimental_cargo_workspaces = true,
            |f: &mut FutureFlags| f.experimental_python_workspaces = true,
            |f: &mut FutureFlags| f.experimental_go_workspaces = true,
        ] {
            let mut flags = javascript;
            enable(&mut flags);
            assert!(super::discovery_probes_tools(&flags), "{flags:?}");
        }
    }
}

#[cfg(test)]
mod libc_tests {
    use turborepo_platform::Libc as Host;

    use super::{Libc, host_libc};

    #[test]
    fn classified_hosts_select_their_libc_identity_and_unclassified_hosts_fail_closed() {
        let gnu = host_libc(Ok(Some(Host::Glibc {
            version: "2.39".into(),
        })));
        assert_eq!(gnu, Ok(Libc::Gnu { abi: "2.39".into() }));
        assert_eq!(host_libc(Ok(None)), Ok(Libc::None));
        // musl hosts get their own family; activation admits verify-system only.
        let musl = host_libc(Ok(Some(Host::Musl {
            version: "1.2.5".into(),
        })));
        assert_eq!(
            musl,
            Ok(Libc::Musl {
                abi: "1.2.5".into()
            })
        );
        // The real host classification agrees with the execution context.
        if cfg!(target_os = "linux") {
            let context = super::context();
            match turborepo_platform::host_libc() {
                Ok(Some(_)) => assert!(context.is_ok(), "{context:?}"),
                _ => assert!(context.is_err()),
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod requirement_tests {
    use turborepo_platform::{Architecture, OperatingSystem, Platform};
    use turborepo_setup::{
        execution_identity::{ADAPTERS, ExecutionContext, ExecutionSnapshot, Libc},
        lock::{Lock, SCHEMA_VERSION, additional},
    };
    use turborepo_shim::capabilities::{Capabilities, EXPERIMENTAL_LOCK_SCHEMA, FEATURES};

    fn requirement(clone_local: bool) -> Vec<String> {
        let tool = |adapter: &str, version: &str| {
            serde_json::json!({"adapter": adapter, "version": version,
                "declarations": [{"file": "package.json", "field": adapter}],
                "installation": {"kind": "verify-system", "executables": [adapter]}})
        };
        let lock = serde_json::json!({"schemaVersion": 0, "tools": {
            "node": tool("node", "24.0.0"), "pnpm": tool("pnpm", "10.0.0")}});
        let lock = Lock::parse(&serde_json::to_vec(&lock).unwrap()).unwrap();
        let context = ExecutionContext::new(
            Platform::new(OperatingSystem::Linux, Architecture::X64),
            Libc::Musl {
                abi: "1.2.5".into(),
            },
            "x86_64-linux".into(),
            ["baseline".into()].into(),
        )
        .unwrap();
        let ids = ["node", "pnpm"].map(String::from).into();
        let snapshot = if clone_local {
            ExecutionSnapshot::select_clone_local(&lock, context, &ids).unwrap()
        } else {
            ExecutionSnapshot::select(&lock, context, &ids).unwrap()
        };
        let requirement = super::peer_requirement(&snapshot);
        assert_eq!(requirement.schema(), SCHEMA_VERSION);
        // This build advertises everything it selects: a same-build
        // repository-local turbo is always compatible.
        if super::context().is_ok() {
            assert_eq!(Capabilities::current("v").shortfall(&requirement), None);
        }
        requirement.features().map(String::from).collect()
    }

    #[test]
    fn requirements_name_adapters_verify_system_and_clone_local_selections() {
        assert_eq!(requirement(false), ["node", "pnpm", "verify-system"]);
        let clone_local = requirement(true);
        assert_eq!(
            clone_local,
            ["clone-local", "node", "pnpm", "verify-system"]
        );
    }

    #[test]
    fn every_selectable_adapter_and_mode_is_advertised_for_the_lock_schema() {
        assert_eq!(EXPERIMENTAL_LOCK_SCHEMA, SCHEMA_VERSION);
        assert!(ADAPTERS.contains(&additional::ADAPTER));
        for name in ADAPTERS.iter().chain(&["verify-system", "clone-local"]) {
            assert!(FEATURES.contains(name), "{name} is not advertised");
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use turborepo_setup::lock::Platform;

    #[test]
    fn only_windows_x64_qualifies_for_managed_execution() {
        let context = super::context();
        if cfg!(target_arch = "x86_64") {
            let platform = context.map(|c| c.artifact_platform());
            assert!(matches!(platform, Ok(Platform::WindowsX64)), "{platform:?}");
        } else {
            assert!(context.is_err());
        }
    }
}
