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
//! Support is experimental and gated behind
//! `futureFlags.experimentalGoWorkspaces`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    path::Path,
    process::Command,
    sync::Arc,
};

use serde::Deserialize;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::{
    change_knowledge::ChangeObservation,
    external_resolution::{
        ExternalPackageIdentity, ExternalResolutionData, ExternalResolutionDomain,
        GO_RESOLUTION_DOMAIN, PackageResolution, ResolutionCompleteness,
    },
    native_tasks::{
        NativeCommandArguments, NativeCommandProgram, NativeTask, NativeTaskContract,
        PassThroughPlacement, TaskEntrypoint, WorkingDirectoryPolicy,
    },
    package_json::PackageJson,
    prune_knowledge::{PruneDomain, PrunePlan},
    relationships::{DependencyKind, Relationship},
    task_contracts::DependencySourceInputs,
    toolchain::{
        self, DerivedInputSafety, DerivedOutputs, DiscoverPackagesFuture, DiscoveredPackage,
        DiscoveredPackages, RepositoryContributor, TaskDefaults, ToolchainId, WorkspaceRoot,
    },
};

/// The conventional file name for a Go workspace definition.
pub const GO_WORK: &str = "go.work";

/// The conventional file name for a Go module manifest.
pub const GO_MOD: &str = "go.mod";

/// The conventional per-module checksum file.
pub const GO_SUM: &str = "go.sum";

/// The conventional workspace checksum file.
pub const GO_WORK_SUM: &str = "go.work.sum";

/// Stable identity for workspace-wide Go verification tasks.
pub const GO_WORKSPACE_SCOPE: &str = "go-workspace";

