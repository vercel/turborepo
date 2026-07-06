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
//! A synthetic package named [`WORKSPACE_PACKAGE_NAME`], anchored at the
//! root `Cargo.toml` and depending on every crate, represents the workspace
//! itself; it will host workspace-scoped verification verbs (`cargo test
//! --workspace`, ...) once command resolution gains a toolchain surface.
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

/// Name of the synthetic package that represents the Cargo workspace itself.
/// A real workspace member with this name is skipped with a warning.
pub const WORKSPACE_PACKAGE_NAME: &str = "cargo";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to run `cargo metadata`: {0}")]
    MetadataSpawn(#[source] io::Error),
    #[error("`cargo metadata` failed: {stderr}")]
    Metadata { stderr: String },
    #[error("failed to parse `cargo metadata` output: {0}")]
    Parse(#[from] serde_json::Error),
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

/// Cargo-specific details for a discovered package, retained by the
/// [`CargoToolchain`] (keyed by package name) rather than attached to the
/// toolchain-neutral `PackageInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoPackageDetails {
    pub kind: CargoPackageKind,
    /// The crate's deliverable targets (empty for libraries and the
    /// workspace package).
    pub deliverables: Vec<Deliverable>,
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

/// The display string for a Cargo task's command, derived from the same
/// tables as execution so it cannot drift.
pub fn display_command(kind: CargoPackageKind, task: &str, package: &str) -> Option<String> {
    let subcommand = task_subcommand(kind, task)?;
    Some(match kind {
        CargoPackageKind::Entrypoint => format!("cargo {subcommand} --package={package}"),
        CargoPackageKind::Workspace => format!("cargo {subcommand} --workspace"),
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

/// The Cargo toolchain. Registered in the
/// [`crate::toolchain::ToolchainRegistry`] when
/// `futureFlags.experimentalCargoWorkspaces` is enabled and the repository
/// root contains a `Cargo.toml`.
pub struct CargoToolchain {
    repo_root: AbsoluteSystemPathBuf,
    /// Per-package details recorded during discovery, consumed by command
    /// resolution. Keyed by package name.
    details: std::sync::Mutex<HashMap<String, CargoPackageDetails>>,
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
}

impl Toolchain for CargoToolchain {
    fn id(&self) -> ToolchainId {
        ToolchainId::CARGO
    }

    fn task_command(
        &self,
        repo_root: &AbsoluteSystemPath,
        package: &crate::package_graph::PackageInfo,
        task: &str,
        pass_through_args: Option<&[String]>,
    ) -> Result<Option<toolchain::TaskCommand>, toolchain::Error> {
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
        let mut args: Vec<std::ffi::OsString> = vec![subcommand.into(), scope.into()];
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

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            // Discovery spawns `cargo metadata` synchronously, so keep it off
            // the async runtime like the JavaScript manifest-parsing path.
            let crates =
                turborepo_rayon_compat::block_in_place(|| discover_crates(&self.repo_root))
                    .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;

            // Each crate becomes a package. Internal dependencies are
            // expressed as `workspace:*` specifiers in the synthesized
            // descriptor so the existing dependency splitter wires
            // crate->crate edges (powering `--filter`/`--affected`).
            // Discovery only reports dependencies on other discovered
            // crates, so every synthesized specifier resolves internally and
            // Cargo edges never leak into unresolved externals.
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
                self.record_details(
                    cargo_crate.name.clone(),
                    CargoPackageDetails {
                        kind,
                        deliverables: cargo_crate.deliverables,
                    },
                );
                crate_names.push(cargo_crate.name.clone());
                packages.push(DiscoveredPackage {
                    descriptor: PackageJson {
                        name: Some(Spanned::new(cargo_crate.name)),
                        dependencies: Some(dependencies),
                        ..Default::default()
                    },
                    manifest_path: cargo_crate.manifest_path,
                });
            }

            // The synthetic workspace package, anchored at the root
            // Cargo.toml. It depends on every crate so `--affected` and
            // dependent-filters propagate crate changes to it.
            if !crate_names.is_empty() {
                self.record_details(
                    WORKSPACE_PACKAGE_NAME.to_string(),
                    CargoPackageDetails {
                        kind: CargoPackageKind::Workspace,
                        deliverables: Vec::new(),
                    },
                );
                let dependencies = crate_names
                    .into_iter()
                    .map(|name| (name, "workspace:*".to_string()))
                    .collect();
                packages.push(DiscoveredPackage {
                    descriptor: PackageJson {
                        name: Some(Spanned::new(WORKSPACE_PACKAGE_NAME.to_string())),
                        dependencies: Some(dependencies),
                        ..Default::default()
                    },
                    manifest_path: self.repo_root.join_component(CARGO_TOML),
                });
            }

            Ok(packages)
        })
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
}

impl CargoCrate {
    /// Whether this crate is an entrypoint: it produces deliverable
    /// artifacts.
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

    Ok(connect_crates(parse_members(
        repo_root,
        &root_manifest_path,
        metadata,
    )))
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
                let dir = metadata_path(&path)?;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_synthesizes_descriptors() {
        let (_tmp, root) = tempdir_root();
        write_fixture_workspace(&root);

        let toolchain = CargoToolchain::new(root.clone());
        assert_eq!(toolchain.id(), ToolchainId::CARGO);

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
        assert_eq!(names, vec!["app", "cargo", "lib-a", "lib-a-test-util"]);

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
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_empty_without_manifest() {
        let (_tmp, root) = tempdir_root();
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
            toolchain: ToolchainId::CARGO,
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
        let workspace = package_info(WORKSPACE_PACKAGE_NAME, "Cargo.toml");

        // Entrypoint build: scoped to the crate, serialized on the cargo
        // group, run from the workspace root.
        let cmd = toolchain
            .task_command(&root, &app, "build", None)
            .unwrap()
            .expect("entrypoint build resolves");
        assert_eq!(cmd.args, os_args(&["build", "--package=app"]));
        assert_eq!(cmd.cwd, root);
        assert_eq!(cmd.serial_group.as_deref(), Some("cargo"));

        // `run` is exempt from the serial group and forwards pass-through
        // args to the binary after `--`.
        let cmd = toolchain
            .task_command(&root, &app, "dev", Some(&["--port".to_string()]))
            .unwrap()
            .expect("entrypoint dev resolves to cargo run");
        assert_eq!(cmd.args, os_args(&["run", "--package=app", "--", "--port"]));
        assert_eq!(cmd.serial_group, None);

        // Other subcommands attach pass-through args as cargo flags, no
        // separator.
        let cmd = toolchain
            .task_command(&root, &app, "build", Some(&["--release".to_string()]))
            .unwrap()
            .expect("entrypoint build resolves");
        assert_eq!(cmd.args, os_args(&["build", "--package=app", "--release"]));

        // Libraries are no-ops; entrypoints do not run verification verbs.
        assert!(
            toolchain
                .task_command(&root, &lib_a, "build", None)
                .unwrap()
                .is_none()
        );
        assert!(
            toolchain
                .task_command(&root, &app, "test", None)
                .unwrap()
                .is_none()
        );

        // The workspace package hosts verification verbs at workspace scope.
        let cmd = toolchain
            .task_command(&root, &workspace, "lint", None)
            .unwrap()
            .expect("workspace lint resolves to clippy");
        assert_eq!(cmd.args, os_args(&["clippy", "--workspace"]));
        assert_eq!(cmd.serial_group.as_deref(), Some("cargo"));

        // Harness-forwarding subcommands separate pass-through args with
        // `--`; e.g. `turbo test -- --nocapture` reaches the test harness.
        let cmd = toolchain
            .task_command(
                &root,
                &workspace,
                "test",
                Some(&["--nocapture".to_string()]),
            )
            .unwrap()
            .expect("workspace test resolves");
        assert_eq!(
            cmd.args,
            os_args(&["test", "--workspace", "--", "--nocapture"])
        );
        assert!(
            toolchain
                .task_command(&root, &workspace, "build", None)
                .unwrap()
                .is_none(),
            "workspace-wide build would duplicate entrypoint builds"
        );

        // Display strings derive from the same tables.
        assert_eq!(
            toolchain.task_display_command(&app, "build").as_deref(),
            Some("cargo build --package=app")
        );
        assert_eq!(
            toolchain
                .task_display_command(&workspace, "test")
                .as_deref(),
            Some("cargo test --workspace")
        );
        assert_eq!(toolchain.task_display_command(&lib_a, "build"), None);
    }
}
