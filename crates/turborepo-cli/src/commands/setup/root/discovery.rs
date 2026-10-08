//! Invocation-to-root state, not a replacement for the native/config byte
//! snapshot.

use std::fmt;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_turbo_json::FutureFlags;

use super::{Error, SetupRoot, infer};
use crate::cli::Args;

/// Policy eligibility only, not discovered declarations or shipping support.
/// Node declarations are considered when setup is enabled. Non-JS markers and
/// declarations are ignored unless their matching workspace flag is enabled.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::commands::setup) struct SourceEligibility {
    node: bool,
    cargo: bool,
    python: bool,
    go: bool,
}

// Pre-factor accessors for the future locked executor; no adapters enabled
// here.
impl SourceEligibility {
    fn from_flags(flags: FutureFlags) -> Self {
        Self {
            node: flags.experimental_setup,
            cargo: flags.experimental_setup && flags.experimental_cargo_workspaces,
            python: flags.experimental_setup && flags.experimental_python_workspaces,
            go: flags.experimental_setup && flags.experimental_go_workspaces,
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "pending guarded provisioner6320")
    )]
    pub fn node(self) -> bool {
        self.node
    }
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "pending guarded provisioner6320")
    )]
    pub fn cargo(self) -> bool {
        self.cargo
    }
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "pending guarded provisioner6320")
    )]
    pub fn python(self) -> bool {
        self.python
    }
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "pending guarded provisioner6320")
    )]
    pub fn go(self) -> bool {
        self.go
    }
}

#[derive(PartialEq, Eq)]
enum ConfigScope {
    RootFiles(Option<AbsoluteSystemPathBuf>),
    UnsupportedCustom,
}

fn config_scope(
    root: &SetupRoot,
    explicit_config: Option<&AbsoluteSystemPath>,
) -> Result<ConfigScope, Error> {
    // Inference canonicalizes explicit configs. Keep the original selector in
    // scope validation too: a custom symlink name is not a Snapshot input.
    if let Some(config) = explicit_config
        && (!turborepo_types::CONFIG_FILES
            .iter()
            .any(|name| config.file_name() == Some(*name))
            || config
                .parent()
                .map(AbsoluteSystemPath::to_realpath)
                .transpose()?
                .as_deref()
                != Some(&*root.path))
    {
        return Ok(ConfigScope::UnsupportedCustom);
    }
    let Some(config) = &root.config else {
        return Ok(ConfigScope::RootFiles(None));
    };
    let real = config.to_realpath()?;
    if config.parent() == Some(&*root.path)
        && real.parent() == Some(&*root.path)
        && [config, &real].iter().all(|path| {
            turborepo_types::CONFIG_FILES
                .iter()
                .any(|name| path.file_name() == Some(*name))
        })
    {
        Ok(ConfigScope::RootFiles(Some(real)))
    } else {
        Ok(ConfigScope::UnsupportedCustom)
    }
}

/// Immutable validated read-only discovery. Paths are resolved against the
/// original process cwd once; inference canonicalizes them on every check, so
/// replacing a cwd/config symlink cannot redirect a captured invocation
/// silently. Fields stay private; callers cannot substitute an outer root or
/// new flags.
///
/// This captures discovery identity and policy, NOT config/declaration bytes.
/// The locked executor must capture its Snapshot at `snapshot_root()`, reject a
/// false/error `revalidate()` before initializing storage, and check again
/// inside Store's checked pre-publication callback while holding the Snapshot
/// writer guard AND Store lock through promotion. Byte drift remains Snapshot's
/// job. Arbitrary editors are not serialized; this is not an atomic editor
/// transaction.
pub(in crate::commands::setup) struct Discovery {
    cwd: AbsoluteSystemPathBuf,
    explicit_cwd: bool,
    config: Option<AbsoluteSystemPathBuf>,
    root: SetupRoot,
    scope: ConfigScope,
    eligibility: SourceEligibility,
}

// Do not expose config contents, invocation paths, or policy in debug output.
impl fmt::Debug for Discovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Discovery").finish_non_exhaustive()
    }
}

impl Discovery {
    pub fn capture(args: &Args) -> Result<Self, Error> {
        Self::capture_at(args, &AbsoluteSystemPathBuf::cwd()?)
    }

    fn capture_at(args: &Args, invocation: &AbsoluteSystemPath) -> Result<Self, Error> {
        let cwd = args.cwd.as_deref().map_or_else(
            || invocation.to_owned(),
            |cwd| AbsoluteSystemPathBuf::from_unknown(invocation, cwd),
        );
        // Global --root-turbo-json remains relative to the invocation, NOT --cwd.
        let config = args
            .root_turbo_json
            .as_deref()
            .map(|path| AbsoluteSystemPathBuf::from_unknown(invocation, path));
        let explicit_cwd = args.cwd.is_some();
        let root = infer(&cwd, explicit_cwd, config.as_deref())?;
        let scope = config_scope(&root, config.as_deref())?;
        let eligibility = SourceEligibility::from_flags(root.flags);
        Ok(Self {
            cwd,
            explicit_cwd,
            config,
            root,
            scope,
            eligibility,
        })
    }

    pub fn root_path(&self) -> &AbsoluteSystemPath {
        &self.root.path
    }
    pub fn flags(&self) -> FutureFlags {
        self.root.flags
    }

    // These are preparatory APIs, not a shipped installation path. Current run
    // only gates/normalizes requests and still returns NotImplemented.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "pending guarded provisioner6320")
    )]
    pub fn source_eligibility(&self) -> SourceEligibility {
        self.eligibility
    }

    /// Reject unsupported custom configuration BEFORE Snapshot/storage/network.
    /// Current parse/gating-only run still accepts it, as before this
    /// pre-factor.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "pending guarded provisioner6320")
    )]
    pub fn snapshot_root(&self) -> Result<&AbsoluteSystemPath, Error> {
        match self.scope {
            ConfigScope::RootFiles(_) => Ok(self.root_path()),
            ConfigScope::UnsupportedCustom => Err(Error::UnsupportedConfig),
        }
    }

    /// Same original invocation, same inference algorithm. A changed identity
    /// or flag/policy returns false; malformed/ambiguous/nested roots
    /// preserve the original root diagnostic rather than flattening it to a
    /// generic conflict. Never recaptures or updates this expected state on
    /// drift.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "pending guarded provisioner6320")
    )]
    pub fn revalidate(&self) -> Result<bool, Error> {
        self.snapshot_root()?;
        let current = infer(&self.cwd, self.explicit_cwd, self.config.as_deref())?;
        Ok(
            config_scope(&current, self.config.as_deref())? == self.scope
                && SourceEligibility::from_flags(current.flags) == self.eligibility
                && current == self.root,
        )
    }
}

#[cfg(test)]
mod tests;
