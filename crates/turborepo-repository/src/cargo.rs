//! The Cargo toolchain: Rust crates as Turborepo packages.
//!
//! Turborepo does not replace Cargo — Cargo is itself a build system with
//! its own dependency graph, scheduler, and incremental cache. Turborepo's
//! job is orchestration: decide *which* crates are in scope and *whether*
//! anything changed, then hand the work to Cargo and get out of the way.
//!
//! Discovery shells out to `cargo metadata`, because Cargo is the only
//! correct implementation of its own workspace-membership semantics (member
//! globs, automatic path-dependency members, excludes, target-specific
//! dependency tables, renames). Crates are classified into two shapes:
//!
//! * **Entrypoints** — crates with `bin`/`cdylib`/`staticlib` targets: the
//!   deliverables of the workspace.
//! * **Libraries** — everything else. They exist in the package graph (so
//!   `--filter` and `--affected` propagate through them): being buildable is
//!   not the same as being an entrypoint.
//!
//! Verification verbs execute per crate when a crate is filtered (`lib-a#test`
//! → `cargo test --package=lib-a`). A synthetic package anchored at the root
//! `Cargo.toml` and depending on every crate represents the workspace itself;
//! it runs the same verbs at workspace scope for unfiltered runs (`<name>#test`
//! → `cargo test --workspace`; see [`workspace_subcommand`]). Its name is
//! declared by the user in the root manifest — using Turborepo with Rust
//! requires naming the workspace:
//!
//! ```toml
//! [workspace.metadata]
//! name = "acme"
//! ```
//!
//! Support is experimental and gated behind
//! `futureFlags.experimentalCargoWorkspaces`.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
};

use serde::Deserialize;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::{
    change_knowledge::ChangeObservation,
    external_resolution::{
        ExternalPackageIdentity, ExternalResolutionData, ExternalResolutionDomain,
        PackageResolution, ResolutionCompleteness,
    },
    package_json::{DependencyKind, PackageJson},
    prune_knowledge::{PruneDomain, PrunePlan},
    relationships::Relationship,
    toolchain::{
        self, DiscoverPackagesFuture, DiscoveredPackage, DiscoveredPackages, RepositoryContributor,
        ToolchainId, WorkspaceRoot,
    },
};

/// The conventional file name for a Cargo manifest.
pub const CARGO_TOML: &str = "Cargo.toml";

