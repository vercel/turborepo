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
//! A synthetic package anchored at the root `Cargo.toml` and depending on
//! every crate represents the workspace itself; it hosts the
//! workspace-scoped verification verbs (`<name>#test` → `cargo test
//! --workspace`, ...; see [`workspace_subcommand`]). Its name is declared
//! by the user in the root manifest — using Turborepo with Rust requires
//! naming the workspace:
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
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::Spanned;

use crate::{
    package_json::PackageJson,
    toolchain::{self, DiscoverPackagesFuture, DiscoveredPackage, Toolchain, ToolchainId},
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
    #[error(
        "Cargo package {name:?} is defined in the root Cargo.toml, which Turborepo cannot model \
         as a package safely. Move it into a subdirectory and add it to `[workspace].members`."
    )]
    UnsupportedRootPackage { name: String },
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

/// The Rust compiler version and host triple, as a hashable external-dependency
/// identity.
///
/// Run from `repo_root` so rustup's shim resolves `rust-toolchain`
/// overrides the same way a task's `cargo` invocation will. Participating
/// in the external-dependency hash means compiling with a different
/// toolchain or on a different host never restores incompatible artifacts.
pub fn rustc_identity(
    repo_root: &AbsoluteSystemPath,
) -> Result<turborepo_lockfiles::Package, Error> {
    rustc_info(repo_root).map(|(identity, _)| identity)
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

/// Per-crate external dependency closures from Cargo.lock, for the crates'
/// external-dependency hashes.
///
/// A missing, unreadable, or unparsable lockfile is a hard error — silently
/// hashing nothing would be unsound.
pub fn external_closures(
    repo_root: &AbsoluteSystemPath,
    members: &[String],
) -> Result<HashMap<String, HashSet<turborepo_lockfiles::Package>>, Error> {
    let lock_path = repo_root.join_component(CARGO_LOCK);
    let contents = match lock_path.read_to_string() {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(Error::MissingLockfile);
        }
        Err(error) => return Err(Error::LockfileRead(error)),
    };
    Ok(turborepo_lockfiles::cargo_external_closures(
        &contents, members,
    )?)
}

/// Verify Cargo can resolve the workspace without changing Cargo.lock and that
/// every resolved local package is an in-repository workspace member.
/// Validation happens before task hashes and cache lookup, so artifacts are
/// always keyed by sources Turborepo can hash, watch, and prune.
pub fn validate_lockfile(repo_root: &AbsoluteSystemPath) -> Result<(), Error> {
    let lock_path = repo_root.join_component(CARGO_LOCK);
    match lock_path.read_to_string() {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(Error::MissingLockfile);
        }
        Err(error) => return Err(Error::LockfileRead(error)),
    }

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

    let metadata: ResolvedMetadata = serde_json::from_slice(&output.stdout)?;
    validate_resolved_local_packages(repo_root, metadata)
}

fn validate_resolved_local_packages(
    repo_root: &AbsoluteSystemPath,
    metadata: ResolvedMetadata,
) -> Result<(), Error> {
    let real_repo_root = repo_root
        .to_realpath()
        .map_err(|source| Error::LocalPackagePath {
            path: repo_root.to_string(),
            source,
        })?;
    let root_manifest_path = real_repo_root.join_component(CARGO_TOML);
    for package in metadata.packages {
        if package.source.is_some() {
            continue;
        }
        let Some(manifest_path) = metadata_path(&package.manifest_path) else {
            return Err(Error::OutsideRepositoryLocalPackage {
                name: package.name,
                manifest_path: package.manifest_path,
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
                name: package.name,
                manifest_path: package.manifest_path,
            });
        }
        if real_manifest_path == root_manifest_path {
            return Err(Error::UnsupportedRootPackage { name: package.name });
        }
        if !metadata.workspace_members.contains(&package.id) {
            return Err(Error::NonMemberLocalPackage {
                name: package.name,
                manifest_path: package.manifest_path,
            });
        }
    }

    Ok(())
}

/// How a Cargo-toolchain package participates in task execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CargoPackageKind {
    /// An internal library crate: present in the package graph for
    /// `--filter`/`--affected` propagation, but tasks are no-ops — Cargo
    /// builds libraries implicitly as part of an entrypoint's closure.
    Library,
    /// A crate with `bin`/`cdylib`/`staticlib` targets: a deliverable.
    /// `build`/`run` tasks execute `cargo <verb> --package=<crate>`.
    Entrypoint,
    /// The synthetic user-named workspace package hosting
    /// workspace-scoped verification verbs (`cargo test --workspace`, ...).
    Workspace,
}

/// Cargo-specific details for a discovered package, retained by the
/// [`CargoToolchain`] (keyed by package name) rather than attached to the
/// toolchain-neutral `PackageInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoPackageDetails {
    pub kind: CargoPackageKind,
    /// The crate's deliverable targets (empty for libraries and the
    /// workspace package).
    pub deliverables: Vec<Deliverable>,
    pub manifest_alters_output_layout: bool,
    /// The crate's directory, repo-root-relative in unix form (empty for
    /// the synthetic workspace package).
    pub dir: String,
}

/// Map a Turborepo task name to the Cargo subcommand that implements it for
/// an entrypoint crate. Entrypoints only build and run — verification verbs
/// happen at workspace scope.
pub fn entrypoint_subcommand(task: &str) -> Option<&'static str> {
    match task {
        "build" => Some("build"),
        "run" | "dev" => Some("run"),
        _ => None,
    }
}

/// Map a Turborepo task name to the Cargo subcommand that implements it at
/// workspace scope (the synthetic user-named workspace package).
///
/// `build` is deliberately absent: building is entrypoint-scoped
/// (`cargo build --package=<crate>`), and a workspace-wide build would
/// duplicate that work in a second cargo process.
pub fn workspace_subcommand(task: &str) -> Option<&'static str> {
    match task {
        "test" => Some("test"),
        "check" => Some("check"),
        "lint" | "clippy" => Some("clippy"),
        "doc" | "docs" => Some("doc"),
        "bench" => Some("bench"),
        _ => None,
    }
}

/// The Cargo subcommand a task resolves to for a package, given its
/// [`CargoPackageKind`]. `None` means the task is a no-op for this package
/// (like a missing package.json script).
pub fn task_subcommand(kind: CargoPackageKind, task: &str) -> Option<&'static str> {
    match kind {
        CargoPackageKind::Entrypoint => entrypoint_subcommand(task),
        CargoPackageKind::Workspace => workspace_subcommand(task),
        CargoPackageKind::Library => None,
    }
}

/// The display string for a Cargo task's command, derived from the same
/// tables as execution so it cannot drift.
pub fn display_command(kind: CargoPackageKind, task: &str, package: &str) -> Option<String> {
    let subcommand = task_subcommand(kind, task)?;
    Some(match kind {
        CargoPackageKind::Entrypoint => {
            format!("cargo {subcommand} --package={package} --locked")
        }
        CargoPackageKind::Workspace => format!("cargo {subcommand} --workspace --locked"),
        CargoPackageKind::Library => return None,
    })
}

