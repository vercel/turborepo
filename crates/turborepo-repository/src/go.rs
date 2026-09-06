//! The Go toolchain: modules discovered from a `go.work` workspace.
//!
//! Turborepo does not replace the Go toolchain — Go owns module resolution,
//! compilation, and testing. Turborepo's job is orchestration: decide *which*
//! modules are in scope and how they relate, using authoritative output from
//! the `go` command.
//!
//! Discovery reads workspace membership from `go work edit -json`, module paths
//! from `go mod edit -json`, and internal relationships from `go mod graph`.
//!
//! Support is experimental and gated behind `futureFlags.experimentalGoWorkspaces`.

use std::{
    collections::{HashMap, HashSet},
    io,
    path::Path,
    process::Command,
};

use serde::Deserialize;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    package_json::PackageJson,
    relationships::{DependencyKind, Relationship},
    toolchain::{
        self, DiscoverPackagesFuture, DiscoveredPackage, DiscoveredPackages, RepositoryContributor,
        ToolchainId, WorkspaceRoot,
    },
};

/// The conventional file name for a Go workspace definition.
pub const GO_WORK: &str = "go.work";

/// The conventional file name for a Go module manifest.
pub const GO_MOD: &str = "go.mod";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to run `{command}`: {source}")]
    CommandSpawn {
        command: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("`{command}` failed: {stderr}")]
    CommandFailed {
        command: &'static str,
        stderr: String,
    },
    #[error("failed to parse `{command}` output: {source}")]
    CommandParse {
        command: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("root {path} is not a Go workspace (missing go.work)")]
    NotAWorkspace { path: String },
    #[error("go.work lists no workspace modules")]
    EmptyWorkspace,
    #[error("go.work member at {path} is outside the repository")]
    MemberOutsideRepository { path: String },
    #[error("go.work member at {path} has no go.mod")]
    MissingGoMod { path: String },
    #[error("go.mod at {path} has no module path")]
    MissingModulePath { path: String },
    #[error(
        "duplicate Go module identity {module_path:?} (also declared at {other_manifest})"
    )]
    DuplicateModuleIdentity {
        module_path: String,
        other_manifest: String,
    },
    #[error(
        "go.work member at {path} resolves to module {module_path:?}, which collides with the \
         repository root module definition"
    )]
    RootDefinitionCollision {
        path: String,
        module_path: String,
    },
    #[error(
        "local Go module {module_path:?} at {manifest_path} is outside the repository and cannot \
         be cached, watched, or pruned safely"
    )]
    OutsideRepositoryLocalModule {
        module_path: String,
        manifest_path: String,
    },
    #[error(
        "local Go module {module_path:?} at {manifest_path} is not a go.work member and cannot be \
         hashed or pruned safely"
    )]
    NonMemberLocalModule {
        module_path: String,
        manifest_path: String,
    },
    #[error("malformed `go mod graph` line: {line}")]
    MalformedModGraphLine { line: String },
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

/// A single Go module discovered within a `go.work` workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoModule {
    /// The module path from `go.mod` (for example `example.com/api`).
    pub module_path: String,
    /// Absolute path to the module's `go.mod`.
    pub manifest_path: AbsoluteSystemPathBuf,
    /// Direct internal relationships to other workspace modules.
    pub relationships: Vec<Relationship>,
}

/// The result of Go workspace discovery.
#[derive(Debug, Default)]
pub struct DiscoveredWorkspace {
    pub modules: Vec<GoModule>,
}

#[derive(Debug, Deserialize)]
struct GoWorkJson {
    #[serde(rename = "Use")]
    use_: Option<Vec<GoWorkUse>>,
}