/// The conventional file name for a Cargo lockfile.
pub const CARGO_LOCK: &str = "Cargo.lock";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to run `cargo metadata`: {0}")]
    MetadataSpawn(#[source] io::Error),
    #[error("`cargo metadata` failed: {stderr}")]
    Metadata { stderr: String },
    #[error("failed to parse `cargo metadata` output: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("failed to read Cargo.lock: {0}")]
    LockfileRead(#[source] io::Error),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::CargoLockError),
    #[error("failed to parse root Cargo.toml: {0}")]
    ManifestParse(#[from] Box<toml_edit::TomlError>),
    #[error("root Cargo.toml has no [workspace] table")]
    NotAWorkspace,
    #[error(
        "The Cargo workspace has no name.\n\nTurborepo needs a name for the workspace's tasks \
         (`<name>#test`), filters (`--filter=<name>`), and configuration. Add one to the root \
         Cargo.toml:\n\n    [workspace.metadata]\n    name = \"my-workspace\""
    )]
    MissingWorkspaceName,
    #[error(
        "invalid Cargo workspace name {name:?}: {reason}. Set a valid name in the root Cargo.toml \
         under `[workspace.metadata] name`."
    )]
    InvalidWorkspaceName { name: String, reason: String },
    #[error(
        "the Cargo workspace name {name:?} collides with the crate of the same name at {dir}. \
         Pick a different `[workspace.metadata] name`."
    )]
    WorkspaceNameCollision { name: String, dir: String },
    #[error(
        "Cargo.lock is required for Cargo workspace caching. Run `cargo generate-lockfile` and \
         commit the result."
    )]
    MissingLockfile,
    #[error(
        "Cargo.lock is out of date or could not be validated. Run `cargo metadata` to refresh it, \
         then commit the result.\n\nCargo reported:\n{stderr}"
    )]
    InvalidLockfile { stderr: String },
    #[error("failed to validate Cargo.lock with `cargo metadata --locked`: {0}")]
    LockfileValidationSpawn(#[source] io::Error),
    #[error(
        "Cargo local package {name:?} at {manifest_path} is outside the repository and cannot be \
         cached, watched, or pruned safely. Move it into the repository and make it a workspace \
         member."
    )]
    OutsideRepositoryLocalPackage { name: String, manifest_path: String },
    #[error(
        "Cargo local package {name:?} at {manifest_path} is not a workspace member and cannot be \
         hashed or pruned safely. Add it to `[workspace].members` and remove it from \
         `[workspace].exclude`."
    )]
    NonMemberLocalPackage { name: String, manifest_path: String },
    #[error("failed to resolve Cargo local package path {path}: {source}")]
    LocalPackagePath {
        path: String,
        #[source]
        source: turbopath::PathError,
    },
    #[error("failed to read workspace file: {0}")]
    WorkspaceFileRead(#[source] io::Error),
    #[error("failed to run `rustc -vV`: {0}")]
    RustcSpawn(#[source] io::Error),
    #[error("`rustc -vV` failed: {stderr}")]
    Rustc { stderr: String },
    #[error("`rustc -vV` output is not UTF-8: {0}")]
    RustcOutputUtf8(#[from] std::str::Utf8Error),
    #[error("invalid `rustc -vV` output: {reason}")]
    InvalidRustcOutput { reason: &'static str },
    #[error(transparent)]
    ResolutionPath(#[from] turbopath::PathError),
}

fn package_resolution(
    package: impl Into<String>,
    identities: &HashSet<turborepo_lockfiles::Package>,
) -> PackageResolution {
    PackageResolution::new(
        package,
        identities.iter().map(|identity| {
            ExternalPackageIdentity::new(identity.key.clone(), identity.version.clone())
        }),
    )
}

fn parse_rustc_info(stdout: &[u8]) -> Result<(turborepo_lockfiles::Package, String), Error> {
    let stdout = std::str::from_utf8(stdout)?;
    let lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines
        .first()
        .filter(|line| {
            line.strip_prefix("rustc ")
                .is_some_and(|version| !version.trim().is_empty())
        })
        .ok_or(Error::InvalidRustcOutput {
            reason: "missing compiler version",
        })?;
    let mut hosts = lines
        .iter()
        .filter_map(|line| line.strip_prefix("host:").map(str::trim));
    let host = hosts
        .next()
        .filter(|host| !host.is_empty())
        .ok_or(Error::InvalidRustcOutput {
            reason: "missing host triple",
        })?;
    if hosts.next().is_some() {
        return Err(Error::InvalidRustcOutput {
            reason: "multiple host triples",
        });
    }

    Ok((
        turborepo_lockfiles::Package {
            key: "rustc".to_string(),
            version: lines.join("\n"),
        },
        host.to_string(),
    ))
}

#[cfg(test)]
fn parse_rustc_identity(stdout: &[u8]) -> Result<turborepo_lockfiles::Package, Error> {
    parse_rustc_info(stdout).map(|(identity, _)| identity)
}

fn rustc_info(
    repo_root: &AbsoluteSystemPath,
) -> Result<(turborepo_lockfiles::Package, String), Error> {
    let output = std::process::Command::new("rustc")
        .arg("-vV")
        .current_dir(repo_root.as_std_path())
        .output()
        .map_err(Error::RustcSpawn)?;
    if !output.status.success() {
        return Err(Error::Rustc {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    parse_rustc_info(&output.stdout)
}

fn rustc_supported_targets(repo_root: &AbsoluteSystemPath) -> HashSet<String> {
    std::process::Command::new("rustc")
        .args(["--print", "target-list"])
        .current_dir(repo_root.as_std_path())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|stdout| stdout.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Per-crate external dependency closures from Cargo.lock, for the crates'
/// external-dependency hashes.
///
/// A missing, unreadable, or unparsable lockfile is a hard error — silently
/// hashing nothing would be unsound.
pub fn external_closures(
    repo_root: &AbsoluteSystemPath,
    members: &[String],
) -> Result<HashMap<String, HashSet<turborepo_lockfiles::Package>>, Error> {
    let contents = read_lockfile(repo_root)?;
    external_closures_from_lockfile(&contents, members)
}

fn read_lockfile(repo_root: &AbsoluteSystemPath) -> Result<String, Error> {
    match repo_root.join_component(CARGO_LOCK).read_to_string() {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Error::MissingLockfile),
        Err(error) => Err(Error::LockfileRead(error)),
    }
}

fn external_closures_from_lockfile(
    contents: &str,
    members: &[String],
) -> Result<HashMap<String, HashSet<turborepo_lockfiles::Package>>, Error> {
    Ok(turborepo_lockfiles::cargo_external_closures(
        contents, members,
    )?)
}

/// Verify Cargo can resolve the workspace without changing Cargo.lock and that
/// every resolved local package is an in-repository workspace member.
/// Validation happens before task hashes and cache lookup, so artifacts are
/// always keyed by sources Turborepo can hash, watch, and prune.
pub fn validate_lockfile(repo_root: &AbsoluteSystemPath) -> Result<(), Error> {
    require_lockfile(repo_root)?;
    let metadata = locked_metadata(repo_root)?;
    validate_resolved_local_packages(repo_root, &metadata)
}

fn require_lockfile(repo_root: &AbsoluteSystemPath) -> Result<(), Error> {
    read_lockfile(repo_root).map(drop)
}

fn locked_metadata(repo_root: &AbsoluteSystemPath) -> Result<Metadata, Error> {
    let root_manifest_path = repo_root.join_component(CARGO_TOML);
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--all-features",
            "--manifest-path",
            root_manifest_path.as_str(),
        ])
        .current_dir(repo_root.as_std_path())
        .output()
        .map_err(Error::LockfileValidationSpawn)?;
    if !output.status.success() {
        return Err(Error::InvalidLockfile {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

fn validate_resolved_local_packages(
    repo_root: &AbsoluteSystemPath,
    metadata: &Metadata,
) -> Result<(), Error> {
    let real_repo_root = repo_root
        .to_realpath()
        .map_err(|source| Error::LocalPackagePath {
            path: repo_root.to_string(),
            source,
        })?;
    for package in &metadata.packages {
        if package.source.is_some() {
            continue;
        }
        let Some(manifest_path) = metadata_path(&package.manifest_path) else {
            return Err(Error::OutsideRepositoryLocalPackage {
                name: package.name.clone(),
                manifest_path: package.manifest_path.clone(),
            });
        };
        let real_manifest_path =
            manifest_path
                .to_realpath()
                .map_err(|source| Error::LocalPackagePath {
                    path: package.manifest_path.clone(),
                    source,
                })?;
        if !real_repo_root.contains(&real_manifest_path) {
            return Err(Error::OutsideRepositoryLocalPackage {
                name: package.name.clone(),
                manifest_path: package.manifest_path.clone(),
            });
        }
        if !metadata.workspace_members.contains(&package.id) {
            return Err(Error::NonMemberLocalPackage {
                name: package.name.clone(),
                manifest_path: package.manifest_path.clone(),
            });
        }
    }

    Ok(())
}

/// How a Cargo-toolchain package participates in task execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CargoPackageKind {
    /// An internal library crate. Filtered build and verification tasks execute
    /// `cargo <verb> --package=<crate>`; unfiltered builds prefer entrypoints
    /// because Cargo builds their library dependency closures implicitly.
    Library,
    /// A crate with `bin`/`cdylib`/`staticlib` targets: a deliverable.
    /// Build, run, and verification tasks execute
    /// `cargo <verb> --package=<crate>`.
    Entrypoint,
    /// The user-named workspace aggregate hosting workspace-scoped
    /// verification tasks (`cargo test --workspace`, ...).
    Workspace,
}

/// Cargo-specific details captured in immutable task-contract knowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoPackageDetails {
    pub kind: CargoPackageKind,
    /// The crate's deliverable targets (empty for libraries and the
    /// workspace aggregate).
    pub deliverables: Vec<Deliverable>,
    pub manifest_alters_output_layout: bool,
}

const VERIFICATION_SUBCOMMANDS: &[(&str, &str)] = &[
    ("test", "test"),
    ("check", "check"),
    ("lint", "clippy"),
    ("format", "fmt"),
];

const ENTRYPOINT_SUBCOMMANDS: &[(&str, &str)] =
    &[("build", "build"), ("run", "run"), ("dev", "run")];

const LIBRARY_SUBCOMMANDS: &[(&str, &str)] = &[("build", "build")];

fn subcommand(task: &str, tasks: &'static [(&'static str, &'static str)]) -> Option<&'static str> {
    tasks
        .iter()
        .find_map(|(name, subcommand)| (*name == task).then_some(*subcommand))
}

/// Map a Turborepo task name to the Cargo subcommand that implements it for
/// an entrypoint crate.
pub fn entrypoint_subcommand(task: &str) -> Option<&'static str> {
    subcommand(task, ENTRYPOINT_SUBCOMMANDS).or_else(|| library_subcommand(task))
}

/// Map a Turborepo task name to the Cargo subcommand that implements it for a
/// library crate.
pub fn library_subcommand(task: &str) -> Option<&'static str> {
    subcommand(task, LIBRARY_SUBCOMMANDS).or_else(|| subcommand(task, VERIFICATION_SUBCOMMANDS))
}

/// Map a verification task to the Cargo subcommand that implements it at
/// workspace scope (the synthetic user-named package).
pub fn workspace_subcommand(task: &str) -> Option<&'static str> {
    subcommand(task, VERIFICATION_SUBCOMMANDS)
}

fn registered_tasks(details: &CargoPackageDetails) -> Vec<&'static str> {
    let runnable = details
        .deliverables
        .iter()
        .filter(|deliverable| deliverable.kind == DeliverableKind::Bin)
        .count()
        == 1;
    let mut tasks: Vec<_> = match details.kind {
        CargoPackageKind::Entrypoint => ENTRYPOINT_SUBCOMMANDS,
        CargoPackageKind::Library => LIBRARY_SUBCOMMANDS,
        CargoPackageKind::Workspace => VERIFICATION_SUBCOMMANDS,
    }
    .iter()
    .map(|(task, _)| *task)
    .collect();
    if details.kind != CargoPackageKind::Workspace {
        tasks.extend(VERIFICATION_SUBCOMMANDS.iter().map(|(task, _)| *task));
    }
    tasks.retain(|task| runnable || !matches!(*task, "run" | "dev"));
    tasks
}

/// The Cargo subcommand a task resolves to for a package, given its
/// [`CargoPackageKind`]. `None` means the task is a no-op for this package
/// (like a missing package.json script).
pub fn task_subcommand(kind: CargoPackageKind, task: &str) -> Option<&'static str> {
    match kind {
        CargoPackageKind::Entrypoint => entrypoint_subcommand(task),
        CargoPackageKind::Workspace => workspace_subcommand(task),
        CargoPackageKind::Library => library_subcommand(task),
    }
}

/// The display string for a Cargo task's command, derived from the same
/// tables as execution so it cannot drift.
pub fn display_command(kind: CargoPackageKind, task: &str, package: &str) -> Option<String> {
    let subcommand = task_subcommand(kind, task)?;
    Some(match (kind, subcommand) {
        (CargoPackageKind::Entrypoint | CargoPackageKind::Library, "fmt") => {
            format!("cargo fmt --package={package}")
        }
        (CargoPackageKind::Workspace, "fmt") => "cargo fmt --all".to_string(),
        (CargoPackageKind::Entrypoint | CargoPackageKind::Library, _) => {
            format!("cargo {subcommand} --package={package} --locked")
        }
        (CargoPackageKind::Workspace, _) => format!("cargo {subcommand} --workspace --locked"),
    })
}

/// Build native-task facts for a Cargo package from its verb tables.
pub fn native_tasks_for_package(
    details: &CargoPackageDetails,
    package: &str,
) -> Vec<crate::native_tasks::NativeTask> {
    use crate::native_tasks::{
        NativeCommandArguments, NativeCommandProgram, NativeTask, NativeTaskContract,
        PassThroughPlacement, PassThroughSeparator, WorkingDirectoryPolicy,
    };

    let mut tasks: Vec<_> = registered_tasks(details)
        .into_iter()
        .filter_map(|task| {
            let subcommand = task_subcommand(details.kind, task)?;
            let display = display_command(details.kind, task, package)?;
            let scope_arg = match (details.kind, subcommand) {
                (CargoPackageKind::Workspace, "fmt") => "--all".to_string(),
                (CargoPackageKind::Workspace, _) => "--workspace".to_string(),
                (CargoPackageKind::Entrypoint | CargoPackageKind::Library, _) => {
                    format!("--package={package}")
                }
            };
            Some(
                NativeTask::command_task(
                    task,
                    display,
                    NativeCommandProgram::Tool("cargo".to_string()),
                    NativeCommandArguments {
                        prefix: vec![subcommand.to_string(), scope_arg],
                        pass_through_placement: PassThroughPlacement::AfterSuffix,
                        pass_through_separator: pass_through_uses_separator(subcommand)
                            .then(|| PassThroughSeparator::Fixed("--".to_string())),
                        suffix: (subcommand != "fmt")
                            .then(|| "--locked".to_string())
                            .into_iter()
                            .collect(),
                    },
                    (!matches!(subcommand, "run" | "fmt")).then(|| "cargo".to_string()),
                    WorkingDirectoryPolicy::RepositoryRoot,
                )
                .with_contract(NativeTaskContract::new(
                    cargo_task_defaults(details, task),
                    cargo_task_entrypoint(details.kind, task),
                    true,
                )),
            )
        })
        .collect();
    if !tasks.iter().any(|task| task.name() == "build") {
        tasks.push(NativeTask::contract_task(
            "build",
            NativeTaskContract::new(
                cargo_task_defaults(details, "build"),
                cargo_task_entrypoint(details.kind, "build"),
                false,
            ),
        ));
    }
    tasks
}

fn cargo_task_defaults(details: &CargoPackageDetails, task: &str) -> toolchain::TaskDefaults {
    let cache = task_subcommand(details.kind, task).and_then(|subcommand| {
        (subcommand == "run"
            || subcommand == "fmt"
            || (details.kind == CargoPackageKind::Library && subcommand == "build"))
            .then_some(false)
    });
    toolchain::TaskDefaults { cache }
}

fn cargo_task_entrypoint(
    kind: CargoPackageKind,
    task: &str,
) -> Option<crate::native_tasks::TaskEntrypoint> {
    use crate::native_tasks::TaskEntrypoint;

    library_subcommand(task)?;
    Some(match (task == "build", kind) {
        (true, CargoPackageKind::Workspace) => TaskEntrypoint::Excluded,
        (true, CargoPackageKind::Entrypoint) => TaskEntrypoint::Preferred,
        (false, CargoPackageKind::Workspace) => TaskEntrypoint::PreferredOnly,
        _ => TaskEntrypoint::Candidate,
    })
}

/// Whether pass-through args for `subcommand` must follow a `--` separator.
/// These subcommands forward everything after `--` to the underlying tool
/// (the built binary for `run`, the test harness, clippy's lint flags, or
/// rustfmt); the remaining subcommands take no trailing args, so pass-through
/// args are attached directly as cargo flags.
pub fn pass_through_uses_separator(subcommand: &str) -> bool {
    matches!(subcommand, "test" | "run" | "clippy" | "fmt")
}

/// Standard Cargo and cc-rs environment variables that can change build
/// outputs or select the tools that produce them. Patterns cover Cargo's
/// profile/target configuration and cc-rs's target-qualified forms without
/// pulling unrelated Cargo credentials or network settings into task hashes.
/// Build-script-specific variables remain explicit task config.
pub const HASHED_ENV_VARS: &[&str] = &[
    // Compiler and rustdoc selection and flags.
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTC_BOOTSTRAP",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTDOC",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    // Environment equivalents of Cargo's [build] configuration.
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET_DIR",
    "CARGO_BUILD_ARTIFACT_DIR",
    "CARGO_BUILD_BUILD_DIR",
    "CARGO_BUILD_TARGET",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTDOC",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_BUILD_RUSTDOCFLAGS",
    "CARGO_INCREMENTAL",
    "CARGO_BUILD_INCREMENTAL",
    // Cargo normalizes profile names and target triples into these families.
    "CARGO_PROFILE_*",
    "CARGO_PROFILE_*_DIR_NAME",
    "CARGO_TARGET_*",
    // Native toolchain variables recognized by cc-rs. `VAR_*` covers both
    // raw and underscore-normalized target suffixes.
    "CC",
    "CC_*",
    "HOST_CC",
    "TARGET_CC",
    "CFLAGS",
    "CFLAGS_*",
    "HOST_CFLAGS",
    "TARGET_CFLAGS",
    "CXX",
    "CXX_*",
    "HOST_CXX",
    "TARGET_CXX",
    "CXXFLAGS",
    "CXXFLAGS_*",
    "HOST_CXXFLAGS",
    "TARGET_CXXFLAGS",
    "CXXSTDLIB",
    "CXXSTDLIB_*",
    "HOST_CXXSTDLIB",
    "TARGET_CXXSTDLIB",
    "AR",
    "AR_*",
    "HOST_AR",
    "TARGET_AR",
    "ARFLAGS",
    "ARFLAGS_*",
    "HOST_ARFLAGS",
    "TARGET_ARFLAGS",
    "RANLIB",
    "RANLIB_*",
    "HOST_RANLIB",
    "TARGET_RANLIB",
    "RANLIBFLAGS",
    "RANLIBFLAGS_*",
    "HOST_RANLIBFLAGS",
    "TARGET_RANLIBFLAGS",
    "NVCC",
    "NVCC_*",
    "HOST_NVCC",
    "TARGET_NVCC",
    "CRATE_CC_NO_DEFAULTS",
    "CROSS_COMPILE",
    // SDK selection is consumed directly by cc-rs on these platforms.
    "SDKROOT",
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "WATCHOS_DEPLOYMENT_TARGET",
    "TVOS_DEPLOYMENT_TARGET",
    "XROS_DEPLOYMENT_TARGET",
    "WASI_SDK_PATH",
    "WASI_SYSROOT",
    "WASM_MUSL_SYSROOT",
];

pub(crate) const TASK_IO_ENV_VARS: &[&str] = &[
    "CARGO_BUILD_ARTIFACT_DIR",
    "CARGO_BUILD_TARGET",
    "CARGO_BUILD_TARGET_DIR",
    "CARGO_HOME",
    "CARGO_PROFILE_*_DIR_NAME",
    "CARGO_TARGET_DIR",
    "RUSTC",
    "CARGO_BUILD_RUSTC",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
];

/// Rewrite the workspace root Cargo.toml for a pruned repository containing
/// only `kept_dirs` (workspace-relative unix paths of the retained crates).
///
/// * `members` becomes the explicit kept list — glob patterns like `crates/*`
///   would otherwise still match removed directories' absence, but explicitness
///   costs nothing and `default-members`/path hygiene need the concrete set
///   anyway.
/// * `default-members` is filtered to kept dirs (dropped when empty), since
///   entries referencing removed crates make Cargo error at load.
/// * `[workspace.dependencies]` entries whose `path` points at a removed crate
///   are dropped: no kept crate can reference them (anything referenced is in
///   the closure and therefore kept), and Cargo validates the paths of
///   workspace dependencies eagerly.
///
/// Everything else — profiles, lints, `[patch]`, non-path workspace
/// dependencies, comments, formatting — is preserved via `toml_edit`.
pub fn prune_root_manifest(contents: &str, kept_dirs: &[String]) -> Result<String, Error> {
    let mut doc: toml_edit::DocumentMut = contents.parse().map_err(Box::new)?;
    let normalized_kept: HashSet<String> = kept_dirs.iter().map(|d| normalize_dir(d)).collect();

    let workspace = doc
        .get_mut("workspace")
        .and_then(|item| item.as_table_like_mut())
        .ok_or(Error::NotAWorkspace)?;

    let mut members = toml_edit::Array::new();
    let mut sorted_dirs = kept_dirs.to_vec();
    sorted_dirs.sort();
    sorted_dirs.dedup();
    for dir in &sorted_dirs {
        members.push(dir.as_str());
    }
    workspace.insert("members", toml_edit::value(members));

    if let Some(default_members) = workspace
        .get_mut("default-members")
        .and_then(|item| item.as_array_mut())
    {
        default_members.retain(|entry| {
            entry
                .as_str()
                .is_some_and(|dir| normalized_kept.contains(&normalize_dir(dir)))
        });
        if default_members.is_empty() {
            workspace.remove("default-members");
        }
    }

    if let Some(dependencies) = workspace
        .get_mut("dependencies")
        .and_then(|item| item.as_table_like_mut())
    {
        let removed: Vec<String> = dependencies
            .iter()
            .filter(|(_, value)| {
                value
                    .get("path")
                    .and_then(|path| path.as_str())
                    .is_some_and(|path| !normalized_kept.contains(&normalize_dir(path)))
            })
            .map(|(name, _)| name.to_string())
            .collect();
        for name in removed {
            dependencies.remove(&name);
        }
    }

    Ok(doc.to_string())
}

/// Normalize a manifest-relative directory path for comparison: unix
/// separators, no leading `./`, no trailing `/`.
fn normalize_dir(dir: &str) -> String {
    dir.replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

fn join_prefix(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

/// Input globs whose changes should invalidate a Cargo task's cache: the
/// workspace root manifest (profiles, lints, `[patch]`, and feature
/// unification all live there), Cargo config files, and pinned toolchain
/// files — expressed relative to the task's package directory via `prefix`
/// (the path from the package to the repo root, e.g. `../..`; empty for the
/// workspace package). Globs that don't match anything (e.g. a missing
/// `rust-toolchain` file) simply contribute nothing.
///
/// Cargo.lock is deliberately absent: locked dependencies participate in
/// each crate task's external-dependency hash, scoped to that crate's
/// transitive closure (see [`external_closures`]), so a dependency bump only
/// invalidates the crates that actually depend on it. The compiler identity
/// participates the same way (see [`rustc_info`]).
pub fn hash_input_globs(prefix: &str) -> Vec<String> {
    [
        "Cargo.toml",
        ".cargo/config.toml",
        ".cargo/config",
        "rust-toolchain.toml",
        "rust-toolchain",
    ]
    .iter()
    .map(|rel| join_prefix(prefix, rel))
    .collect()
}

/// Input globs covering a Cargo crate's sources, with Turborepo's own task
/// log directory excluded. Explicit input globs hash the filesystem (unlike
/// default hashing, which is git-index based), so without the exclusion the
/// `.turbo/turbo-<task>.log` written by each run would invalidate the next
/// run's hash.
fn crate_source_globs(prefix: &str, crate_path: &str) -> [String; 2] {
    let base = join_prefix(prefix, crate_path);
    [format!("{base}/**"), format!("!{base}/.turbo/**")]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CargoWorkspaceDetails {
    target_directory: AbsoluteSystemPathBuf,
    host_target: String,
    supported_targets: HashSet<String>,
    repository_config_alters_output_layout: bool,
    repository_config_untracked: bool,
    external_config_present: bool,
    manifest_alters_profile_dirs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoTaskContract {
    repo_root: AbsoluteSystemPathBuf,
    package: CargoPackageDetails,
    workspace: Option<CargoWorkspaceDetails>,
}

impl CargoTaskContract {
    fn new(
        repo_root: AbsoluteSystemPathBuf,
        package: CargoPackageDetails,
        workspace: Option<CargoWorkspaceDetails>,
    ) -> Self {
        Self {
            repo_root,
            package,
            workspace,
        }
    }

    /// Classifies Cargo package sources for dependent derived-input closures.
    /// Workspace aggregates have no package source directory to include.
    pub(crate) fn dependency_source_inputs(&self) -> crate::task_contracts::DependencySourceInputs {
        if self.package.kind == CargoPackageKind::Workspace {
            crate::task_contracts::DependencySourceInputs::Exclude
        } else {
            crate::task_contracts::DependencySourceInputs::Include
        }
    }

    pub(crate) fn compile_cache_env(
        &self,
        endpoint: &toolchain::CompileCacheEndpoint,
        task_env: &std::collections::HashMap<String, String>,
    ) -> Vec<(String, String)> {
        cargo_compile_cache_env(endpoint, task_env)
    }

    pub(crate) fn derived_task_io(
        &self,
        package: &crate::package_graph::PackageTaskContext<'_>,
        task: &str,
        path_to_root: &str,
        dependencies: &[crate::package_graph::PackageTaskContext<'_>],
        wants_automatic_inputs: bool,
        context: &toolchain::TaskIOContext<'_>,
    ) -> Option<toolchain::DerivedTaskIO> {
        let subcommand = task_subcommand(self.package.kind, task)?;
        let mut io = toolchain::DerivedTaskIO {
            input_globs: hash_input_globs(path_to_root),
            env: HASHED_ENV_VARS.iter().map(|var| var.to_string()).collect(),
            ..Default::default()
        };
        if subcommand == "fmt" {
            io.input_globs.extend(
                ["rustfmt.toml", ".rustfmt.toml"].map(|path| join_prefix(path_to_root, path)),
            );
            io.env.push("RUSTFMT".to_string());
        }
        if let Some(workspace) = &self.workspace
            && (workspace.repository_config_untracked || workspace.external_config_present)
        {
            io.input_safety = toolchain::DerivedInputSafety::Untracked;
            if workspace.repository_config_untracked {
                io.input_globs.retain(|glob| {
                    !glob.ends_with(".cargo/config.toml") && !glob.ends_with(".cargo/config")
                });
            }
        }

        let dependency_globs = || {
            let mut unknown = false;
            let mut globs: Vec<String> = dependencies
                .iter()
                .filter(
                    |dependency| match dependency.task_contract().dependency_source_inputs() {
                        crate::task_contracts::DependencySourceInputs::Include => true,
                        crate::task_contracts::DependencySourceInputs::Exclude => false,
                        crate::task_contracts::DependencySourceInputs::Unknown => {
                            unknown = true;
                            false
                        }
                    },
                )
                .flat_map(|dependency| {
                    crate_source_globs(path_to_root, dependency.directory().to_unix().as_str())
                })
                .collect();
            globs.sort();
            globs.dedup();
            (globs, unknown)
        };

        match self.package.kind {
            CargoPackageKind::Entrypoint | CargoPackageKind::Library => {
                if wants_automatic_inputs {
                    io.package_default_inputs = Some(true);
                    let (globs, unknown) = dependency_globs();
                    io.input_globs.extend(globs);
                    if unknown {
                        io.input_safety = toolchain::DerivedInputSafety::Untracked;
                    }
                }
                if subcommand == "build" {
                    if self.package.kind == CargoPackageKind::Library {
                        io.outputs = toolchain::DerivedOutputs::Unavailable;
                    } else {
                        io.outputs = self
                            .workspace
                            .as_ref()
                            .and_then(|workspace| {
                                let layout = cargo_output_layout(
                                    &self.repo_root,
                                    workspace,
                                    &self.package,
                                    context,
                                )?;
                                let effective_target =
                                    layout.target.as_deref().unwrap_or(&workspace.host_target);
                                let platform = target_platform(effective_target)?;
                                let package_directory = self.repo_root.resolve(package.directory());
                                let target_directory =
                                    AnchoredSystemPathBuf::relative_path_between(
                                        &package_directory,
                                        &layout.target_directory,
                                    )
                                    .to_unix();
                                Some(toolchain::DerivedOutputs::Resolved(
                                    deliverable_output_paths(
                                        target_directory.as_str(),
                                        layout.target.as_deref(),
                                        &layout.profile,
                                        platform,
                                        &self.package.deliverables,
                                    ),
                                ))
                            })
                            .unwrap_or(toolchain::DerivedOutputs::Unavailable);
                    }
                }
            }
            CargoPackageKind::Workspace => {
                if wants_automatic_inputs {
                    io.package_default_inputs = Some(false);
                    let (globs, unknown) = dependency_globs();
                    io.input_globs.extend(globs);
                    if unknown {
                        io.input_safety = toolchain::DerivedInputSafety::Untracked;
                    }
                }
            }
        }
        Some(io)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CargoTargetPlatform {
    Unix,
    Apple,
    WindowsMsvc,
    WindowsGnu,
}

fn target_platform(target: &str) -> Option<CargoTargetPlatform> {
    if target.is_empty()
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    let parts: Vec<&str> = target.split('-').collect();
    if parts.contains(&"windows") {
        return if parts.contains(&"msvc") {
            Some(CargoTargetPlatform::WindowsMsvc)
        } else if parts.iter().any(|part| matches!(*part, "gnu" | "gnullvm")) {
            Some(CargoTargetPlatform::WindowsGnu)
        } else {
            None
        };
    }
    if parts.contains(&"apple") && parts.contains(&"darwin") {
        return Some(CargoTargetPlatform::Apple);
    }
    parts
        .iter()
        .any(|part| {
            matches!(
                *part,
                "linux"
                    | "android"
                    | "freebsd"
                    | "netbsd"
                    | "openbsd"
                    | "dragonfly"
                    | "solaris"
                    | "illumos"
            )
        })
        .then_some(CargoTargetPlatform::Unix)
}

fn deliverable_basename(deliverable: &Deliverable, platform: CargoTargetPlatform) -> String {
    let name = &deliverable.name;
    match (deliverable.kind, platform) {
        (
            DeliverableKind::Bin,
            CargoTargetPlatform::WindowsMsvc | CargoTargetPlatform::WindowsGnu,
        ) => format!("{name}.exe"),
        (DeliverableKind::Bin, _) => name.clone(),
        (DeliverableKind::Cdylib, CargoTargetPlatform::Apple) => format!("lib{name}.dylib"),
        (
            DeliverableKind::Cdylib,
            CargoTargetPlatform::WindowsMsvc | CargoTargetPlatform::WindowsGnu,
        ) => format!("{name}.dll"),
        (DeliverableKind::Cdylib, CargoTargetPlatform::Unix) => format!("lib{name}.so"),
        (DeliverableKind::Staticlib, CargoTargetPlatform::WindowsMsvc) => format!("{name}.lib"),
        (DeliverableKind::Staticlib, _) => format!("lib{name}.a"),
    }
}

fn deliverable_output_paths(
    target_directory: &str,
    target: Option<&str>,
    profile: &str,
    platform: CargoTargetPlatform,
    deliverables: &[Deliverable],
) -> Vec<String> {
    let mut directory = target_directory.to_string();
    if let Some(target) = target {
        directory = join_prefix(&directory, target);
    }
    directory = join_prefix(&directory, profile);
    deliverables
        .iter()
        .map(|deliverable| join_prefix(&directory, &deliverable_basename(deliverable, platform)))
        .collect()
}

fn set_once(slot: &mut Option<String>, value: String) -> Option<()> {
    if slot.is_some() || value.is_empty() {
        return None;
    }
    *slot = Some(value);
    Some(())
}

#[derive(Debug, PartialEq, Eq)]
struct CargoOutputArguments {
    profile: String,
    target: Option<String>,
    target_directory: Option<String>,
}

fn cargo_output_arguments(args: &[String]) -> Option<CargoOutputArguments> {
    let mut release = false;
    let mut profile = None;
    let mut target = None;
    let mut target_directory = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let separate_value = |index: &mut usize| {
            *index += 1;
            args.get(*index)
                .cloned()
                .filter(|value| !value.is_empty() && !value.starts_with('-'))
        };
        match arg.as_str() {
            "-r" | "--release" if !release => release = true,
            "-r" | "--release" => return None,
            "--profile" => set_once(&mut profile, separate_value(&mut index)?)?,
            "--target" => set_once(&mut target, separate_value(&mut index)?)?,
            "--target-dir" => set_once(&mut target_directory, separate_value(&mut index)?)?,
            "--features" | "-F" | "--jobs" | "-j" | "--color" | "--message-format" => {
                separate_value(&mut index)?;
            }
            "-q"
            | "-v"
            | "--quiet"
            | "--verbose"
            | "--future-incompat-report"
            | "--keep-going"
            | "--all-features"
            | "--no-default-features"
            | "--timings"
            | "--ignore-rust-version"
            | "--locked"
            | "--offline"
            | "--frozen" => {}
            _ if arg.starts_with("--profile=") => {
                set_once(&mut profile, arg["--profile=".len()..].to_string())?
            }
            _ if arg.starts_with("--target=") => {
                set_once(&mut target, arg["--target=".len()..].to_string())?
            }
            _ if arg.starts_with("--target-dir=") => set_once(
                &mut target_directory,
                arg["--target-dir=".len()..].to_string(),
            )?,
            _ if [
                "--features=",
                "--jobs=",
                "--color=",
                "--message-format=",
                "--timings=",
            ]
            .iter()
            .any(|prefix| {
                arg.strip_prefix(prefix)
                    .is_some_and(|value| !value.is_empty())
            }) => {}
            _ if arg.len() > 2
                && (arg.starts_with("-F")
                    || arg.starts_with("-j")
                    || (arg.starts_with('-') && arg[1..].bytes().all(|byte| byte == b'v'))) => {}
            _ => return None,
        }
        index += 1;
    }
    if release && profile.is_some() {
        return None;
    }
    let profile = if release {
        "release".to_string()
    } else {
        match profile.as_deref() {
            None | Some("dev" | "test") => "debug".to_string(),
            Some("release" | "bench") => "release".to_string(),
            Some(profile)
                if profile
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) =>
            {
                profile.to_string()
            }
            Some(_) => return None,
        }
    };
    Some(CargoOutputArguments {
        profile,
        target,
        target_directory,
    })
}

fn contains_glob_syntax(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']' | b'{' | b'}'))
}

fn target_directory_within_repo(
    repo_root: &AbsoluteSystemPath,
    target_directory: &AbsoluteSystemPath,
) -> bool {
    if !repo_root.contains(target_directory) {
        return false;
    }
    let Ok(real_repo_root) = dunce::canonicalize(repo_root.as_std_path()) else {
        return false;
    };
    let mut existing_ancestor = target_directory.as_std_path();
    while !existing_ancestor.exists() {
        let Some(parent) = existing_ancestor.parent() else {
            return false;
        };
        existing_ancestor = parent;
    }
    dunce::canonicalize(existing_ancestor)
        .is_ok_and(|ancestor| ancestor.starts_with(real_repo_root))
}

#[derive(Debug, PartialEq, Eq)]
struct CargoOutputLayout {
    profile: String,
    target: Option<String>,
    target_directory: AbsoluteSystemPathBuf,
}

fn cargo_output_layout(
    repo_root: &AbsoluteSystemPath,
    workspace: &CargoWorkspaceDetails,
    package: &CargoPackageDetails,
    context: &toolchain::TaskIOContext<'_>,
) -> Option<CargoOutputLayout> {
    let environment = context.environment;
    let profile_dir_name = environment.iter().any(|(name, _)| {
        let name = name.to_ascii_uppercase();
        name.starts_with("CARGO_PROFILE_") && name.ends_with("_DIR_NAME")
    });

    if package.manifest_alters_output_layout
        || workspace.repository_config_alters_output_layout
        || workspace.repository_config_untracked
        || workspace.external_config_present
        || workspace.manifest_alters_profile_dirs
        || environment.get("RUSTC").is_some()
        || environment.get("CARGO_BUILD_RUSTC").is_some()
        || environment.get("CARGO_BUILD_TARGET_DIR").is_some()
        || environment.get("CARGO_BUILD_ARTIFACT_DIR").is_some()
        || profile_dir_name
    {
        return None;
    }

    let arguments = context
        .task_args
        .map_or_else(|| cargo_output_arguments(&[]), cargo_output_arguments)?;
    let target = arguments
        .target
        .or_else(|| environment.get("CARGO_BUILD_TARGET").map(str::to_string));
    if target
        .as_ref()
        .is_some_and(|target| !workspace.supported_targets.contains(target))
    {
        return None;
    }

    let target_directory = if let Some(configured) = arguments
        .target_directory
        .or_else(|| environment.get("CARGO_TARGET_DIR").map(str::to_string))
    {
        if contains_glob_syntax(&configured) {
            return None;
        }
        let path = std::path::Path::new(&configured);
        if path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
        {
            return None;
        }
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo_root.as_std_path().join(path)
        };
        AbsoluteSystemPathBuf::new(path.to_str()?.to_string()).ok()?
    } else {
        workspace.target_directory.clone()
    };
    if contains_glob_syntax(target_directory.as_str())
        || !target_directory_within_repo(repo_root, &target_directory)
    {
        return None;
    }

    Some(CargoOutputLayout {
        profile: arguments.profile,
        target,
        target_directory,
    })
}

fn cargo_change_observation(
    repo_root: &AbsoluteSystemPath,
    target_directory: Option<&AbsoluteSystemPath>,
) -> ChangeObservation {
    let mut observation = ChangeObservation::new()
        .with_rediscovery_file_name(CARGO_TOML)
        .with_resolution_path(CARGO_LOCK);
    if let Some(prefix) = target_directory
        .and_then(|path| repo_root.anchor(path).ok())
        .filter(|path| path.components().next().is_some())
    {
        observation = observation.with_ignore_prefix(prefix.to_unix().to_string());
    }
    observation
}

/// Cargo prune inputs captured atomically with the discovery generation.
#[derive(Debug)]
struct CargoPruneKnowledge {
    domain: crate::prune_knowledge::PruneDomainId,
    lockfile: String,
    root_manifest: String,
    package_directories: HashMap<String, String>,
}

impl CargoPruneKnowledge {
    fn discover(
        repo_root: &AbsoluteSystemPath,
        crates: &[CargoCrate],
        lockfile: String,
    ) -> Result<Self, Error> {
        let root_manifest = repo_root
            .join_component(CARGO_TOML)
            .read_to_string()
            .map_err(Error::WorkspaceFileRead)?;
        let package_directories = crates
            .iter()
            .filter_map(|cargo_crate| {
                let directory = cargo_crate.manifest_path.parent()?;
                let directory = AnchoredSystemPathBuf::new(repo_root, directory).ok()?;
                Some((cargo_crate.name.clone(), directory.to_unix().to_string()))
            })
            .collect();
        Ok(Self {
            domain: crate::prune_knowledge::CARGO_PRUNE_DOMAIN.clone(),
            lockfile,
            root_manifest,
            package_directories,
        })
    }
}

impl PruneDomain for CargoPruneKnowledge {
    fn id(&self) -> &crate::prune_knowledge::PruneDomainId {
        &self.domain
    }

    fn plan(
        &self,
        kept_packages: &[String],
    ) -> Result<Option<PrunePlan>, crate::prune_knowledge::Error> {
        if kept_packages.is_empty() {
            return Ok(None);
        }
        let failed = |error: Error| crate::prune_knowledge::Error::Failed(Box::new(error));
        let pruned_lock = turborepo_lockfiles::cargo_prune_lock(&self.lockfile, kept_packages)
            .map_err(|error| failed(Error::Lockfile(error)))?;

        let mut kept_dirs = Vec::with_capacity(pruned_lock.members.len());
        let mut extra_packages = Vec::new();
        for member in &pruned_lock.members {
            let Some(directory) = self.package_directories.get(member) else {
                tracing::warn!(
                    "Cargo.lock member {member} is not a discovered workspace crate; skipping"
                );
                continue;
            };
            kept_dirs.push(directory.clone());
            if !kept_packages.contains(member) {
                extra_packages.push(member.clone());
            }
        }
        let pruned_manifest =
            prune_root_manifest(&self.root_manifest, &kept_dirs).map_err(failed)?;
        Ok(Some(PrunePlan {
            extra_packages,
            root_files: vec![
                (CARGO_LOCK.to_string(), pruned_lock.lockfile),
                (CARGO_TOML.to_string(), pruned_manifest),
            ],
            copy_paths: [
                "rust-toolchain.toml",
                "rust-toolchain",
                ".cargo/config.toml",
                ".cargo/config",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }))
    }

    fn finalize(&self, pruned_root: &AbsoluteSystemPath) -> Vec<String> {
        finalize_cargo_prune(pruned_root)
    }
}

/// The Cargo repository contributor. Registered during graph construction when
/// `futureFlags.experimentalCargoWorkspaces` is enabled and the repository
/// root contains a `Cargo.toml`.
pub(crate) struct CargoContributor {
    repo_root: AbsoluteSystemPathBuf,
}

impl CargoContributor {
    pub(crate) fn new(repo_root: AbsoluteSystemPathBuf) -> Arc<Self> {
        Arc::new(Self { repo_root })
    }
}

/// Project execution-only compiler-cache settings from Cargo task knowledge.
/// User-managed wrappers and sccache settings remain authoritative.
fn cargo_compile_cache_env(
    endpoint: &toolchain::CompileCacheEndpoint,
    task_env: &std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
    if task_env.contains_key("RUSTC_WRAPPER")
        || task_env.keys().any(|key| key.starts_with("SCCACHE_"))
    {
        return Vec::new();
    }
    let ambient_incremental = task_env.get("CARGO_INCREMENTAL").map(String::as_str);
    if ambient_incremental.is_some_and(|value| value != "0") {
        return Vec::new();
    }

    let mut vars = vec![
        ("RUSTC_WRAPPER".to_string(), endpoint.wrapper.clone()),
        (
            toolchain::COMPILE_CACHE_WRAPPER_ENV.to_string(),
            "1".to_string(),
        ),
        ("SCCACHE_WEBDAV_ENDPOINT".to_string(), endpoint.url.clone()),
        ("SCCACHE_WEBDAV_TOKEN".to_string(), endpoint.token.clone()),
        (
            "SCCACHE_SERVER_PORT".to_string(),
            endpoint.server_port.to_string(),
        ),
        (
            "SCCACHE_IGNORE_SERVER_IO_ERROR".to_string(),
            "1".to_string(),
        ),
    ];
    if ambient_incremental.is_none() {
        vars.push(("CARGO_INCREMENTAL".to_string(), "0".to_string()));
    }
    vars
}

/// Let Cargo remove feature-dead entries after the reachability-based lockfile
/// projection. Failure is non-fatal: the superset lock remains buildable.
fn finalize_cargo_prune(pruned_root: &AbsoluteSystemPath) -> Vec<String> {
    let sync = |offline: bool| {
        let mut cmd = std::process::Command::new("cargo");
        cmd.args(["metadata", "--format-version", "1"]);
        if offline {
            cmd.arg("--offline");
        }
        cmd.current_dir(pruned_root.as_std_path()).output()
    };
    match sync(true).and_then(|offline| {
        if offline.status.success() {
            Ok(offline)
        } else {
            sync(false)
        }
    }) {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            tracing::warn!(
                "unable to canonicalize the pruned Cargo.lock; `cargo build --locked` may require \
                 a lockfile refresh: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Err(error) => {
            tracing::warn!("unable to run cargo to canonicalize the pruned Cargo.lock: {error}");
        }
    }
    vec![CARGO_LOCK.to_string()]
}

enum ContributorMetadata {
    Absent,
    Resolved {
        metadata: Metadata,
        lockfile: String,
    },
    Unresolved(Error),
}

impl ContributorMetadata {
    fn lockfile(&self) -> Option<&str> {
        match self {
            Self::Resolved { lockfile, .. } => Some(lockfile),
            Self::Absent | Self::Unresolved(_) => None,
        }
    }
}

fn discover_contributor_workspace(
    repo_root: &AbsoluteSystemPath,
) -> Result<(DiscoveredWorkspace, ContributorMetadata), Error> {
    let root_manifest_path = repo_root.join_component(CARGO_TOML);
    if !root_manifest_path.exists() {
        return Ok((discover_crates(repo_root)?, ContributorMetadata::Absent));
    }

    let lockfile = match read_lockfile(repo_root) {
        Ok(lockfile) => lockfile,
        Err(error) => {
            return Ok((
                discover_crates(repo_root)?,
                ContributorMetadata::Unresolved(error),
            ));
        }
    };

    match locked_metadata(repo_root) {
        Ok(metadata) => {
            let workspace = workspace_from_metadata(repo_root, &metadata)?;
            Ok((
                workspace,
                ContributorMetadata::Resolved { metadata, lockfile },
            ))
        }
        Err(error) => Ok((
            discover_crates(repo_root)?,
            ContributorMetadata::Unresolved(error),
        )),
    }
}

fn validate_contributor_metadata(
    repo_root: &AbsoluteSystemPath,
    metadata: ContributorMetadata,
) -> Result<(), Error> {
    match metadata {
        ContributorMetadata::Absent => Ok(()),
        ContributorMetadata::Resolved { metadata, lockfile } => {
            if read_lockfile(repo_root)? != lockfile {
                return Err(Error::InvalidLockfile {
                    stderr: "Cargo.lock changed during repository discovery".to_string(),
                });
            }
            validate_resolved_local_packages(repo_root, &metadata)
        }
        ContributorMetadata::Unresolved(error) => {
            require_lockfile(repo_root)?;
            Err(error)
        }
    }
}

impl RepositoryContributor for CargoContributor {
    fn id(&self) -> ToolchainId {
        ToolchainId::RUST
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            // Discovery spawns `cargo metadata` synchronously, so keep it off
            // the async runtime like the JavaScript manifest-parsing path.
            let (workspace, metadata) = turborepo_rayon_compat::block_in_place(|| {
                discover_contributor_workspace(&self.repo_root)
            })
            .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;
            let workspace_roots = self
                .repo_root
                .join_component(CARGO_TOML)
                .exists()
                .then(|| WorkspaceRoot::new("cargo", self.repo_root.clone()))
                .into_iter()
                .collect();
            let target_directory = workspace.target_directory.clone();
            let crates = workspace.crates;

            if crates.is_empty() {
                if workspace.has_packages {
                    turborepo_rayon_compat::block_in_place(|| {
                        validate_contributor_metadata(&self.repo_root, metadata)
                    })
                    .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;
                }
                return Ok(DiscoveredPackages::new(Vec::new(), workspace_roots));
            }

            // Using Turborepo with Rust requires naming the workspace: the
            // synthetic workspace package is a real package (task keys,
            // filters), and every package must have a name. Only enforced
            // when there are crates to host — a memberless manifest doesn't
            // demand a name for nothing.
            let workspace_name = workspace
                .name
                .ok_or_else(|| toolchain::Error::Failed(Box::new(Error::MissingWorkspaceName)))?;

            let change_observation =
                cargo_change_observation(&self.repo_root, target_directory.as_deref());
            let lockfile = if let Some(lockfile) = metadata.lockfile() {
                lockfile.to_string()
            } else {
                read_lockfile(&self.repo_root)
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))?
            };
            let prune_domain =
                CargoPruneKnowledge::discover(&self.repo_root, &crates, lockfile.clone())
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;

            // Each crate contributes its already-classified native internal
            // relationships directly. No JavaScript dependency descriptor or
            // package-manager policy participates in Cargo graph assembly.
            // External dependencies (locked crates.io/git packages plus the
            // compiler itself) participate in each crate task's hash through
            // the same external-dependency mechanism JS packages use, scoped
            // to the crate's transitive closure — a dependency bump only
            // invalidates crates that actually depend on it, and a toolchain
            // change invalidates everything.
            let all_names: Vec<String> = crates.iter().map(|c| c.name.clone()).collect();
            let (rustc, host_target, supported_targets, mut closures) =
                turborepo_rayon_compat::block_in_place(|| {
                    validate_contributor_metadata(&self.repo_root, metadata)?;
                    let (rustc, host_target) = rustc_info(&self.repo_root)?;
                    let mut supported_targets = rustc_supported_targets(&self.repo_root);
                    supported_targets.insert(host_target.clone());
                    Ok::<_, Error>((
                        rustc,
                        host_target,
                        supported_targets,
                        external_closures_from_lockfile(&lockfile, &all_names)?,
                    ))
                })
                .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;
            let workspace_contract_details = target_directory.map(|target_directory| {
                let startup_environment = CargoHomeEnvironment::current();
                let config = cargo_config_influence(&self.repo_root, &startup_environment);
                CargoWorkspaceDetails {
                    target_directory,
                    host_target,
                    supported_targets,
                    repository_config_alters_output_layout: config.repository_alters_output_layout,
                    repository_config_untracked: config.repository_config_untracked,
                    external_config_present: config.external_present,
                    manifest_alters_profile_dirs: manifest_alters_profile_dirs(&self.repo_root),
                }
            });
            let workspace_externals: HashSet<turborepo_lockfiles::Package> = closures
                .values()
                .flatten()
                .cloned()
                .chain(std::iter::once(rustc.clone()))
                .collect();

            let mut packages = Vec::with_capacity(crates.len() + 1);
            let mut resolutions = Vec::with_capacity(crates.len() + 1);
            let mut crate_names = Vec::with_capacity(crates.len());
            for cargo_crate in crates {
                let relationships = cargo_crate.relationships.clone();
                let kind = if cargo_crate.is_entrypoint() {
                    CargoPackageKind::Entrypoint
                } else {
                    CargoPackageKind::Library
                };
                let details = CargoPackageDetails {
                    kind,
                    deliverables: cargo_crate.deliverables,
                    manifest_alters_output_layout: cargo_crate.manifest_alters_output_layout,
                };
                let native_tasks = native_tasks_for_package(&details, &cargo_crate.name);
                let task_contract = CargoTaskContract::new(
                    self.repo_root.clone(),
                    details.clone(),
                    workspace_contract_details.clone(),
                );
                let external_dependencies: HashSet<turborepo_lockfiles::Package> = closures
                    .remove(&cargo_crate.name)
                    .unwrap_or_default()
                    .into_iter()
                    .chain(std::iter::once(rustc.clone()))
                    .collect();
                resolutions.push(package_resolution(
                    cargo_crate.name.clone(),
                    &external_dependencies,
                ));
                crate_names.push(cargo_crate.name.clone());
                packages.push(
                    DiscoveredPackage::package(
                        Some(cargo_crate.name.clone()),
                        PackageJson::default(),
                        cargo_crate.manifest_path,
                    )
                    .with_native_relationships(relationships)
                    .with_native_tasks(native_tasks)
                    .with_task_contract(
                        crate::task_contracts::ScopeTaskContract::cargo(task_contract),
                    ),
                );
            }

            // The workspace aggregate, anchored at the root
            // Cargo.toml and named by the user via `[workspace.metadata]
            // name`. It depends on every crate so `--affected` and
            // dependent-filters propagate crate changes to it.
            if !crate_names.is_empty() {
                let workspace_package_details = CargoPackageDetails {
                    kind: CargoPackageKind::Workspace,
                    deliverables: Vec::new(),
                    manifest_alters_output_layout: false,
                };
                let workspace_native_tasks =
                    native_tasks_for_package(&workspace_package_details, &workspace_name);
                let task_contract = CargoTaskContract::new(
                    self.repo_root.clone(),
                    workspace_package_details.clone(),
                    workspace_contract_details.clone(),
                );
                crate_names.sort();
                let relationships = crate_names
                    .into_iter()
                    .map(|name| Relationship::internal(name, DependencyKind::Production))
                    .collect();
                resolutions.push(package_resolution(
                    workspace_name.clone(),
                    &workspace_externals,
                ));
                packages.push(
                    DiscoveredPackage::aggregate(
                        workspace_name,
                        PackageJson::default(),
                        self.repo_root.join_component(CARGO_TOML),
                    )
                    .with_native_relationships(relationships)
                    .with_native_tasks(workspace_native_tasks)
                    .with_task_contract(
                        crate::task_contracts::ScopeTaskContract::cargo(task_contract),
                    ),
                );
            }

            let members = resolutions
                .iter()
                .map(|resolution| resolution.package().to_string())
                .collect::<Vec<_>>();
            let resolution = ExternalResolutionDomain::new(
                crate::external_resolution::CARGO_RESOLUTION_DOMAIN.clone(),
                ToolchainId::RUST,
                AnchoredSystemPathBuf::default(),
                members,
                [AnchoredSystemPathBuf::from_raw(CARGO_LOCK)
                    .map_err(Error::from)
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))?],
                ExternalResolutionData::Resolved {
                    completeness: ResolutionCompleteness::Complete,
                    packages: resolutions,
                },
            );
            Ok(DiscoveredPackages::new(packages, workspace_roots)
                .with_external_resolution(resolution)
                .with_change_observation(change_observation)
                .with_prune_domain(Arc::new(prune_domain)))
        })
    }
}

