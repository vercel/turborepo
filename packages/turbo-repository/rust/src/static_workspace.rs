use std::collections::HashSet;

use napi::Error;
use napi_derive::napi;
use turbopath::AnchoredSystemPath;
use turborepo_repository::package_graph::PackageName;

use crate::{Workspace, internal::DiscoveryMode};

/// A real package identified from manifests without running a language
/// toolchain. No dependency edges or task metadata are implied by this
/// inventory entry.
#[napi(object)]
pub struct StaticPackage {
    pub name: String,
    pub absolute_path: String,
    pub relative_path: String,
    /// Native manifest path, relative to the workspace root.
    pub manifest_path: String,
    /// Language ecosystem, such as javascript, rust, python, or go.
    pub toolchain: String,
}

/// A contributed workspace root, separate from the inventory of real packages.
#[napi(object)]
pub struct StaticWorkspaceRoot {
    pub toolchain: String,
    pub kind: String,
    pub relative_path: String,
}

/// Packages that may be affected, including transitive input dependents.
#[napi(object)]
pub struct StaticAffectedPackages {
    pub packages: Vec<StaticPackage>,
    /// True when incomplete metadata, custom global inputs, or a topology
    /// change required returning all packages rather than a narrower closure.
    pub conservative: bool,
}

/// Subprocess-free package inventory and conservative affectedness.
///
/// This is deliberately separate from Workspace: native dependency edges and
/// tasks are not loaded and cannot be requested through this object. It uses
/// the same JavaScript workspace-root inference and toolchain feature flags as
/// Workspace.find. The repository native addon is still required, but language
/// toolchain executables (cargo, uv, go, etc.) are not.
#[napi]
pub struct StaticWorkspace {
    workspace: Workspace,
}

#[napi]
impl StaticWorkspace {
    #[napi(factory)]
    pub async fn find(path: Option<String>) -> Result<Self, Error> {
        let workspace = Workspace::find_with_mode(path, DiscoveryMode::Static).await?;
        Ok(Self { workspace })
    }

    #[napi(getter)]
    pub fn absolute_path(&self) -> String {
        self.workspace.absolute_path.clone()
    }

    /// Whether package dependency relationships are fully loaded. This does
    /// not claim completeness of task-level inputs or execution contracts.
    #[napi(getter)]
    pub fn dependency_graph_complete(&self) -> Result<bool, Error> {
        Ok(self.workspace.graph()?.unloaded_owners().is_empty())
    }

    /// Whether declared local package inputs can be inferred without language
    /// executables. This is independent of native task metadata completeness.
    #[napi(getter)]
    pub fn affectedness_complete(&self) -> bool {
        self.workspace
            .static_affectedness
            .as_ref()
            .is_some_and(|knowledge| knowledge.is_complete())
    }

    /// Toolchains with inventoried scopes whose authoritative metadata is
    /// absent.
    #[napi(getter)]
    pub fn unloaded_toolchains(&self) -> Result<Vec<String>, Error> {
        Ok(self
            .workspace
            .graph()?
            .unloaded_owners()
            .into_iter()
            .map(|id| id.to_string())
            .collect())
    }

    /// Lists real packages, excluding root/aggregate execution scopes.
    /// Co-located packages remain distinct. Sorted by directory, toolchain,
    /// and name.
    #[napi]
    pub async fn find_packages(&self) -> Result<Vec<StaticPackage>, Error> {
        self.inventory()
    }