#[derive(Debug, Deserialize)]
struct GoWorkUse {
    #[serde(rename = "DiskPath")]
    disk_path: Option<String>,
    #[serde(rename = "ModulePath")]
    module_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoModEditJson {
    #[serde(rename = "Module")]
    module: GoModModule,
    #[serde(rename = "Replace")]
    replace: Option<Vec<GoModReplace>>,
}

#[derive(Debug, Deserialize)]
struct GoModModule {
    #[serde(rename = "Path")]
    path: String,
}

#[derive(Debug, Deserialize)]
struct GoModReplace {
    #[serde(rename = "Old")]
    old: GoModModuleRef,
    #[serde(rename = "New")]
    new: GoModModuleRef,
}

#[derive(Debug, Deserialize)]
struct GoModModuleRef {
    #[serde(rename = "Path")]
    path: Option<String>,
    #[serde(rename = "Version")]
    #[allow(dead_code)]
    version: Option<String>,
}

fn run_go(
    repo_root: &AbsoluteSystemPath,
    args: &[&str],
    command: &'static str,
) -> Result<std::process::Output, Error> {
    Command::new("go")
        .args(args)
        .current_dir(repo_root.as_std_path())
        .output()
        .map_err(|source| Error::CommandSpawn { command, source })
}

fn go_work_json(repo_root: &AbsoluteSystemPath) -> Result<GoWorkJson, Error> {
    let output = run_go(repo_root, &["work", "edit", "-json"], "go work edit -json")?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: "go work edit -json",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|source| Error::CommandParse {
        command: "go work edit -json",
        source,
    })
}

fn go_mod_edit_json(
    repo_root: &AbsoluteSystemPath,
    module_dir: &AbsoluteSystemPath,
) -> Result<GoModEditJson, Error> {
    let manifest = module_dir.join_component(GO_MOD);
    let output = run_go(
        repo_root,
        &["mod", "edit", "-json", manifest.as_str()],
        "go mod edit -json",
    )?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: "go mod edit -json",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|source| Error::CommandParse {
        command: "go mod edit -json",
        source,
    })
}

fn go_mod_graph(repo_root: &AbsoluteSystemPath) -> Result<String, Error> {
    let output = run_go(repo_root, &["mod", "graph"], "go mod graph")?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: "go mod graph",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn join_relative_path(
    base: &AbsoluteSystemPath,
    relative: &str,
) -> Result<AbsoluteSystemPathBuf, Error> {
    if Path::new(relative).is_absolute() {
        return AbsoluteSystemPathBuf::try_from(Path::new(relative)).map_err(Error::from);
    }
    let components = relative
        .trim_start_matches("./")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    Ok(base.join_components(&components))
}

fn resolve_member_dir(
    repo_root: &AbsoluteSystemPath,
    disk_path: &str,
) -> Result<AbsoluteSystemPathBuf, Error> {
    let member = join_relative_path(repo_root, disk_path)?;
    if !member.starts_with(repo_root) {
        return Err(Error::MemberOutsideRepository {
            path: member.to_string(),
        });
    }
    Ok(member)
}

fn module_path_without_version(dep: &str) -> &str {
    dep.split('@').next().unwrap_or(dep)
}

fn validate_local_replacements(
    repo_root: &AbsoluteSystemPath,
    module_dir: &AbsoluteSystemPath,
    module_path: &str,
    replacements: Option<&[GoModReplace]>,
    member_paths: &HashSet<String>,
) -> Result<(), Error> {
    let replacements = replacements.unwrap_or_default();
    for replacement in replacements {
        let new = &replacement.new;
        let local_path = new.path.as_deref().filter(|path| {
            !path.contains('@') && (path.starts_with('.') || Path::new(path).is_absolute())
        });
        if let Some(local_path) = local_path {
            let resolved = join_relative_path(module_dir, local_path)?;
            if !resolved.starts_with(repo_root) {
                return Err(Error::OutsideRepositoryLocalModule {
                    module_path: module_path.to_string(),
                    manifest_path: resolved.join_component(GO_MOD).to_string(),
                });
            }
            let resolved_module_path = go_mod_edit_json(repo_root, &resolved)?.module.path;
            if !member_paths.contains(&resolved_module_path) {
                return Err(Error::NonMemberLocalModule {
                    module_path: resolved_module_path,
                    manifest_path: resolved.join_component(GO_MOD).to_string(),
                });
            }
        }
    }
    Ok(())
}

fn relationships_from_mod_graph(
    graph: &str,
    module_paths: &HashSet<String>,
) -> Result<HashMap<String, Vec<Relationship>>, Error> {
    let mut relationships: HashMap<String, Vec<Relationship>> = HashMap::new();
    for line in graph.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (from, to) = line
            .split_once(' ')
            .ok_or_else(|| Error::MalformedModGraphLine {
                line: line.to_string(),
            })?;
        if !module_paths.contains(from) {
            continue;
        }
        let dependency = module_path_without_version(to);
        if module_paths.contains(dependency) {
            relationships
                .entry(from.to_string())
                .or_default()
                .push(Relationship::internal(dependency, DependencyKind::Production));
        }
    }
    Ok(relationships)
}