const GO_DIST_DIR: &str = "dist";

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
    #[error("duplicate Go module identity {module_path:?} (also declared at {other_manifest})")]
    DuplicateModuleIdentity {
        module_path: String,
        other_manifest: String,
    },
    #[error(
        "Go workspace aggregate name {name:?} collides with a module identity. Rename that module \
         before enabling native Go workspace support."
    )]
    WorkspaceNameCollision { name: String },
    #[error(
        "go.work member at {path} resolves to module {module_path:?}, which collides with the \
         repository root module definition"
    )]
    RootDefinitionCollision { path: String, module_path: String },
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
    #[error(
        "Go module graph references unresolved external module {module:?}; run `go mod tidy` in \
         each workspace module and commit go.sum/go.work.sum"
    )]
    UnknownResolutionModule { module: String },
    #[error("failed to read Go workspace file: {0}")]
    WorkspaceFileRead(#[source] io::Error),
    #[error("failed to resolve Go module path {path}: {source}")]
    ModulePath {
        path: String,
        #[source]
        source: turbopath::PathError,
    },
    #[error("go.work has no `go` directive")]
    MissingGoDirective,
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
    /// Repository-relative module directory.
    pub directory: AnchoredSystemPathBuf,
    /// Direct internal relationships to other workspace modules.
    pub relationships: Vec<Relationship>,
    /// A single runnable main package, when Go can identify one unambiguously.
    pub runnable: Option<GoRunnable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoRunnable {
    pub import_path: String,
    pub output_name: String,
}

/// The result of Go workspace discovery.
#[derive(Debug, Default)]
pub struct DiscoveredWorkspace {
    pub modules: Vec<GoModule>,
    work: GoWorkJson,
    graph: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct GoWorkJson {
    #[serde(rename = "Go")]
    go: Option<String>,
    #[serde(rename = "Toolchain")]
    toolchain: Option<String>,
    #[serde(rename = "Godebug")]
    godebug: Option<Vec<GoWorkGodebug>>,
    #[serde(rename = "Use")]
    use_: Option<Vec<GoWorkUse>>,
    #[serde(rename = "Replace")]
    replace: Option<Vec<GoWorkReplace>>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoWorkUse {
    #[serde(rename = "DiskPath")]
    disk_path: Option<String>,
    #[serde(rename = "ModulePath")]
    module_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoWorkGodebug {
    #[serde(rename = "Key")]
    key: String,
    #[serde(rename = "Value")]
    value: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GoWorkReplace {
    #[serde(rename = "Old")]
    old: GoModModuleRef,
    #[serde(rename = "New")]
    new: GoModModuleRef,
}

#[derive(Debug, Deserialize)]
struct GoModEditJson {
    #[serde(rename = "Module")]
    module: Option<GoModModule>,
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
    #[allow(dead_code)]
    old: GoModModuleRef,
    #[serde(rename = "New")]
    new: GoModModuleRef,
}

#[derive(Debug, Clone, Deserialize)]
struct GoModModuleRef {
    #[serde(rename = "Path")]
    path: Option<String>,
    #[serde(rename = "Version")]
    #[allow(dead_code)]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoListPackage {
    #[serde(rename = "ImportPath")]
    import_path: String,
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct GoListModule {
    #[serde(rename = "Path")]
    path: String,
    #[serde(default, rename = "Version")]
    version: String,
    #[serde(default, rename = "Sum")]
    sum: String,
    #[serde(default, rename = "GoModSum")]
    go_mod_sum: String,
    #[serde(default, rename = "Main")]
    main: bool,
    #[serde(rename = "Replace")]
    replace: Option<Box<GoListModule>>,
}

#[derive(Debug)]
struct GoToolchainIdentity {
    package: ExternalPackageIdentity,
    target_os: String,
    environment: BTreeMap<String, serde_json::Value>,
}

fn run_go(
    repo_root: &AbsoluteSystemPath,
    args: &[&str],
    command: &'static str,
) -> Result<std::process::Output, Error> {
    run_go_at(repo_root, repo_root, args, command)
}

fn run_go_at(
    repo_root: &AbsoluteSystemPath,
    cwd: &AbsoluteSystemPath,
    args: &[&str],
    command: &'static str,
) -> Result<std::process::Output, Error> {
    Command::new("go")
        .args(args)
        .current_dir(cwd.as_std_path())
        .env("GOWORK", repo_root.join_component(GO_WORK).as_str())
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

fn go_toolchain_identity(repo_root: &AbsoluteSystemPath) -> Result<GoToolchainIdentity, Error> {
    let version = run_go(repo_root, &["version"], "go version")?;
    if !version.status.success() {
        return Err(Error::CommandFailed {
            command: "go version",
            stderr: String::from_utf8_lossy(&version.stderr).trim().to_string(),
        });
    }
    let environment = run_go(repo_root, &["env", "-json"], "go env -json")?;
    if !environment.status.success() {
        return Err(Error::CommandFailed {
            command: "go env -json",
            stderr: String::from_utf8_lossy(&environment.stderr)
                .trim()
                .to_string(),
        });
    }
    let values: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&environment.stdout)
        .map_err(|source| Error::CommandParse {
        command: "go env -json",
        source,
    })?;
    let target_os = values
        .get("GOOS")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let details = stable_toolchain_details(repo_root, &values);
    let version = String::from_utf8_lossy(&version.stdout).trim().to_string();
    Ok(GoToolchainIdentity {
        package: ExternalPackageIdentity::new("go", format!("{version}\n{details}"))
            .with_human_name("go"),
        target_os,
        environment: values,
    })
}

fn stable_toolchain_details(
    repo_root: &AbsoluteSystemPath,
    values: &BTreeMap<String, serde_json::Value>,
) -> String {
    let stable_names = [
        "GO111MODULE",
        "GO386",
        "GOAMD64",
        "GOARCH",
        "GOARM",
        "GOARM64",
        "GCCGO",
        "GOEXPERIMENT",
        "GOFLAGS",
        "GOHOSTARCH",
        "GOHOSTOS",
        "GOMIPS",
        "GOMIPS64",
        "GOOS",
        "GOTOOLCHAIN",
        "GOVERSION",
        "GOWASM",
        "CGO_ENABLED",
        "CC",
        "CXX",
        "CGO_CFLAGS",
        "CGO_CPPFLAGS",
        "CGO_CXXFLAGS",
        "CGO_FFLAGS",
        "CGO_LDFLAGS",
        "PKG_CONFIG",
    ];
    let root = repo_root.as_str();
    stable_names
        .iter()
        .filter_map(|name| {
            values
                .get(*name)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    (
                        *name,
                        value.replace(root, "$TURBO_ROOT$").replace('\\', "/"),
                    )
                })
        })
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn go_list_modules(repo_root: &AbsoluteSystemPath) -> Result<Vec<GoListModule>, Error> {
    let output = run_go(
        repo_root,
        &["list", "-mod=readonly", "-m", "-json", "all"],
        "go list -mod=readonly -m -json all",
    )?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: "go list -mod=readonly -m -json all",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<GoListModule>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::CommandParse {
            command: "go list -mod=readonly -m -json all",
            source,
        })
}

fn runnable_package(
    repo_root: &AbsoluteSystemPath,
    module_dir: &AbsoluteSystemPath,
    module_path: &str,
) -> Result<Option<GoRunnable>, Error> {
    let output = run_go_at(
        repo_root,
        module_dir,
        &["list", "-json", "./..."],
        "go list -json ./...",
    )?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: "go list -json ./...",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let packages = serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<GoListPackage>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::CommandParse {
            command: "go list -json ./...",
            source,
        })?;
    let mains = packages
        .into_iter()
        .filter(|package| package.name == "main")
        .collect::<Vec<_>>();
    if mains.len() != 1 {
        return Ok(None);
    }
    let Some(main) = mains.into_iter().next() else {
        return Ok(None);
    };
    let output_name = main
        .import_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(module_path)
        .to_string();
    Ok(Some(GoRunnable {
        import_path: main.import_path,
        output_name,
    }))
}

fn join_relative_path(
    base: &AbsoluteSystemPath,
    relative: &str,
) -> Result<AbsoluteSystemPathBuf, Error> {
    Ok(AbsoluteSystemPathBuf::from_unknown(base, relative))
}

fn resolve_member_dir(
    repo_root: &AbsoluteSystemPath,
    disk_path: &str,
) -> Result<AbsoluteSystemPathBuf, Error> {
    let member = join_relative_path(repo_root, disk_path)?;
    if !path_is_within_repository(repo_root, &member)? {
        return Err(Error::MemberOutsideRepository {
            path: member.to_string(),
        });
    }
    Ok(member)
}

fn path_is_within_repository(
    repo_root: &AbsoluteSystemPath,
    path: &AbsoluteSystemPath,
) -> Result<bool, Error> {
    if !repo_root.contains(path) {
        return Ok(false);
    }
    if !path.exists() {
        return Ok(true);
    }
    let real_root = repo_root
        .to_realpath()
        .map_err(|source| Error::ModulePath {
            path: repo_root.to_string(),
            source,
        })?;
    let real_path = path.to_realpath().map_err(|source| Error::ModulePath {
        path: path.to_string(),
        source,
    })?;
    Ok(real_root.contains(&real_path))
}

fn required_module_path<'a>(
    module: &'a GoModEditJson,
    manifest_path: &AbsoluteSystemPath,
) -> Result<&'a str, Error> {
    module
        .module
        .as_ref()
        .map(|module| module.path.as_str())
        .filter(|path| !path.is_empty())
        .ok_or_else(|| Error::MissingModulePath {
            path: manifest_path.to_string(),
        })
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
            if !path_is_within_repository(repo_root, &resolved)? {
                return Err(Error::OutsideRepositoryLocalModule {
                    module_path: module_path.to_string(),
                    manifest_path: resolved.join_component(GO_MOD).to_string(),
                });
            }
            let module = go_mod_edit_json(repo_root, &resolved)?;
            let resolved_module_path =
                required_module_path(&module, &resolved.join_component(GO_MOD))?.to_string();
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
                .push(Relationship::internal(
                    dependency,
                    DependencyKind::Production,
                ));
        }
    }
    Ok(relationships)
}

fn module_identity(module: &GoListModule) -> ExternalPackageIdentity {
    fn describe(module: &GoListModule) -> String {
        format!(
            "path={};version={};sum={};go_mod_sum={}",
            module.path, module.version, module.sum, module.go_mod_sum
        )
    }
    let mut version = describe(module);
    if let Some(replacement) = module.replace.as_deref() {
        version.push_str(";replace=");
        version.push_str(&describe(replacement));
    }
    ExternalPackageIdentity::new(module.path.clone(), version).with_human_name(module.path.clone())
}

