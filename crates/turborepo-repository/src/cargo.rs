//! Cargo workspace discovery and Turborepo's knowledge of Cargo.
//!
//! Turborepo does not replace Cargo — Cargo is itself a build system with
//! its own dependency graph, scheduler, and incremental cache. Turborepo's
//! job is orchestration: decide *which* crates are in scope and *whether*
//! anything changed, then hand the work to a single `cargo` invocation and
//! get out of the way.
//!
//! Discovery shells out to `cargo metadata`, because Cargo is the only
//! correct implementation of its own workspace-membership semantics (member
//! globs, automatic path-dependency members, excludes, target-specific
//! dependency tables, renames). Crates are classified into two shapes:
//!
//! * **Entrypoints** — crates with `bin`/`cdylib`/`staticlib` targets: the
//!   deliverables of the workspace. These get real `build`/`run` tasks (`cargo
//!   build --package=<crate>`); Cargo builds their dependency closure
//!   internally in one process.
//! * **Libraries** — everything else. They exist in the package graph (so
//!   `--filter` and `--affected` propagate through them) but get no commands:
//!   being buildable is not the same as being an entrypoint.
//!
//! Verification verbs (`test`, `check`, `lint`, `doc`, `bench`) run once at
//! workspace scope (`cargo <verb> --workspace`) on a synthetic package named
//! [`WORKSPACE_PACKAGE_NAME`], matching how Cargo users actually run them.
//!
//! This module also owns the task-name → Cargo verb mapping (shared by the
//! executor and run summaries so display can't drift from execution) and the
//! input globs / env vars that participate in Cargo task hashes.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
};

use serde::Deserialize;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

/// The conventional file name for a Cargo manifest.
pub const CARGO_TOML: &str = "Cargo.toml";

/// The conventional file name for a Cargo lockfile.
pub const CARGO_LOCK: &str = "Cargo.lock";

/// Name of the synthetic package that hosts workspace-scoped Cargo tasks
/// (`cargo#test`, `cargo#lint`, ...). A real workspace member with this name
/// collides and hard-errors, like any other duplicate package name.
pub const WORKSPACE_PACKAGE_NAME: &str = "cargo";

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
/// workspace scope (the synthetic [`WORKSPACE_PACKAGE_NAME`] package).
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

/// The command displayed for a Cargo task in run summaries and dry-runs.
/// Derived from the same tables the executor uses, so summaries always show
/// the command that actually runs.
pub fn display_command(kind: CargoPackageKind, task: &str, package: &str) -> Option<String> {
    let verb = task_subcommand(kind, task)?;
    Some(match kind {
        CargoPackageKind::Entrypoint => format!("cargo {verb} --package={package}"),
        CargoPackageKind::Workspace => format!("cargo {verb} --workspace"),
        CargoPackageKind::Library => return None,
    })
}

/// Whether pass-through args for this Cargo subcommand must be placed after
/// a `--` separator.
///
/// `cargo test`/`bench`/`run` forward post-`--` args to the test harness or
/// binary, and `cargo clippy` forwards them to rustc. The remaining verbs
/// (`build`, `check`, `doc`) reject a `--` separator outright, so their
/// pass-through args are appended directly as cargo flags (e.g.
/// `turbo build -- --release` becomes `cargo build --package=x --release`).
pub fn pass_through_uses_separator(subcommand: &str) -> bool {
    matches!(subcommand, "test" | "bench" | "run" | "clippy")
}

/// Environment variables that change what Cargo builds or where it writes
/// artifacts. These participate in a crate task's hash so flipping them
/// invalidates the cache. `RUSTC_WRAPPER` is included so enabling a compile
/// cache like sccache invalidates prior task results.
pub const HASHED_ENV_VARS: &[&str] = &[
    "RUSTFLAGS",
    "RUSTC_WRAPPER",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET",
];

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
/// invalidates the crates that actually depend on it. The compiler version
/// participates the same way (see [`rustc_version`]). For fine-grained
/// remote compile caching, use sccache (`RUSTC_WRAPPER`) — that is the layer
/// where per-compilation caching is sound, and it cooperates with Cargo
/// rather than competing with it.
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