/// Discover all Go modules listed by the repository-root `go.work`.
pub fn discover_workspace(repo_root: &AbsoluteSystemPath) -> Result<DiscoveredWorkspace, Error> {
    let work_path = repo_root.join_component(GO_WORK);
    if !work_path.exists() {
        return Ok(DiscoveredWorkspace::default());
    }

    let work = go_work_json(repo_root)?;
    let uses = work.use_.ok_or(Error::EmptyWorkspace)?;
    if uses.is_empty() {
        return Err(Error::EmptyWorkspace);
    }

    let root_module_path = if repo_root.join_component(GO_MOD).exists() {
        go_mod_edit_json(repo_root, repo_root).ok().and_then(|json| {
            let path = json.module.path;
            (!path.is_empty()).then_some(path)
        })
    } else {
        None
    };

    let mut modules = Vec::new();
    let mut identities: HashMap<String, String> = HashMap::new();
    let mut member_paths = HashSet::new();
    let mut pending_replacements = Vec::new();

    for entry in uses {
        let member_dir = match entry.disk_path.as_deref() {
            Some(disk_path) => resolve_member_dir(repo_root, disk_path)?,
            None => {
                return Err(Error::MalformedModGraphLine {
                    line: "go.work Use entry is missing DiskPath".to_string(),
                });
            }
        };
        if !member_dir.join_component(GO_MOD).exists() {
            return Err(Error::MissingGoMod {
                path: member_dir.to_string(),
            });
        }

        let module_json = go_mod_edit_json(repo_root, &member_dir)?;
        let module_path = entry
            .module_path
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| module_json.module.path.clone());
        if module_path.is_empty() {
            return Err(Error::MissingModulePath {
                path: member_dir.join_component(GO_MOD).to_string(),
            });
        }
        if let Some(root_path) = &root_module_path
            && root_path == &module_path
        {
            return Err(Error::RootDefinitionCollision {
                path: member_dir.to_string(),
                module_path,
            });
        }
        if let Some(other_manifest) = identities.get(&module_path) {
            return Err(Error::DuplicateModuleIdentity {
                module_path,
                other_manifest: other_manifest.clone(),
            });
        }
        identities.insert(
            module_path.clone(),
            member_dir.join_component(GO_MOD).to_string(),
        );
        member_paths.insert(module_path.clone());
        pending_replacements.push((
            member_dir.clone(),
            module_path.clone(),
            module_json.replace,
        ));

        modules.push(GoModule {
            module_path,
            manifest_path: member_dir.join_component(GO_MOD),
            relationships: Vec::new(),
        });
    }

    for (member_dir, module_path, replacements) in pending_replacements {
        validate_local_replacements(
            repo_root,
            &member_dir,
            &module_path,
            replacements.as_deref(),
            &member_paths,
        )?;
    }

    let graph = go_mod_graph(repo_root)?;
    let mut relationship_map = relationships_from_mod_graph(&graph, &member_paths)?;
    for module in &mut modules {
        module.relationships = relationship_map
            .remove(&module.module_path)
            .unwrap_or_default();
    }

    Ok(DiscoveredWorkspace { modules })
}

fn package_from_module(module: &GoModule) -> DiscoveredPackage {
    let descriptor = PackageJson {
        name: Some(turborepo_errors::Spanned::new(module.module_path.clone())),
        ..Default::default()
    };
    DiscoveredPackage::package(
        Some(module.module_path.clone()),
        descriptor,
        module.manifest_path.clone(),
    )
    .with_native_relationships(module.relationships.clone())
}

/// The Go repository contributor. Registered during graph construction when
/// `futureFlags.experimentalGoWorkspaces` is enabled.
pub(crate) struct GoContributor {
    repo_root: AbsoluteSystemPathBuf,
}

impl GoContributor {
    pub(crate) fn new(repo_root: AbsoluteSystemPathBuf) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { repo_root })
    }
}

