//! Exact-root strict activation shared by `turbo run` and `turbo exec`.
//! No provisioning, HTTP, ancestor inventory search or process-global mutation.

use std::{collections::BTreeSet, ffi::OsString, path::Path, sync::Arc};

use turborepo_run::{EnvironmentVariableMap, ManagedEnvironment, ManagedRun};
use turborepo_setup::execution_identity::{ExecutionContext, Libc};

use crate::commands::setup::authorization::Authorization;

pub(crate) fn select(
    base: &crate::commands::CommandBase,
) -> Result<Option<Arc<ManagedRun>>, turborepo_run::Error> {
    if !base.opts.future_flags.experimental_setup {
        return Ok(None);
    }
    // These initial managed adapters must not probe unprovisioned native tools.
    let flags = &base.opts.future_flags;
    for (enabled, marker) in [
        (flags.experimental_cargo_workspaces, "Cargo.toml"),
        (flags.experimental_python_workspaces, "pyproject.toml"),
        (flags.experimental_go_workspaces, "go.work"),
        (flags.experimental_go_workspaces, "go.mod"),
    ] {
        if enabled
            && base.repo_root.join_component(marker).exists()
            && std::fs::symlink_metadata(base.repo_root.join_component("turbo.lock")).is_ok()
        {
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
}

impl ManagedTools {
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
        match std::fs::symlink_metadata(root.join("turbo.lock")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("cannot read turbo.lock; run turbo setup: {error}")),
            Ok(_) => {}
        }
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
        let authorization = Authorization::inspect(&canonical, user_base, context()?)
            .map_err(|e| e.guidance(ci))?;
        let snapshot = authorization
            .with_current(|p| p.snapshot().clone())
            .map_err(|e| e.guidance(ci))?;
        let identity = snapshot.fingerprint().to_owned();
        let tools = Arc::new(Self {
            authorization,
            inherited_path: std::env::var_os("PATH"),
            identity,
            ci,
        });
        tools.revalidate()?;
        Ok(Some(Arc::new(ManagedRun::new(snapshot, tools)?)))
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
            .map_err(|e| e.guidance(self.ci))?
    }

    fn cache_identity(&self) -> &str {
        &self.identity
    }
}

fn qualified_libc(host: Option<turborepo_platform::Libc>) -> Result<Libc, String> {
    match host {
        Some(turborepo_platform::Libc::Glibc { version }) => Ok(Libc::Gnu { abi: version }),
        None if cfg!(any(target_os = "macos", windows)) => Ok(Libc::None),
        _ => Err("managed Node/pnpm run is unsupported on this host".into()),
    }
}

/// Conservative native execution identity. No readiness-only/unprobed labels.
/// These initial adapters are supported only on native macOS, GNU Linux and
/// Windows x64. Windows arm64 is unqualified and never borrows x64 artifacts.
pub(crate) fn context() -> Result<ExecutionContext, String> {
    if !cfg!(any(
        target_os = "macos",
        target_os = "linux",
        all(windows, target_arch = "x86_64")
    )) {
        return Err("managed Node/pnpm run is unsupported on this host".into());
    }
    // Released Linux binaries are musl-static; their target is not the host's
    // execution ABI. Use the same bounded, non-executing detection as setup.
    let libc = qualified_libc(turborepo_platform::host_libc().map_err(|e| e.to_string())?)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_abi_uses_host_glibc_version_and_rejects_musl() {
        assert_eq!(
            qualified_libc(Some(turborepo_platform::Libc::Glibc {
                version: "2.39".into(),
            }))
            .unwrap(),
            Libc::Gnu { abi: "2.39".into() }
        );
        assert!(qualified_libc(Some(turborepo_platform::Libc::Musl)).is_err());
        assert_eq!(
            qualified_libc(None).is_ok(),
            cfg!(any(target_os = "macos", windows))
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
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