/// The Cargo default build directory, relative to the repo root.
pub const TARGET_DIR: &str = "target";

/// Whether `name` is a valid Cargo crate name for our purposes. Cargo itself
/// enforces this for published crates; local manifests are looser, so guard
/// against names that would break downstream task identifiers.
pub fn is_valid_crate_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A deliverable artifact an entrypoint crate produces: the target name plus
/// the artifact flavor, which determines the file names Cargo writes to the
/// target directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deliverable {
    /// The target name as reported by `cargo metadata`. Bin targets keep
    /// their manifest spelling; lib-flavored targets are already
    /// snake_cased, matching the artifact file name.
    pub name: String,
    pub kind: DeliverableKind,
}

/// The artifact flavor of a [`Deliverable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliverableKind {
    /// An executable: `<name>` / `<name>.exe`.
    Bin,
    /// A C-compatible dynamic library: `lib<name>.so` / `lib<name>.dylib` /
    /// `<name>.dll`.
    Cdylib,
    /// A C-compatible static archive: `lib<name>.a` / `<name>.lib`.
    Staticlib,
}

/// A single Rust crate discovered within a Cargo workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoCrate {
    /// The crate's package name (from `[package].name`).
    pub name: String,
    /// Absolute path to the crate's `Cargo.toml`.
    pub manifest_path: AbsoluteSystemPathBuf,
    /// Direct relationships to other workspace crates, resolved by Cargo.
    /// Development edges that would make task ordering cyclic remain as
    /// hash/affectedness inputs without participating in ordering.
    pub relationships: Vec<Relationship>,
    /// The crate's deliverable targets. Non-empty exactly when the crate is
    /// an entrypoint (has `bin`/`cdylib`/`staticlib` targets).
    pub deliverables: Vec<Deliverable>,
    pub manifest_alters_output_layout: bool,
}