impl RepositoryContributor for GoContributor {
    fn id(&self) -> ToolchainId {
        ToolchainId::GO
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            let workspace = turborepo_rayon_compat::block_in_place(|| {
                discover_workspace(&self.repo_root)
            })
            .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;

            let workspace_roots = self
                .repo_root
                .join_component(GO_WORK)
                .exists()
                .then(|| WorkspaceRoot::new("go", self.repo_root.clone()))
                .into_iter()
                .collect();

            let packages = workspace
                .modules
                .iter()
                .map(package_from_module)
                .collect();

            Ok(DiscoveredPackages::new(packages, workspace_roots))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_workspace(root: &AbsoluteSystemPath, layout: &[(&str, &str, &str)]) {
        fs::create_dir_all(root.as_std_path()).unwrap();
        let mut work = String::from("go 1.22\n\nuse (\n");
        for (dir, module_path, contents) in layout {
            let module_dir = join_relative_path(&root, dir).unwrap();
            fs::create_dir_all(module_dir.as_std_path()).unwrap();
            let go_mod = if contents.is_empty() {
                format!("module {module_path}\n\ngo 1.22\n")
            } else {
                contents.to_string()
            };
            module_dir
                .join_component(GO_MOD)
                .create_with_contents(go_mod)
                .unwrap();
            work.push_str(&format!("\t./{dir}\n"));
        }
        work.push_str(")\n");
        root.join_component(GO_WORK)
            .create_with_contents(work)
            .unwrap();
    }

    #[test]
    fn discovers_multiple_modules() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        write_workspace(
            &root,
            &[
                ("apps/api", "example.com/api", ""),
                ("packages/lib", "example.com/lib", ""),
            ],
        );
        join_relative_path(&root, "apps/api")
            .unwrap()
            .join_component("main.go")
            .create_with_contents("package main\n")
            .unwrap();
        join_relative_path(&root, "packages/lib")
            .unwrap()
            .join_component("lib.go")
            .create_with_contents("package lib\n")
            .unwrap();

        let workspace = discover_workspace(&root).expect("workspace discovery succeeds");
        assert_eq!(workspace.modules.len(), 2);
        let paths = workspace
            .modules
            .iter()
            .map(|module| module.module_path.as_str())
            .collect::<HashSet<_>>();
        assert!(paths.contains("example.com/api"));
        assert!(paths.contains("example.com/lib"));
    }

    #[test]
    fn rejects_duplicate_module_identity() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        write_workspace(
            &root,
            &[
                ("apps/api", "example.com/shared", ""),
                ("packages/lib", "example.com/shared", ""),
            ],
        );

        let error = discover_workspace(&root).expect_err("duplicate identities fail");
        assert!(
            error.to_string().contains("duplicate Go module identity"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_member_outside_repository() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(GO_WORK)
            .create_with_contents("go 1.22\n\nuse ../outside\n")
            .unwrap();

        let error = discover_workspace(&root).expect_err("outside members fail");
        assert!(
            error.to_string().contains("outside the repository"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn models_internal_relationships() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        write_workspace(
            &root,
            &[
                (
                    "apps/api",
                    "example.com/api",
                    "module example.com/api\n\ngo 1.22\n\nrequire example.com/lib v0.0.0\n\nreplace example.com/lib => ../../packages/lib\n",
                ),
                ("packages/lib", "example.com/lib", ""),
            ],
        );
        join_relative_path(&root, "apps/api")
            .unwrap()
            .join_component("main.go")
            .create_with_contents("package main\nimport _ \"example.com/lib\"\nfunc main() {}\n")
            .unwrap();
        join_relative_path(&root, "packages/lib")
            .unwrap()
            .join_component("lib.go")
            .create_with_contents("package lib\n")
            .unwrap();

        let workspace = discover_workspace(&root).expect("workspace discovery succeeds");
        let api = workspace
            .modules
            .iter()
            .find(|module| module.module_path == "example.com/api")
            .expect("api module");
        assert_eq!(api.relationships.len(), 1);
        assert_eq!(
            api.relationships[0].target(),
            &crate::relationships::RelationshipTarget::Internal("example.com/lib".to_string())
        );
    }

    #[test]
    fn relationships_from_mod_graph_parses_workspace_edges() {
        let graph = "example.com/api example.com/lib@v0.0.0\nexample.com/api go@1.22\n";
        let module_paths = HashSet::from([
            "example.com/api".to_string(),
            "example.com/lib".to_string(),
        ]);
        let relationships = relationships_from_mod_graph(graph, &module_paths).unwrap();
        assert_eq!(relationships["example.com/api"].len(), 1);
    }
}