/// The version of the Rust compiler that Cargo will invoke, as a hashable
/// external-dependency identity, or `None` (with a warning) when rustc
/// can't be queried.
///
/// Run from `repo_root` so rustup's shim resolves `rust-toolchain`
/// overrides the same way a task's `cargo` invocation will. Participating
/// in the external-dependency hash means compiling with a different
/// toolchain never restores another toolchain's artifacts — the gap that
/// made remote cache sharing unsound when no toolchain file was committed.
pub fn rustc_version(repo_root: &AbsoluteSystemPath) -> Option<turborepo_lockfiles::Package> {
    let output = std::process::Command::new("rustc")
        .arg("--version")
        .current_dir(repo_root.as_std_path())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!version.is_empty()).then_some(turborepo_lockfiles::Package {
                key: "rustc".to_string(),
                version,
            })
        }
        Ok(output) => {
            tracing::warn!(
                "`rustc --version` failed; the compiler version will not participate in Cargo \
                 task hashes: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                "unable to run `rustc --version`; the compiler version will not participate in \
                 Cargo task hashes: {error}"
            );
            None
        }
    }
}

/// Per-crate external dependency closures from Cargo.lock, for the crates'
/// external-dependency hashes.
///
/// A missing Cargo.lock yields an empty map (the workspace is unpinned;
/// Cargo will create the lockfile on first build). An unreadable or
/// unparsable lockfile is a hard error — silently hashing nothing would be
/// unsound.
pub fn external_closures(
    repo_root: &AbsoluteSystemPath,
    members: &[String],
) -> Result<HashMap<String, HashSet<turborepo_lockfiles::Package>>, Error> {
    let lock_path = repo_root.join_component(CARGO_LOCK);
    let contents = match lock_path.read_to_string() {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(HashMap::new());
        }
        Err(error) => return Err(Error::LockfileRead(error)),
    };
    Ok(turborepo_lockfiles::cargo_external_closures(
        &contents, members,
    )?)
}

/// Output globs for an entrypoint crate's `build` task: the artifacts Cargo
/// places in `target/<profile>/` — uplifted binaries plus cdylib/staticlib
/// libraries. These are the workspace's deliverables — the only artifacts
/// worth caching at the task level. Cargo's internal `target/` state (deps,
/// fingerprints) is deliberately not cached: it is Cargo's own incremental
/// cache, and tarballing it fights Cargo instead of leaning on it.
///
/// The profile segment is a wildcard, so `--release` and custom profiles
/// (`--profile=my-profile`) are cached without configuration — pass-through
/// args participate in the task hash, so each profile gets its own cache
/// entry. Every platform's file name is emitted for each deliverable
/// (`.so`, `.dylib`, `.dll`, ...); globs that match nothing contribute
/// nothing, and task hashes already segment by platform via the artifacts
/// themselves.
///
/// Builds using `CARGO_TARGET_DIR` or `--target <triple>` write elsewhere
/// (`CARGO_TARGET_DIR` and `CARGO_BUILD_TARGET` are hashed, but the
/// artifact locations differ); declare explicit `outputs` in turbo.json for
/// those layouts.
pub fn deliverable_output_globs(prefix: &str, deliverables: &[Deliverable]) -> Vec<String> {
    deliverables
        .iter()
        .flat_map(|deliverable| {
            let name = &deliverable.name;
            let basenames = match deliverable.kind {
                DeliverableKind::Bin => vec![name.clone(), format!("{name}.exe")],
                DeliverableKind::Cdylib => vec![
                    format!("lib{name}.so"),
                    format!("lib{name}.dylib"),
                    format!("{name}.dll"),
                ],
                DeliverableKind::Staticlib => {
                    vec![format!("lib{name}.a"), format!("{name}.lib")]
                }
            };
            basenames
                .into_iter()
                .map(move |basename| join_prefix(prefix, &format!("target/*/{basename}")))
        })
        .collect()
}

fn join_prefix(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

/// Whether `name` is a valid Cargo package name (`[A-Za-z0-9_-]+`).
///
/// Cargo enforces this itself, but manifests are untrusted input and crate
/// names flow into `cargo --package=<name>` argv and cache output glob
/// patterns, so we validate at the discovery boundary instead of relying on
/// downstream consumers.
pub fn is_valid_crate_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
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
    /// The synthetic [`WORKSPACE_PACKAGE_NAME`] package hosting
    /// workspace-scoped verification verbs (`cargo test --workspace`, ...).
    Workspace,
}

/// A deliverable artifact an entrypoint crate produces: the target name plus
/// the artifact flavor, which determines the file names Cargo writes to
/// `target/debug/`.
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