    #[napi]
    pub fn workspace_roots(&self) -> Result<Vec<StaticWorkspaceRoot>, Error> {
        let graph = self.workspace.graph()?;
        let mut roots = graph
            .repository_discovery_snapshot()
            .workspace_roots
            .into_iter()
            .map(|root| {
                Ok(StaticWorkspaceRoot {
                    toolchain: root.toolchain.to_string(),
                    kind: root.kind,
                    relative_path: graph
                        .repo_root()
                        .anchor(&root.path)
                        .map_err(|error| Error::from_reason(error.to_string()))?
                        .to_string(),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        roots.sort_by(|a, b| {
            (&a.toolchain, &a.kind, &a.relative_path).cmp(&(
                &b.toolchain,
                &b.kind,
                &b.relative_path,
            ))
        });
        Ok(roots)
    }

    /// Returns candidates from workspace-relative changed paths, including
    /// deleted paths. Does not read Git history or execute subprocesses.
    ///
    /// Cargo path dependencies and uv workspace/path sources contribute static
    /// input relationships, including optional and conditional inputs. Source
    /// changes select their owners and transitive dependents across ecosystems.
    /// Native lockfile edits invalidate that workspace and its dependents.
    /// Manifest edits fall back to all packages: removed scopes can erase
    /// cross-language relationships. Unresolved static inputs, matching global
    /// inputs, and JavaScript topology edits also fall back to all packages.
    /// Native task metadata stays unloaded. This does not analyze arbitrary
    /// build-script reads or external resolution.
    #[napi]
    pub async fn affected_candidates(
        &self,
        files: Vec<String>,
    ) -> Result<StaticAffectedPackages, Error> {
        let files = normalize_paths(files)?;
        if files.is_empty() {
            return Ok(StaticAffectedPackages {
                packages: Vec::new(),
                conservative: false,
            });
        }
        let graph = self.workspace.graph()?;
        if !self.affectedness_complete()
            || turborepo_repository::static_dependencies::global_inputs_changed(
                &self.workspace.global_inputs,
                &files,
            )
            || files.iter().any(|file| is_topology_change(file))
        {
            return Ok(StaticAffectedPackages {
                packages: self.inventory()?,
                conservative: true,
            });
        }
        let knowledge = self
            .workspace
            .static_affectedness
            .as_ref()
            .ok_or_else(|| Error::from_reason("static dependency knowledge unavailable"))?;
        let mut seeds = Vec::new();
        let mut ordinary_files = Vec::new();
        for file in files {
            let path = AnchoredSystemPath::new(&file)
                .map_err(|error| Error::from_reason(error.to_string()))?;
            let native = knowledge.seeds_for_path(&graph.repo_root().resolve(path));
            if native.is_empty() {
                ordinary_files.push(file);
            } else {
                seeds.extend(native);
            }
        }
        // Keep native workspace invalidations scoped; ordinary paths still use
        // the shared package mapper, including its co-located ownership rules.
        if !ordinary_files.is_empty() {
            seeds.extend(
                self.workspace
                    .affected_packages(ordinary_files, None, Some(false))
                    .await?
                    .into_iter()
                    .map(|package| PackageName::Other(package.name)),
            );
        }
        let affected = knowledge
            .affected_by(graph, &seeds)
            .map_err(|error| Error::from_reason(error.to_string()))?;
        let names = affected
            .into_iter()
            .map(|name| name.to_string())
            .collect::<HashSet<_>>();
        Ok(StaticAffectedPackages {
            packages: self
                .inventory()?
                .into_iter()
                .filter(|package| names.contains(&package.name))
                .collect(),
            conservative: false,
        })
    }
}

impl StaticWorkspace {
    fn inventory(&self) -> Result<Vec<StaticPackage>, Error> {
        let graph = self.workspace.graph()?;
        let mut packages = graph
            .package_task_contexts()
            .filter(|context| graph.is_real_package(context.package()))
            .map(|context| {
                let manifest = graph
                    .package_definition_path(context.package())
                    .ok_or_else(|| {
                        Error::from_reason("inventoried package has no manifest path")
                    })?;
                let toolchain = context.toolchain().ok_or_else(|| {
                    Error::from_reason("inventoried package has no toolchain provenance")
                })?;
                Ok(StaticPackage {
                    name: context.package().to_string(),
                    absolute_path: graph.repo_root().resolve(context.directory()).to_string(),
                    relative_path: context.directory().to_string(),
                    manifest_path: manifest.to_string(),
                    toolchain: toolchain.to_string(),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        packages.sort_by(|a, b| {
            (&a.relative_path, &a.toolchain, &a.name).cmp(&(
                &b.relative_path,
                &b.toolchain,
                &b.name,
            ))
        });
        Ok(packages)
    }
}

fn normalize_paths(files: Vec<String>) -> Result<Vec<String>, Error> {
    files
        .into_iter()
        .map(|file| {
            if std::path::Path::new(&file).is_absolute() {
                return Err(Error::from_reason(
                    "changed paths must be relative to the workspace root",
                ));
            }
            let path = AnchoredSystemPath::new(&file)
                .map_err(|error| Error::from_reason(error.to_string()))?
                .clean();
            let path = path.to_string();
            if path == ".." || path.starts_with(&format!("..{}", std::path::MAIN_SEPARATOR)) {
                return Err(Error::from_reason(
                    "changed paths must not escape the workspace root",
                ));
            }
            Ok(path)
        })
        .collect()
}

fn is_topology_change(file: &str) -> bool {
    let path = std::path::Path::new(file);
    if matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("Cargo.toml" | "pyproject.toml")
    ) {
        return true;
    }
    path.parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
        && matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some(
                "package.json"
                    | "package-lock.json"
                    | "pnpm-lock.yaml"
                    | "pnpm-workspace.yaml"
                    | "yarn.lock"
                    | "bun.lock"
                    | "bun.lockb"
            )
        )
}