fn external_resolutions(
    graph: &str,
    workspace: &DiscoveredWorkspace,
    listed: &[GoListModule],
    toolchain: &ExternalPackageIdentity,
) -> Result<Vec<PackageResolution>, Error> {
    let internal: HashSet<&str> = workspace
        .modules
        .iter()
        .map(|module| module.module_path.as_str())
        .collect();
    let mut by_reference = HashMap::new();
    for module in listed.iter().filter(|module| !module.main) {
        let identity = module_identity(module);
        by_reference.insert(module.path.clone(), identity.clone());
        if !module.version.is_empty() {
            by_reference.insert(format!("{}@{}", module.path, module.version), identity);
        }
    }
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for line in graph.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (from, to) = line
            .split_once(' ')
            .ok_or_else(|| Error::MalformedModGraphLine {
                line: line.to_string(),
            })?;
        adjacency.entry(from).or_default().push(to);
    }
    let mut resolutions = Vec::with_capacity(workspace.modules.len() + 1);
    let mut aggregate = HashSet::new();
    for module in &workspace.modules {
        let mut identities = HashSet::from([toolchain.clone()]);
        let mut pending = vec![module.module_path.as_str()];
        let mut seen = HashSet::new();
        while let Some(node) = pending.pop() {
            if !seen.insert(node) {
                continue;
            }
            for dependency in adjacency.get(node).into_iter().flatten() {
                let path = module_path_without_version(dependency);
                if internal.contains(path) {
                    pending.push(path);
                    continue;
                }
                pending.push(*dependency);
                if matches!(path, "go" | "toolchain") {
                    continue;
                }
                let identity = by_reference
                    .get(*dependency)
                    .or_else(|| by_reference.get(path))
                    .ok_or_else(|| Error::UnknownResolutionModule {
                        module: (*dependency).to_string(),
                    })?
                    .clone();
                aggregate.insert(identity.clone());
                identities.insert(identity);
            }
        }
        resolutions.push(PackageResolution::new(
            module.module_path.clone(),
            identities,
        ));
    }
    aggregate.insert(toolchain.clone());
    resolutions.push(PackageResolution::new(GO_WORKSPACE_SCOPE, aggregate));
    Ok(resolutions)
}