impl CargoCrate {
    /// Whether this crate is an entrypoint: it produces deliverable
    /// artifacts.
    pub fn is_entrypoint(&self) -> bool {
        !self.deliverables.is_empty()
    }
}

/// The result of Cargo workspace discovery: the member crates plus the
/// user-declared workspace name.
#[derive(Debug)]
pub struct DiscoveredWorkspace {
    /// The workspace's name from `[workspace.metadata] name`, validated
    /// against the crate set when present. Not required at this layer —
    /// it only becomes mandatory when the workspace package is actually
    /// synthesized (see [`RepositoryContributor::discover_packages`]), so
    /// manifests without members don't demand a name for nothing.
    pub name: Option<String>,
    pub crates: Vec<CargoCrate>,
    /// Whether Cargo reported any workspace packages before Turborepo's
    /// repository-boundary filtering. A workspace with packages that all get
    /// filtered must still run full validation rather than be mistaken for a
    /// memberless virtual workspace.
    pub has_packages: bool,
    pub target_directory: Option<AbsoluteSystemPathBuf>,
}

fn manifest_alters_profile_dirs(repo_root: &AbsoluteSystemPath) -> bool {
    let Ok(contents) = repo_root.join_component(CARGO_TOML).read_to_string() else {
        return true;
    };
    let Ok(manifest) = contents.parse::<toml_edit::DocumentMut>() else {
        return true;
    };
    manifest
        .get("profile")
        .and_then(toml_edit::Item::as_table_like)
        .is_some_and(|profiles| {
            profiles
                .iter()
                .any(|(_, profile)| profile.get("dir-name").is_some())
        })
}

#[derive(Debug, Default)]
struct CargoConfigInfluence {
    repository_alters_output_layout: bool,
    repository_config_untracked: bool,
    external_present: bool,
}

fn path_contains_symlink(repo_root: &AbsoluteSystemPath, path: &std::path::Path) -> bool {
    let Ok(relative) = path.strip_prefix(repo_root.as_std_path()) else {
        return true;
    };
    let mut current = repo_root.as_std_path().to_path_buf();
    if std::fs::symlink_metadata(&current)
        .map_or(true, |metadata| metadata.file_type().is_symlink())
    {
        return true;
    }
    for component in relative.components() {
        current.push(component);
        if std::fs::symlink_metadata(&current)
            .map_or(true, |metadata| metadata.file_type().is_symlink())
        {
            return true;
        }
    }
    false
}

fn config_alters_output_layout(
    repo_root: &AbsoluteSystemPath,
    path: &std::path::Path,
) -> Option<(bool, bool)> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(_) => return Some((true, true)),
    }
    let has_symlink = path_contains_symlink(repo_root, path);
    let contained = dunce::canonicalize(repo_root.as_std_path())
        .ok()
        .zip(dunce::canonicalize(path).ok())
        .is_some_and(|(root, config)| config.starts_with(root));
    if !contained {
        return Some((true, true));
    }
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return Some((true, has_symlink)),
    };
    let config = match contents.parse::<toml_edit::DocumentMut>() {
        Ok(config) => config,
        Err(_) => return Some((true, has_symlink)),
    };
    let build = config.get("build");
    // Cargo metadata reports the effective target-dir as an absolute path, so
    // that key is resolved separately with repository containment checks.
    let profile_dir_name = config
        .get("profile")
        .and_then(toml_edit::Item::as_table_like)
        .is_some_and(|profiles| {
            profiles
                .iter()
                .any(|(_, profile)| profile.get("dir-name").is_some())
        });
    let includes = config.get("include").is_some();
    Some((
        build.is_some_and(|build| {
            ["target", "rustc", "artifact-dir"]
                .iter()
                .any(|key| build.get(key).is_some())
        }) || profile_dir_name
            || includes,
        has_symlink || includes,
    ))
}

#[derive(Debug, Default)]
struct CargoHomeEnvironment {
    cargo_home: Option<std::ffi::OsString>,
    user_profile: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
}

impl CargoHomeEnvironment {
    fn current() -> Self {
        // `var_os` preserves non-UTF-8 values and follows Windows' case-insensitive
        // lookup.
        Self {
            cargo_home: std::env::var_os("CARGO_HOME"),
            user_profile: std::env::var_os("USERPROFILE"),
            home: std::env::var_os("HOME"),
        }
    }
}

fn cargo_home_candidates(
    repo_root: &AbsoluteSystemPath,
    environment: &CargoHomeEnvironment,
    windows: bool,
) -> Vec<std::path::PathBuf> {
    if let Some(cargo_home) = environment.cargo_home.as_deref() {
        let cargo_home = std::path::Path::new(cargo_home);
        return vec![if cargo_home.is_absolute() {
            cargo_home.to_path_buf()
        } else {
            repo_root.as_std_path().join(cargo_home)
        }];
    }

    let mut candidates = Vec::new();
    if windows && let Some(user_profile) = environment.user_profile.as_deref() {
        candidates.push(std::path::Path::new(user_profile).join(".cargo"));
    }
    if let Some(home) = environment.home.as_deref() {
        let home = std::path::Path::new(home).join(".cargo");
        if !candidates.contains(&home) {
            candidates.push(home);
        }
    }
    candidates
}

fn cargo_config_influence(
    repo_root: &AbsoluteSystemPath,
    environment: &CargoHomeEnvironment,
) -> CargoConfigInfluence {
    let repository_cargo = repo_root.as_std_path().join(".cargo");
    let mut influence = CargoConfigInfluence::default();
    for name in ["config.toml", "config"] {
        if let Some((alters_output_layout, untracked)) =
            config_alters_output_layout(repo_root, &repository_cargo.join(name))
        {
            influence.repository_alters_output_layout |= alters_output_layout;
            influence.repository_config_untracked |= untracked;
        }
    }

    let ancestor_cargo_homes = repo_root
        .as_std_path()
        .ancestors()
        .skip(1)
        .map(|ancestor| ancestor.join(".cargo"));
    let cargo_homes = cargo_home_candidates(repo_root, environment, cfg!(windows));
    for cargo_home in ancestor_cargo_homes.chain(cargo_homes) {
        if cargo_home == repository_cargo {
            continue;
        }
        for name in ["config.toml", "config"] {
            match std::fs::symlink_metadata(cargo_home.join(name)) {
                Ok(_) => influence.external_present = true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => influence.external_present = true,
            }
        }
    }
    influence
}

