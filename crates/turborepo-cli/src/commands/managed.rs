//! Exact-root strict activation shared by `turbo run` and `turbo exec`.
//! No provisioning, HTTP, ancestor inventory search or process-global mutation.

use std::{collections::BTreeSet, ffi::OsString, path::Path, sync::Arc};

use turborepo_run::{EnvironmentVariableMap, ManagedEnvironment, ManagedRun};
use turborepo_setup::execution_identity::{ExecutionContext, Libc};
use turborepo_tool_install::Store;

use crate::commands::setup::authorization::{self, Authorization};

pub(crate) fn select(
    base: &crate::commands::CommandBase,
) -> Result<Option<Arc<ManagedRun>>, turborepo_run::Error> {
    if !base.opts.future_flags.experimental_setup {
        return Ok(None);
    }
    // These initial managed adapters must not probe unprovisioned native tools.
    let flags = &base.opts.future_flags;
    let managed = || {
        std::fs::symlink_metadata(base.repo_root.join_component("turbo.lock")).is_ok()
            || Store::selects_record(base.repo_root.as_std_path()).unwrap_or(true)
    };
    for (enabled, marker) in [
        (flags.experimental_cargo_workspaces, "Cargo.toml"),
        (flags.experimental_python_workspaces, "pyproject.toml"),
        (flags.experimental_go_workspaces, "go.work"),
        (flags.experimental_go_workspaces, "go.mod"),
    ] {
        if enabled && base.repo_root.join_component(marker).exists() && managed() {
            return Err(turborepo_run::Error::ManagedTools(
                "initial managed run supports Node/pnpm JS repositories only; use a supported \
                 setup adapter"
                    .into(),
            ));
        }
    }
    let user_base = dirs_next::data_local_dir().ok_or_else(|| {
        turborepo_run::Error::ManagedTools("cannot locate setup user state directory".into())
    })?;
    ManagedTools::inspect(
        base.repo_root.as_std_path(),
        base.opts.repo_opts.root_turbo_json_path.as_std_path(),
        true,
        &user_base,
        context,
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
    /// A root turbo.lock, or without one the sealed selection of this clone's
    /// `turbo setup --no-lock` generation, selects managed mode. Either way
    /// activation is equally strict and never downloads.
    pub(crate) fn inspect(
        root: &Path,
        config: &Path,
        enabled: bool,
        user_base: &Path,
        context: impl FnOnce() -> Result<ExecutionContext, String>,
    ) -> Result<Option<Arc<ManagedRun>>, String> {
        // A lock alone must never change unconfigured-repository behavior.
        if !enabled {
            return Ok(None);
        }
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
        let ci = turborepo_ci::is_ci();
        let explain = |e: authorization::Error| guidance(&e, ci, clone_local);
        let authorization = if clone_local {
            Authorization::inspect_clone_local(&canonical, user_base, context()?)
        } else {
            Authorization::inspect(&canonical, user_base, context()?)
        }
        .map_err(explain)?;
        let snapshot = authorization
            .with_current(|p| p.snapshot().clone())
            .map_err(explain)?;
        let identity = snapshot.fingerprint().to_owned();
        let tools = Arc::new(Self {
            authorization,
            inherited_path: std::env::var_os("PATH"),
            identity,
            ci,
            clone_local,
        });
        tools.revalidate()?;
        Ok(Some(Arc::new(ManagedRun::new(snapshot, tools)?)))
    }
}

fn guidance(error: &authorization::Error, ci: bool, clone_local: bool) -> String {
    if clone_local {
        error.no_lock_guidance()
    } else {
        error.guidance(ci)
    }
}

impl ManagedEnvironment for ManagedTools {
    fn revalidate(&self) -> Result<EnvironmentVariableMap, String> {
        self.authorization
            .with_current(|plan| {
                let mut overrides = EnvironmentVariableMap::default();
                let path = plan
                    .path(self.inherited_path.as_deref())
                    .map_err(|e| e.to_string())?;
                overrides.insert(
                    "PATH".into(),
                    path.into_string().map_err(|_| {
                        "managed PATH is not UTF-8; run from a UTF-8 repository path".to_owned()
                    })?,
                );
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
/// These initial adapters are supported only on native macOS, GNU Linux and
/// Windows x64. Windows arm64 is unqualified and never borrows x64 artifacts.
pub(crate) fn context() -> Result<ExecutionContext, String> {
    if !cfg!(any(
        target_os = "macos",
        all(target_os = "linux", target_env = "gnu"),
        all(windows, target_arch = "x86_64")
    )) {
        return Err("managed Node/pnpm run is unsupported on this host".into());
    }
    let libc = {
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        {
            let abi = unsafe { std::ffi::CStr::from_ptr(libc::gnu_get_libc_version()) }
                .to_str()
                .map_err(|e| e.to_string())?
                .to_owned();
            Libc::Gnu { abi }
        }
        #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
        {
            Libc::None
        }
    };
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