/// Discover all Go modules listed by the repository-root `go.work`.
pub fn discover_workspace(repo_root: &AbsoluteSystemPath) -> Result<DiscoveredWorkspace, Error> {
    let work_path = repo_root.join_component(GO_WORK);
    if !work_path.exists() {
        return Ok(DiscoveredWorkspace::default());
    }

    let work = go_work_json(repo_root)?;
    let uses = work.use_.clone().ok_or(Error::EmptyWorkspace)?;
    if uses.is_empty() {
        return Err(Error::EmptyWorkspace);
    }

    let root_module_path = if repo_root.join_component(GO_MOD).exists() {
        let root_module = go_mod_edit_json(repo_root, repo_root)?;
        Some(required_module_path(&root_module, &repo_root.join_component(GO_MOD))?.to_string())
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
        let module_path = match entry.module_path.filter(|path| !path.is_empty()) {
            Some(path) => path,
            None => {
                required_module_path(&module_json, &member_dir.join_component(GO_MOD))?.to_string()
            }
        };
        if module_path == GO_WORKSPACE_SCOPE {
            return Err(Error::WorkspaceNameCollision {
                name: GO_WORKSPACE_SCOPE.to_string(),
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
        pending_replacements.push((member_dir.clone(), module_path.clone(), module_json.replace));
        let directory = AnchoredSystemPathBuf::new(repo_root, &member_dir)?;

        modules.push(GoModule {
            module_path,
            manifest_path: member_dir.join_component(GO_MOD),
            directory,
            relationships: Vec::new(),
            runnable: None,
        });
    }

    for module in &mut modules {
        let module_dir = repo_root.resolve(&module.directory);
        module.runnable = runnable_package(repo_root, &module_dir, &module.module_path)?;
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
    for replacement in work.replace.as_deref().unwrap_or_default() {
        let Some(local_path) = replacement.new.path.as_deref().filter(|path| {
            !path.contains('@') && (path.starts_with('.') || Path::new(path).is_absolute())
        }) else {
            continue;
        };
        let resolved = join_relative_path(repo_root, local_path)?;
        let replaced_module = replacement.old.path.as_deref().unwrap_or("<unknown>");
        if !path_is_within_repository(repo_root, &resolved)? {
            return Err(Error::OutsideRepositoryLocalModule {
                module_path: replaced_module.to_string(),
                manifest_path: resolved.join_component(GO_MOD).to_string(),
            });
        }
        let module = go_mod_edit_json(repo_root, &resolved)?;
        let resolved_module_path =
            required_module_path(&module, &resolved.join_component(GO_MOD))?.to_string();
        if !member_paths.contains(&resolved_module_path) {
            return Err(Error::NonMemberLocalModule {
                module_path: resolved_module_path,
                manifest_path: resolved.join_component(GO_MOD).to_string(),
            });
        }
    }

    let graph = go_mod_graph(repo_root)?;
    let mut relationship_map = relationships_from_mod_graph(&graph, &member_paths)?;
    for module in &mut modules {
        module.relationships = relationship_map
            .remove(&module.module_path)
            .unwrap_or_default();
    }

    Ok(DiscoveredWorkspace {
        modules,
        work,
        graph,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoPackageKind {
    Module { runnable: Option<GoRunnable> },
    Workspace,
}

/// Go variables that can alter compilation, package selection, tests, or the
/// target platform. Mutable caches, credentials, proxies, telemetry, and
/// checkout-local paths are deliberately absent.
pub const HASHED_ENV_VARS: &[&str] = &[
    "GO111MODULE",
    "GO386",
    "GOAMD64",
    "GOARCH",
    "GOARM",
    "GOARM64",
    "GOCACHEPROG",
    "GCCGO",
    "GOEXPERIMENT",
    "GOFLAGS",
    "GOMIPS",
    "GOMIPS64",
    "GOOS",
    "GOTOOLCHAIN",
    "GOWASM",
    "CGO_ENABLED",
    "CC",
    "CXX",
    "CGO_CFLAGS",
    "CGO_CPPFLAGS",
    "CGO_CXXFLAGS",
    "CGO_FFLAGS",
    "CGO_LDFLAGS",
    "PKG_CONFIG",
];

fn go_task_entrypoint(kind: &GoPackageKind, task: &str) -> Option<TaskEntrypoint> {
    Some(match (kind, task) {
        (GoPackageKind::Module { runnable: Some(_) }, "build") => TaskEntrypoint::Preferred,
        (GoPackageKind::Module { .. }, "build") => TaskEntrypoint::Candidate,
        (GoPackageKind::Workspace, "build") => TaskEntrypoint::Excluded,
        (GoPackageKind::Workspace, "test" | "vet" | "format" | "lint") => {
            TaskEntrypoint::PreferredOnly
        }
        (GoPackageKind::Module { .. }, _) => TaskEntrypoint::Candidate,
        (GoPackageKind::Workspace, _) => return None,
    })
}

fn go_command_task(
    kind: &GoPackageKind,
    task: &str,
    prefix: Vec<String>,
    suffix: Vec<String>,
    pass_through_placement: PassThroughPlacement,
    cache: Option<bool>,
) -> NativeTask {
    let display = std::iter::once("go".to_string())
        .chain(prefix.iter().cloned())
        .chain(suffix.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    NativeTask::command_task(
        task,
        display,
        NativeCommandProgram::Tool("go".to_string()),
        NativeCommandArguments {
            prefix,
            pass_through_placement,
            pass_through_separator: None,
            suffix,
        },
        None,
        WorkingDirectoryPolicy::PackageDirectory,
    )
    .with_contract(NativeTaskContract::new(
        TaskDefaults { cache },
        go_task_entrypoint(kind, task),
        true,
    ))
}

fn go_tasks_for_package(
    kind: &GoPackageKind,
    target_os: &str,
    workspace_module_patterns: &[String],
) -> Vec<NativeTask> {
    let patterns = match kind {
        GoPackageKind::Module { .. } => vec!["./...".to_string()],
        GoPackageKind::Workspace => workspace_module_patterns.to_vec(),
    };
    let mut tasks = Vec::new();
    match kind {
        GoPackageKind::Module { runnable } => {
            let (prefix, suffix, cache) = if let Some(runnable) = runnable {
                let extension = if target_os == "windows" { ".exe" } else { "" };
                (
                    vec![
                        "build".to_string(),
                        "-o".to_string(),
                        format!("{GO_DIST_DIR}/{}{extension}", runnable.output_name),
                    ],
                    vec![runnable.import_path.clone()],
                    None,
                )
            } else {
                (vec!["build".to_string()], patterns.clone(), Some(false))
            };
            tasks.push(go_command_task(
                kind,
                "build",
                prefix,
                suffix,
                PassThroughPlacement::BeforeSuffix,
                cache,
            ));
            if let Some(runnable) = runnable {
                for task in ["run", "dev"] {
                    tasks.push(go_command_task(
                        kind,
                        task,
                        vec!["run".to_string(), runnable.import_path.clone()],
                        Vec::new(),
                        PassThroughPlacement::AfterSuffix,
                        Some(false),
                    ));
                }
            }
        }
        GoPackageKind::Workspace => tasks.push(NativeTask::contract_task(
            "build",
            NativeTaskContract::new(
                TaskDefaults::default(),
                Some(TaskEntrypoint::Excluded),
                false,
            ),
        )),
    }
    for (task, subcommand, cache) in [
        ("test", "test", None),
        ("vet", "vet", None),
        ("format", "fmt", Some(false)),
    ] {
        tasks.push(go_command_task(
            kind,
            task,
            vec![subcommand.to_string()],
            patterns.clone(),
            PassThroughPlacement::BeforeSuffix,
            cache,
        ));
    }
    tasks.push(
        NativeTask::aggregate("lint", ["vet"]).with_contract(NativeTaskContract::new(
            TaskDefaults::default(),
            go_task_entrypoint(kind, "lint"),
            false,
        )),
    );
    tasks
}

fn source_globs(prefix: &str, directory: &str) -> [String; 4] {
    let base = if prefix.is_empty() {
        directory.to_string()
    } else {
        format!("{prefix}/{directory}")
    };
    [
        format!("{base}/**"),
        format!("!{base}/.git/**"),
        format!("!{base}/.turbo/**"),
        format!("!{base}/{GO_DIST_DIR}/**"),
    ]
}

fn root_input(prefix: &str, file: &str) -> String {
    if prefix.is_empty() {
        file.to_string()
    } else {
        format!("{prefix}/{file}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoTaskContract {
    kind: GoPackageKind,
    toolchain_identified: bool,
    target_os: String,
    cache_prefixes: Vec<String>,
}

impl GoTaskContract {
    fn new(
        kind: GoPackageKind,
        toolchain_identified: bool,
        target_os: String,
        cache_prefixes: Vec<String>,
    ) -> Self {
        Self {
            kind,
            toolchain_identified,
            target_os,
            cache_prefixes,
        }
    }

    pub(crate) fn dependency_source_inputs(&self) -> DependencySourceInputs {
        match &self.kind {
            GoPackageKind::Module { .. } => DependencySourceInputs::Include,
            GoPackageKind::Workspace => DependencySourceInputs::Exclude,
        }
    }

    pub(crate) fn derived_task_io(
        &self,
        _package: &crate::package_graph::PackageTaskContext<'_>,
        task: &str,
        path_to_root: &str,
        dependencies: &[crate::package_graph::PackageTaskContext<'_>],
        wants_automatic_inputs: bool,
        context: &toolchain::TaskIOContext<'_>,
    ) -> Option<toolchain::DerivedTaskIO> {
        if !matches!(task, "build" | "test" | "vet" | "format" | "run" | "dev") {
            return None;
        }
        let mut io = toolchain::DerivedTaskIO {
            input_globs: [GO_WORK, GO_WORK_SUM]
                .map(|file| root_input(path_to_root, file))
                .to_vec(),
            env: HASHED_ENV_VARS
                .iter()
                .map(|name| name.to_string())
                .collect(),
            forbidden_output_prefixes: vec![
                root_input(path_to_root, ".turbo"),
                root_input(path_to_root, ".git"),
            ],
            ..Default::default()
        };
        io.forbidden_output_prefixes.extend(
            self.cache_prefixes
                .iter()
                .map(|prefix| root_input(path_to_root, prefix)),
        );
        if !self.toolchain_identified {
            io.input_safety = DerivedInputSafety::Untracked;
            io.cache_reason = Some("Turborepo could not identify the Go toolchain".to_string());
        }
        if wants_automatic_inputs {
            match &self.kind {
                GoPackageKind::Module { .. } => {
                    io.package_default_inputs = Some(true);
                    io.input_globs.push(format!("!{GO_DIST_DIR}/**"));
                }
                GoPackageKind::Workspace => io.package_default_inputs = Some(false),
            }
            for dependency in dependencies {
                match dependency.task_contract().dependency_source_inputs() {
                    DependencySourceInputs::Include => io.input_globs.extend(source_globs(
                        path_to_root,
                        dependency.directory().to_unix().as_str(),
                    )),
                    DependencySourceInputs::Exclude => {}
                    DependencySourceInputs::Unknown => {
                        io.input_safety = DerivedInputSafety::Untracked;
                    }
                }
            }
            io.input_globs.sort();
            io.input_globs.dedup();
        }
        if task == "build" {
            io.outputs = match &self.kind {
                GoPackageKind::Module {
                    runnable: Some(runnable),
                } if context.task_args.is_none_or(<[_]>::is_empty) => {
                    let extension = if self.target_os == "windows" {
                        ".exe"
                    } else {
                        ""
                    };
                    DerivedOutputs::Resolved(vec![format!(
                        "{GO_DIST_DIR}/{}{extension}",
                        runnable.output_name
                    )])
                }
                GoPackageKind::Module { runnable: Some(_) } => {
                    io.cache_reason =
                        Some("Go build arguments can change the derived binary output".to_string());
                    DerivedOutputs::Unavailable
                }
                GoPackageKind::Module { runnable: None } => {
                    io.cache_reason = Some(
                        "Go library builds have no stable outputs for Turborepo to restore"
                            .to_string(),
                    );
                    DerivedOutputs::Unavailable
                }
                GoPackageKind::Workspace => DerivedOutputs::Resolved(Vec::new()),
            };
        }
        Some(io)
    }
}

fn package_from_module(
    module: &GoModule,
    toolchain_identified: bool,
    target_os: &str,
    cache_prefixes: &[String],
    workspace_module_patterns: &[String],
) -> DiscoveredPackage {
    let kind = GoPackageKind::Module {
        runnable: module.runnable.clone(),
    };
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
    .with_native_tasks(go_tasks_for_package(
        &kind,
        target_os,
        workspace_module_patterns,
    ))
    .with_task_contract(crate::task_contracts::ScopeTaskContract::go(
        GoTaskContract::new(
            kind,
            toolchain_identified,
            target_os.to_string(),
            cache_prefixes.to_vec(),
        ),
    ))
}

fn go_cache_prefixes(
    repo_root: &AbsoluteSystemPath,
    environment: &BTreeMap<String, serde_json::Value>,
) -> Vec<String> {
    let mut prefixes = Vec::new();
    for name in ["GOCACHE", "GOMODCACHE"] {
        let Some(path) = environment
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(|path| AbsoluteSystemPathBuf::from_unknown(repo_root, path))
        else {
            continue;
        };
        if let Ok(prefix) = AnchoredSystemPathBuf::new(repo_root, &path)
            && prefix.components().next().is_some()
        {
            prefixes.push(prefix.to_unix().to_string());
        }
    }
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

fn go_change_observation(modules: &[GoModule], cache_prefixes: &[String]) -> ChangeObservation {
    let mut observation = ChangeObservation::new()
        .with_rediscovery_file_name(GO_WORK)
        .with_rediscovery_file_name(GO_MOD)
        .with_resolution_path(GO_WORK_SUM);
    for module in modules {
        observation = observation.with_resolution_path(
            module
                .directory
                .join_component(GO_SUM)
                .to_unix()
                .to_string(),
        );
    }
    for prefix in cache_prefixes {
        observation = observation.with_ignore_prefix(prefix.clone());
    }
    observation
}

fn render_module_ref(reference: &GoModModuleRef) -> Option<String> {
    let path = reference.path.as_deref()?;
    Some(
        match reference
            .version
            .as_deref()
            .filter(|version| !version.is_empty())
        {
            Some(version) => format!("{path} {version}"),
            None => path.to_string(),
        },
    )
}

fn normalize_go_directory(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

fn render_pruned_go_work(work: &GoWorkJson, kept_directories: &[String]) -> Result<String, Error> {
    let go = work.go.as_deref().ok_or(Error::MissingGoDirective)?;
    let mut output = format!("go {go}\n");
    if let Some(toolchain) = work
        .toolchain
        .as_deref()
        .filter(|toolchain| !toolchain.is_empty())
    {
        output.push_str(&format!("\ntoolchain {toolchain}\n"));
    }
    for setting in work.godebug.as_deref().unwrap_or_default() {
        output.push_str(&format!("\ngodebug {}={}\n", setting.key, setting.value));
    }
    let mut directories = kept_directories
        .iter()
        .map(|directory| normalize_go_directory(directory))
        .collect::<Vec<_>>();
    directories.sort();
    directories.dedup();
    output.push_str("\nuse (\n");
    for directory in &directories {
        output.push_str(&format!("\t./{directory}\n"));
    }
    output.push_str(")\n");
    let retained: HashSet<&str> = directories.iter().map(String::as_str).collect();
    for replacement in work.replace.as_deref().unwrap_or_default() {
        let Some(old) = render_module_ref(&replacement.old) else {
            continue;
        };
        let Some(new) = render_module_ref(&replacement.new) else {
            continue;
        };
        let local = replacement
            .new
            .path
            .as_deref()
            .filter(|path| path.starts_with('.'))
            .map(normalize_go_directory);
        if local
            .as_deref()
            .is_some_and(|directory| !retained.contains(directory))
        {
            continue;
        }
        output.push_str(&format!("\nreplace {old} => {new}\n"));
    }
    Ok(output)
}

#[derive(Debug)]
struct GoPruneKnowledge {
    domain: crate::prune_knowledge::PruneDomainId,
    work: GoWorkJson,
    directories: HashMap<String, String>,
    dependencies: HashMap<String, Vec<String>>,
}

impl GoPruneKnowledge {
    fn new(workspace: &DiscoveredWorkspace) -> Self {
        let directories = workspace
            .modules
            .iter()
            .map(|module| {
                (
                    module.module_path.clone(),
                    module.directory.to_unix().to_string(),
                )
            })
            .collect();
        let dependencies = workspace
            .modules
            .iter()
            .map(|module| {
                let dependencies = module
                    .relationships
                    .iter()
                    .filter_map(|relationship| match relationship.target() {
                        crate::relationships::RelationshipTarget::Internal(target) => {
                            Some(target.clone())
                        }
                        crate::relationships::RelationshipTarget::UnresolvedExternal { .. } => None,
                    })
                    .collect();
                (module.module_path.clone(), dependencies)
            })
            .collect();
        Self {
            domain: crate::prune_knowledge::GO_PRUNE_DOMAIN.clone(),
            work: workspace.work.clone(),
            directories,
            dependencies,
        }
    }
}

impl PruneDomain for GoPruneKnowledge {
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
        let mut retained: HashSet<String> = kept_packages.iter().cloned().collect();
        let mut pending = kept_packages.to_vec();
        while let Some(package) = pending.pop() {
            for dependency in self.dependencies.get(&package).into_iter().flatten() {
                if retained.insert(dependency.clone()) {
                    pending.push(dependency.clone());
                }
            }
        }
        let mut directories = retained
            .iter()
            .filter_map(|package| self.directories.get(package).cloned())
            .collect::<Vec<_>>();
        directories.sort();
        let work = render_pruned_go_work(&self.work, &directories)
            .map_err(|error| crate::prune_knowledge::Error::Failed(Box::new(error)))?;
        let mut extra_packages = retained
            .into_iter()
            .filter(|package| !kept_packages.contains(package))
            .collect::<Vec<_>>();
        extra_packages.sort();
        Ok(Some(PrunePlan {
            extra_packages,
            root_files: vec![(GO_WORK.to_string(), work)],
            copy_paths: vec![GO_WORK_SUM.to_string()],
        }))
    }
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
            let (workspace, toolchain, listed) = turborepo_rayon_compat::block_in_place(|| {
                let workspace = discover_workspace(&self.repo_root)?;
                if workspace.modules.is_empty() {
                    return Ok::<_, Error>((workspace, None, Vec::new()));
                }
                let toolchain = go_toolchain_identity(&self.repo_root)?;
                let listed = go_list_modules(&self.repo_root)?;
                Ok((workspace, Some(toolchain), listed))
            })
            .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;

            let workspace_roots = self
                .repo_root
                .join_component(GO_WORK)
                .exists()
                .then(|| WorkspaceRoot::new("go", self.repo_root.clone()))
                .into_iter()
                .collect();

            let Some(toolchain_identity) = toolchain else {
                return Ok(DiscoveredPackages::new(Vec::new(), workspace_roots));
            };
            let cache_prefixes =
                go_cache_prefixes(&self.repo_root, &toolchain_identity.environment);
            let workspace_patterns = workspace
                .modules
                .iter()
                .map(|module| format!("./{}/...", module.directory.to_unix()))
                .collect::<Vec<_>>();
            let mut packages = workspace
                .modules
                .iter()
                .map(|module| {
                    package_from_module(
                        module,
                        true,
                        &toolchain_identity.target_os,
                        &cache_prefixes,
                        &workspace_patterns,
                    )
                })
                .collect::<Vec<_>>();
            let workspace_kind = GoPackageKind::Workspace;
            let workspace_relationships = workspace
                .modules
                .iter()
                .map(|module| {
                    Relationship::internal(module.module_path.clone(), DependencyKind::Production)
                })
                .collect();
            packages.push(
                DiscoveredPackage::aggregate(
                    GO_WORKSPACE_SCOPE.to_string(),
                    PackageJson::default(),
                    self.repo_root.join_component(GO_WORK),
                )
                .with_native_relationships(workspace_relationships)
                .with_native_tasks(go_tasks_for_package(
                    &workspace_kind,
                    &toolchain_identity.target_os,
                    &workspace_patterns,
                ))
                .with_task_contract(crate::task_contracts::ScopeTaskContract::go(
                    GoTaskContract::new(
                        workspace_kind,
                        true,
                        toolchain_identity.target_os.clone(),
                        cache_prefixes.clone(),
                    ),
                )),
            );
            let resolutions = external_resolutions(
                &workspace.graph,
                &workspace,
                &listed,
                &toolchain_identity.package,
            )
            .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;
            let members = resolutions
                .iter()
                .map(|resolution| resolution.package().to_string())
                .collect::<Vec<_>>();
            let anchored = |path| {
                AnchoredSystemPathBuf::from_raw(path)
                    .map_err(|error| toolchain::Error::Failed(Box::new(Error::Path(error))))
            };
            let mut definition_sources = vec![anchored(GO_WORK)?];
            if self.repo_root.join_component(GO_WORK_SUM).exists() {
                definition_sources.push(anchored(GO_WORK_SUM)?);
            }
            definition_sources.extend(workspace.modules.iter().flat_map(|module| {
                let mut paths = vec![module.directory.join_component(GO_MOD)];
                if self
                    .repo_root
                    .resolve(&module.directory)
                    .join_component(GO_SUM)
                    .exists()
                {
                    paths.push(module.directory.join_component(GO_SUM));
                }
                paths
            }));
            let resolution = ExternalResolutionDomain::new(
                GO_RESOLUTION_DOMAIN.clone(),
                ToolchainId::GO,
                AnchoredSystemPathBuf::default(),
                members,
                definition_sources,
                ExternalResolutionData::Resolved {
                    completeness: ResolutionCompleteness::Complete,
                    packages: resolutions,
                },
            );
            let change_observation = go_change_observation(&workspace.modules, &cache_prefixes);
            let prune = GoPruneKnowledge::new(&workspace);
            Ok(DiscoveredPackages::new(packages, workspace_roots)
                .with_external_resolution(resolution)
                .with_change_observation(change_observation)
                .with_prune_domain(Arc::new(prune)))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn go_available() -> bool {
        which::which("go").is_ok()
    }

    fn write_workspace(root: &AbsoluteSystemPath, layout: &[(&str, &str, &str)]) {
        fs::create_dir_all(root.as_std_path()).unwrap();
        let mut work = String::from("go 1.22\n\nuse (\n");
        for (dir, module_path, contents) in layout {
            let module_dir = join_relative_path(root, dir).unwrap();
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
        if !go_available() {
            return;
        }

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
        if !go_available() {
            return;
        }

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
        if !go_available() {
            return;
        }

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

    #[cfg(unix)]
    #[test]
    fn rejects_member_symlink_outside_repository() {
        use std::os::unix::fs::symlink;

        if !go_available() {
            return;
        }

        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(
            outside.path().join(GO_MOD),
            "module example.com/outside\n\ngo 1.22\n",
        )
        .unwrap();
        symlink(outside.path(), tempdir.path().join("linked")).unwrap();
        root.join_component(GO_WORK)
            .create_with_contents("go 1.22\n\nuse ./linked\n")
            .unwrap();

        let error = discover_workspace(&root).expect_err("symlinked outside members fail");
        assert!(
            error.to_string().contains("outside the repository"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn reports_unnamed_workspace_module() {
        if !go_available() {
            return;
        }

        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let module = root.join_component("module");
        module.create_dir_all().unwrap();
        module
            .join_component(GO_MOD)
            .create_with_contents("go 1.22\n")
            .unwrap();
        root.join_component(GO_WORK)
            .create_with_contents("go 1.22\n\nuse ./module\n")
            .unwrap();

        let error = discover_workspace(&root).expect_err("unnamed modules fail");
        assert!(
            matches!(error, Error::MissingModulePath { .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_reserved_workspace_identity() {
        if !go_available() {
            return;
        }

        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        write_workspace(&root, &[("module", GO_WORKSPACE_SCOPE, "")]);
        let error = discover_workspace(&root).expect_err("reserved identity fails");
        assert!(matches!(error, Error::WorkspaceNameCollision { .. }));
    }

    #[test]
    fn rejects_root_module_as_workspace_member() {
        if !go_available() {
            return;
        }

        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(GO_MOD)
            .create_with_contents("module example.com/root\n\ngo 1.22\n")
            .unwrap();
        root.join_component("root.go")
            .create_with_contents("package root\n")
            .unwrap();
        root.join_component(GO_WORK)
            .create_with_contents("go 1.22\n\nuse .\n")
            .unwrap();
        let error = discover_workspace(&root).expect_err("root workspace modules fail");
        assert!(matches!(error, Error::RootDefinitionCollision { .. }));
    }

    #[test]
    fn rejects_local_replacement_to_nonmember() {
        if !go_available() {
            return;
        }

        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        write_workspace(
            &root,
            &[(
                "apps/api",
                "example.com/api",
                "module example.com/api\n\ngo 1.22\n\nrequire example.com/local v0.0.0\n\nreplace \
                 example.com/local => ../../packages/local\n",
            )],
        );
        let local = root.join_components(&["packages", "local"]);
        local.create_dir_all().unwrap();
        local
            .join_component(GO_MOD)
            .create_with_contents("module example.com/local\n\ngo 1.22\n")
            .unwrap();
        local
            .join_component("local.go")
            .create_with_contents("package local\n")
            .unwrap();
        root.join_components(&["apps", "api", "api.go"])
            .create_with_contents("package api\n\nimport _ \"example.com/local\"\n")
            .unwrap();

        let error = discover_workspace(&root).expect_err("nonmember replacement fails");
        assert!(matches!(error, Error::NonMemberLocalModule { .. }));
    }

    #[test]
    fn models_internal_relationships() {
        if !go_available() {
            return;
        }

        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        write_workspace(
            &root,
            &[
                (
                    "apps/api",
                    "example.com/api",
                    "module example.com/api\n\ngo 1.22\n\nrequire example.com/lib \
                     v0.0.0\n\nreplace example.com/lib => ../../packages/lib\n",
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
        let module_paths =
            HashSet::from(["example.com/api".to_string(), "example.com/lib".to_string()]);
        let relationships = relationships_from_mod_graph(graph, &module_paths).unwrap();
        assert_eq!(relationships["example.com/api"].len(), 1);
    }

    #[test]
    fn native_task_table_renders_exact_module_and_workspace_commands() {
        let runnable = GoRunnable {
            import_path: "example.com/api/cmd/server".to_string(),
            output_name: "server".to_string(),
        };
        let module = GoPackageKind::Module {
            runnable: Some(runnable),
        };
        let patterns = vec![
            "./apps/api/...".to_string(),
            "./packages/lib/...".to_string(),
        ];
        let tasks = go_tasks_for_package(&module, "linux", &patterns);
        let build = tasks
            .iter()
            .find(|task| task.name() == "build")
            .expect("build task");
        let command = build.command().expect("build command");
        assert_eq!(
            command.arguments.prefix,
            ["build", "-o", "dist/server"].map(str::to_string)
        );
        assert_eq!(
            command.arguments.suffix,
            ["example.com/api/cmd/server"].map(str::to_string)
        );
        assert_eq!(
            command.arguments.pass_through_placement,
            PassThroughPlacement::BeforeSuffix
        );
        assert_eq!(
            build.contract().entrypoint(),
            Some(TaskEntrypoint::Preferred)
        );
        assert!(tasks.iter().any(|task| task.name() == "run"));
        assert!(tasks.iter().any(|task| task.name() == "dev"));
        assert!(matches!(
            tasks
                .iter()
                .find(|task| task.name() == "lint")
                .expect("lint aggregate")
                .execution(),
            crate::native_tasks::NativeTaskExecution::Aggregate(children)
                if children.as_ref() == ["vet"]
        ));

        let workspace = go_tasks_for_package(&GoPackageKind::Workspace, "linux", &patterns);
        let test = workspace
            .iter()
            .find(|task| task.name() == "test")
            .and_then(NativeTask::command)
            .expect("workspace test command");
        assert_eq!(test.arguments.prefix, ["test"].map(str::to_string));
        assert_eq!(test.arguments.suffix, patterns);
    }

    #[test]
    fn library_modules_do_not_receive_unsafe_run_defaults() {
        let tasks = go_tasks_for_package(&GoPackageKind::Module { runnable: None }, "linux", &[]);
        assert!(!tasks.iter().any(|task| task.name() == "run"));
        assert!(!tasks.iter().any(|task| task.name() == "dev"));
        let build = tasks
            .iter()
            .find(|task| task.name() == "build")
            .expect("build task");
        assert_eq!(build.contract().defaults().cache, Some(false));
    }

    #[test]
    fn pruned_go_work_is_deterministic_and_preserves_directives() {
        let work = GoWorkJson {
            go: Some("1.24.0".to_string()),
            toolchain: Some("go1.24.2".to_string()),
            godebug: Some(vec![GoWorkGodebug {
                key: "gotypesalias".to_string(),
                value: "1".to_string(),
            }]),
            ..Default::default()
        };
        let rendered =
            render_pruned_go_work(&work, &["packages/lib".to_string(), "apps/api".to_string()])
                .expect("go.work renders");
        assert_eq!(
            rendered,
            "go 1.24.0\n\ntoolchain go1.24.2\n\ngodebug gotypesalias=1\n\nuse \
             (\n\t./apps/api\n\t./packages/lib\n)\n"
        );
    }

    #[test]
    fn external_resolution_is_scoped_to_each_module_closure() {
        let root = AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" })
            .expect("root path");
        let workspace = DiscoveredWorkspace {
            modules: vec![
                GoModule {
                    module_path: "example.com/a".to_string(),
                    manifest_path: root.join_components(&["a", GO_MOD]),
                    directory: AnchoredSystemPathBuf::from_raw("a").expect("relative path"),
                    relationships: Vec::new(),
                    runnable: None,
                },
                GoModule {
                    module_path: "example.com/b".to_string(),
                    manifest_path: root.join_components(&["b", GO_MOD]),
                    directory: AnchoredSystemPathBuf::from_raw("b").expect("relative path"),
                    relationships: Vec::new(),
                    runnable: None,
                },
            ],
            work: GoWorkJson::default(),
            graph: "example.com/a example.net/one@v1.0.0\nexample.com/b example.net/two@v2.0.0\n"
                .to_string(),
        };
        let listed = vec![
            GoListModule {
                path: "example.net/one".to_string(),
                version: "v1.0.0".to_string(),
                sum: "h1:one".to_string(),
                go_mod_sum: "h1:one-mod".to_string(),
                main: false,
                replace: None,
            },
            GoListModule {
                path: "example.net/two".to_string(),
                version: "v2.0.0".to_string(),
                sum: "h1:two".to_string(),
                go_mod_sum: "h1:two-mod".to_string(),
                main: false,
                replace: None,
            },
        ];
        let toolchain = ExternalPackageIdentity::new("go", "go1.24");
        let resolutions = external_resolutions(&workspace.graph, &workspace, &listed, &toolchain)
            .expect("resolution");
        let a = resolutions
            .iter()
            .find(|resolution| resolution.package() == "example.com/a")
            .expect("a resolution");
        assert!(
            a.identities()
                .iter()
                .any(|identity| identity.key() == "example.net/one")
        );
        assert!(
            !a.identities()
                .iter()
                .any(|identity| identity.key() == "example.net/two")
        );
        let aggregate = resolutions
            .iter()
            .find(|resolution| resolution.package() == GO_WORKSPACE_SCOPE)
            .expect("aggregate resolution");
        assert_eq!(aggregate.identities().len(), 3);
    }

    #[test]
    fn external_resolution_traverses_internal_workspace_dependencies() {
        let root = AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" })
            .expect("root path");
        let workspace = DiscoveredWorkspace {
            modules: vec![
                GoModule {
                    module_path: "example.com/app".to_string(),
                    manifest_path: root.join_components(&["app", GO_MOD]),
                    directory: AnchoredSystemPathBuf::from_raw("app").expect("relative path"),
                    relationships: Vec::new(),
                    runnable: None,
                },
                GoModule {
                    module_path: "example.com/lib".to_string(),
                    manifest_path: root.join_components(&["lib", GO_MOD]),
                    directory: AnchoredSystemPathBuf::from_raw("lib").expect("relative path"),
                    relationships: Vec::new(),
                    runnable: None,
                },
            ],
            work: GoWorkJson::default(),
            graph: "example.com/app example.com/lib@v0.0.0\nexample.com/lib \
                    example.net/shared@v1.0.0\n"
                .to_string(),
        };
        let listed = vec![GoListModule {
            path: "example.net/shared".to_string(),
            version: "v1.0.0".to_string(),
            sum: "h1:shared".to_string(),
            go_mod_sum: "h1:shared-mod".to_string(),
            main: false,
            replace: None,
        }];
        let resolutions = external_resolutions(
            &workspace.graph,
            &workspace,
            &listed,
            &ExternalPackageIdentity::new("go", "go1.24"),
        )
        .expect("resolution");
        for package in ["example.com/app", "example.com/lib", GO_WORKSPACE_SCOPE] {
            let resolution = resolutions
                .iter()
                .find(|resolution| resolution.package() == package)
                .expect("package resolution");
            assert!(
                resolution
                    .identities()
                    .iter()
                    .any(|identity| identity.key() == "example.net/shared"),
                "{package} must include the transitive shared module"
            );
        }
    }

    #[test]
    fn external_resolution_rejects_unknown_graph_nodes() {
        let root = AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" })
            .expect("root path");
        let workspace = DiscoveredWorkspace {
            modules: vec![GoModule {
                module_path: "example.com/app".to_string(),
                manifest_path: root.join_components(&["app", GO_MOD]),
                directory: AnchoredSystemPathBuf::from_raw("app").expect("relative path"),
                relationships: Vec::new(),
                runnable: None,
            }],
            work: GoWorkJson::default(),
            graph: "example.com/app example.net/missing@v1.0.0\n".to_string(),
        };
        let error = external_resolutions(
            &workspace.graph,
            &workspace,
            &[],
            &ExternalPackageIdentity::new("go", "go1.24"),
        )
        .expect_err("unknown module graph nodes fail");
        assert!(matches!(error, Error::UnknownResolutionModule { .. }));
    }

    #[test]
    fn replacement_and_integrity_facts_change_external_identity() {
        let base = GoListModule {
            path: "example.net/dependency".to_string(),
            version: "v1.0.0".to_string(),
            sum: "h1:archive".to_string(),
            go_mod_sum: "h1:manifest".to_string(),
            main: false,
            replace: None,
        };
        let original = module_identity(&base);
        let replaced = module_identity(&GoListModule {
            replace: Some(Box::new(GoListModule {
                path: "example.net/fork".to_string(),
                version: "v1.0.1".to_string(),
                sum: "h1:fork-archive".to_string(),
                go_mod_sum: "h1:fork-manifest".to_string(),
                main: false,
                replace: None,
            })),
            ..base
        });
        assert_ne!(original, replaced);
        assert!(replaced.version().contains("replace=path=example.net/fork"));
        assert!(replaced.version().contains("sum=h1:fork-archive"));
    }

    #[test]
    fn toolchain_details_include_behavior_but_exclude_paths_and_credentials() {
        let root = AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" })
            .expect("root path");
        let values = BTreeMap::from([
            ("GOOS".to_string(), serde_json::json!("linux")),
            ("GOARCH".to_string(), serde_json::json!("amd64")),
            (
                "GOFLAGS".to_string(),
                serde_json::json!(format!("-modfile={}/alternate.mod", root)),
            ),
            (
                "GOCACHE".to_string(),
                serde_json::json!(format!("{}/.cache/go", root)),
            ),
            (
                "GOPROXY".to_string(),
                serde_json::json!("https://token@example.test"),
            ),
        ]);
        let details = stable_toolchain_details(&root, &values);
        assert!(details.contains("GOOS=linux"));
        assert!(details.contains("GOARCH=amd64"));
        assert!(details.contains("GOFLAGS=-modfile=$TURBO_ROOT$/alternate.mod"));
        assert!(!details.contains("GOCACHE"));
        assert!(!details.contains("GOPROXY"));
        assert!(!details.contains(root.as_str()));
    }

    #[test]
    fn change_observation_covers_workspace_module_sums_and_caches() {
        let module = GoModule {
            module_path: "example.com/api".to_string(),
            manifest_path: AbsoluteSystemPathBuf::new(if cfg!(windows) {
                r"C:\repo\apps\api\go.mod"
            } else {
                "/repo/apps/api/go.mod"
            })
            .expect("manifest path"),
            directory: AnchoredSystemPathBuf::from_raw("apps/api").expect("relative path"),
            relationships: Vec::new(),
            runnable: None,
        };
        let observation = go_change_observation(&[module], &[".cache/go-build".to_string()]);
        assert_eq!(
            observation,
            ChangeObservation::new()
                .with_rediscovery_file_name(GO_WORK)
                .with_rediscovery_file_name(GO_MOD)
                .with_resolution_path(GO_WORK_SUM)
                .with_resolution_path("apps/api/go.sum")
                .with_ignore_prefix(".cache/go-build")
        );
    }
}