/// Whether pass-through args for `subcommand` must follow a `--` separator.
/// These subcommands forward everything after `--` to the underlying tool
/// (the built binary for `run`, the test/bench harness, clippy's lint
/// flags); the remaining subcommands take no trailing args, so pass-through
/// args are attached directly as cargo flags.
pub fn pass_through_uses_separator(subcommand: &str) -> bool {
    matches!(subcommand, "test" | "bench" | "run" | "clippy")
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

const TASK_IO_ENV_VARS: &[&str] = &[
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
/// participates the same way (see [`rustc_identity`]).
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

#[derive(Debug, Clone)]
struct CargoWorkspaceDetails {
    target_directory: AbsoluteSystemPathBuf,
    host_target: String,
    repository_config_alters_output_layout: bool,
    repository_config_untracked: bool,
    external_config_present: bool,
    manifest_alters_profile_dirs: bool,
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
    prefix: &str,
    profile: &str,
    platform: CargoTargetPlatform,
    deliverables: &[Deliverable],
) -> Vec<String> {
    let directory = join_prefix(prefix, &format!("target/{profile}"));
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

fn cargo_profile_directory(args: &[String]) -> Option<String> {
    let mut release = false;
    let mut profile = None;
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
    if release {
        return Some("release".to_string());
    }
    match profile.as_deref() {
        None | Some("dev" | "test") => Some("debug".to_string()),
        Some("release" | "bench") => Some("release".to_string()),
        Some(profile)
            if profile
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) =>
        {
            Some(profile.to_string())
        }
        Some(_) => None,
    }
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

fn cargo_output_profile(
    repo_root: &AbsoluteSystemPath,
    workspace: &CargoWorkspaceDetails,
    package: &CargoPackageDetails,
    context: &toolchain::TaskIOContext<'_>,
) -> Option<String> {
    let environment = context.environment;
    let profile_dir_name = environment.iter().any(|(name, _)| {
        let name = name.to_ascii_uppercase();
        name.starts_with("CARGO_PROFILE_") && name.ends_with("_DIR_NAME")
    });
    let default_target_directory = repo_root.join_component(TARGET_DIR);

    if package.manifest_alters_output_layout
        || workspace.repository_config_alters_output_layout
        || workspace.external_config_present
        || workspace.manifest_alters_profile_dirs
        || environment.get("RUSTC").is_some()
        || environment.get("CARGO_BUILD_RUSTC").is_some()
        || environment.get("CARGO_BUILD_TARGET").is_some()
        || environment.get("CARGO_TARGET_DIR").is_some()
        || environment.get("CARGO_BUILD_TARGET_DIR").is_some()
        || environment.get("CARGO_BUILD_ARTIFACT_DIR").is_some()
        || profile_dir_name
        || workspace.target_directory != default_target_directory
        || !target_directory_within_repo(repo_root, &workspace.target_directory)
    {
        return None;
    }

    context
        .task_args
        .map_or_else(|| Some("debug".to_string()), cargo_profile_directory)
}

/// The Cargo toolchain. Registered in the
/// [`crate::toolchain::ToolchainRegistry`] when
/// `futureFlags.experimentalCargoWorkspaces` is enabled and the repository
/// root contains a `Cargo.toml`.
pub struct CargoToolchain {
    repo_root: AbsoluteSystemPathBuf,
    /// Per-package details recorded during discovery, consumed by command
    /// resolution. Keyed by package name.
    details: std::sync::Mutex<HashMap<String, CargoPackageDetails>>,
    workspace_details: std::sync::Mutex<Option<CargoWorkspaceDetails>>,
    /// The cargo binary, resolved lazily so runs without Cargo tasks never
    /// pay for a PATH scan.
    cargo_binary: std::sync::OnceLock<Result<std::path::PathBuf, which::Error>>,
}

#[derive(Debug, thiserror::Error)]
enum CargoCommandError {
    #[error("Unable to find cargo binary: {0}")]
    Which(#[from] which::Error),
}

impl CargoToolchain {
    pub fn new(repo_root: AbsoluteSystemPathBuf) -> Arc<Self> {
        Arc::new(Self {
            repo_root,
            details: std::sync::Mutex::new(HashMap::new()),
            workspace_details: std::sync::Mutex::new(None),
            cargo_binary: std::sync::OnceLock::new(),
        })
    }

    fn package_details(&self, package: &str) -> Option<CargoPackageDetails> {
        self.details
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(package)
            .cloned()
    }

    fn record_details(&self, package: String, details: CargoPackageDetails) {
        self.details
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(package, details);
    }

    fn workspace_details(&self) -> Option<CargoWorkspaceDetails> {
        self.workspace_details
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Toolchain for CargoToolchain {
    fn id(&self) -> ToolchainId {
        ToolchainId::RUST
    }

    fn task_io_env_vars(&self) -> &[&str] {
        TASK_IO_ENV_VARS
    }

    fn task_command(
        &self,
        repo_root: &AbsoluteSystemPath,
        package: &crate::package_graph::PackageInfo,
        task: &str,
        pass_through_args: Option<&[String]>,
        override_command: Option<&[String]>,
    ) -> Result<Option<toolchain::TaskCommand>, toolchain::Error> {
        // An override replaces the verb-table resolution and applies to any
        // crate — including libraries, which map no verbs of their own. The
        // serial group survives when the override still invokes cargo: the
        // group exists because of cargo's build-directory lock, a property
        // of the binary, not of the verb table.
        if let Some(override_command) = override_command {
            let serial_group = (override_command.first().map(String::as_str) == Some("cargo"))
                .then(|| "cargo".to_string());
            return Ok(toolchain::override_task_command(
                repo_root,
                package,
                override_command,
                pass_through_args,
                serial_group,
            ));
        }
        let Some(name) = package.package_name() else {
            return Ok(None);
        };
        let Some(details) = self.package_details(&name) else {
            return Ok(None);
        };
        let Some(subcommand) = task_subcommand(details.kind, task) else {
            return Ok(None);
        };

        let cargo_binary = self
            .cargo_binary
            .get_or_init(|| which::which("cargo"))
            .as_deref()
            .map_err(|err| toolchain::Error::Failed(Box::new(CargoCommandError::Which(*err))))?;

        let scope = match details.kind {
            // `--package=<name>` as a single token so a hostile crate name
            // can never be interpreted as a separate flag.
            CargoPackageKind::Entrypoint => format!("--package={name}"),
            CargoPackageKind::Workspace => "--workspace".to_string(),
            // Libraries never map to a subcommand.
            CargoPackageKind::Library => return Ok(None),
        };
        let mut args: Vec<std::ffi::OsString> =
            vec![subcommand.into(), scope.into(), "--locked".into()];
        if let Some(pass_through_args) = pass_through_args {
            if pass_through_uses_separator(subcommand) {
                args.push("--".into());
            }
            args.extend(pass_through_args.iter().map(std::ffi::OsString::from));
        }

        Ok(Some(toolchain::TaskCommand {
            program: cargo_binary.as_os_str().to_owned(),
            args,
            // Scoping flags select the work, so we always run from the
            // workspace root.
            cwd: repo_root.to_owned(),
            // Concurrent cargo processes serialize on Cargo's
            // build-directory lock anyway (while emitting "Blocking waiting
            // for file lock" noise), so run them one at a time and let each
            // cargo use all cores internally. `cargo run` is exempt: the
            // process outlives its build phase (dev servers etc.) and would
            // starve the group.
            serial_group: (subcommand != "run").then(|| "cargo".to_string()),
        }))
    }

    fn task_display_command(
        &self,
        package: &crate::package_graph::PackageInfo,
        task: &str,
    ) -> Option<String> {
        let name = package.package_name()?;
        let details = self.package_details(&name)?;
        display_command(details.kind, task, &name)
    }

    fn task_defaults(
        &self,
        package: &crate::package_graph::PackageInfo,
        task: &str,
    ) -> toolchain::TaskDefaults {
        let cache = package
            .package_name()
            .and_then(|name| self.package_details(&name))
            .and_then(|details| task_subcommand(details.kind, task))
            .is_some_and(|subcommand| subcommand == "run")
            .then_some(false);

        toolchain::TaskDefaults { cache }
    }

    /// Route rustc invocations through the embedded sccache, with the
    /// Turborepo-served endpoint as its webdav storage backend. The wrapper
    /// is the running turbo binary itself (which dispatches invocations
    /// marked by [`toolchain::COMPILE_CACHE_WRAPPER_ENV`] to the sccache it
    /// embeds), so nothing needs to be installed. sccache fetches per
    /// compilation-unit objects lazily at rustc invocation time, so no
    /// state needs restoring before the task starts.
    ///
    /// `CARGO_INCREMENTAL=0` accompanies the wrapper because sccache cannot
    /// cache incrementally-compiled crates and would fall back to plain
    /// compilation for them.
    ///
    /// These are injected at execution time only and deliberately do not
    /// participate in the task hash: a compile cache is output-transparent,
    /// so enabling it must not invalidate existing task artifacts.
    ///
    /// Composition with the task environment:
    ///
    /// - A pre-existing `RUSTC_WRAPPER` or any `SCCACHE_*` variable signals a
    ///   competing compiler-cache configuration; injecting on top of it could
    ///   hijack that setup's backend, so the whole set stands down.
    ///   (`RUSTC_WRAPPER` participates in task hashes via [`HASHED_ENV_VARS`],
    ///   so a user wrapper also invalidates caches — the injected one
    ///   deliberately does not.)
    /// - A pre-existing `CARGO_INCREMENTAL=0` is common CI hygiene, not a
    ///   competing cache: the rest is injected and the explicit value is left
    ///   alone. (When absent, `CARGO_INCREMENTAL=0` is injected because sccache
    ///   cannot cache incrementally-compiled crates.) Any *other* explicit
    ///   `CARGO_INCREMENTAL` value stands the set down: incremental compilation
    ///   was deliberately requested, and sccache's wrapper hard-exits when it
    ///   sees `CARGO_INCREMENTAL=1`, which would fail the build.
    fn compile_cache_env(
        &self,
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
            // The compile cache is an optimization: if the server cannot be
            // reached or started (storage outage mid-run, port trouble),
            // the wrapper warns and runs the compiler directly instead of
            // failing the build.
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

    fn defines_task(&self, package: &crate::package_graph::PackageInfo, task: &str) -> bool {
        package
            .package_name()
            .and_then(|name| self.package_details(&name))
            .and_then(|details| task_subcommand(details.kind, task))
            .is_some()
    }

    fn derives_task_io(&self, package: &crate::package_graph::PackageInfo, task: &str) -> bool {
        // Mirrors the early returns of `derived_task_io`: a known crate
        // with a Cargo subcommand for this task.
        self.defines_task(package, task)
    }

    fn watch_spec(&self) -> toolchain::WatchSpec {
        watch_spec()
    }

    /// Prune the Cargo workspace machinery around the kept crates:
    ///
    /// * `Cargo.lock` is subset to the closure of the kept crates, so `cargo
    ///   build --locked` succeeds in the pruned output.
    /// * The lock walk may surface members beyond Turborepo's package-graph
    ///   closure (Cargo.lock merges dev-dependency edges, including
    ///   cycle-participating ones the package graph drops). Their manifests are
    ///   referenced by kept crates, so they are reported as extra packages to
    ///   keep.
    /// * The root `Cargo.toml` is rewritten: explicit `members`, filtered
    ///   `default-members`, `[workspace.dependencies]` path entries to removed
    ///   crates dropped.
    /// * Toolchain and Cargo config files are carried over.
    fn prune_plan(
        &self,
        kept_packages: &[String],
    ) -> Result<Option<toolchain::PrunePlan>, toolchain::Error> {
        if kept_packages.is_empty() {
            return Ok(None);
        }
        let failed = |err: Error| toolchain::Error::Failed(Box::new(err));

        let lock_path = self.repo_root.join_component(CARGO_LOCK);
        let lock_contents = match lock_path.read_to_string() {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(failed(Error::MissingLockfile));
            }
            Err(error) => return Err(failed(Error::LockfileRead(error))),
        };
        let pruned_lock = turborepo_lockfiles::cargo_prune_lock(&lock_contents, kept_packages)
            .map_err(|err| failed(Error::Lockfile(err)))?;

        let mut kept_dirs = Vec::with_capacity(pruned_lock.members.len());
        let mut extra_packages = Vec::new();
        for member in &pruned_lock.members {
            let Some(details) = self.package_details(member) else {
                // A lock member that discovery never saw; the lockfile and
                // the workspace disagree. Keep going — the manifest rewrite
                // simply won't list it, and cargo will report specifics.
                tracing::warn!(
                    "Cargo.lock member {member} is not a discovered workspace crate; skipping"
                );
                continue;
            };
            kept_dirs.push(details.dir.clone());
            if !kept_packages.contains(member) {
                extra_packages.push(member.clone());
            }
        }

        let manifest_contents = self
            .repo_root
            .join_component(CARGO_TOML)
            .read_to_string()
            .map_err(|err| failed(Error::WorkspaceFileRead(err)))?;
        let pruned_manifest =
            prune_root_manifest(&manifest_contents, &kept_dirs).map_err(failed)?;

        Ok(Some(toolchain::PrunePlan {
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
            .iter()
            .map(|path| path.to_string())
            .collect(),
        }))
    }

    /// Our lock subset is reachability-based, but Cargo's real resolution
    /// is feature-aware: shrinking the workspace can deactivate features
    /// that were the only reason some packages were in the closure. Rather
    /// than reimplement feature unification, let Cargo minimally sync its
    /// own lockfile (every retained pin is preserved; only feature-dead
    /// entries are dropped) so `cargo build --locked` passes in the pruned
    /// output. Try `--offline` first — removals need no network — but
    /// workspaces with git patches need their git databases, which a cold
    /// machine won't have cached, so fall back to a networked sync. Failure
    /// is not fatal: the superset lock still builds correctly, it just
    /// isn't `--locked`-clean.
    fn prune_finalize(&self, pruned_root: &AbsoluteSystemPath) -> Vec<String> {
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
                    "unable to canonicalize the pruned Cargo.lock; `cargo build --locked` may \
                     require a lockfile refresh: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Err(error) => {
                tracing::warn!(
                    "unable to run cargo to canonicalize the pruned Cargo.lock: {error}"
                );
            }
        }
        vec![CARGO_LOCK.to_string()]
    }

    fn derived_task_io(
        &self,
        package: &crate::package_graph::PackageInfo,
        task: &str,
        path_to_root: &str,
        dependencies: &[&crate::package_graph::PackageInfo],
        wants_automatic_inputs: bool,
        context: &toolchain::TaskIOContext<'_>,
    ) -> Option<toolchain::DerivedTaskIO> {
        let name = package.package_name()?;
        let details = self.package_details(&name)?;
        let subcommand = task_subcommand(details.kind, task)?;

        // The workspace lockfile/manifest, Cargo config, and pinned
        // rust-toolchain files are hashed (dependency, profile, or toolchain
        // changes invalidate the cache), along with the env vars that change
        // what Cargo builds. These apply regardless of explicit user
        // `inputs`.
        let mut io = toolchain::DerivedTaskIO {
            input_globs: hash_input_globs(path_to_root),
            env: HASHED_ENV_VARS.iter().map(|var| var.to_string()).collect(),
            ..Default::default()
        };
        if let Some(workspace) = self.workspace_details()
            && (workspace.repository_config_untracked || workspace.external_config_present)
        {
            io.input_safety = toolchain::DerivedInputSafety::Untracked;
            if workspace.repository_config_untracked {
                io.input_globs.retain(|glob| {
                    !glob.ends_with(".cargo/config.toml") && !glob.ends_with(".cargo/config")
                });
            }
        }

        // Source globs for the crates whose code this task compiles,
        // filtered to real crates (the synthetic workspace package has no
        // sources of its own).
        let dependency_globs = || {
            let mut globs: Vec<String> = dependencies
                .iter()
                .filter(|dep| dep.toolchain == ToolchainId::RUST)
                .filter(|dep| {
                    dep.package_name()
                        .and_then(|dep_name| self.package_details(&dep_name))
                        .is_some_and(|details| details.kind != CargoPackageKind::Workspace)
                })
                .flat_map(|dep| {
                    crate_source_globs(path_to_root, dep.package_path().to_unix().as_str())
                })
                .collect();
            globs.sort();
            globs
        };

        match details.kind {
            // An entrypoint build compiles its whole dependency closure in
            // one cargo process, so the closure's sources are flattened
            // into this task's inputs — invalidation must not depend on
            // users wiring up `dependsOn` between crates. The crate's
            // bin/cdylib/staticlib artifacts are the deliverables and the
            // only target/ contents worth caching; Cargo's internal target/
            // state is its own incremental cache and is left alone.
            CargoPackageKind::Entrypoint => {
                if wants_automatic_inputs {
                    io.package_default_inputs = Some(true);
                    io.input_globs.extend(dependency_globs());
                }
                if subcommand == "build" {
                    io.outputs = self
                        .workspace_details()
                        .and_then(|workspace| {
                            let profile = cargo_output_profile(
                                &self.repo_root,
                                &workspace,
                                &details,
                                context,
                            )?;
                            let platform = target_platform(&workspace.host_target)?;
                            Some(toolchain::DerivedOutputs::Resolved(
                                deliverable_output_paths(
                                    path_to_root,
                                    &profile,
                                    platform,
                                    &details.deliverables,
                                ),
                            ))
                        })
                        .unwrap_or(toolchain::DerivedOutputs::Unavailable);
                }
            }
            // The workspace package's directory is the repo root, so
            // default hashing would pull in the entire repository
            // (including JS packages). Hash the crate directories instead —
            // its dependencies are exactly the crates.
            CargoPackageKind::Workspace => {
                if wants_automatic_inputs {
                    io.package_default_inputs = Some(false);
                    io.input_globs.extend(dependency_globs());
                }
            }
            // Libraries never map to a subcommand; unreachable while
            // `subcommand` is `Some`.
            CargoPackageKind::Library => return None,
        }

        Some(io)
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            // Discovery spawns `cargo metadata` synchronously, so keep it off
            // the async runtime like the JavaScript manifest-parsing path.
            let workspace =
                turborepo_rayon_compat::block_in_place(|| discover_crates(&self.repo_root))
                    .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;
            let target_directory = workspace.target_directory.clone();
            let crates = workspace.crates;

            if crates.is_empty() {
                if workspace.has_packages {
                    turborepo_rayon_compat::block_in_place(|| validate_lockfile(&self.repo_root))
                        .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;
                }
                return Ok(Vec::new());
            }

            // Using Turborepo with Rust requires naming the workspace: the
            // synthetic workspace package is a real package (task keys,
            // filters), and every package must have a name. Only enforced
            // when there are crates to host — a memberless manifest doesn't
            // demand a name for nothing.
            let workspace_name = workspace
                .name
                .ok_or_else(|| toolchain::Error::Failed(Box::new(Error::MissingWorkspaceName)))?;

            // Each crate becomes a package. Internal dependencies are
            // expressed as `workspace:*` specifiers in the synthesized
            // descriptor so the existing dependency splitter wires
            // crate->crate edges (powering `--filter`/`--affected`).
            // Discovery only reports dependencies on other discovered
            // crates, so every synthesized specifier resolves internally and
            // Cargo edges never leak into unresolved externals.
            // External dependencies (locked crates.io/git packages plus the
            // compiler itself) participate in each crate task's hash through
            // the same external-dependency mechanism JS packages use, scoped
            // to the crate's transitive closure — a dependency bump only
            // invalidates crates that actually depend on it, and a toolchain
            // change invalidates everything.
            let all_names: Vec<String> = crates.iter().map(|c| c.name.clone()).collect();
            let (rustc, host_target, mut closures) = turborepo_rayon_compat::block_in_place(|| {
                validate_lockfile(&self.repo_root)?;
                let (rustc, host_target) = rustc_info(&self.repo_root)?;
                Ok::<_, Error>((
                    rustc,
                    host_target,
                    external_closures(&self.repo_root, &all_names)?,
                ))
            })
            .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;
            if let Some(target_directory) = target_directory {
                let startup_environment = CargoHomeEnvironment::current();
                let config = cargo_config_influence(&self.repo_root, &startup_environment);
                *self
                    .workspace_details
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some(CargoWorkspaceDetails {
                        target_directory,
                        host_target,
                        repository_config_alters_output_layout: config
                            .repository_alters_output_layout,
                        repository_config_untracked: config.repository_config_untracked,
                        external_config_present: config.external_present,
                        manifest_alters_profile_dirs: manifest_alters_profile_dirs(&self.repo_root),
                    });
            }
            let workspace_externals: HashSet<turborepo_lockfiles::Package> = closures
                .values()
                .flatten()
                .cloned()
                .chain(std::iter::once(rustc.clone()))
                .collect();

            let mut packages = Vec::with_capacity(crates.len() + 1);
            let mut crate_names = Vec::with_capacity(crates.len());
            for cargo_crate in crates {
                let dependencies = cargo_crate
                    .internal_dependencies
                    .iter()
                    .map(|dep| (dep.clone(), "workspace:*".to_string()))
                    .collect();
                let kind = if cargo_crate.is_entrypoint() {
                    CargoPackageKind::Entrypoint
                } else {
                    CargoPackageKind::Library
                };
                let dir = cargo_crate
                    .manifest_path
                    .parent()
                    .and_then(|dir| {
                        turbopath::AnchoredSystemPathBuf::new(&self.repo_root, dir).ok()
                    })
                    .map(|dir| dir.to_unix().to_string())
                    .unwrap_or_default();
                self.record_details(
                    cargo_crate.name.clone(),
                    CargoPackageDetails {
                        kind,
                        deliverables: cargo_crate.deliverables,
                        manifest_alters_output_layout: cargo_crate.manifest_alters_output_layout,
                        dir,
                    },
                );
                let external_dependencies: HashSet<turborepo_lockfiles::Package> = closures
                    .remove(&cargo_crate.name)
                    .unwrap_or_default()
                    .into_iter()
                    .chain(std::iter::once(rustc.clone()))
                    .collect();
                crate_names.push(cargo_crate.name.clone());
                packages.push(DiscoveredPackage {
                    descriptor: PackageJson {
                        name: Some(Spanned::new(cargo_crate.name)),
                        dependencies: Some(dependencies),
                        ..Default::default()
                    },
                    manifest_path: cargo_crate.manifest_path,
                    external_dependencies: Some(external_dependencies),
                });
            }

            // The synthetic workspace package, anchored at the root
            // Cargo.toml and named by the user via `[workspace.metadata]
            // name`. It depends on every crate so `--affected` and
            // dependent-filters propagate crate changes to it.
            if !crate_names.is_empty() {
                self.record_details(
                    workspace_name.clone(),
                    CargoPackageDetails {
                        kind: CargoPackageKind::Workspace,
                        deliverables: Vec::new(),
                        manifest_alters_output_layout: false,
                        dir: String::new(),
                    },
                );
                let dependencies = crate_names
                    .into_iter()
                    .map(|name| (name, "workspace:*".to_string()))
                    .collect();
                packages.push(DiscoveredPackage {
                    descriptor: PackageJson {
                        name: Some(Spanned::new(workspace_name)),
                        dependencies: Some(dependencies),
                        ..Default::default()
                    },
                    manifest_path: self.repo_root.join_component(CARGO_TOML),
                    // Workspace-scoped verbs run every crate, so the union
                    // of all closures is this package's external surface.
                    external_dependencies: Some(workspace_externals),
                });
            }

            Ok(packages)
        })
    }
}

/// The Cargo default build directory, relative to the repo root.
pub const TARGET_DIR: &str = "target";

/// How filesystem events relate to Cargo in watch mode. Manifests and the
/// lockfile define the crate set and its edges — any change makes the
/// watcher's package graph stale, so they trigger full rediscovery
/// (`Cargo.toml` files under `target/` are build byproducts, not workspace
/// definition, and are exempted via the ignore prefix). Events under the
/// root `target/` directory are dropped entirely: Cargo writes there
/// continuously during builds, and letting those events through would
/// re-trigger the very tasks that produced them — usually `target/` is
/// gitignored, but a feedback loop must not depend on a `.gitignore` entry.
pub fn watch_spec() -> toolchain::WatchSpec {
    toolchain::WatchSpec {
        definition_file_names: vec![CARGO_TOML.to_string()],
        definition_paths: vec![CARGO_LOCK.to_string()],
        ignore_prefixes: vec![TARGET_DIR.to_string()],
    }
}

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
    /// Names of other workspace crates this crate depends on, resolved by
    /// Cargo itself (`cargo metadata`). Dev-dependency edges that would form
    /// a cycle are dropped, since Cargo permits dev-dep cycles but the
    /// package graph must remain a DAG.
    pub internal_dependencies: Vec<String>,
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
    /// synthesized (see [`Toolchain::discover_packages`]), so manifests
    /// without members don't demand a name for nothing.
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
            ["target", "target-dir", "rustc", "artifact-dir"]
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
/// are invalid, are skipped with a warning. A `[package]` in the root
/// manifest is skipped too: its directory would be the entire repository.
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

    let has_packages = !metadata.packages.is_empty();
    let name = workspace_name(&metadata)?;
    let target_directory = metadata_path(&metadata.target_directory);
    let crates = connect_crates(parse_members(repo_root, &root_manifest_path, metadata));

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
    dev: bool,
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
    root_manifest_path: &AbsoluteSystemPath,
    metadata: Metadata,
) -> Vec<ParsedCrate> {
    let mut parsed = Vec::new();
    for package in metadata.packages {
        let Some(manifest_path) = metadata_path(&package.manifest_path) else {
            tracing::warn!(
                "skipping Cargo crate {}: non-absolute manifest path {}",
                package.name,
                package.manifest_path
            );
            continue;
        };
        if &*manifest_path == root_manifest_path {
            tracing::warn!(
                "ignoring [package] in the root Cargo.toml: a crate at the repository root is not \
                 supported as a Turborepo package"
            );
            continue;
        }
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
                    dev: dep.kind.as_deref() == Some("dev"),
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

/// Resolve dependency edges to crate names by manifest directory and drop
/// dev-dependency edges that would form a cycle (Cargo permits dev-dep
/// cycles; the package graph is a DAG).
fn connect_crates(parsed: Vec<ParsedCrate>) -> Vec<CargoCrate> {
    let dir_to_name: HashMap<&AbsoluteSystemPath, &str> = parsed
        .iter()
        .filter_map(|c| Some((c.manifest_path.parent()?, c.name.as_str())))
        .collect();

    let mut adjacency: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    let mut dev_edges: Vec<(&str, &str)> = Vec::new();
    for parsed_crate in &parsed {
        let from = parsed_crate.name.as_str();
        adjacency.entry(from).or_default();
        for dep in &parsed_crate.dependencies {
            let Some(&to) = dir_to_name.get(&*dep.dir) else {
                // Path dependency on a non-member (e.g. outside the repo).
                continue;
            };
            if to == from {
                continue;
            }
            if dep.dev {
                dev_edges.push((from, to));
            } else {
                adjacency.entry(from).or_default().insert(to);
            }
        }
    }
    // Deterministic order so the same dev edge always wins when a cycle must
    // be broken.
    dev_edges.sort_unstable();
    dev_edges.dedup();
    for (from, to) in dev_edges {
        if reaches(&adjacency, to, from) {
            tracing::debug!(
                "dropping dev-dependency edge {from} -> {to}: it would create a cycle in the \
                 package graph"
            );
        } else {
            adjacency.entry(from).or_default().insert(to);
        }
    }

    let mut edges: HashMap<String, Vec<String>> = adjacency
        .into_iter()
        .map(|(name, deps)| {
            (
                name.to_string(),
                deps.into_iter().map(String::from).collect(),
            )
        })
        .collect();

    parsed
        .into_iter()
        .map(|parsed_crate| CargoCrate {
            internal_dependencies: edges.remove(parsed_crate.name.as_str()).unwrap_or_default(),
            name: parsed_crate.name,
            manifest_path: parsed_crate.manifest_path,
            deliverables: parsed_crate.deliverables,
            manifest_alters_output_layout: parsed_crate.manifest_alters_output_layout,
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

/// The subset of `cargo metadata --no-deps` output we consume. With
/// `--no-deps`, `packages` contains exactly the workspace members.
#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
    target_directory: String,
    /// The `[workspace.metadata]` table, serialized as JSON. Carries the
    /// user-declared workspace name.
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct MetadataPackage {
    name: String,
    manifest_path: String,
    #[serde(default)]
    dependencies: Vec<MetadataDependency>,
    #[serde(default)]
    targets: Vec<MetadataTarget>,
}

#[derive(Debug, Deserialize)]
struct MetadataDependency {
    /// Absolute path to the dependency's directory, present only for path
    /// dependencies.
    path: Option<String>,
    /// `null` for normal deps, `"dev"` or `"build"` otherwise.
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MetadataTarget {
    name: String,
    kind: Vec<String>,
}

/// The subset of full `cargo metadata --locked --all-features` output needed
/// to distinguish external packages, workspace members, and unsupported local
/// path packages.
#[derive(Debug, Deserialize)]
struct ResolvedMetadata {
    packages: Vec<ResolvedMetadataPackage>,
    workspace_members: HashSet<String>,
}

#[derive(Debug, Deserialize)]
struct ResolvedMetadataPackage {
    id: String,
    name: String,
    source: Option<String>,
    manifest_path: String,
}

#[cfg(test)]
mod test {
    use turbopath::{AbsoluteSystemPathBuf, IntoUnix};

    use super::*;

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
    async fn test_cargo_toolchain_rejects_root_package() {
        let (_tmp, root) = tempdir_root();
        write(
            &root,
            &["Cargo.toml"],
            "[package]\nname = \"root-package\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[workspace]\nmembers = []\nresolver = \"2\"\n",
        );
        write(&root, &["src", "lib.rs"], "");
        generate_lockfile(&root);

        let error = CargoToolchain::new(root)
            .discover_packages()
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("root-package")
                && error.to_string().contains("root Cargo.toml"),
            "unexpected validation result: {error}"
        );
    }

    fn output_test_workspace(root: &AbsoluteSystemPath) -> CargoWorkspaceDetails {
        CargoWorkspaceDetails {
            target_directory: root.join_component(TARGET_DIR),
            host_target: "x86_64-unknown-linux-gnu".to_string(),
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
            dir: "crates/app".to_string(),
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
    fn test_cargo_profile_directory_resolves_precedence_and_builtin_mappings() {
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
            assert_eq!(cargo_profile_directory(&args).as_deref(), expected);
        }
        for args in [
            vec!["--release", "--profile=ci"],
            vec!["--profile=ci", "--profile=dev"],
            vec!["--release", "--release"],
            vec!["--profile=../release"],
        ] {
            let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(cargo_profile_directory(&args), None);
        }
    }

    #[test]
    fn test_cargo_profile_directory_accepts_only_known_neutral_flags() {
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
        assert_eq!(cargo_profile_directory(&neutral).as_deref(), Some("debug"));
        assert_eq!(
            cargo_profile_directory(&["--future-layout-control".to_string()]),
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
            "../..",
            "release",
            CargoTargetPlatform::Unix,
            &[Deliverable {
                name: "app".to_string(),
                kind: DeliverableKind::Bin,
            }],
        );
        assert_eq!(outputs, ["../../target/release/app"]);
        assert!(outputs.iter().all(|output| !output.contains('*')));
    }

    #[test]
    fn test_cargo_output_profile_fails_closed_for_layout_controls() {
        let (_tmp, root) = tempdir_root();
        let workspace = output_test_workspace(&root);
        let package = output_test_package();
        let empty_environment = toolchain::TaskIOEnvironment::default();
        let supported_args = ["--release".to_string()];
        let supported = toolchain::TaskIOContext {
            task_args: Some(&supported_args),
            environment: &empty_environment,
        };
        assert_eq!(
            cargo_output_profile(&root, &workspace, &package, &supported).as_deref(),
            Some("release")
        );

        for name in [
            "CARGO_BUILD_TARGET",
            "CARGO_TARGET_DIR",
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
                cargo_output_profile(&root, &workspace, &package, &context),
                None
            );
        }

        for args in [
            vec!["--target=x86_64-unknown-linux-gnu".to_string()],
            vec!["--target-dir=other".to_string()],
            vec!["--config=build.target-dir='other'".to_string()],
            vec!["--future-layout-control".to_string()],
        ] {
            let context = toolchain::TaskIOContext {
                task_args: Some(&args),
                environment: &empty_environment,
            };
            assert_eq!(
                cargo_output_profile(&root, &workspace, &package, &context),
                None
            );
        }
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
        assert_eq!(app.internal_dependencies, vec!["lib-a".to_string()]);

        let lib_a = &crates[1];
        assert!(
            !lib_a.is_entrypoint(),
            "plain lib crate is not an entrypoint"
        );
        assert!(lib_a.deliverables.is_empty());
        // The dev-dep edge lib-a -> lib-a-test-util closes a cycle with the
        // normal edge lib-a-test-util -> lib-a, so it must be dropped.
        assert!(
            lib_a.internal_dependencies.is_empty(),
            "cycle-closing dev edge should be dropped, got {:?}",
            lib_a.internal_dependencies
        );

        let test_util = &crates[2];
        assert_eq!(test_util.internal_dependencies, vec!["lib-a".to_string()]);
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

        let toolchain = CargoToolchain::new(root.clone());
        let err = toolchain.discover_packages().await.unwrap_err();
        assert!(
            err.to_string().contains("[workspace.metadata]"),
            "the error must show the fix, got: {err}"
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
    fn test_discover_crates_skips_root_crate() {
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
            vec!["member"],
            "the root crate's directory is the whole repository, so it is not a package"
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
        assert_eq!(crates[0].internal_dependencies, vec!["helper".to_string()]);
    }

    #[test]
    fn test_compile_cache_env_routes_rustc_through_sccache() {
        let (_tmp, root) = tempdir_root();
        let toolchain = CargoToolchain::new(root);
        let endpoint = toolchain::CompileCacheEndpoint {
            url: "http://127.0.0.1:42123".to_string(),
            token: "proxy-token".to_string(),
            wrapper: "/path/to/turbo".to_string(),
            server_port: 46123,
        };
        assert_eq!(
            toolchain.compile_cache_env(&endpoint, &std::collections::HashMap::new()),
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
        let (_tmp, root) = tempdir_root();
        let toolchain = CargoToolchain::new(root);
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
        assert!(toolchain.compile_cache_env(&endpoint, &env).is_empty());

        // Any SCCACHE_* variable signals a user-managed sccache setup.
        let env = std::collections::HashMap::from([(
            "SCCACHE_GHA_ENABLED".to_string(),
            "true".to_string(),
        )]);
        assert!(toolchain.compile_cache_env(&endpoint, &env).is_empty());
    }

    #[test]
    fn test_compile_cache_env_tolerates_ambient_cargo_incremental() {
        // CI images commonly export CARGO_INCREMENTAL=0 (this repository's
        // own setup-environment action does). That is ambient hygiene, not
        // a competing compiler cache: the injection proceeds and the
        // explicit value is left alone.
        let (_tmp, root) = tempdir_root();
        let toolchain = CargoToolchain::new(root);
        let endpoint = toolchain::CompileCacheEndpoint {
            url: "http://127.0.0.1:42123".to_string(),
            token: "proxy-token".to_string(),
            wrapper: "/path/to/turbo".to_string(),
            server_port: 46123,
        };
        let env =
            std::collections::HashMap::from([("CARGO_INCREMENTAL".to_string(), "0".to_string())]);

        let vars = toolchain.compile_cache_env(&endpoint, &env);
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
        assert!(toolchain.compile_cache_env(&endpoint, &env).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_synthesizes_descriptors() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let toolchain = CargoToolchain::new(root.clone());
        assert_eq!(toolchain.id(), ToolchainId::RUST);

        let mut packages = toolchain.discover_packages().await.unwrap();
        packages.sort_by(|a, b| {
            a.descriptor
                .name
                .as_ref()
                .map(|name| name.as_inner())
                .cmp(&b.descriptor.name.as_ref().map(|name| name.as_inner()))
        });

        let names: Vec<&str> = packages
            .iter()
            .map(|p| p.descriptor.name.as_ref().unwrap().as_inner().as_str())
            .collect();
        assert_eq!(names, vec!["app", "fixture-ws", "lib-a", "lib-a-test-util"]);

        for package in &packages {
            let rustc = package
                .external_dependencies
                .as_ref()
                .and_then(|dependencies| {
                    dependencies
                        .iter()
                        .find(|dependency| dependency.key == "rustc")
                })
                .expect("compiler identity stamps every Cargo package");
            let mut lines = rustc.version.lines();
            assert!(lines.next().is_some_and(|line| line.starts_with("rustc ")));
            assert!(
                lines.any(|line| { line.starts_with("host: ") && line.len() > "host: ".len() })
            );
        }

        let app = &packages[0];
        assert_eq!(
            app.descriptor.dependencies.as_ref().unwrap()["lib-a"],
            "workspace:*"
        );
        assert_eq!(
            app.manifest_path,
            root.join_components(&["crates", "app", "Cargo.toml"])
        );

        // The synthetic workspace package is anchored at the root manifest
        // and depends on every crate.
        let workspace = &packages[1];
        assert_eq!(workspace.manifest_path, root.join_component(CARGO_TOML));
        let workspace_deps = workspace.descriptor.dependencies.as_ref().unwrap();
        assert_eq!(workspace_deps.len(), 3);
        assert!(workspace_deps.values().all(|v| v == "workspace:*"));

        // This all-local fixture has no external lockfile dependencies; the
        // compiler identity is the only external identity.
        let app_externals = app.external_dependencies.as_ref().unwrap();
        assert_eq!(app_externals.len(), 1);
        let lib_a_externals = packages[2].external_dependencies.as_ref().unwrap();
        assert_eq!(lib_a_externals.len(), 1);
        let workspace_externals = workspace.external_dependencies.as_ref().unwrap();
        assert_eq!(workspace_externals.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_empty_without_manifest() {
        let (_tmp, root) = tempdir_root();
        let toolchain = CargoToolchain::new(root);
        assert!(toolchain.discover_packages().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_empty_for_memberless_workspace() {
        let (_tmp, root) = tempdir_root();
        write(&root, &["Cargo.toml"], "[workspace]\nmembers = []\n");

        let toolchain = CargoToolchain::new(root);
        assert!(toolchain.discover_packages().await.unwrap().is_empty());
    }

    fn package_info(name: &str, manifest_rel: &str) -> crate::package_graph::PackageInfo {
        crate::package_graph::PackageInfo {
            package_json: PackageJson {
                name: Some(Spanned::new(name.to_string())),
                ..Default::default()
            },
            package_json_path: turbopath::AnchoredSystemPathBuf::from_raw(
                manifest_rel.replace('/', std::path::MAIN_SEPARATOR_STR),
            )
            .unwrap(),
            toolchain: ToolchainId::RUST,
            ..Default::default()
        }
    }

    fn os_args(args: &[&str]) -> Vec<std::ffi::OsString> {
        args.iter().map(std::ffi::OsString::from).collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_task_commands() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let toolchain = CargoToolchain::new(root.clone());
        // Discovery records the per-package details command resolution uses.
        toolchain.discover_packages().await.unwrap();

        let app = package_info("app", "crates/app/Cargo.toml");
        let lib_a = package_info("lib-a", "crates/lib-a/Cargo.toml");
        let workspace = package_info("fixture-ws", "Cargo.toml");

        // Entrypoint build: scoped to the crate, serialized on the cargo
        // group, run from the workspace root.
        let cmd = toolchain
            .task_command(&root, &app, "build", None, None)
            .unwrap()
            .expect("entrypoint build resolves");
        assert_eq!(cmd.args, os_args(&["build", "--package=app", "--locked"]));
        assert_eq!(cmd.cwd, root);
        assert_eq!(cmd.serial_group.as_deref(), Some("cargo"));

        // `run` is exempt from the serial group and forwards pass-through
        // args to the binary after `--`.
        let cmd = toolchain
            .task_command(&root, &app, "dev", Some(&["--port".to_string()]), None)
            .unwrap()
            .expect("entrypoint dev resolves to cargo run");
        assert_eq!(
            cmd.args,
            os_args(&["run", "--package=app", "--locked", "--", "--port"])
        );
        assert_eq!(cmd.serial_group, None);

        // Other subcommands attach pass-through args as cargo flags, no
        // separator.
        let cmd = toolchain
            .task_command(&root, &app, "build", Some(&["--release".to_string()]), None)
            .unwrap()
            .expect("entrypoint build resolves");
        assert_eq!(
            cmd.args,
            os_args(&["build", "--package=app", "--locked", "--release"])
        );

        // Libraries are no-ops; entrypoints do not run verification verbs.
        assert!(
            toolchain
                .task_command(&root, &lib_a, "build", None, None)
                .unwrap()
                .is_none()
        );
        assert!(
            toolchain
                .task_command(&root, &app, "test", None, None)
                .unwrap()
                .is_none()
        );

        // The workspace package hosts verification verbs at workspace scope.
        let cmd = toolchain
            .task_command(&root, &workspace, "lint", None, None)
            .unwrap()
            .expect("workspace lint resolves to clippy");
        assert_eq!(cmd.args, os_args(&["clippy", "--workspace", "--locked"]));
        assert_eq!(cmd.serial_group.as_deref(), Some("cargo"));

        // Harness-forwarding subcommands separate pass-through args with
        // `--`; e.g. `turbo test -- --nocapture` reaches the test harness.
        let cmd = toolchain
            .task_command(
                &root,
                &workspace,
                "test",
                Some(&["--nocapture".to_string()]),
                None,
            )
            .unwrap()
            .expect("workspace test resolves");
        assert_eq!(
            cmd.args,
            os_args(&["test", "--workspace", "--locked", "--", "--nocapture"])
        );
        assert!(
            toolchain
                .task_command(&root, &workspace, "build", None, None)
                .unwrap()
                .is_none(),
            "workspace-wide build would duplicate entrypoint builds"
        );

        // Display strings derive from the same tables.
        assert_eq!(
            toolchain.task_display_command(&app, "build").as_deref(),
            Some("cargo build --package=app --locked")
        );
        assert_eq!(
            toolchain
                .task_display_command(&workspace, "test")
                .as_deref(),
            Some("cargo test --workspace --locked")
        );
        assert_eq!(toolchain.task_display_command(&lib_a, "build"), None);

        assert_eq!(toolchain.task_defaults(&app, "run").cache, Some(false));
        assert_eq!(toolchain.task_defaults(&app, "dev").cache, Some(false));
        assert_eq!(toolchain.task_defaults(&app, "build").cache, None);
        assert_eq!(toolchain.task_defaults(&workspace, "test").cache, None);
        assert_eq!(toolchain.task_defaults(&lib_a, "run").cache, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_command_override_frame() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let toolchain = CargoToolchain::new(root.clone());
        toolchain.discover_packages().await.unwrap();

        let lib_a = package_info("lib-a", "crates/lib-a/Cargo.toml");
        let workspace = package_info("fixture-ws", "Cargo.toml");

        // An override applies to any crate — including libraries, which map
        // no verbs of their own. cwd is the package's directory, and an
        // argv still invoking cargo keeps the serial group (the group
        // exists because of cargo's build-directory lock).
        let override_argv = vec!["cargo".to_string(), "fuzz".to_string(), "run".to_string()];
        let cmd = toolchain
            .task_command(&root, &lib_a, "fuzz", None, Some(&override_argv))
            .unwrap()
            .expect("override defines the task for a library crate");
        assert_eq!(cmd.program, std::ffi::OsString::from("cargo"));
        assert_eq!(cmd.args, os_args(&["fuzz", "run"]));
        assert_eq!(cmd.cwd, root.join_components(&["crates", "lib-a"]));
        assert_eq!(cmd.serial_group.as_deref(), Some("cargo"));

        // A non-cargo argv drops the group; pass-through args append
        // verbatim (no separator injection).
        let override_argv = vec!["./scripts/test.sh".to_string()];
        let cmd = toolchain
            .task_command(
                &root,
                &workspace,
                "test",
                Some(&["--fast".to_string()]),
                Some(&override_argv),
            )
            .unwrap()
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

        let toolchain = CargoToolchain::new(root.clone());
        toolchain.discover_packages().await.unwrap();

        let app = package_info("app", "crates/app/Cargo.toml");
        let lib_a = package_info("lib-a", "crates/lib-a/Cargo.toml");
        let workspace = package_info("fixture-ws", "Cargo.toml");
        let environment = toolchain::TaskIOEnvironment::default();
        let context = toolchain::TaskIOContext {
            task_args: None,
            environment: &environment,
        };

        // defines_task mirrors the verb tables.
        assert!(toolchain.defines_task(&app, "build"));
        assert!(!toolchain.defines_task(&app, "test"));
        assert!(toolchain.defines_task(&workspace, "test"));
        assert!(!toolchain.defines_task(&lib_a, "build"));

        // Entrypoint build with automatic inputs: workspace files + the
        // dependency crate closure as inputs (own sources via default
        // hashing), deliverables as outputs.
        let deps = [&lib_a];
        let io = toolchain
            .derived_task_io(&app, "build", "../..", &deps, true, &context)
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
        let toolchain::DerivedOutputs::Resolved(outputs) = &io.outputs else {
            panic!("Cargo host outputs must remain resolved");
        };
        let workspace_details = toolchain.workspace_details().unwrap();
        let platform = target_platform(&workspace_details.host_target).unwrap();
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
        let unsupported = toolchain
            .derived_task_io(&app, "build", "../..", &deps, true, &unsupported_context)
            .expect("entrypoint build derives IO");
        assert_eq!(unsupported.outputs, toolchain::DerivedOutputs::Unavailable);

        // Explicit inputs without $TURBO_DEFAULT$: workspace files still
        // apply, but no closure globs and no default-hashing override.
        let io = toolchain
            .derived_task_io(&app, "build", "../..", &deps, false, &context)
            .expect("entrypoint build derives IO");
        assert!(io.input_globs.contains(&"../../Cargo.toml".to_string()));
        assert!(!io.input_globs.iter().any(|glob| glob.contains("lib-a")));
        assert_eq!(io.package_default_inputs, None);

        // Non-build entrypoint verbs cache no deliverables.
        let io = toolchain
            .derived_task_io(&app, "dev", "../..", &deps, true, &context)
            .expect("entrypoint dev derives IO");
        assert_eq!(io.outputs, toolchain::DerivedOutputs::Resolved(Vec::new()));

        // The workspace package hashes crate directories instead of the
        // repo root's default file set.
        let deps = [&app, &lib_a];
        let io = toolchain
            .derived_task_io(&workspace, "test", "", &deps, true, &context)
            .expect("workspace test derives IO");
        assert_eq!(io.package_default_inputs, Some(false));
        assert!(io.input_globs.contains(&"crates/app/**".to_string()));
        assert!(io.input_globs.contains(&"crates/lib-a/**".to_string()));
        assert!(io.input_globs.contains(&"Cargo.toml".to_string()));
        assert_eq!(io.outputs, toolchain::DerivedOutputs::Resolved(Vec::new()));

        // Libraries derive nothing.
        assert!(
            toolchain
                .derived_task_io(&lib_a, "build", "../..", &[], true, &context)
                .is_none()
        );
    }
}