/// Cargo-specific details attached to a [`super::package_graph::PackageInfo`]
/// when its toolchain is Cargo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoPackageDetails {
    pub kind: CargoPackageKind,
    /// The crate's deliverable targets (empty for libraries and the
    /// workspace package). Used to derive cacheable output paths.
    pub deliverables: Vec<Deliverable>,
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
}

impl CargoCrate {
    /// Whether this crate is an entrypoint: it produces deliverable
    /// artifacts, so it gets real `build`/`run` tasks.
    pub fn is_entrypoint(&self) -> bool {
        !self.deliverables.is_empty()
    }
}

/// Discover all Rust crates in the Cargo workspace rooted at `repo_root` by
/// invoking `cargo metadata --no-deps`.
///
/// Returns an empty vec if `repo_root` has no `Cargo.toml`. A root manifest
/// that exists but that Cargo rejects is an error — the user opted into
/// Cargo support, so silently discovering nothing would be misleading.
/// `--no-deps` skips registry resolution, so no lockfile or network access
/// is required.
///
/// Crates whose manifests live outside the repository root, or whose names
/// are invalid, are skipped with a warning. A `[package]` in the root
/// manifest is skipped too: its directory would be the entire repository.
pub fn discover_crates(repo_root: &AbsoluteSystemPath) -> Result<Vec<CargoCrate>, Error> {
    let root_manifest_path = repo_root.join_component(CARGO_TOML);
    if !root_manifest_path.exists() {
        return Ok(Vec::new());
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

    connect_crates(parse_members(repo_root, &root_manifest_path, metadata))
}

/// A workspace member parsed from `cargo metadata`, before dependency edges
/// are resolved to crate names.
struct ParsedCrate {
    name: String,
    manifest_path: AbsoluteSystemPathBuf,
    dependencies: Vec<ResolvedDep>,
    deliverables: Vec<Deliverable>,
}

/// A path dependency resolved to the directory Cargo reports for it.
struct ResolvedDep {
    dir: AbsoluteSystemPathBuf,
    dev: bool,
}

fn parse_members(
    repo_root: &AbsoluteSystemPath,
    root_manifest_path: &AbsoluteSystemPath,
    metadata: Metadata,
) -> Vec<ParsedCrate> {
    let mut parsed = Vec::new();
    for package in metadata.packages {
        let Ok(manifest_path) = AbsoluteSystemPathBuf::new(package.manifest_path.clone()) else {
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
        if package.name == WORKSPACE_PACKAGE_NAME {
            tracing::warn!(
                "skipping Cargo crate {:?}: the name is reserved for Turborepo's synthetic \
                 workspace package",
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
                let dir = AbsoluteSystemPathBuf::new(path).ok()?;
                Some(ResolvedDep {
                    dir,
                    dev: dep.kind.as_deref() == Some("dev"),
                })
            })
            .collect();

        parsed.push(ParsedCrate {
            name: package.name,
            manifest_path,
            dependencies,
            deliverables,
        });
    }
    parsed
}

/// Resolve dependency edges to crate names by manifest directory and drop
/// dev-dependency edges that would form a cycle (Cargo permits dev-dep
/// cycles; the package graph is a DAG).
fn connect_crates(parsed: Vec<ParsedCrate>) -> Result<Vec<CargoCrate>, Error> {
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

    Ok(parsed
        .into_iter()
        .map(|parsed_crate| CargoCrate {
            internal_dependencies: edges.remove(parsed_crate.name.as_str()).unwrap_or_default(),
            name: parsed_crate.name,
            manifest_path: parsed_crate.manifest_path,
            deliverables: parsed_crate.deliverables,
        })
        .collect())
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

#[cfg(test)]
mod test {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    fn tempdir_root() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::new(
            tmp.path()
                .canonicalize()
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

    /// Write a small workspace: `app` (bin) depends on `lib-a` (lib), plus a
    /// dev-dep cycle between `lib-a` and `lib-a-test-util`.
    fn write_fixture_workspace(root: &AbsoluteSystemPathBuf) {
        write(
            root,
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
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
    }

    #[test]
    fn test_discover_crates_via_metadata() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let mut crates = discover_crates(&root).unwrap();
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
        assert!(discover_crates(&root).unwrap().is_empty());
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

        let crates = discover_crates(&root).unwrap();
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

        let mut crates = discover_crates(&root).unwrap();
        crates.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            crates.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["app", "helper"]
        );
        assert_eq!(crates[0].internal_dependencies, vec!["helper".to_string()]);
    }

    #[test]
    fn test_entrypoint_subcommand_mapping() {
        assert_eq!(entrypoint_subcommand("build"), Some("build"));
        assert_eq!(entrypoint_subcommand("run"), Some("run"));
        assert_eq!(entrypoint_subcommand("dev"), Some("run"));
        assert_eq!(entrypoint_subcommand("test"), None);
        assert_eq!(entrypoint_subcommand("lint"), None);
    }

    #[test]
    fn test_workspace_subcommand_mapping() {
        assert_eq!(
            workspace_subcommand("build"),
            None,
            "build is entrypoint-scoped; a workspace build would duplicate it"
        );
        assert_eq!(workspace_subcommand("test"), Some("test"));
        assert_eq!(workspace_subcommand("check"), Some("check"));
        assert_eq!(workspace_subcommand("lint"), Some("clippy"));
        assert_eq!(workspace_subcommand("clippy"), Some("clippy"));
        assert_eq!(workspace_subcommand("doc"), Some("doc"));
        assert_eq!(workspace_subcommand("docs"), Some("doc"));
        assert_eq!(workspace_subcommand("bench"), Some("bench"));
        assert_eq!(workspace_subcommand("run"), None);
        assert_eq!(workspace_subcommand("deploy"), None);
    }

    #[test]
    fn test_library_tasks_are_noops() {
        for task in ["build", "test", "run", "lint"] {
            assert_eq!(task_subcommand(CargoPackageKind::Library, task), None);
        }
    }

    #[test]
    fn test_display_command_matches_execution_tables() {
        assert_eq!(
            display_command(CargoPackageKind::Entrypoint, "build", "app").as_deref(),
            Some("cargo build --package=app")
        );
        assert_eq!(
            display_command(CargoPackageKind::Workspace, "lint", WORKSPACE_PACKAGE_NAME).as_deref(),
            Some("cargo clippy --workspace")
        );
        assert_eq!(
            display_command(CargoPackageKind::Entrypoint, "lint", "app"),
            None
        );
        assert_eq!(
            display_command(CargoPackageKind::Library, "build", "lib-a"),
            None
        );
    }

    #[test]
    fn test_pass_through_separator_per_verb() {
        // Harness-forwarding verbs take args after `--`.
        for verb in ["test", "bench", "run", "clippy"] {
            assert!(pass_through_uses_separator(verb), "{verb}");
        }
        // These verbs hard-error on a `--` separator.
        for verb in ["build", "check", "doc"] {
            assert!(!pass_through_uses_separator(verb), "{verb}");
        }
    }

    #[test]
    fn test_hash_input_globs_prefixing() {
        assert_eq!(
            hash_input_globs("../.."),
            vec![
                "../../Cargo.toml",
                "../../.cargo/config.toml",
                "../../.cargo/config",
                "../../rust-toolchain.toml",
                "../../rust-toolchain",
            ]
        );
        assert_eq!(hash_input_globs("")[0], "Cargo.toml");
        // Cargo.lock is hashed per-crate via external dependency closures,
        // not as a global file input.
        assert!(!hash_input_globs("").iter().any(|g| g.contains("lock")));
    }

    #[test]
    fn test_deliverable_output_globs() {
        let deliverables = vec![
            Deliverable {
                name: "app".to_string(),
                kind: DeliverableKind::Bin,
            },
            Deliverable {
                name: "my_native".to_string(),
                kind: DeliverableKind::Cdylib,
            },
            Deliverable {
                name: "my_archive".to_string(),
                kind: DeliverableKind::Staticlib,
            },
        ];
        assert_eq!(
            deliverable_output_globs("../..", &deliverables),
            vec![
                "../../target/*/app",
                "../../target/*/app.exe",
                "../../target/*/libmy_native.so",
                "../../target/*/libmy_native.dylib",
                "../../target/*/my_native.dll",
                "../../target/*/libmy_archive.a",
                "../../target/*/my_archive.lib",
            ]
        );
        assert!(deliverable_output_globs("../..", &[]).is_empty());
    }

    #[test]
    fn test_is_valid_crate_name() {
        assert!(is_valid_crate_name("my-crate"));
        assert!(is_valid_crate_name("my_crate2"));
        assert!(!is_valid_crate_name(""));
        assert!(!is_valid_crate_name("../escape"));
        assert!(!is_valid_crate_name("a*b"));
        assert!(!is_valid_crate_name("a b"));
    }
}