/// Discover all Rust crates in the Cargo workspace rooted at `repo_root` by
/// invoking `cargo metadata --no-deps`.
///
/// Returns an empty workspace if `repo_root` has no `Cargo.toml`. A root
/// manifest that exists but that Cargo rejects is an error — the user opted
/// into Cargo support, so silently discovering nothing would be misleading.
/// `--no-deps` skips registry resolution, so no lockfile or network access
/// is required.
///
/// Crates whose manifests live outside the repository root, or whose names
/// are invalid, are skipped with a warning. A `[package]` in the root manifest
/// is modeled as a normal Cargo package anchored at the repository root.
pub fn discover_crates(repo_root: &AbsoluteSystemPath) -> Result<DiscoveredWorkspace, Error> {
    let root_manifest_path = repo_root.join_component(CARGO_TOML);
    if !root_manifest_path.exists() {
        return Ok(DiscoveredWorkspace {
            name: None,
            crates: Vec::new(),
            has_packages: false,
            target_directory: None,
        });
    }

    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
            root_manifest_path.as_str(),
        ])
        .current_dir(repo_root.as_std_path())
        .output()
        .map_err(Error::MetadataSpawn)?;
    if !output.status.success() {
        return Err(Error::Metadata {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let metadata: Metadata = serde_json::from_slice(&output.stdout)?;

    workspace_from_metadata(repo_root, &metadata)
}

fn workspace_from_metadata(
    repo_root: &AbsoluteSystemPath,
    metadata: &Metadata,
) -> Result<DiscoveredWorkspace, Error> {
    let has_packages = !metadata.workspace_members.is_empty();
    let name = workspace_name(metadata)?;
    let target_directory = metadata_path(&metadata.target_directory);
    let packages = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .cloned()
        .collect();
    let crates = connect_crates(parse_members(repo_root, packages));

    if let Some(name) = &name
        && let Some(collision) = crates.iter().find(|c| &c.name == name)
    {
        return Err(Error::WorkspaceNameCollision {
            name: name.clone(),
            dir: collision
                .manifest_path
                .parent()
                .map(|dir| dir.to_string())
                .unwrap_or_default(),
        });
    }

    Ok(DiscoveredWorkspace {
        name,
        crates,
        has_packages,
        target_directory,
    })
}

/// Extract and validate the user-declared workspace name from the
/// `[workspace.metadata]` table. The name becomes a package name — it
/// appears in task keys (`<name>#test`) and `--filter` expressions — so it
/// follows the same shape rules as crate names.
fn workspace_name(metadata: &Metadata) -> Result<Option<String>, Error> {
    let Some(value) = metadata.metadata.get("name") else {
        return Ok(None);
    };
    let Some(name) = value.as_str() else {
        return Err(Error::InvalidWorkspaceName {
            name: value.to_string(),
            reason: "must be a string".to_string(),
        });
    };
    if !is_valid_crate_name(name) {
        return Err(Error::InvalidWorkspaceName {
            name: name.to_string(),
            reason: "names may only contain alphanumeric characters, `-`, and `_`".to_string(),
        });
    }
    // Legal, but re-introduces exactly the toolchain-id/package-name
    // confusion user-chosen names exist to remove.
    if name == "rust" || name == "javascript" {
        tracing::warn!(
            "the Cargo workspace is named {name:?}, which is also a toolchain id; consider a more \
             distinctive name"
        );
    }
    Ok(Some(name.to_string()))
}

/// A workspace member parsed from `cargo metadata`, before dependency edges
/// are resolved to crate names.
struct ParsedCrate {
    name: String,
    manifest_path: AbsoluteSystemPathBuf,
    dependencies: Vec<ResolvedDep>,
    deliverables: Vec<Deliverable>,
    manifest_alters_output_layout: bool,
}

/// A path dependency resolved to the directory Cargo reports for it.
struct ResolvedDep {
    dir: AbsoluteSystemPathBuf,
    kind: DependencyKind,
}

/// Normalize a path reported by `cargo metadata` into an
/// [`AbsoluteSystemPathBuf`]. On Windows, Cargo reports canonicalized
/// dependency paths in verbatim form (`\\?\C:\...`) while manifest paths
/// stay plain — `dunce::simplified` strips the verbatim prefix so the two
/// families compare equal.
fn metadata_path(path: &str) -> Option<AbsoluteSystemPathBuf> {
    AbsoluteSystemPathBuf::new(
        dunce::simplified(std::path::Path::new(path))
            .to_str()?
            .to_owned(),
    )
    .ok()
}

fn manifest_alters_output_layout(manifest_path: &AbsoluteSystemPath) -> bool {
    let Ok(contents) = manifest_path.read_to_string() else {
        return true;
    };
    let Ok(manifest) = contents.parse::<toml_edit::DocumentMut>() else {
        return true;
    };
    let enables_per_package_target = manifest
        .get("cargo-features")
        .and_then(toml_edit::Item::as_array)
        .is_some_and(|features| {
            features
                .iter()
                .any(|feature| feature.as_str() == Some("per-package-target"))
        });
    let package_selects_target = manifest.get("package").is_some_and(|package| {
        package.get("default-target").is_some() || package.get("forced-target").is_some()
    });
    let target_renames_output = ["bin", "example", "test", "bench"]
        .iter()
        .filter_map(|kind| manifest.get(kind)?.as_array_of_tables())
        .flatten()
        .any(|target| target.contains_key("filename"));
    enables_per_package_target || package_selects_target || target_renames_output
}

fn parse_members(
    repo_root: &AbsoluteSystemPath,
    packages: Vec<MetadataPackage>,
) -> Vec<ParsedCrate> {
    let mut parsed = Vec::new();
    for package in packages {
        let Some(manifest_path) = metadata_path(&package.manifest_path) else {
            tracing::warn!(
                "skipping Cargo crate {}: non-absolute manifest path {}",
                package.name,
                package.manifest_path
            );
            continue;
        };
        if !repo_root.contains(&manifest_path) {
            tracing::warn!(
                "skipping Cargo crate {}: manifest {manifest_path} is outside the repository",
                package.name
            );
            continue;
        }
        if !is_valid_crate_name(&package.name) {
            tracing::warn!(
                "skipping Cargo manifest {manifest_path}: invalid crate name {:?}",
                package.name
            );
            continue;
        }

        // A target's `kind` distinguishes real bins from tests/benches/
        // build scripts (which share the `bin` crate-type). A single lib
        // target can carry multiple flavors (`crate-type = ["lib",
        // "cdylib", "staticlib"]`), so each flavor becomes its own
        // deliverable.
        let deliverables: Vec<Deliverable> = package
            .targets
            .iter()
            .flat_map(|target| {
                target.kind.iter().filter_map(|kind| {
                    let kind = match kind.as_str() {
                        "bin" => DeliverableKind::Bin,
                        "cdylib" => DeliverableKind::Cdylib,
                        "staticlib" => DeliverableKind::Staticlib,
                        _ => return None,
                    };
                    Some(Deliverable {
                        name: target.name.clone(),
                        kind,
                    })
                })
            })
            .collect();

        let dependencies = package
            .dependencies
            .into_iter()
            .filter_map(|dep| {
                let path = dep.path?;
                let dir = metadata_path(&path)?;
                Some(ResolvedDep {
                    dir,
                    kind: if dep.kind.as_deref() == Some("dev") {
                        DependencyKind::Development
                    } else if dep.optional {
                        DependencyKind::Optional
                    } else {
                        DependencyKind::Production
                    },
                })
            })
            .collect();

        let manifest_alters_output_layout = manifest_alters_output_layout(&manifest_path);
        parsed.push(ParsedCrate {
            name: package.name,
            manifest_path,
            dependencies,
            deliverables,
            manifest_alters_output_layout,
        });
    }
    parsed
}

/// Resolve dependency edges to crate names by manifest directory. Development
/// edges that would form a cycle remain compilation inputs but do not order
/// tasks, since Cargo permits dev-dependency cycles while the task graph is a
/// DAG.
fn connect_crates(parsed: Vec<ParsedCrate>) -> Vec<CargoCrate> {
    let dir_to_name: HashMap<&AbsoluteSystemPath, &str> = parsed
        .iter()
        .filter_map(|c| Some((c.manifest_path.parent()?, c.name.as_str())))
        .collect();

    let mut adjacency: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    let mut relationships: HashMap<String, Vec<Relationship>> = HashMap::new();
    let mut dev_edges: Vec<(&str, &str, DependencyKind)> = Vec::new();
    for parsed_crate in &parsed {
        let from = parsed_crate.name.as_str();
        adjacency.entry(from).or_default();
        relationships.entry(from.to_string()).or_default();
        for dep in &parsed_crate.dependencies {
            let Some(&to) = dir_to_name.get(&*dep.dir) else {
                // Path dependency on a non-member (e.g. outside the repo).
                continue;
            };
            if to == from {
                continue;
            }
            if dep.kind == DependencyKind::Development {
                dev_edges.push((from, to, dep.kind));
            } else {
                adjacency.entry(from).or_default().insert(to);
                relationships
                    .entry(from.to_string())
                    .or_default()
                    .push(Relationship::internal(to, dep.kind));
            }
        }
    }
    // Deterministic order so the same dev edge always wins when a cycle must
    // be broken.
    dev_edges.sort_unstable_by(|left, right| (left.0, left.1).cmp(&(right.0, right.1)));
    dev_edges.dedup();
    for (from, to, kind) in dev_edges {
        if reaches(&adjacency, to, from) {
            tracing::debug!(
                "dropping dev-dependency edge {from} -> {to}: it would create a cycle in the \
                 package graph"
            );
            relationships
                .entry(from.to_string())
                .or_default()
                .push(Relationship::internal_input(to, kind));
        } else {
            adjacency.entry(from).or_default().insert(to);
            relationships
                .entry(from.to_string())
                .or_default()
                .push(Relationship::internal(to, kind));
        }
    }

    parsed
        .into_iter()
        .map(|parsed_crate| {
            let mut crate_relationships = relationships
                .remove(parsed_crate.name.as_str())
                .unwrap_or_default();
            crate_relationships
                .sort_by(|left, right| left.declaration_name().cmp(right.declaration_name()));
            crate_relationships.dedup();
            CargoCrate {
                relationships: crate_relationships,
                name: parsed_crate.name,
                manifest_path: parsed_crate.manifest_path,
                deliverables: parsed_crate.deliverables,
                manifest_alters_output_layout: parsed_crate.manifest_alters_output_layout,
            }
        })
        .collect()
}

/// Whether `target` is reachable from `start` in the current adjacency map.
fn reaches(adjacency: &HashMap<&str, BTreeSet<&str>>, start: &str, target: &str) -> bool {
    if start == target {
        return true;
    }
    let mut stack = vec![start];
    let mut visited = HashSet::new();
    while let Some(node) = stack.pop() {
        if !visited.insert(node) {
            continue;
        }
        if let Some(next) = adjacency.get(node) {
            for &dep in next {
                if dep == target {
                    return true;
                }
                stack.push(dep);
            }
        }
    }
    false
}

/// The subset of `cargo metadata` output used for discovery and validation.
#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
    workspace_members: HashSet<String>,
    target_directory: String,
    /// The `[workspace.metadata]` table, serialized as JSON. Carries the
    /// user-declared workspace name.
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
struct MetadataPackage {
    id: String,
    name: String,
    source: Option<String>,
    manifest_path: String,
    #[serde(default)]
    dependencies: Vec<MetadataDependency>,
    #[serde(default)]
    targets: Vec<MetadataTarget>,
}

#[derive(Clone, Debug, Deserialize)]
struct MetadataDependency {
    /// Absolute path to the dependency's directory, present only for path
    /// dependencies.
    path: Option<String>,
    /// `null` for normal deps, `"dev"` or `"build"` otherwise.
    kind: Option<String>,
    #[serde(default)]
    optional: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct MetadataTarget {
    name: String,
    kind: Vec<String>,
}

#[cfg(test)]
mod test {
    use turbopath::{AbsoluteSystemPathBuf, IntoUnix};

    use super::*;

    #[test]
    fn full_metadata_discovers_only_workspace_members() {
        let (_temp, root) = tempdir_root();
        let member_manifest = root.join_components(&["crates", "member", CARGO_TOML]);
        let metadata = Metadata {
            packages: vec![
                MetadataPackage {
                    id: "member 0.1.0 (path+file:///repo/crates/member)".to_string(),
                    name: "member".to_string(),
                    source: None,
                    manifest_path: member_manifest.to_string(),
                    dependencies: Vec::new(),
                    targets: Vec::new(),
                },
                MetadataPackage {
                    id: "registry 1.0.0 (registry+https://example.com/index)".to_string(),
                    name: "registry".to_string(),
                    source: Some("registry+https://example.com/index".to_string()),
                    manifest_path: "/registry/registry-1.0.0/Cargo.toml".to_string(),
                    dependencies: Vec::new(),
                    targets: Vec::new(),
                },
            ],
            workspace_members: HashSet::from([
                "member 0.1.0 (path+file:///repo/crates/member)".to_string()
            ]),
            target_directory: root.join_component("target").to_string(),
            metadata: serde_json::json!({ "name": "workspace" }),
        };

        let workspace = workspace_from_metadata(&root, &metadata).unwrap();
        assert!(workspace.has_packages);
        assert_eq!(workspace.crates.len(), 1);
        assert_eq!(workspace.crates[0].name, "member");
    }

    #[test]
    fn crates_register_scoped_tasks() {
        let details = |kind, deliverables| CargoPackageDetails {
            kind,
            deliverables,
            manifest_alters_output_layout: false,
        };
        let deliverable = |name: &str, kind| Deliverable {
            name: name.to_string(),
            kind,
        };
        let verification = ["test", "check", "lint", "format"];

        for entrypoint in [
            details(
                CargoPackageKind::Entrypoint,
                vec![deliverable("ffi", DeliverableKind::Cdylib)],
            ),
            details(
                CargoPackageKind::Entrypoint,
                vec![
                    deliverable("one", DeliverableKind::Bin),
                    deliverable("two", DeliverableKind::Bin),
                ],
            ),
        ] {
            let tasks = registered_tasks(&entrypoint);
            assert_eq!(
                tasks,
                ["build"]
                    .into_iter()
                    .chain(verification)
                    .collect::<Vec<_>>()
            );
        }

        let binary = details(
            CargoPackageKind::Entrypoint,
            vec![deliverable("app", DeliverableKind::Bin)],
        );
        assert_eq!(
            registered_tasks(&binary),
            ["build", "run", "dev"]
                .into_iter()
                .chain(verification)
                .collect::<Vec<_>>()
        );

        let library = details(CargoPackageKind::Library, Vec::new());
        assert_eq!(
            registered_tasks(&library),
            ["build"]
                .into_iter()
                .chain(verification)
                .collect::<Vec<_>>()
        );
    }

    fn tempdir_root() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        // dunce: `cargo metadata` reports plain (non-verbatim) paths on
        // Windows, so the fixture root must be plain too.
        let root = AbsoluteSystemPathBuf::new(
            dunce::canonicalize(tmp.path())
                .unwrap()
                .to_string_lossy()
                .to_string(),
        )
        .unwrap();
        (tmp, root)
    }

    fn write(root: &AbsoluteSystemPathBuf, rel: &[&str], contents: &str) {
        let path = root.join_components(rel);
        std::fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(path.as_std_path(), contents).unwrap();
    }

    fn generate_lockfile(root: &AbsoluteSystemPath) {
        let output = std::process::Command::new("cargo")
            .arg("generate-lockfile")
            .current_dir(root.as_std_path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "failed to generate fixture lockfile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_local_dependency_workspace(
        root: &AbsoluteSystemPathBuf,
        dependency_table: &str,
        exclude_local: bool,
    ) {
        let exclude = if exclude_local {
            "exclude = [\"crates/local\"]\n"
        } else {
            ""
        };
        write(
            root,
            &["Cargo.toml"],
            &format!("[workspace]\nmembers = [\"crates/app\"]\n{exclude}resolver = \"2\"\n"),
        );
        write(
            root,
            &["crates", "app", "Cargo.toml"],
            &format!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
                 \"2021\"\n\n{dependency_table}"
            ),
        );
        write(root, &["crates", "app", "src", "main.rs"], "fn main() {}\n");
        write(
            root,
            &["crates", "local", "Cargo.toml"],
            "[package]\nname = \"local\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(root, &["crates", "local", "src", "lib.rs"], "");
        generate_lockfile(root);
    }

    /// Write a small workspace: `app` (bin) depends on `lib-a` (lib), plus a
    /// dev-dep cycle between `lib-a` and `lib-a-test-util`.
    fn write_fixture_workspace(root: &AbsoluteSystemPathBuf) {
        write(
            root,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \
             \"2\"\n\n[workspace.metadata]\nname = \"fixture-ws\"\n",
        );
        write(
            root,
            &["crates", "app", "Cargo.toml"],
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[dependencies]\nlib-a = { path = \"../lib-a\" }\n",
        );
        write(root, &["crates", "app", "src", "main.rs"], "fn main() {}\n");
        write(
            root,
            &["crates", "lib-a", "Cargo.toml"],
            "[package]\nname = \"lib-a\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[dev-dependencies]\nlib-a-test-util = { path = \"../lib-a-test-util\" }\n",
        );
        write(root, &["crates", "lib-a", "src", "lib.rs"], "");
        write(
            root,
            &["crates", "lib-a-test-util", "Cargo.toml"],
            "[package]\nname = \"lib-a-test-util\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[dependencies]\nlib-a = { path = \"../lib-a\" }\n",
        );
        write(root, &["crates", "lib-a-test-util", "src", "lib.rs"], "");
        // The lockfile must match the manifests exactly: discovery validates
        // it with `cargo metadata --locked` before computing closures.
        write(
            root,
            &["Cargo.lock"],
            r#"version = 4

[[package]]
name = "app"
version = "0.1.0"
dependencies = ["lib-a"]

[[package]]
name = "lib-a"
version = "0.1.0"
dependencies = ["lib-a-test-util"]

[[package]]
name = "lib-a-test-util"
version = "0.1.0"
dependencies = ["lib-a"]
"#,
        );
    }

    #[test]
    fn test_validate_lockfile_rejects_missing_and_stale_files() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);
        let lock_path = root.join_component(CARGO_LOCK);
        let original_lock = lock_path.read_to_string().unwrap();

        validate_lockfile(&root).unwrap();
        assert_eq!(lock_path.read_to_string().unwrap(), original_lock);

        write(
            &root,
            &["crates", "app", "Cargo.toml"],
            "[package]\nname = \"app\"\nversion = \"0.2.0\"\nedition = \
             \"2021\"\n\n[dependencies]\nlib-a = { path = \"../lib-a\" }\n",
        );
        let error = validate_lockfile(&root).unwrap_err();
        assert!(matches!(error, Error::InvalidLockfile { .. }));
        assert_eq!(lock_path.read_to_string().unwrap(), original_lock);

        std::fs::remove_file(lock_path.as_std_path()).unwrap();
        let error = validate_lockfile(&root).unwrap_err();
        assert!(matches!(error, Error::MissingLockfile));
    }

    #[test]
    fn test_validate_lockfile_accepts_automatic_path_member() {
        let (_tmp, root) = tempdir_root();
        write_local_dependency_workspace(
            &root,
            "[dependencies]\nlocal = { path = \"../local\" }\n",
            false,
        );

        validate_lockfile(&root).unwrap();
    }

    #[test]
    fn test_validate_lockfile_rejects_nonmember_path_dependency_kinds() {
        for dependency_table in [
            "[dependencies]\nlocal = { path = \"../local\" }\n",
            "[build-dependencies]\nlocal = { path = \"../local\" }\n",
            "[dev-dependencies]\nlocal = { path = \"../local\" }\n",
            "[target.'cfg(target_os = \"none\")'.dependencies]\nlocal = { path = \"../local\" }\n",
            "[dependencies]\nlocal = { path = \"../local\", optional = true }\n",
        ] {
            let (_tmp, root) = tempdir_root();
            write_local_dependency_workspace(&root, dependency_table, true);

            let error = validate_lockfile(&root).unwrap_err();
            assert!(
                matches!(error, Error::NonMemberLocalPackage { ref name, .. } if name == "local"),
                "unexpected validation result for {dependency_table:?}: {error}"
            );
        }
    }

    #[test]
    fn test_validate_lockfile_rejects_outside_repository_path_dependency() {
        let (_tmp, root) = tempdir_root();
        let repo = root.join_component("repo");
        let outside = root.join_component("outside");
        write(
            &repo,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/app\"]\nresolver = \"2\"\n",
        );
        write(
            &repo,
            &["crates", "app", "Cargo.toml"],
            &format!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
                 \"2021\"\n\n[dependencies]\noutside = {{ path = '{}' }}\n",
                outside.as_str().into_unix()
            ),
        );
        write(
            &repo,
            &["crates", "app", "src", "main.rs"],
            "fn main() {}\n",
        );
        write(
            &outside,
            &["Cargo.toml"],
            "[package]\nname = \"outside\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&outside, &["src", "lib.rs"], "");
        generate_lockfile(&repo);

        let error = validate_lockfile(&repo).unwrap_err();
        assert!(
            matches!(error, Error::OutsideRepositoryLocalPackage { ref name, .. } if name == "outside"),
            "unexpected validation result: {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_accepts_root_package() {
        let (_tmp, root) = tempdir_root();
        write(
            &root,
            &["Cargo.toml"],
            "[package]\nname = \"root-package\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[workspace]\nmembers = []\nresolver = \
             \"2\"\n\n[workspace.metadata]\nname = \"root-workspace\"\n",
        );
        write(&root, &["src", "lib.rs"], "");
        generate_lockfile(&root);

        let discovered = CargoContributor::new(root)
            .discover_packages()
            .await
            .unwrap();
        let names: Vec<_> = discovered
            .packages()
            .iter()
            .filter_map(|package| package.clone().into_parts().name)
            .collect();
        assert_eq!(names, ["root-package", "root-workspace"]);
    }

    fn output_test_workspace(root: &AbsoluteSystemPath) -> CargoWorkspaceDetails {
        CargoWorkspaceDetails {
            target_directory: root.join_component(TARGET_DIR),
            host_target: "x86_64-unknown-linux-gnu".to_string(),
            supported_targets: HashSet::from([
                "x86_64-unknown-linux-gnu".to_string(),
                "aarch64-apple-darwin".to_string(),
                "x86_64-pc-windows-msvc".to_string(),
            ]),
            repository_config_alters_output_layout: false,
            repository_config_untracked: false,
            external_config_present: false,
            manifest_alters_profile_dirs: false,
        }
    }

    fn output_test_package() -> CargoPackageDetails {
        CargoPackageDetails {
            kind: CargoPackageKind::Entrypoint,
            deliverables: vec![Deliverable {
                name: "app".to_string(),
                kind: DeliverableKind::Bin,
            }],
            manifest_alters_output_layout: false,
        }
    }

    #[test]
    fn test_rustup_selection_environment_is_hashed_and_projected() {
        for variable in ["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
            assert!(HASHED_ENV_VARS.contains(&variable));
            assert!(TASK_IO_ENV_VARS.contains(&variable));
        }
        assert!(!HASHED_ENV_VARS.contains(&"RUSTUP_DIST_SERVER"));
        assert!(!TASK_IO_ENV_VARS.contains(&"RUSTUP_UPDATE_ROOT"));
    }

    #[test]
    fn test_cargo_output_arguments_resolve_profiles_and_selectors() {
        for (args, expected) in [
            (vec![], Some("debug")),
            (vec!["--release"], Some("release")),
            (vec!["-r"], Some("release")),
            (vec!["--profile", "dev"], Some("debug")),
            (vec!["--profile=test"], Some("debug")),
            (vec!["--profile=release"], Some("release")),
            (vec!["--profile", "bench"], Some("release")),
            (vec!["--profile=ci"], Some("ci")),
        ] {
            let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(
                cargo_output_arguments(&args)
                    .map(|arguments| arguments.profile)
                    .as_deref(),
                expected
            );
        }
        let selectors = [
            "--target=aarch64-apple-darwin".to_string(),
            "--target-dir".to_string(),
            "build-target".to_string(),
        ];
        assert_eq!(
            cargo_output_arguments(&selectors),
            Some(CargoOutputArguments {
                profile: "debug".to_string(),
                target: Some("aarch64-apple-darwin".to_string()),
                target_directory: Some("build-target".to_string()),
            })
        );
        for args in [
            vec!["--release", "--profile=ci"],
            vec!["--profile=ci", "--profile=dev"],
            vec!["--release", "--release"],
            vec!["--profile=../release"],
            vec!["--target=one", "--target=two"],
            vec!["--target-dir=one", "--target-dir=two"],
        ] {
            let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(cargo_output_arguments(&args), None);
        }
    }

    #[test]
    fn test_cargo_output_arguments_accept_only_known_neutral_flags() {
        let neutral = [
            "--all-features",
            "--features=one,two",
            "-vv",
            "--jobs=2",
            "--message-format=json",
            "--timings=html",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        assert_eq!(
            cargo_output_arguments(&neutral)
                .map(|arguments| arguments.profile)
                .as_deref(),
            Some("debug")
        );
        assert_eq!(
            cargo_output_arguments(&["--future-layout-control".to_string()]),
            None
        );
    }

    #[test]
    fn test_cargo_deliverable_basenames_are_platform_exact() {
        let deliverable = |kind| Deliverable {
            name: "app".to_string(),
            kind,
        };
        for (kind, platform, expected) in [
            (DeliverableKind::Bin, CargoTargetPlatform::Unix, "app"),
            (DeliverableKind::Bin, CargoTargetPlatform::Apple, "app"),
            (
                DeliverableKind::Bin,
                CargoTargetPlatform::WindowsMsvc,
                "app.exe",
            ),
            (
                DeliverableKind::Bin,
                CargoTargetPlatform::WindowsGnu,
                "app.exe",
            ),
            (
                DeliverableKind::Cdylib,
                CargoTargetPlatform::Unix,
                "libapp.so",
            ),
            (
                DeliverableKind::Cdylib,
                CargoTargetPlatform::Apple,
                "libapp.dylib",
            ),
            (
                DeliverableKind::Cdylib,
                CargoTargetPlatform::WindowsMsvc,
                "app.dll",
            ),
            (
                DeliverableKind::Cdylib,
                CargoTargetPlatform::WindowsGnu,
                "app.dll",
            ),
            (
                DeliverableKind::Staticlib,
                CargoTargetPlatform::Unix,
                "libapp.a",
            ),
            (
                DeliverableKind::Staticlib,
                CargoTargetPlatform::Apple,
                "libapp.a",
            ),
            (
                DeliverableKind::Staticlib,
                CargoTargetPlatform::WindowsMsvc,
                "app.lib",
            ),
            (
                DeliverableKind::Staticlib,
                CargoTargetPlatform::WindowsGnu,
                "libapp.a",
            ),
        ] {
            assert_eq!(deliverable_basename(&deliverable(kind), platform), expected);
        }
        assert_eq!(
            target_platform("x86_64-unknown-linux-gnu"),
            Some(CargoTargetPlatform::Unix)
        );
        assert_eq!(
            target_platform("aarch64-apple-darwin"),
            Some(CargoTargetPlatform::Apple)
        );
        assert_eq!(
            target_platform("x86_64-pc-windows-msvc"),
            Some(CargoTargetPlatform::WindowsMsvc)
        );
        assert_eq!(
            target_platform("x86_64-pc-windows-gnu"),
            Some(CargoTargetPlatform::WindowsGnu)
        );
        assert_eq!(target_platform("custom-target.json"), None);
        assert_eq!(target_platform("thumbv7em-none-eabihf"), None);
    }

    #[test]
    fn test_cargo_output_paths_are_exact_and_have_no_wildcards() {
        let outputs = deliverable_output_paths(
            "../../target",
            None,
            "release",
            CargoTargetPlatform::Unix,
            &[Deliverable {
                name: "app".to_string(),
                kind: DeliverableKind::Bin,
            }],
        );
        assert_eq!(outputs, ["../../target/release/app"]);
        assert!(outputs.iter().all(|output| !output.contains('*')));
        let targeted = deliverable_output_paths(
            "../../artifacts",
            Some("x86_64-pc-windows-msvc"),
            "debug",
            CargoTargetPlatform::WindowsMsvc,
            &[Deliverable {
                name: "app".to_string(),
                kind: DeliverableKind::Bin,
            }],
        );
        assert_eq!(
            targeted,
            ["../../artifacts/x86_64-pc-windows-msvc/debug/app.exe"]
        );
        assert!(targeted.iter().all(|output| !output.contains('*')));
    }

    #[test]
    fn test_cargo_output_layout_resolves_target_and_directory_precedence() {
        let (_tmp, root) = tempdir_root();
        let workspace = output_test_workspace(&root);
        let package = output_test_package();
        let empty_environment = toolchain::TaskIOEnvironment::default();
        let default_args = ["--release".to_string()];
        let default_context = toolchain::TaskIOContext {
            task_args: Some(&default_args),
            environment: &empty_environment,
        };
        assert_eq!(
            cargo_output_layout(&root, &workspace, &package, &default_context),
            Some(CargoOutputLayout {
                profile: "release".to_string(),
                target: None,
                target_directory: root.join_component(TARGET_DIR),
            })
        );

        let environment = toolchain::TaskIOEnvironment::new(HashMap::from([
            (
                "CARGO_BUILD_TARGET".to_string(),
                "aarch64-apple-darwin".to_string(),
            ),
            ("CARGO_TARGET_DIR".to_string(), "env-target".to_string()),
        ]));
        let environment_context = toolchain::TaskIOContext {
            task_args: None,
            environment: &environment,
        };
        assert_eq!(
            cargo_output_layout(&root, &workspace, &package, &environment_context),
            Some(CargoOutputLayout {
                profile: "debug".to_string(),
                target: Some("aarch64-apple-darwin".to_string()),
                target_directory: root.join_component("env-target"),
            })
        );

        let cli_args = [
            "--release".to_string(),
            "--target=x86_64-pc-windows-msvc".to_string(),
            "--target-dir=cli-target".to_string(),
        ];
        let cli_context = toolchain::TaskIOContext {
            task_args: Some(&cli_args),
            environment: &environment,
        };
        assert_eq!(
            cargo_output_layout(&root, &workspace, &package, &cli_context),
            Some(CargoOutputLayout {
                profile: "release".to_string(),
                target: Some("x86_64-pc-windows-msvc".to_string()),
                target_directory: root.join_component("cli-target"),
            })
        );
    }

    #[test]
    fn test_cargo_output_layout_fails_closed_for_unsupported_controls() {
        let (_tmp, root) = tempdir_root();
        let workspace = output_test_workspace(&root);
        let package = output_test_package();
        let empty_environment = toolchain::TaskIOEnvironment::default();

        for name in [
            "CARGO_BUILD_TARGET_DIR",
            "CARGO_BUILD_ARTIFACT_DIR",
            "RUSTC",
            "CARGO_BUILD_RUSTC",
            "CARGO_PROFILE_CI_DIR_NAME",
        ] {
            let environment = toolchain::TaskIOEnvironment::new(HashMap::from([(
                name.to_string(),
                "configured".to_string(),
            )]));
            let context = toolchain::TaskIOContext {
                task_args: None,
                environment: &environment,
            };
            assert_eq!(
                cargo_output_layout(&root, &workspace, &package, &context),
                None
            );
        }

        for args in [
            vec!["--target=thumbv7em-none-eabihf".to_string()],
            vec!["--target=custom-target.json".to_string()],
            vec!["--target-dir=../outside".to_string()],
            vec!["--config=build.target-dir='other'".to_string()],
            vec!["--future-layout-control".to_string()],
        ] {
            let context = toolchain::TaskIOContext {
                task_args: Some(&args),
                environment: &empty_environment,
            };
            assert_eq!(
                cargo_output_layout(&root, &workspace, &package, &context),
                None
            );
        }

        let mut unsafe_workspace = workspace.clone();
        unsafe_workspace.repository_config_untracked = true;
        let context = toolchain::TaskIOContext {
            task_args: None,
            environment: &empty_environment,
        };
        assert_eq!(
            cargo_output_layout(&root, &unsafe_workspace, &package, &context),
            None
        );
    }

    #[test]
    fn test_target_directory_containment_rejects_absolute_and_lexical_escapes() {
        let (_tmp, root) = tempdir_root();
        assert!(target_directory_within_repo(
            &root,
            &root.join_components(&["new", "target"])
        ));

        let outside = tempfile::tempdir().unwrap();
        let outside_path = AbsoluteSystemPathBuf::new(
            dunce::canonicalize(outside.path())
                .unwrap()
                .to_string_lossy()
                .to_string(),
        )
        .unwrap();
        assert!(!target_directory_within_repo(&root, &outside_path));

        #[cfg(windows)]
        {
            let other_drive = if root
                .as_str()
                .get(..2)
                .is_some_and(|drive| drive.eq_ignore_ascii_case("C:"))
            {
                "D:"
            } else {
                "C:"
            };
            let other_root = AbsoluteSystemPathBuf::new(format!(r"{other_drive}\outside")).unwrap();
            assert!(!target_directory_within_repo(&root, &other_root));
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_target_directory_containment_rejects_symlink_escapes() {
        let (_tmp, root) = tempdir_root();
        let outside = tempfile::tempdir().unwrap();
        let escape = root.join_component("escape");
        std::os::unix::fs::symlink(outside.path(), escape.as_std_path()).unwrap();
        assert!(!target_directory_within_repo(
            &root,
            &escape.join_component("target")
        ));

        let contained = root.join_component("contained");
        std::fs::create_dir_all(contained.as_std_path()).unwrap();
        let link = root.join_component("contained-link");
        std::os::unix::fs::symlink(contained.as_std_path(), link.as_std_path()).unwrap();
        assert!(target_directory_within_repo(
            &root,
            &link.join_component("target")
        ));
    }

    #[test]
    fn test_manifest_layout_controls_are_detected() {
        let (_tmp, root) = tempdir_root();
        let manifest = root.join_component(CARGO_TOML);
        for contents in [
            "cargo-features = [\"different-binary-name\"]\n\n[[bin]]\nname = \"app\"\nfilename = \
             \"renamed\"\n",
            "cargo-features = [\"per-package-target\"]\n\n[package]\nname = \"app\"\nversion = \
             \"0.1.0\"\n",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\ndefault-target = \"host\"\n",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nforced-target = \"host\"\n",
        ] {
            write(&root, &[CARGO_TOML], contents);
            assert!(manifest_alters_output_layout(&manifest));
        }
        write(
            &root,
            &[CARGO_TOML],
            "[workspace]\nmembers = []\n\n[profile.ci]\ninherits = \"dev\"\ndir-name = \
             \"ci-output\"\n",
        );
        assert!(manifest_alters_profile_dirs(&root));
    }

    #[test]
    fn test_repository_and_external_config_influence_is_detected() {
        let (_tmp, root) = tempdir_root();
        let repo = root.join_component("repo");
        std::fs::create_dir_all(repo.as_std_path()).unwrap();
        write(
            &repo,
            &[".cargo", "config.toml"],
            "[build]\ntarget-dir = \"configured-target\"\n",
        );
        assert!(
            !cargo_config_influence(&repo, &CargoHomeEnvironment::default())
                .repository_alters_output_layout
        );
        write(
            &repo,
            &[".cargo", "config.toml"],
            "[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n",
        );
        assert!(
            cargo_config_influence(&repo, &CargoHomeEnvironment::default())
                .repository_alters_output_layout
        );
        write(
            &repo,
            &[".cargo", "config.toml"],
            "include = \"other-config.toml\"\n",
        );
        assert!(
            cargo_config_influence(&repo, &CargoHomeEnvironment::default())
                .repository_config_untracked
        );
        write(&root, &[".cargo", "config.toml"], "[net]\nretry = 2\n");
        assert!(cargo_config_influence(&repo, &CargoHomeEnvironment::default()).external_present);
    }

    #[cfg(unix)]
    #[test]
    fn test_escaping_repository_config_is_detected() {
        let (_tmp, root) = tempdir_root();
        let repo = root.join_component("repo");
        std::fs::create_dir_all(repo.join_component(".cargo").as_std_path()).unwrap();
        let outside = root.join_component("outside.toml");
        outside
            .create_with_contents("[build]\ntarget = \"host\"\n")
            .unwrap();
        std::os::unix::fs::symlink(
            outside.as_std_path(),
            repo.join_components(&[".cargo", "config.toml"])
                .as_std_path(),
        )
        .unwrap();
        let influence = cargo_config_influence(&repo, &CargoHomeEnvironment::default());
        assert!(influence.repository_alters_output_layout);
        assert!(influence.repository_config_untracked);
    }

    #[cfg(unix)]
    #[test]
    fn test_internal_repository_config_symlink_is_untracked() {
        let (_tmp, root) = tempdir_root();
        let repo = root.join_component("repo");
        std::fs::create_dir_all(repo.join_component(".cargo").as_std_path()).unwrap();
        let target = repo.join_component("cargo-config.toml");
        target.create_with_contents("[net]\nretry = 2\n").unwrap();
        std::os::unix::fs::symlink(
            target.as_std_path(),
            repo.join_components(&[".cargo", "config.toml"])
                .as_std_path(),
        )
        .unwrap();

        let influence = cargo_config_influence(&repo, &CargoHomeEnvironment::default());
        assert!(!influence.repository_alters_output_layout);
        assert!(influence.repository_config_untracked);
    }

    #[cfg(unix)]
    #[test]
    fn test_config_beneath_symlinked_cargo_directory_is_untracked() {
        let (_tmp, root) = tempdir_root();
        let repo = root.join_component("repo");
        std::fs::create_dir_all(repo.as_std_path()).unwrap();
        let cargo_target = repo.join_component("cargo-config");
        std::fs::create_dir_all(cargo_target.as_std_path()).unwrap();
        cargo_target
            .join_component("config.toml")
            .create_with_contents("[net]\nretry = 2\n")
            .unwrap();
        std::os::unix::fs::symlink(
            cargo_target.as_std_path(),
            repo.join_component(".cargo").as_std_path(),
        )
        .unwrap();

        let influence = cargo_config_influence(&repo, &CargoHomeEnvironment::default());
        assert!(!influence.repository_alters_output_layout);
        assert!(influence.repository_config_untracked);
    }

    #[cfg(unix)]
    #[test]
    fn test_non_utf8_cargo_home_path_is_preserved() {
        use std::os::unix::ffi::OsStringExt;

        let (_tmp, root) = tempdir_root();
        let relative = std::ffi::OsString::from_vec(b"cargo-\xff".to_vec());
        let environment = CargoHomeEnvironment {
            cargo_home: Some(relative.clone()),
            ..Default::default()
        };
        assert_eq!(
            cargo_home_candidates(&root, &environment, false),
            [root.as_std_path().join(relative)]
        );
    }

    #[test]
    fn test_parse_rustc_identity_includes_host() {
        let identity = parse_rustc_identity(
            b"rustc 1.96.0-nightly (f5eca4fcf 2026-04-09)\n\
binary: rustc\n\
commit-hash: f5eca4fcf\n\
host: aarch64-apple-darwin\n\
release: 1.96.0-nightly\n",
        )
        .unwrap();

        assert_eq!(identity.key, "rustc");
        assert_eq!(
            identity.version,
            concat!(
                "rustc 1.96.0-nightly (f5eca4fcf 2026-04-09)\n",
                "binary: rustc\n",
                "commit-hash: f5eca4fcf\n",
                "host: aarch64-apple-darwin\n",
                "release: 1.96.0-nightly"
            )
        );
    }

    #[test]
    fn test_parse_rustc_identity_changes_with_host() {
        let macos =
            parse_rustc_identity(b"rustc 1.85.0 (abc 2025-01-01)\nhost: x86_64-apple-darwin\n")
                .unwrap();
        let linux = parse_rustc_identity(
            b"rustc 1.85.0 (abc 2025-01-01)\nhost: x86_64-unknown-linux-gnu\n",
        )
        .unwrap();

        assert_ne!(macos, linux);
    }

    #[test]
    fn test_parse_rustc_identity_requires_host() {
        let error =
            parse_rustc_identity(b"rustc 1.85.0 (abc 2025-01-01)\nrelease: 1.85.0\n").unwrap_err();

        assert!(matches!(
            error,
            Error::InvalidRustcOutput {
                reason: "missing host triple"
            }
        ));
    }

    #[test]
    fn test_discover_crates_via_metadata() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let mut crates = discover_crates(&root).unwrap().crates;
        crates.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(
            crates.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["app", "lib-a", "lib-a-test-util"]
        );

        let app = &crates[0];
        assert!(app.is_entrypoint(), "bin crate should be an entrypoint");
        assert_eq!(
            app.deliverables,
            vec![Deliverable {
                name: "app".to_string(),
                kind: DeliverableKind::Bin,
            }]
        );
        assert_eq!(
            app.relationships,
            vec![Relationship::internal("lib-a", DependencyKind::Production)]
        );

        let lib_a = &crates[1];
        assert!(
            !lib_a.is_entrypoint(),
            "plain lib crate is not an entrypoint"
        );
        assert!(lib_a.deliverables.is_empty());
        // The dev-dep edge lib-a -> lib-a-test-util closes a cycle with the
        // normal edge lib-a-test-util -> lib-a, so it remains an input without
        // ordering tasks.
        assert_eq!(
            lib_a.relationships,
            vec![Relationship::internal_input(
                "lib-a-test-util",
                DependencyKind::Development
            )]
        );

        let test_util = &crates[2];
        assert_eq!(
            test_util.relationships,
            vec![Relationship::internal("lib-a", DependencyKind::Production)]
        );
    }

    #[test]
    fn test_discover_crates_preserves_relationship_kinds() {
        let (_tmp, root) = tempdir_root();
        write(
            &root,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
        );
        write(
            &root,
            &["crates", "app", "Cargo.toml"],
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[dependencies]\noptional-lib = { path = \"../optional-lib\", optional = \
             true }\n\n[build-dependencies]\nbuild-lib = { path = \"../build-lib\" \
             }\n\n[target.'cfg(target_os = \"none\")'.dependencies]\ntarget-lib = { path = \
             \"../target-lib\" }\n",
        );
        write(&root, &["crates", "app", "src", "lib.rs"], "");
        for name in ["optional-lib", "build-lib", "target-lib"] {
            write(
                &root,
                &["crates", name, "Cargo.toml"],
                &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            );
            write(&root, &["crates", name, "src", "lib.rs"], "");
        }

        let workspace = discover_crates(&root).unwrap();
        let app = workspace
            .crates
            .iter()
            .find(|cargo_crate| cargo_crate.name == "app")
            .unwrap();

        assert_eq!(
            app.relationships,
            vec![
                Relationship::internal("build-lib", DependencyKind::Production),
                Relationship::internal("optional-lib", DependencyKind::Optional),
                Relationship::internal("target-lib", DependencyKind::Production),
            ]
        );
    }

    #[test]
    fn test_discover_crates_not_a_workspace() {
        let (_tmp, root) = tempdir_root();
        let workspace = discover_crates(&root).unwrap();
        assert!(workspace.crates.is_empty());
        assert!(workspace.name.is_none());
    }

    #[test]
    fn test_workspace_name_discovered_and_validated() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let workspace = discover_crates(&root).unwrap();
        assert_eq!(workspace.name.as_deref(), Some("fixture-ws"));

        // A name colliding with a crate is an error naming the crate's
        // location, not a silent skip.
        write(
            &root,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \
             \"2\"\n\n[workspace.metadata]\nname = \"lib-a\"\n",
        );
        let err = discover_crates(&root).unwrap_err();
        assert!(
            matches!(err, Error::WorkspaceNameCollision { ref name, .. } if name == "lib-a"),
            "expected collision error, got: {err}"
        );

        // Shape rules match crate names: `#` can never appear in a task key.
        write(
            &root,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \
             \"2\"\n\n[workspace.metadata]\nname = \"bad#name\"\n",
        );
        assert!(matches!(
            discover_crates(&root).unwrap_err(),
            Error::InvalidWorkspaceName { .. }
        ));

        // A non-string name is rejected rather than coerced.
        write(
            &root,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \
             \"2\"\n\n[workspace.metadata]\nname = 42\n",
        );
        assert!(matches!(
            discover_crates(&root).unwrap_err(),
            Error::InvalidWorkspaceName { .. }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_missing_workspace_name_is_an_error() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);
        // Remove the name: crates exist, so the workspace package would be
        // synthesized — and every package must have a name.
        write(
            &root,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
        );

        let toolchain = CargoContributor::new(root.clone());
        let err = toolchain.discover_packages().await.unwrap_err();
        assert!(
            err.to_string().contains("[workspace.metadata]"),
            "the error must show the fix, got: {err}"
        );

        std::fs::remove_file(root.join_component(CARGO_LOCK)).unwrap();
        let err = CargoContributor::new(root.clone())
            .discover_packages()
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("[workspace.metadata]"),
            "workspace naming must be validated before the lockfile, got: {err}"
        );

        // Crate discovery itself still works: the name is only mandatory
        // for package synthesis.
        assert_eq!(discover_crates(&root).unwrap().crates.len(), 3);
    }

    #[test]
    fn test_discover_crates_malformed_root_errors() {
        let (_tmp, root) = tempdir_root();
        write(&root, &["Cargo.toml"], "[workspace\nmembers = [");
        assert!(
            discover_crates(&root).is_err(),
            "a broken root manifest should surface an error, not silently discover nothing"
        );
    }

    #[test]
    fn test_discover_crates_includes_root_crate() {
        let (_tmp, root) = tempdir_root();
        write(
            &root,
            &["Cargo.toml"],
            "[package]\nname = \"root-crate\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[workspace]\nmembers = [\"crates/*\"]\n",
        );
        write(&root, &["src", "lib.rs"], "");
        write(
            &root,
            &["crates", "member", "Cargo.toml"],
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&root, &["crates", "member", "src", "lib.rs"], "");

        let crates = discover_crates(&root).unwrap().crates;
        assert_eq!(
            crates.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["member", "root-crate"]
        );
    }

    #[test]
    fn test_discover_crates_auto_includes_path_dependency_members() {
        // `tools/helper` matches no `members` glob but is a path dependency
        // of a member; Cargo treats it as an automatic workspace member and
        // so must we (via `cargo metadata`).
        let (_tmp, root) = tempdir_root();
        write(
            &root,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
        );
        write(
            &root,
            &["crates", "app", "Cargo.toml"],
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[dependencies]\nhelper = { path = \"../../tools/helper\" }\n",
        );
        write(&root, &["crates", "app", "src", "lib.rs"], "");
        write(
            &root,
            &["tools", "helper", "Cargo.toml"],
            "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&root, &["tools", "helper", "src", "lib.rs"], "");

        let mut crates = discover_crates(&root).unwrap().crates;
        crates.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            crates.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["app", "helper"]
        );
        assert_eq!(
            crates[0].relationships,
            vec![Relationship::internal("helper", DependencyKind::Production)]
        );
    }

    #[test]
    fn test_compile_cache_env_routes_rustc_through_sccache() {
        let endpoint = toolchain::CompileCacheEndpoint {
            url: "http://127.0.0.1:42123".to_string(),
            token: "proxy-token".to_string(),
            wrapper: "/path/to/turbo".to_string(),
            server_port: 46123,
        };
        assert_eq!(
            cargo_compile_cache_env(&endpoint, &std::collections::HashMap::new()),
            vec![
                ("RUSTC_WRAPPER".to_string(), "/path/to/turbo".to_string()),
                ("TURBO_SCCACHE_WRAPPER".to_string(), "1".to_string()),
                (
                    "SCCACHE_WEBDAV_ENDPOINT".to_string(),
                    "http://127.0.0.1:42123".to_string()
                ),
                (
                    "SCCACHE_WEBDAV_TOKEN".to_string(),
                    "proxy-token".to_string()
                ),
                ("SCCACHE_SERVER_PORT".to_string(), "46123".to_string()),
                (
                    "SCCACHE_IGNORE_SERVER_IO_ERROR".to_string(),
                    "1".to_string()
                ),
                ("CARGO_INCREMENTAL".to_string(), "0".to_string()),
            ]
        );
        // The injected wrapper key must be a hashed env var so a
        // user-supplied wrapper invalidates task results (the injected one
        // is execution-only and deliberately does not).
        assert!(HASHED_ENV_VARS.contains(&"RUSTC_WRAPPER"));
    }

    #[test]
    fn test_compile_cache_env_stands_down_for_competing_configuration() {
        let endpoint = toolchain::CompileCacheEndpoint {
            url: "http://127.0.0.1:42123".to_string(),
            token: "proxy-token".to_string(),
            wrapper: "/path/to/turbo".to_string(),
            server_port: 46123,
        };

        // A user-supplied wrapper wins; injecting SCCACHE_* on top of it
        // could hijack its backend, so nothing is injected.
        let env = std::collections::HashMap::from([(
            "RUSTC_WRAPPER".to_string(),
            "/home/user/bin/my-wrapper".to_string(),
        )]);
        assert!(cargo_compile_cache_env(&endpoint, &env).is_empty());

        // Any SCCACHE_* variable signals a user-managed sccache setup.
        let env = std::collections::HashMap::from([(
            "SCCACHE_GHA_ENABLED".to_string(),
            "true".to_string(),
        )]);
        assert!(cargo_compile_cache_env(&endpoint, &env).is_empty());
    }

    #[test]
    fn test_compile_cache_env_tolerates_ambient_cargo_incremental() {
        // CI images commonly export CARGO_INCREMENTAL=0 (this repository's
        // own setup-environment action does). That is ambient hygiene, not
        // a competing compiler cache: the injection proceeds and the
        // explicit value is left alone.
        let endpoint = toolchain::CompileCacheEndpoint {
            url: "http://127.0.0.1:42123".to_string(),
            token: "proxy-token".to_string(),
            wrapper: "/path/to/turbo".to_string(),
            server_port: 46123,
        };
        let env =
            std::collections::HashMap::from([("CARGO_INCREMENTAL".to_string(), "0".to_string())]);

        let vars = cargo_compile_cache_env(&endpoint, &env);
        assert!(
            vars.iter().any(|(key, _)| key == "RUSTC_WRAPPER"),
            "injection must proceed despite ambient CARGO_INCREMENTAL=0"
        );
        assert!(
            !vars.iter().any(|(key, _)| key == "CARGO_INCREMENTAL"),
            "an explicit CARGO_INCREMENTAL must not be overridden"
        );

        // Any other explicit value means incremental compilation was
        // deliberately requested — incompatible with sccache, whose wrapper
        // hard-exits on CARGO_INCREMENTAL=1. Stand down entirely.
        let env =
            std::collections::HashMap::from([("CARGO_INCREMENTAL".to_string(), "1".to_string())]);
        assert!(cargo_compile_cache_env(&endpoint, &env).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_emits_native_relationships_without_dependency_descriptors() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let toolchain = CargoContributor::new(root.clone());
        assert_eq!(toolchain.id(), ToolchainId::RUST);

        let (packages, roots, resolutions, changes, prune_domains) =
            toolchain.discover_packages().await.unwrap().into_parts();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].kind(), "cargo");
        assert_eq!(roots[0].path(), root.as_ref());
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].toolchain(), &ToolchainId::RUST);
        assert_eq!(changes.len(), 1);
        assert_eq!(prune_domains.len(), 1);
        let prune_plan = prune_domains[0]
            .plan(&["app".to_string()])
            .unwrap()
            .expect("a retained Cargo crate produces a prune plan");
        assert_eq!(
            prune_plan
                .root_files
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            [CARGO_LOCK, CARGO_TOML]
        );
        assert!(
            prune_plan
                .copy_paths
                .iter()
                .any(|path| path == ".cargo/config")
        );
        assert_eq!(resolutions[0].definition_sources()[0].as_str(), CARGO_LOCK);
        let ExternalResolutionData::Resolved {
            completeness,
            packages: resolution_packages,
        } = resolutions[0].data()
        else {
            panic!("Cargo resolution must be complete")
        };
        assert_eq!(completeness, &ResolutionCompleteness::Complete);
        assert_eq!(resolution_packages.len(), 4);
        // Producers supply normalized identities; generation construction owns
        // byte-compatible package fingerprints.
        assert!(
            resolution_packages
                .iter()
                .all(|package| package.fingerprint().is_none())
        );
        assert!(resolution_packages.iter().all(|package| {
            package
                .identities()
                .iter()
                .any(|identity| identity.key() == "rustc")
        }));
        assert!(
            resolution_packages
                .iter()
                .all(|package| package.identities().len() == 1),
            "the all-local fixture should expose only compiler identities"
        );
        let mut packages: Vec<_> = packages
            .into_iter()
            .map(DiscoveredPackage::into_parts)
            .collect();
        packages.sort_by(|a, b| a.name.cmp(&b.name));

        let names: Vec<&str> = packages
            .iter()
            .map(|package| package.name.as_deref().unwrap())
            .collect();
        assert_eq!(names, vec!["app", "fixture-ws", "lib-a", "lib-a-test-util"]);

        let app = &packages[0];
        assert!(app.descriptor.dependencies.is_none());
        assert!(app.descriptor.dev_dependencies.is_none());
        let compile_cache_env = app
            .task_contract
            .as_ref()
            .expect("Cargo discovery contributes a task contract")
            .compile_cache_env(
                &toolchain::CompileCacheEndpoint {
                    url: "http://127.0.0.1:42123".to_string(),
                    token: "proxy-token".to_string(),
                    wrapper: "/path/to/turbo".to_string(),
                    server_port: 46123,
                },
                &std::collections::HashMap::new(),
            );
        assert!(
            compile_cache_env
                .iter()
                .any(|(key, _)| key == "RUSTC_WRAPPER")
        );
        assert_eq!(
            app.native_relationships.as_deref(),
            Some(&[Relationship::internal("lib-a", DependencyKind::Production)][..])
        );
        assert_eq!(
            app.manifest_path,
            root.join_components(&["crates", "app", "Cargo.toml"])
        );

        // The workspace aggregate is anchored at the root manifest
        // and depends on every crate.
        let workspace = &packages[1];
        assert_eq!(workspace.manifest_path, root.join_component(CARGO_TOML));
        assert!(workspace.descriptor.dependencies.is_none());
        let workspace_relationships = workspace.native_relationships.as_ref().unwrap();
        assert_eq!(workspace_relationships.len(), 3);
        assert_eq!(
            workspace_relationships,
            &[
                Relationship::internal("app", DependencyKind::Production),
                Relationship::internal("lib-a", DependencyKind::Production),
                Relationship::internal("lib-a-test-util", DependencyKind::Production),
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_empty_without_manifest() {
        let (_tmp, root) = tempdir_root();
        let toolchain = CargoContributor::new(root);
        let (packages, roots, resolutions, changes, prune_domains) =
            toolchain.discover_packages().await.unwrap().into_parts();
        assert!(packages.is_empty());
        assert!(roots.is_empty());
        assert!(resolutions.is_empty());
        assert!(changes.is_empty());
        assert!(prune_domains.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_empty_for_memberless_workspace() {
        let (_tmp, root) = tempdir_root();
        write(&root, &["Cargo.toml"], "[workspace]\nmembers = []\n");

        let toolchain = CargoContributor::new(root);
        let (packages, roots, resolutions, changes, prune_domains) =
            toolchain.discover_packages().await.unwrap().into_parts();
        assert!(packages.is_empty());
        assert_eq!(roots.len(), 1);
        assert!(resolutions.is_empty());
        assert!(changes.is_empty());
        assert!(prune_domains.is_empty());
    }

    #[rustfmt::skip]
    fn task_context<'a>(
        _toolchain: &CargoContributor,
        root: &'a AbsoluteSystemPath,
        name: &str,
        directory: &'a str,
    ) -> crate::package_graph::PackageTaskContext<'a> {
        let kind = if directory.is_empty() {
            crate::package_graph::PackageTaskContextKind::Aggregate
        } else {
            crate::package_graph::PackageTaskContextKind::Package
        };
        let cargo_kind = if directory.is_empty() {
            CargoPackageKind::Workspace
        } else if name == "app" {
            CargoPackageKind::Entrypoint
        } else {
            CargoPackageKind::Library
        };
        let deliverables = if cargo_kind == CargoPackageKind::Entrypoint {
            vec![Deliverable {
                name: name.to_string(),
                kind: DeliverableKind::Bin,
            }]
        } else {
            Vec::new()
        };
        let details = CargoPackageDetails {
            kind: cargo_kind,
            deliverables,
            manifest_alters_output_layout: false,
        };
        let native_tasks = Some(native_tasks_for_package(&details, name));
        let task_contract = (kind == crate::package_graph::PackageTaskContextKind::Package).then(|| {
            crate::task_contracts::ScopeTaskContract::derived(
                ToolchainId::RUST,
                None,
                std::collections::BTreeMap::new(),
                std::collections::BTreeMap::new(),
            )
            .with_dependency_source_inputs(
                crate::task_contracts::DependencySourceInputs::Include,
            )
        });
        crate::package_graph::PackageTaskContext::new_for_test_with_native_tasks(
            name.into(),
            root,
            turbopath::AnchoredSystemPath::new(directory).unwrap(),
            kind,
            Some(&ToolchainId::RUST),
            native_tasks,
            task_contract,
        )
    }

    fn os_args(args: &[&str]) -> Vec<std::ffi::OsString> {
        args.iter().map(std::ffi::OsString::from).collect()
    }

    fn resolve_cargo_cmd(
        context: &crate::package_graph::PackageTaskContext<'_>,
        task: &str,
        pass_through_args: Option<&[String]>,
        override_command: Option<&[String]>,
    ) -> Option<crate::toolchain::TaskCommand> {
        let cargo_binary = override_command
            .is_none()
            .then(|| which::which("cargo").ok())
            .flatten();
        if let Some(native_task) = context.native_tasks().get(task) {
            return crate::native_tasks::resolve_task_command(
                context,
                native_task,
                None,
                None,
                cargo_binary.as_deref(),
                pass_through_args,
                override_command,
            )
            .unwrap();
        }
        let override_command = override_command?;
        let serial_group = (override_command.first().map(String::as_str) == Some("cargo"))
            .then(|| "cargo".to_string());
        crate::toolchain::override_task_command(
            context,
            override_command,
            pass_through_args,
            serial_group,
        )
    }

    #[rustfmt::skip]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_task_commands() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let toolchain = CargoContributor::new(root.clone());
        let app_context = task_context(&toolchain, &root, "app", "crates/app");
        let lib_a_context = task_context(&toolchain, &root, "lib-a", "crates/lib-a");
        let workspace_context = task_context(&toolchain, &root, "fixture-ws", "");

        // Entrypoint build: scoped to the crate, serialized on the cargo
        // group, run from the workspace root.
        let cmd = resolve_cargo_cmd(&app_context, "build", None, None)
            .expect("entrypoint build resolves");
        assert_eq!(cmd.args, os_args(&["build", "--package=app", "--locked"]));
        assert_eq!(cmd.cwd, root);
        assert_eq!(cmd.serial_group.as_deref(), Some("cargo"));

        // `run` is exempt from the serial group and forwards pass-through
        // args to the binary after `--`.
        let cmd = resolve_cargo_cmd(&app_context, "dev", Some(&["--port".to_string()]), None).expect("entrypoint dev resolves to cargo run");
        assert_eq!(
            cmd.args,
            os_args(&["run", "--package=app", "--locked", "--", "--port"])
        );
        assert_eq!(cmd.serial_group, None);

        // Other subcommands attach pass-through args as cargo flags, no
        // separator.
        let cmd = resolve_cargo_cmd(
                &app_context,
                "build",
                Some(&["--release".to_string()]),
                None,
            ).expect("entrypoint build resolves");
        assert_eq!(
            cmd.args,
            os_args(&["build", "--package=app", "--locked", "--release"])
        );

        // A filtered library build resolves directly to that package.
        let cmd = resolve_cargo_cmd(&lib_a_context, "build", None, None)
            .expect("library build resolves");
        assert_eq!(cmd.args, os_args(&["build", "--package=lib-a", "--locked"]));
        let cmd = resolve_cargo_cmd(&lib_a_context, "test", None, None)
            .expect("library test resolves");
        assert_eq!(cmd.args, os_args(&["test", "--package=lib-a", "--locked"]));
        let cmd = resolve_cargo_cmd(&app_context, "check", None, None)
            .expect("entrypoint check resolves");
        assert_eq!(cmd.args, os_args(&["check", "--package=app", "--locked"]));
        let cmd = resolve_cargo_cmd(
                &lib_a_context,
                "format",
                Some(&["--check".to_string()]),
                None,
            ).expect("library format resolves");
        assert_eq!(cmd.args, os_args(&["fmt", "--package=lib-a", "--", "--check"]));
        assert_eq!(cmd.serial_group, None);

        // The workspace package runs verification verbs at workspace scope.
        let cmd = resolve_cargo_cmd(&workspace_context, "lint", None, None)
            .expect("workspace lint resolves to clippy");
        assert_eq!(cmd.args, os_args(&["clippy", "--workspace", "--locked"]));
        assert_eq!(cmd.serial_group.as_deref(), Some("cargo"));
        let cmd = resolve_cargo_cmd(&workspace_context, "format", None, None)
            .expect("workspace format resolves");
        assert_eq!(cmd.args, os_args(&["fmt", "--all"]));
        assert_eq!(cmd.serial_group, None);

        // Harness-forwarding subcommands separate pass-through args with
        // `--`; e.g. `turbo test -- --nocapture` reaches the test harness.
        let cmd = resolve_cargo_cmd(
                &workspace_context,
                "test",
                Some(&["--nocapture".to_string()]),
                None,
            ).expect("workspace test resolves");
        assert_eq!(
            cmd.args,
            os_args(&["test", "--workspace", "--locked", "--", "--nocapture"])
        );
        assert!(
            resolve_cargo_cmd(&workspace_context, "build", None, None)
                .is_none(),
            "workspace-wide build would duplicate entrypoint builds"
        );

        // Display strings derive from the same tables.
        assert_eq!(app_context.native_tasks().get("build").and_then(|t| t.display()), Some("cargo build --package=app --locked"));
        assert_eq!(workspace_context.native_tasks().get("test").and_then(|t| t.display()), Some("cargo test --workspace --locked"));
        assert_eq!(lib_a_context.native_tasks().get("test").and_then(|t| t.display()), Some("cargo test --package=lib-a --locked"));
        assert_eq!(workspace_context.native_tasks().get("format").and_then(|t| t.display()), Some("cargo fmt --all"));
        assert_eq!(lib_a_context.native_tasks().get("format").and_then(|t| t.display()), Some("cargo fmt --package=lib-a"));
        assert_eq!(
            lib_a_context.native_tasks().get("build").and_then(|t| t.display()),
            Some("cargo build --package=lib-a --locked")
        );

        let app_build = app_context.native_tasks().get("build").unwrap().contract();
        let library_build = lib_a_context.native_tasks().get("build").unwrap().contract();
        let workspace_build = workspace_context.native_tasks().get("build").unwrap().contract();
        assert_eq!(app_context.native_tasks().get("run").unwrap().contract().defaults().cache, Some(false));
        assert_eq!(app_context.native_tasks().get("dev").unwrap().contract().defaults().cache, Some(false));
        assert_eq!(app_build.defaults().cache, None);
        assert_eq!(workspace_context.native_tasks().get("test").unwrap().contract().defaults().cache, None);
        assert_eq!(lib_a_context.native_tasks().get("test").unwrap().contract().defaults().cache, None);
        assert_eq!(workspace_context.native_tasks().get("format").unwrap().contract().defaults().cache, Some(false));
        assert_eq!(lib_a_context.native_tasks().get("format").unwrap().contract().defaults().cache, Some(false));
        assert_eq!(library_build.defaults().cache, Some(false));
        assert!(app_build.derives_io());
        assert_eq!(
            app_context
                .native_tasks()
                .override_serial_group(&["cargo".to_string(), "fuzz".to_string()]),
            Some("cargo".to_string())
        );
        assert_eq!(
            app_context
                .native_tasks()
                .override_serial_group(&["./script".to_string()]),
            None
        );

        assert_eq!(
            app_build.entrypoint(),
            Some(crate::task_contracts::TaskEntrypoint::Preferred)
        );
        assert_eq!(
            library_build.entrypoint(),
            Some(crate::task_contracts::TaskEntrypoint::Candidate)
        );
        assert_eq!(
            workspace_build.entrypoint(),
            Some(crate::task_contracts::TaskEntrypoint::Excluded)
        );
        assert_eq!(
            workspace_context.native_tasks().get("test").unwrap().contract().entrypoint(),
            Some(crate::task_contracts::TaskEntrypoint::PreferredOnly)
        );
        assert_eq!(
            workspace_context.native_tasks().get("format").unwrap().contract().entrypoint(),
            Some(crate::task_contracts::TaskEntrypoint::PreferredOnly)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_command_override_frame() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let toolchain = CargoContributor::new(root.clone());
        toolchain.discover_packages().await.unwrap();

        let lib_a_context = task_context(&toolchain, &root, "lib-a", "crates/lib-a");
        let workspace_context = task_context(&toolchain, &root, "fixture-ws", "");

        // An override applies to any crate and any task. cwd is the package's
        // directory, and an argv still invoking cargo keeps the serial group
        // (the group
        // exists because of cargo's build-directory lock).
        let override_argv = vec!["cargo".to_string(), "fuzz".to_string(), "run".to_string()];
        let cmd = resolve_cargo_cmd(&lib_a_context, "fuzz", None, Some(&override_argv))
            .expect("override defines the task for a library crate");
        assert_eq!(cmd.program, std::ffi::OsString::from("cargo"));
        assert_eq!(cmd.args, os_args(&["fuzz", "run"]));
        assert_eq!(cmd.cwd, root.join_components(&["crates", "lib-a"]));
        assert_eq!(cmd.serial_group.as_deref(), Some("cargo"));

        // A non-cargo argv drops the group; pass-through args append
        // verbatim (no separator injection). Overrides must not require the
        // cargo binary, even for tasks present in the native catalog.
        let override_argv = vec!["./scripts/test.sh".to_string()];
        let cmd = resolve_cargo_cmd(
            &workspace_context,
            "test",
            Some(&["--fast".to_string()]),
            Some(&override_argv),
        )
        .expect("override resolves");
        assert_eq!(cmd.program, std::ffi::OsString::from("./scripts/test.sh"));
        assert_eq!(cmd.args, os_args(&["--fast"]));
        // The workspace package's directory is the repo root.
        assert_eq!(cmd.cwd, root);
        assert_eq!(cmd.serial_group, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_derived_task_io() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let toolchain = CargoContributor::new(root.clone());
        let discovered = toolchain.discover_packages().await.unwrap();
        let contracts: HashMap<_, _> = discovered
            .packages()
            .iter()
            .cloned()
            .filter_map(|package| {
                let parts = package.into_parts();
                Some((parts.name?, parts.task_contract?))
            })
            .collect();
        let app_contract = &contracts["app"];
        let library_contract = &contracts["lib-a"];
        let workspace_contract = &contracts["fixture-ws"];

        let app_ctx = task_context(&toolchain, &root, "app", "crates/app");
        let lib_ctx = task_context(&toolchain, &root, "lib-a", "crates/lib-a");
        let test_util_ctx = task_context(
            &toolchain,
            &root,
            "lib-a-test-util",
            "crates/lib-a-test-util",
        );
        let workspace_ctx = task_context(&toolchain, &root, "fixture-ws", "");
        let environment = toolchain::TaskIOEnvironment::default();
        let context = toolchain::TaskIOContext {
            task_args: None,
            environment: &environment,
        };

        // defines_task mirrors the verb tables.
        assert!(app_ctx.native_tasks().defines("build"));
        assert!(app_ctx.native_tasks().defines("test"));
        assert!(lib_ctx.native_tasks().defines("test"));
        assert!(lib_ctx.native_tasks().defines("build"));
        assert!(workspace_ctx.native_tasks().defines("test"));

        // Entrypoint build with automatic inputs: workspace files + the
        // dependency crate closure as inputs (own sources via default
        // hashing), deliverables as outputs.
        let deps = [lib_ctx.clone()];
        let io = app_contract
            .derived_task_io(&app_ctx, "build", "../..", &deps, true, &context)
            .expect("entrypoint build derives IO");
        assert!(
            !io.input_globs
                .iter()
                .any(|glob| glob.contains("Cargo.lock")),
            "Cargo.lock is hashed via per-crate closures, not as a raw input: {:?}",
            io.input_globs
        );
        assert!(
            io.input_globs
                .contains(&"../../rust-toolchain.toml".to_string())
        );
        assert!(
            io.input_globs
                .contains(&"../../crates/lib-a/**".to_string()),
            "dependency crate sources are inputs, got {:?}",
            io.input_globs
        );
        assert!(
            io.input_globs
                .contains(&"!../../crates/lib-a/.turbo/**".to_string()),
            "dependency crate task logs are excluded, got {:?}",
            io.input_globs
        );
        assert_eq!(io.package_default_inputs, Some(true));
        assert!(io.env.contains(&"RUSTC_WRAPPER".to_string()));
        assert!(io.env.contains(&"RUSTUP_HOME".to_string()));
        assert!(io.env.contains(&"RUSTUP_TOOLCHAIN".to_string()));
        assert!(io.env.contains(&"CARGO_ENCODED_RUSTFLAGS".to_string()));
        assert!(io.env.contains(&"CARGO_PROFILE_*".to_string()));
        assert!(io.env.contains(&"CARGO_TARGET_*".to_string()));
        assert!(io.env.contains(&"CC_*".to_string()));
        assert!(io.env.contains(&"TARGET_CFLAGS".to_string()));

        let custom_toolchain = ToolchainId::new("custom-cargo-producer");
        let custom_dependency =
            crate::package_graph::PackageTaskContext::new_for_test_with_native_tasks(
                "custom-dep".into(),
                &root,
                turbopath::AnchoredSystemPath::new("crates/custom-dep").unwrap(),
                crate::package_graph::PackageTaskContextKind::Package,
                Some(&custom_toolchain),
                None,
                Some(
                    crate::task_contracts::ScopeTaskContract::derived(
                        custom_toolchain.clone(),
                        None,
                        std::collections::BTreeMap::new(),
                        std::collections::BTreeMap::new(),
                    )
                    .with_dependency_source_inputs(
                        crate::task_contracts::DependencySourceInputs::Include,
                    ),
                ),
            );
        let unclassified_rust_dependency =
            crate::package_graph::PackageTaskContext::new_for_test_with_native_tasks(
                "rust-by-id-only".into(),
                &root,
                turbopath::AnchoredSystemPath::new("crates/rust-by-id-only").unwrap(),
                crate::package_graph::PackageTaskContextKind::Package,
                Some(&ToolchainId::RUST),
                None,
                None,
            );
        let capability_io = app_contract
            .derived_task_io(
                &app_ctx,
                "build",
                "../..",
                &[custom_dependency, unclassified_rust_dependency],
                true,
                &context,
            )
            .unwrap();
        assert!(
            capability_io
                .input_globs
                .contains(&"../../crates/custom-dep/**".to_string())
        );
        assert!(
            !capability_io
                .input_globs
                .iter()
                .any(|glob| glob.contains("rust-by-id-only"))
        );
        assert_eq!(
            capability_io.input_safety,
            toolchain::DerivedInputSafety::Untracked
        );
        let excluded_dependency =
            crate::package_graph::PackageTaskContext::new_for_test_with_native_tasks(
                "generated-scope".into(),
                &root,
                turbopath::AnchoredSystemPath::new("generated/scope").unwrap(),
                crate::package_graph::PackageTaskContextKind::Package,
                Some(&custom_toolchain),
                None,
                Some(
                    crate::task_contracts::ScopeTaskContract::derived(
                        custom_toolchain.clone(),
                        None,
                        std::collections::BTreeMap::new(),
                        std::collections::BTreeMap::new(),
                    )
                    .with_dependency_source_inputs(
                        crate::task_contracts::DependencySourceInputs::Exclude,
                    ),
                ),
            );
        let excluded_io = app_contract
            .derived_task_io(
                &app_ctx,
                "build",
                "../..",
                &[excluded_dependency],
                true,
                &context,
            )
            .unwrap();
        assert!(
            !excluded_io
                .input_globs
                .iter()
                .any(|glob| glob.contains("generated/scope"))
        );
        assert_eq!(
            excluded_io.input_safety,
            toolchain::DerivedInputSafety::Tracked
        );
        let toolchain::DerivedOutputs::Resolved(outputs) = &io.outputs else {
            panic!("Cargo host outputs must remain resolved");
        };
        let (_, host_target) = rustc_info(&root).unwrap();
        let platform = target_platform(&host_target).unwrap();
        let basename = deliverable_basename(
            &Deliverable {
                name: "app".to_string(),
                kind: DeliverableKind::Bin,
            },
            platform,
        );
        assert_eq!(outputs, &[format!("../../target/debug/{basename}")]);
        assert!(outputs.iter().all(|output| !output.contains('*')));

        let unsupported_target = ["--target=thumbv7em-none-eabihf".to_string()];
        let unsupported_context = toolchain::TaskIOContext {
            task_args: Some(&unsupported_target),
            environment: &environment,
        };
        let unsupported = app_contract
            .derived_task_io(
                &app_ctx,
                "build",
                "../..",
                &deps,
                true,
                &unsupported_context,
            )
            .expect("entrypoint build derives IO");
        assert_eq!(unsupported.outputs, toolchain::DerivedOutputs::Unavailable);

        // Explicit inputs without $TURBO_DEFAULT$: workspace files still
        // apply, but no closure globs and no default-hashing override.
        let io = app_contract
            .derived_task_io(&app_ctx, "build", "../..", &deps, false, &context)
            .expect("entrypoint build derives IO");
        assert!(io.input_globs.contains(&"../../Cargo.toml".to_string()));
        assert!(!io.input_globs.iter().any(|glob| glob.contains("lib-a")));
        assert_eq!(io.package_default_inputs, None);

        // Non-build entrypoint verbs cache no deliverables.
        let io = app_contract
            .derived_task_io(&app_ctx, "dev", "../..", &deps, true, &context)
            .expect("entrypoint dev derives IO");
        assert_eq!(io.outputs, toolchain::DerivedOutputs::Resolved(Vec::new()));

        // The workspace package hashes crate directories instead of the
        // repo root's default file set.
        let deps = [app_ctx.clone(), lib_ctx.clone()];
        let io = workspace_contract
            .derived_task_io(&workspace_ctx, "test", "", &deps, true, &context)
            .expect("workspace test derives IO");
        assert_eq!(io.package_default_inputs, Some(false));
        assert!(io.input_globs.contains(&"crates/app/**".to_string()));
        assert!(io.input_globs.contains(&"crates/lib-a/**".to_string()));
        assert!(io.input_globs.contains(&"Cargo.toml".to_string()));
        assert_eq!(io.outputs, toolchain::DerivedOutputs::Resolved(Vec::new()));

        // Library verification hashes dev dependencies even when their cycle
        // prevents them from appearing in the package graph.
        let cycle_inputs = [test_util_ctx];
        let io = library_contract
            .derived_task_io(&lib_ctx, "test", "../..", &cycle_inputs, true, &context)
            .expect("library test derives IO");
        assert_eq!(io.package_default_inputs, Some(true));
        assert!(
            io.input_globs
                .contains(&"../../crates/lib-a-test-util/**".to_string()),
            "dev-dependency sources are inputs, got {:?}",
            io.input_globs
        );
        assert_eq!(io.outputs, toolchain::DerivedOutputs::Resolved(Vec::new()));

        // Library build artifacts are Cargo-internal and cannot be restored as
        // stable Turborepo outputs, so implicit caching fails closed.
        let io = library_contract
            .derived_task_io(&lib_ctx, "build", "../..", &cycle_inputs, true, &context)
            .expect("library build derives IO");
        assert_eq!(io.package_default_inputs, Some(true));
        assert_eq!(io.outputs, toolchain::DerivedOutputs::Unavailable);
    }
}
