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
        NativeTaskContract, PassThroughPlacement, TaskEntrypoint, WorkingDirectoryPolicy,
    },
    package_json::PackageJson,
    relationships::{DependencyKind, Relationship},
    task_contracts::DependencySourceInputs,
    toolchain::{
        self, DerivedInputSafety, DerivedOutputs, DiscoverPackagesFuture, DiscoveredPackage,
        DiscoveredPackages, RepositoryContributor, ToolchainId, WorkspaceRoot,
    },
};

/// The conventional file name for a Go workspace definition.
pub const GO_WORK: &str = "go.work";

/// The conventional file name for a Go module manifest.
pub const GO_MOD: &str = "go.mod";

const GO_SUM: &str = "go.sum";
const GO_WORK_SUM: &str = "go.work.sum";
const GO_DIST_DIR: &str = "dist";

/// Deterministic identity for the workspace-wide Go verification scope.
pub const GO_WORKSPACE_NAME: &str = "go-workspace";

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
    #[error("`go env` did not return requested variable {name}")]
    MissingEnvironmentVariable { name: &'static str },
    #[error("failed to serialize normalized Go toolchain identity: {0}")]
    IdentitySerialize(serde_json::Error),
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
    #[error("`go mod graph` references {module}, but `go list -m all` did not resolve it")]
    UnknownResolutionModule { module: String },
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
    /// Package pattern for the sole `main` package used by the `dev` task, when
    /// unambiguous.
    pub runnable_target: Option<String>,
}

/// The result of Go workspace discovery.
#[derive(Debug, Default)]
pub struct DiscoveredWorkspace {
    pub modules: Vec<GoModule>,
    graph: String,
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
    #[allow(dead_code)]
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

#[derive(Debug, Deserialize)]
struct GoListPackage {
    #[serde(rename = "Dir")]
    directory: String,
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
struct GoEnvironment {
    target_os: String,
    build_cache: String,
    module_cache: String,
    fingerprint_values: BTreeMap<String, String>,
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

fn go_list_modules(repo_root: &AbsoluteSystemPath) -> Result<Vec<GoListModule>, Error> {
    const COMMAND: &str = "go list -mod=readonly -m -json all";
    let output = run_go(
        repo_root,
        &["list", "-mod=readonly", "-m", "-json", "all"],
        COMMAND,
    )?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: COMMAND,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<GoListModule>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::CommandParse {
            command: COMMAND,
            source,
        })
}

fn normalize_checkout_path(repo_root: &AbsoluteSystemPath, value: &str) -> String {
    let native_root = repo_root.as_str().trim_end_matches(['/', '\\']);

    #[cfg(windows)]
    {
        // Go may report paths with either separator on Windows, and drive
        // letters are case-insensitive. Normalize separators before replacing
        // checkout roots while preserving the case of all non-root text.
        let slash_root = native_root.replace('\\', "/");
        let slash_value = value.replace('\\', "/");
        let folded_root = slash_root.to_ascii_lowercase();
        let folded_value = slash_value.to_ascii_lowercase();
        let mut normalized = String::with_capacity(slash_value.len());
        let mut copied = 0;
        for (index, _) in folded_value.match_indices(&folded_root) {
            normalized.push_str(&slash_value[copied..index]);
            normalized.push_str("$REPO");
            copied = index + slash_root.len();
        }
        normalized.push_str(&slash_value[copied..]);
        normalized
    }

    #[cfg(not(windows))]
    {
        value.replace(native_root, "$REPO")
    }
}

fn normalized_go_toolchain_version(
    repo_root: &AbsoluteSystemPath,
    go_version: &str,
    values: &BTreeMap<String, String>,
) -> Result<String, Error> {
    let mut identity = values
        .iter()
        .map(|(name, value)| (name.clone(), normalize_checkout_path(repo_root, value)))
        .collect::<BTreeMap<_, _>>();
    identity.insert("go version".to_string(), go_version.trim().to_string());
    serde_json::to_string(&identity).map_err(Error::IdentitySerialize)
}

fn go_toolchain_identity(
    repo_root: &AbsoluteSystemPath,
    environment: &GoEnvironment,
) -> Result<ExternalPackageIdentity, Error> {
    const COMMAND: &str = "go version";
    let output = run_go(repo_root, &["version"], COMMAND)?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: COMMAND,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(ExternalPackageIdentity::new(
        "go",
        normalized_go_toolchain_version(
            repo_root,
            &String::from_utf8_lossy(&output.stdout),
            &environment.fingerprint_values,
        )?,
    )
    .with_human_name("go"))
}

fn fingerprinted_go_environment(
    values: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, Error> {
    FINGERPRINTED_GO_ENV_VARS
        .iter()
        .map(|name| {
            values
                .get(*name)
                .cloned()
                .map(|value| ((*name).to_string(), value))
                .ok_or(Error::MissingEnvironmentVariable { name })
        })
        .collect()
}

fn go_environment(repo_root: &AbsoluteSystemPath) -> Result<GoEnvironment, Error> {
    const COMMAND: &str = "go env -json <fingerprinted variables>";
    let mut requested = FINGERPRINTED_GO_ENV_VARS.to_vec();
    requested.extend(["GOCACHE", "GOMODCACHE"]);
    requested.sort_unstable();
    requested.dedup();
    let args = ["env", "-json"]
        .into_iter()
        .chain(requested)
        .collect::<Vec<_>>();
    let output = run_go(repo_root, &args, COMMAND)?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: COMMAND,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let values: BTreeMap<String, String> =
        serde_json::from_slice(&output.stdout).map_err(|source| Error::CommandParse {
            command: COMMAND,
            source,
        })?;
    let value = |name: &'static str| {
        values
            .get(name)
            .cloned()
            .ok_or(Error::MissingEnvironmentVariable { name })
    };
    Ok(GoEnvironment {
        target_os: value("GOOS")?,
        build_cache: value("GOCACHE")?,
        module_cache: value("GOMODCACHE")?,
        fingerprint_values: fingerprinted_go_environment(&values)?,
    })
}

fn runnable_target(module_dir: &AbsoluteSystemPath) -> Result<Option<String>, Error> {
    const COMMAND: &str = "go list -find -json ./...";

    let output = run_go(module_dir, &["list", "-find", "-json", "./..."], COMMAND)?;
    // The native dev default is optional. If Go cannot classify every package
    // without error, omit it rather than making workspace discovery fail.
    if !output.status.success() {
        return Ok(None);
    }

    let packages =
        serde_json::Deserializer::from_slice(&output.stdout).into_iter::<GoListPackage>();
    let mut runnable = None;
    for package in packages {
        let package = package.map_err(|source| Error::CommandParse {
            command: COMMAND,
            source,
        })?;
        if package.name != "main" {
            continue;
        }
        let Ok(relative) = Path::new(&package.directory).strip_prefix(module_dir.as_std_path())
        else {
            return Ok(None);
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let target = if relative.is_empty() {
            ".".to_string()
        } else {
            format!("./{relative}")
        };
        if runnable.replace(target).is_some() {
            return Ok(None);
        }
    }
    Ok(runnable)
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
    modules: &[GoModule],
    listed: &[GoListModule],
    toolchain: &ExternalPackageIdentity,
) -> Result<Vec<PackageResolution>, Error> {
    let internal: HashSet<&str> = modules
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

    let mut resolutions = Vec::with_capacity(modules.len() + 1);
    let mut aggregate = HashSet::new();
    for module in modules {
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
    resolutions.push(PackageResolution::new(GO_WORKSPACE_NAME, aggregate));
    Ok(resolutions)
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
        go_mod_edit_json(repo_root, repo_root)
            .ok()
            .and_then(|json| {
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
        pending_replacements.push((member_dir.clone(), module_path.clone(), module_json.replace));

        modules.push(GoModule {
            module_path,
            manifest_path: member_dir.join_component(GO_MOD),
            relationships: Vec::new(),
            runnable_target: runnable_target(&member_dir)?,
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

    Ok(DiscoveredWorkspace { modules, graph })
}

fn go_command_task(
    name: &'static str,
    subcommand: &'static str,
    targets: Vec<String>,
    pass_through_placement: crate::native_tasks::PassThroughPlacement,
    cache: Option<bool>,
    entrypoint: crate::native_tasks::TaskEntrypoint,
    cwd_policy: crate::native_tasks::WorkingDirectoryPolicy,
) -> crate::native_tasks::NativeTask {
    use crate::native_tasks::{NativeCommandArguments, NativeCommandProgram, NativeTask};

    NativeTask::command_task(
        name,
        format!("go {subcommand} {}", targets.join(" ")),
        NativeCommandProgram::Tool("go".to_string()),
        NativeCommandArguments {
            prefix: vec![subcommand.to_string()],
            pass_through_placement,
            pass_through_separator: None,
            suffix: targets,
        },
        None,
        cwd_policy,
    )
    .with_contract(NativeTaskContract::new(
        toolchain::TaskDefaults { cache },
        Some(entrypoint),
        true,
    ))
}

fn runnable_output_name(module: &GoModule) -> Option<String> {
    let target = module.runnable_target.as_deref()?;
    if target == "." {
        return module
            .module_path
            .rsplit('/')
            .find(|component| !component.is_empty())
            .map(str::to_string);
    }
    target
        .trim_end_matches('/')
        .rsplit('/')
        .find(|component| !component.is_empty() && *component != ".")
        .map(str::to_string)
}

/// Build the conservative built-in task table for one Go module.
pub fn native_tasks_for_module(
    module: &GoModule,
    target_os: &str,
) -> Vec<crate::native_tasks::NativeTask> {
    let build_entrypoint = if module.runnable_target.is_some() {
        TaskEntrypoint::Preferred
    } else {
        TaskEntrypoint::Candidate
    };
    let output_name = runnable_output_name(module);
    let build_targets = match (&module.runnable_target, &output_name) {
        (Some(target), Some(output_name)) => {
            let extension = if target_os == "windows" { ".exe" } else { "" };
            vec![
                "-o".to_string(),
                format!("{GO_DIST_DIR}/{output_name}{extension}"),
                target.clone(),
            ]
        }
        _ => vec!["./...".to_string()],
    };
    let mut tasks = vec![
        go_command_task(
            "build",
            "build",
            build_targets,
            PassThroughPlacement::BeforeSuffix,
            output_name.is_none().then_some(false),
            build_entrypoint,
            WorkingDirectoryPolicy::PackageDirectory,
        ),
        go_command_task(
            "test",
            "test",
            vec!["./...".to_string()],
            PassThroughPlacement::BeforeSuffix,
            None,
            TaskEntrypoint::Candidate,
            WorkingDirectoryPolicy::PackageDirectory,
        ),
        go_command_task(
            "lint",
            "vet",
            vec!["./...".to_string()],
            PassThroughPlacement::BeforeSuffix,
            None,
            TaskEntrypoint::Candidate,
            WorkingDirectoryPolicy::PackageDirectory,
        ),
        go_command_task(
            "format",
            "fmt",
            vec!["./...".to_string()],
            PassThroughPlacement::BeforeSuffix,
            Some(false),
            TaskEntrypoint::Candidate,
            WorkingDirectoryPolicy::PackageDirectory,
        ),
    ];
    if let Some(target) = &module.runnable_target {
        tasks.push(go_command_task(
            "dev",
            "run",
            vec![target.clone()],
            PassThroughPlacement::AfterSuffix,
            Some(false),
            TaskEntrypoint::Candidate,
            WorkingDirectoryPolicy::PackageDirectory,
        ));
    }
    tasks
}

/// Build workspace-wide verification tasks over explicit module patterns.
pub fn native_tasks_for_workspace(
    module_patterns: &[String],
) -> Vec<crate::native_tasks::NativeTask> {
    use crate::native_tasks::NativeTask;

    let mut module_patterns = module_patterns.to_vec();
    module_patterns.sort();
    module_patterns.dedup();
    let mut tasks = [("test", "test"), ("lint", "vet"), ("format", "fmt")]
        .into_iter()
        .map(|(name, subcommand)| {
            go_command_task(
                name,
                subcommand,
                module_patterns.clone(),
                PassThroughPlacement::BeforeSuffix,
                (name == "format").then_some(false),
                TaskEntrypoint::PreferredOnly,
                WorkingDirectoryPolicy::RepositoryRoot,
            )
        })
        .collect::<Vec<_>>();
    tasks.push(NativeTask::contract_task(
        "build",
        NativeTaskContract::new(
            toolchain::TaskDefaults::default(),
            Some(TaskEntrypoint::Excluded),
            false,
        ),
    ));
    tasks
}

/// Effective `go env` values that alter compilation, package selection, tests,
/// module/workspace selection, target platform, or tool selection.
///
/// Values are captured from structured `go env` output so settings persisted
/// with `go env -w` participate even when they are absent from the process
/// environment. Checkout-root paths are normalized before fingerprinting.
const FINGERPRINTED_GO_ENV_VARS: &[&str] = &[
    // Tool selection and cgo compilation.
    "AR",
    "CC",
    "CGO_CFLAGS",
    "CGO_CPPFLAGS",
    "CGO_CXXFLAGS",
    "CGO_ENABLED",
    "CGO_FFLAGS",
    "CGO_LDFLAGS",
    "CXX",
    "GCCGO",
    "PKG_CONFIG",
    // Module/workspace mode, command flags (including build tags), experiments,
    // and runtime knobs that can alter tests.
    "GO111MODULE",
    "GOCACHEPROG",
    "GODEBUG",
    "GOEXPERIMENT",
    "GOFIPS140",
    "GOFLAGS",
    "GOTOOLCHAIN",
    "GOWORK",
    // Target platform and architecture feature levels.
    "GO386",
    "GOAMD64",
    "GOARCH",
    "GOARM",
    "GOARM64",
    "GOMIPS",
    "GOMIPS64",
    "GOOS",
    "GOPPC64",
    "GORISCV64",
    "GOWASM",
];

/// Process environment variables that alter Go compilation, package
/// selection, tests, or target platform. This existing direct task-hash input
/// remains alongside the effective `go env` fingerprint above.
pub const HASHED_ENV_VARS: &[&str] = &[
    "GO111MODULE",
    "GO386",
    "GOAMD64",
    "GOARCH",
    "GOARM",
    "GOARM64",
    "GOEXPERIMENT",
    "GOMIPS",
    "GOMIPS64",
    "GOOS",
    "GOTOOLCHAIN",
    "GOWASM",
    "CGO_ENABLED",
];

/// Machine-local Go variables required by task execution but excluded from
/// verbatim task hashes. Path-bearing behavior is already represented by its
/// checkout-normalized external identity; cache locations never participate.
pub(crate) const PROJECTED_ONLY_ENV_VARS: &[&str] = &[
    "GOCACHE",
    "GOMODCACHE",
    "AR",
    "CC",
    "CXX",
    "GCCGO",
    "GOCACHEPROG",
    "GOFLAGS",
    "GOWORK",
    "CGO_CFLAGS",
    "CGO_CPPFLAGS",
    "CGO_CXXFLAGS",
    "CGO_FFLAGS",
    "CGO_LDFLAGS",
    "PKG_CONFIG",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum GoContractKind {
    Module { output_name: Option<String> },
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoTaskContract {
    kind: GoContractKind,
    target_os: String,
    cache_prefixes: Vec<String>,
}

impl GoTaskContract {
    fn module(module: &GoModule, target_os: &str, cache_prefixes: &[String]) -> Self {
        Self {
            kind: GoContractKind::Module {
                output_name: runnable_output_name(module),
            },
            target_os: target_os.to_string(),
            cache_prefixes: cache_prefixes.to_vec(),
        }
    }

    fn workspace(target_os: &str, cache_prefixes: &[String]) -> Self {
        Self {
            kind: GoContractKind::Workspace,
            target_os: target_os.to_string(),
            cache_prefixes: cache_prefixes.to_vec(),
        }
    }

    pub(crate) fn dependency_source_inputs(&self) -> DependencySourceInputs {
        match &self.kind {
            GoContractKind::Module { .. } => DependencySourceInputs::Include,
            GoContractKind::Workspace => DependencySourceInputs::Exclude,
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
        if !matches!(task, "build" | "test" | "lint" | "format" | "dev") {
            return None;
        }

        let root_input = |path: &str| {
            if path_to_root.is_empty() {
                path.to_string()
            } else {
                format!("{path_to_root}/{path}")
            }
        };
        let mut io = toolchain::DerivedTaskIO {
            input_globs: [GO_WORK, GO_WORK_SUM].map(root_input).to_vec(),
            env: HASHED_ENV_VARS
                .iter()
                .map(|name| name.to_string())
                .collect(),
            forbidden_output_prefixes: vec![root_input(".git"), root_input(".turbo")],
            ..Default::default()
        };
        for prefix in &self.cache_prefixes {
            let prefix = root_input(prefix);
            io.input_globs.push(format!("!{prefix}/**"));
            io.forbidden_output_prefixes.push(prefix);
        }

        if wants_automatic_inputs {
            match &self.kind {
                GoContractKind::Module { .. } => {
                    io.package_default_inputs = Some(true);
                    io.input_globs.push(format!("!{GO_DIST_DIR}/**"));
                }
                GoContractKind::Workspace => io.package_default_inputs = Some(false),
            }
            for dependency in dependencies {
                match dependency.task_contract().dependency_source_inputs() {
                    DependencySourceInputs::Include => {
                        let directory = root_input(dependency.directory().to_unix().as_str());
                        io.input_globs.extend([
                            format!("{directory}/**"),
                            format!("!{directory}/.git/**"),
                            format!("!{directory}/.turbo/**"),
                            format!("!{directory}/{GO_DIST_DIR}/**"),
                        ]);
                    }
                    DependencySourceInputs::Exclude => {}
                    DependencySourceInputs::Unknown => {
                        io.input_safety = DerivedInputSafety::Untracked;
                        io.cache_reason =
                            Some("a Go dependency has unclassified source inputs".to_string());
                    }
                }
            }
            io.input_globs.sort();
            io.input_globs.dedup();
        }

        if task == "build" {
            io.outputs = match &self.kind {
                GoContractKind::Module {
                    output_name: Some(output_name),
                } if context.task_args.is_none_or(<[_]>::is_empty) => {
                    let extension = if self.target_os == "windows" {
                        ".exe"
                    } else {
                        ""
                    };
                    DerivedOutputs::Resolved(vec![format!(
                        "{GO_DIST_DIR}/{output_name}{extension}"
                    )])
                }
                GoContractKind::Module {
                    output_name: Some(_),
                } => {
                    io.cache_reason =
                        Some("Go build arguments can change the derived binary output".to_string());
                    DerivedOutputs::Unavailable
                }
                GoContractKind::Module { output_name: None } => {
                    io.cache_reason = Some(
                        "Go library builds have no stable outputs for Turborepo to restore"
                            .to_string(),
                    );
                    DerivedOutputs::Unavailable
                }
                GoContractKind::Workspace => DerivedOutputs::Resolved(Vec::new()),
            };
        }
        Some(io)
    }
}

fn go_cache_prefixes(repo_root: &AbsoluteSystemPath, environment: &GoEnvironment) -> Vec<String> {
    let mut prefixes = Vec::new();
    for path in [&environment.build_cache, &environment.module_cache] {
        let path = AbsoluteSystemPathBuf::from_unknown(repo_root, path);
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

fn go_change_observation(
    repo_root: &AbsoluteSystemPath,
    modules: &[GoModule],
    cache_prefixes: &[String],
) -> Result<ChangeObservation, Error> {
    let mut observation = ChangeObservation::new()
        .with_rediscovery_file_name(GO_WORK)
        .with_rediscovery_file_name(GO_MOD)
        .with_resolution_path(GO_WORK_SUM);
    for module in modules {
        let manifest = AnchoredSystemPathBuf::new(repo_root, &module.manifest_path)?;
        let directory = manifest.parent().ok_or_else(|| Error::MissingGoMod {
            path: module.manifest_path.to_string(),
        })?;
        observation = observation
            .with_resolution_path(directory.join_component(GO_SUM).to_unix().to_string());
    }
    for prefix in cache_prefixes {
        observation = observation.with_ignore_prefix(prefix.clone());
    }
    Ok(observation)
}

fn package_from_module(
    module: &GoModule,
    target_os: &str,
    cache_prefixes: &[String],
) -> DiscoveredPackage {
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
    .with_native_tasks(native_tasks_for_module(module, target_os))
    .with_task_contract(crate::task_contracts::ScopeTaskContract::go(
        GoTaskContract::module(module, target_os, cache_prefixes),
    ))
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
            let workspace =
                turborepo_rayon_compat::block_in_place(|| discover_workspace(&self.repo_root))
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;
            let environment =
                turborepo_rayon_compat::block_in_place(|| go_environment(&self.repo_root))
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;
            let cache_prefixes = go_cache_prefixes(&self.repo_root, &environment);
            let change_observation =
                go_change_observation(&self.repo_root, &workspace.modules, &cache_prefixes)
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;

            let workspace_roots = self
                .repo_root
                .join_component(GO_WORK)
                .exists()
                .then(|| WorkspaceRoot::new("go", self.repo_root.clone()))
                .into_iter()
                .collect();

            if workspace.modules.is_empty() {
                return Ok(DiscoveredPackages::new(Vec::new(), workspace_roots)
                    .with_change_observation(change_observation));
            }

            let (listed, toolchain_identity) = turborepo_rayon_compat::block_in_place(|| {
                Ok((
                    go_list_modules(&self.repo_root)?,
                    go_toolchain_identity(&self.repo_root, &environment)?,
                ))
            })
            .map_err(|error: Error| toolchain::Error::Failed(Box::new(error)))?;

            let mut module_patterns = workspace
                .modules
                .iter()
                .filter_map(|module| {
                    let directory = module.manifest_path.parent()?;
                    let directory =
                        turbopath::AnchoredSystemPathBuf::new(&self.repo_root, directory).ok()?;
                    Some(format!("./{}/...", directory.to_unix()))
                })
                .collect::<Vec<_>>();
            module_patterns.sort();

            let mut module_names = workspace
                .modules
                .iter()
                .map(|module| module.module_path.clone())
                .collect::<Vec<_>>();
            module_names.sort();
            let workspace_relationships = module_names
                .into_iter()
                .map(|name| Relationship::internal(name, DependencyKind::Production))
                .collect();

            let mut packages = workspace
                .modules
                .iter()
                .map(|module| package_from_module(module, &environment.target_os, &cache_prefixes))
                .collect::<Vec<_>>();
            packages.push(
                DiscoveredPackage::aggregate(
                    GO_WORKSPACE_NAME.to_string(),
                    PackageJson::default(),
                    self.repo_root.join_component(GO_WORK),
                )
                .with_native_relationships(workspace_relationships)
                .with_native_tasks(native_tasks_for_workspace(&module_patterns))
                .with_task_contract(crate::task_contracts::ScopeTaskContract::go(
                    GoTaskContract::workspace(&environment.target_os, &cache_prefixes),
                )),
            );

            let resolutions = external_resolutions(
                &workspace.graph,
                &workspace.modules,
                &listed,
                &toolchain_identity,
            )
            .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;
            let members = resolutions
                .iter()
                .map(|resolution| resolution.package().to_string())
                .collect::<Vec<_>>();
            let anchored = |path| {
                AnchoredSystemPathBuf::from_raw(path)
                    .map_err(Error::from)
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))
            };
            let mut definition_sources = vec![anchored(GO_WORK)?];
            if self.repo_root.join_component(GO_WORK_SUM).exists() {
                definition_sources.push(anchored(GO_WORK_SUM)?);
            }
            for module in &workspace.modules {
                let manifest = AnchoredSystemPathBuf::new(&self.repo_root, &module.manifest_path)
                    .map_err(Error::from)
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;
                let module_dir = manifest.parent().ok_or_else(|| {
                    toolchain::Error::Failed(Box::new(Error::MissingGoMod {
                        path: module.manifest_path.to_string(),
                    }))
                })?;
                let sum = module_dir.join_component(GO_SUM);
                definition_sources.push(manifest);
                if self.repo_root.resolve(&sum).exists() {
                    definition_sources.push(sum);
                }
            }
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

            Ok(DiscoveredPackages::new(packages, workspace_roots)
                .with_external_resolution(resolution)
                .with_change_observation(change_observation))
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

    fn complete_go_environment() -> BTreeMap<String, String> {
        FINGERPRINTED_GO_ENV_VARS
            .iter()
            .map(|name| ((*name).to_string(), String::new()))
            .collect()
    }

    fn checkout_root(name: &str) -> AbsoluteSystemPathBuf {
        let path = if cfg!(windows) {
            format!(r"C:\checkout\{name}")
        } else {
            format!("/checkout/{name}")
        };
        AbsoluteSystemPathBuf::new(&path).expect("test checkout root is absolute")
    }

    #[test]
    fn toolchain_fingerprint_covers_every_behavior_changing_go_environment_family() {
        let root = checkout_root("repo");
        let values = complete_go_environment();
        let baseline =
            normalized_go_toolchain_version(&root, "go version go1.24.0 linux/amd64", &values)
                .unwrap();

        for variable in FINGERPRINTED_GO_ENV_VARS {
            let mut changed = values.clone();
            changed.insert((*variable).to_string(), format!("changed-{variable}"));
            assert_ne!(
                normalized_go_toolchain_version(
                    &root,
                    "go version go1.24.0 linux/amd64",
                    &changed,
                )
                .unwrap(),
                baseline,
                "{variable} must invalidate the Go resolution fingerprint"
            );
        }
        assert_ne!(
            normalized_go_toolchain_version(&root, "go version go1.25.0 linux/amd64", &values,)
                .unwrap(),
            baseline,
            "the selected Go compiler version must invalidate the fingerprint"
        );

        for variable in [
            "GOOS",
            "GOARCH",
            "GOTOOLCHAIN",
            "GOFLAGS",
            "GODEBUG",
            "CC",
            "CGO_CFLAGS",
        ] {
            assert!(
                FINGERPRINTED_GO_ENV_VARS.contains(&variable),
                "{variable} represents a required target, toolchain, build-tag/test, or cgo family"
            );
        }
    }

    #[test]
    fn toolchain_fingerprint_normalizes_checkout_paths_and_ignores_local_state() {
        let first_root = checkout_root("first");
        let second_root = checkout_root("second");
        let separator = std::path::MAIN_SEPARATOR;
        let first_prefix = first_root.as_str();
        let mut first = complete_go_environment();
        first.extend([
            (
                "CC".to_string(),
                format!(
                    "{first_prefix}{separator}tools{separator}compiler{}",
                    std::env::consts::EXE_SUFFIX
                ),
            ),
            (
                "GOFLAGS".to_string(),
                format!(
                    "-tags=integration \
                     -overlay={first_prefix}{separator}config{separator}overlay.json"
                ),
            ),
            (
                "GOWORK".to_string(),
                format!("{first_prefix}{separator}go.work"),
            ),
        ]);
        let mut second = first.clone();
        for value in second.values_mut() {
            *value = value.replace(first_root.as_str(), second_root.as_str());
        }
        let normalized_first =
            normalized_go_toolchain_version(&first_root, "go version go1.24.0 linux/amd64", &first)
                .unwrap();
        assert_eq!(
            normalized_first,
            normalized_go_toolchain_version(
                &second_root,
                "go version go1.24.0 linux/amd64",
                &second,
            )
            .unwrap(),
        );
        assert!(
            normalized_first.contains(&format!("compiler{}", std::env::consts::EXE_SUFFIX)),
            "normalization must preserve the selected executable suffix"
        );
        for variable in [
            "AR",
            "CC",
            "CXX",
            "GCCGO",
            "GOCACHEPROG",
            "GOFLAGS",
            "GOWORK",
            "CGO_CFLAGS",
            "CGO_CPPFLAGS",
            "CGO_CXXFLAGS",
            "CGO_FFLAGS",
            "CGO_LDFLAGS",
            "PKG_CONFIG",
        ] {
            assert!(FINGERPRINTED_GO_ENV_VARS.contains(&variable));
            assert!(
                !HASHED_ENV_VARS.contains(&variable),
                "{variable} must not be hashed with its raw checkout path"
            );
            assert!(
                PROJECTED_ONLY_ENV_VARS.contains(&variable),
                "{variable} must remain available to Go task I/O"
            );
        }

        // Each tuple names one excluded family and its documented members:
        // mutable caches/temp paths; install/config/toolchain locations;
        // credentials and network resolution policy; host-derived duplicates;
        // telemetry and cosmetic output.
        const EXCLUDED_FAMILIES: &[(&str, &[&str])] = &[
            (
                "mutable cache and temporary paths",
                &["GOCACHE", "GOMODCACHE", "GOPATH", "GOTMPDIR"],
            ),
            (
                "install, configuration, and toolchain locations",
                &["GOBIN", "GOENV", "GOMOD", "GOROOT", "GOTOOLDIR"],
            ),
            (
                "credentials and network resolution policy",
                &[
                    "GOAUTH",
                    "GOINSECURE",
                    "GONOPROXY",
                    "GONOSUMDB",
                    "GOPRIVATE",
                    "GOPROXY",
                    "GOSUMDB",
                    "GOVCS",
                ],
            ),
            (
                "host-derived duplicate values",
                &["GOEXE", "GOGCCFLAGS", "GOHOSTARCH", "GOHOSTOS", "GOVERSION"],
            ),
            (
                "telemetry and cosmetic output",
                &["GOTELEMETRY", "GOTRACEBACK"],
            ),
        ];
        let selected = fingerprinted_go_environment(&first).unwrap();
        for (family, variables) in EXCLUDED_FAMILIES {
            for variable in *variables {
                assert!(
                    !FINGERPRINTED_GO_ENV_VARS.contains(variable),
                    "{variable} from {family} must remain excluded"
                );
                let mut with_local_state = first.clone();
                with_local_state.insert((*variable).to_string(), "machine-specific".to_string());
                assert_eq!(
                    fingerprinted_go_environment(&with_local_state).unwrap(),
                    selected,
                    "{variable} from {family} must not affect the identity"
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn toolchain_fingerprint_normalizes_windows_drive_case_and_path_separators() {
        let root = AbsoluteSystemPathBuf::new(r"C:\checkout\repo").unwrap();
        let mut values = complete_go_environment();
        values.insert(
            "CC".to_string(),
            r"c:/checkout/repo/tools\compiler.exe".to_string(),
        );
        values.insert(
            "GOFLAGS".to_string(),
            r"-overlay=c:\checkout\repo/config\overlay.json".to_string(),
        );

        let normalized =
            normalized_go_toolchain_version(&root, "go version go1.24.0 windows/amd64", &values)
                .unwrap();
        let identity: BTreeMap<String, String> = serde_json::from_str(&normalized).unwrap();

        assert_eq!(identity["CC"], "$REPO/tools/compiler.exe");
        assert_eq!(identity["GOFLAGS"], "-overlay=$REPO/config/overlay.json");
    }

    fn resolution_module(root: &AbsoluteSystemPath, path: &str) -> GoModule {
        let directory = path.rsplit('/').next().expect("module path component");
        GoModule {
            module_path: path.to_string(),
            manifest_path: root.join_components(&[directory, GO_MOD]),
            relationships: Vec::new(),
            runnable_target: None,
        }
    }

    fn resolution_root(tempdir: &tempfile::TempDir) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::try_from(tempdir.path())
            .expect("temporary repository root is absolute")
    }

    fn listed_module(path: &str, version: &str, sum: &str) -> GoListModule {
        GoListModule {
            path: path.to_string(),
            version: version.to_string(),
            sum: sum.to_string(),
            go_mod_sum: format!("{sum}-mod"),
            main: false,
            replace: None,
        }
    }

    fn resolution_keys<'a>(
        resolutions: &'a [PackageResolution],
        package: &str,
    ) -> HashSet<&'a str> {
        resolution_for(resolutions, package)
            .identities()
            .iter()
            .map(ExternalPackageIdentity::key)
            .collect()
    }

    fn resolution_for<'a>(
        resolutions: &'a [PackageResolution],
        package: &str,
    ) -> &'a PackageResolution {
        resolutions
            .iter()
            .find(|resolution| resolution.package() == package)
            .expect("package resolution")
    }

    #[test]
    fn external_resolution_scopes_shared_and_disjoint_closures() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = resolution_root(&tempdir);
        let modules = vec![
            resolution_module(&root, "example.com/app"),
            resolution_module(&root, "example.com/lib"),
            resolution_module(&root, "example.com/other"),
        ];
        let graph = "example.com/app example.com/lib@v0.0.0\nexample.com/lib \
                     example.net/shared@v1.0.0\nexample.com/other example.net/disjoint@v2.0.0\n";
        let listed = vec![
            listed_module("example.net/shared", "v1.0.0", "h1:shared"),
            listed_module("example.net/disjoint", "v2.0.0", "h1:disjoint"),
        ];

        let resolutions = external_resolutions(
            graph,
            &modules,
            &listed,
            &ExternalPackageIdentity::new("go", "go1.24"),
        )
        .expect("resolution succeeds");
        let app = resolution_keys(&resolutions, "example.com/app");
        let lib = resolution_keys(&resolutions, "example.com/lib");
        let other = resolution_keys(&resolutions, "example.com/other");
        let aggregate = resolution_keys(&resolutions, GO_WORKSPACE_NAME);

        for closure in [&app, &lib, &other, &aggregate] {
            assert!(closure.contains("go"));
        }
        for closure in [&app, &lib] {
            assert!(closure.contains("example.net/shared"));
            assert!(!closure.contains("example.net/disjoint"));
            assert!(!closure.contains("example.com/lib"));
        }
        assert!(other.contains("example.net/disjoint"));
        assert!(!other.contains("example.net/shared"));
        assert_eq!(
            aggregate,
            HashSet::from(["go", "example.net/shared", "example.net/disjoint"])
        );

        let changed = external_resolutions(
            graph,
            &modules,
            &[
                listed_module("example.net/shared", "v1.0.0", "h1:changed"),
                listed_module("example.net/disjoint", "v2.0.0", "h1:disjoint"),
            ],
            &ExternalPackageIdentity::new("go", "go1.24"),
        )
        .expect("changed resolution succeeds");
        for package in ["example.com/app", "example.com/lib", GO_WORKSPACE_NAME] {
            assert_ne!(
                resolution_for(&resolutions, package).identities(),
                resolution_for(&changed, package).identities()
            );
        }
        assert_eq!(
            resolution_for(&resolutions, "example.com/other").identities(),
            resolution_for(&changed, "example.com/other").identities()
        );

        let changed_toolchain = external_resolutions(
            graph,
            &modules,
            &listed,
            &ExternalPackageIdentity::new("go", "go1.25"),
        )
        .expect("changed toolchain resolution succeeds");
        for package in [
            "example.com/app",
            "example.com/lib",
            "example.com/other",
            GO_WORKSPACE_NAME,
        ] {
            assert_ne!(
                resolution_for(&resolutions, package).identities(),
                resolution_for(&changed_toolchain, package).identities(),
                "toolchain identity must participate in every Go resolution row"
            );
        }
    }

    #[test]
    fn replacement_and_integrity_facts_change_external_identity() {
        let module = listed_module("example.net/dependency", "v1.0.0", "h1:archive");
        let original = module_identity(&module);
        let replaced = module_identity(&GoListModule {
            replace: Some(Box::new(GoListModule {
                path: "example.net/fork".to_string(),
                version: "v1.0.1".to_string(),
                sum: "h1:fork-archive".to_string(),
                go_mod_sum: "h1:fork-manifest".to_string(),
                main: false,
                replace: None,
            })),
            ..module
        });

        assert_ne!(original, replaced);
        assert!(replaced.version().contains("replace=path=example.net/fork"));
        assert!(replaced.version().contains("sum=h1:fork-archive"));
        assert!(replaced.version().contains("go_mod_sum=h1:fork-manifest"));
    }

    #[test]
    fn external_resolution_rejects_unresolved_graph_modules() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = resolution_root(&tempdir);
        let modules = vec![resolution_module(&root, "example.com/app")];
        let error = external_resolutions(
            "example.com/app example.net/missing@v1.0.0\n",
            &modules,
            &[],
            &ExternalPackageIdentity::new("go", "go1.24"),
        )
        .expect_err("unknown module graph nodes fail");

        assert!(matches!(
            error,
            Error::UnknownResolutionModule { module }
                if module == "example.net/missing@v1.0.0"
        ));
    }

    #[test]
    fn external_resolution_accepts_windows_line_endings() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = resolution_root(&tempdir);
        let resolutions = external_resolutions(
            "example.com/app example.net/dependency@v1.0.0\r\n",
            &[resolution_module(&root, "example.com/app")],
            &[listed_module(
                "example.net/dependency",
                "v1.0.0",
                "h1:dependency",
            )],
            &ExternalPackageIdentity::new("go", "go1.24 windows/amd64"),
        )
        .expect("CRLF resolution succeeds");

        let app = resolution_keys(&resolutions, "example.com/app");
        assert_eq!(app, HashSet::from(["go", "example.net/dependency"]));
    }

    fn task_context<'a>(
        root: &'a AbsoluteSystemPath,
        name: &str,
        directory: &'a str,
        tasks: Vec<crate::native_tasks::NativeTask>,
        kind: crate::package_graph::PackageTaskContextKind,
        contract: crate::task_contracts::ScopeTaskContract,
    ) -> crate::package_graph::PackageTaskContext<'a> {
        crate::package_graph::PackageTaskContext::new_for_test_with_native_tasks(
            name.into(),
            root,
            turbopath::AnchoredSystemPath::new(directory).unwrap(),
            kind,
            Some(&ToolchainId::GO),
            Some(tasks),
            Some(contract),
        )
    }

    fn resolve_go_cmd(
        context: &crate::package_graph::PackageTaskContext<'_>,
        task: &str,
        pass_through_args: Option<&[String]>,
        override_command: Option<&[String]>,
    ) -> crate::toolchain::TaskCommand {
        let native_task = context.native_tasks().get(task).expect("native Go task");
        crate::native_tasks::resolve_task_command(
            context,
            native_task,
            None,
            None,
            Some(Path::new("go")),
            pass_through_args,
            override_command,
        )
        .unwrap()
        .expect("Go command resolves")
    }

    fn task_cache(
        context: &crate::package_graph::PackageTaskContext<'_>,
        task: &str,
    ) -> Option<bool> {
        context
            .native_tasks()
            .get(task)
            .expect("native Go task")
            .contract()
            .defaults()
            .cache
    }

    #[test]
    fn module_task_table_renders_commands_and_argument_placement() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        assert!(root.as_std_path().is_absolute());
        let module = GoModule {
            module_path: "example.com/api".to_string(),
            manifest_path: root.join_components(&["apps", "api", GO_MOD]),
            relationships: Vec::new(),
            runnable_target: Some("./cmd/api".to_string()),
        };
        let context = task_context(
            &root,
            &module.module_path,
            "apps/api",
            native_tasks_for_module(&module, "linux"),
            crate::package_graph::PackageTaskContextKind::Package,
            crate::task_contracts::ScopeTaskContract::go(GoTaskContract::module(
                &module,
                "linux",
                &[],
            )),
        );

        let build = resolve_go_cmd(&context, "build", Some(&["-race".to_string()]), None);
        assert_eq!(
            build.args,
            ["build", "-race", "-o", "dist/api", "./cmd/api"].map(std::ffi::OsString::from)
        );
        assert_eq!(build.program, std::ffi::OsString::from("go"));
        assert_eq!(build.cwd, root.join_components(&["apps", "api"]));
        let lint = resolve_go_cmd(&context, "lint", Some(&["-json".to_string()]), None);
        assert_eq!(
            lint.args,
            ["vet", "-json", "./..."].map(std::ffi::OsString::from)
        );
        assert!(context.native_tasks().get("vet").is_none());
        assert!(
            !native_tasks_for_workspace(&["./apps/api/...".to_string()])
                .iter()
                .any(|task| task.name() == "vet")
        );
        assert_eq!(
            context
                .native_tasks()
                .get("build")
                .unwrap()
                .contract()
                .entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::Preferred)
        );

        let dev = resolve_go_cmd(
            &context,
            "dev",
            Some(&["--port".to_string(), "3000".to_string()]),
            None,
        );
        assert_eq!(
            dev.args,
            ["run", "./cmd/api", "--port", "3000"].map(std::ffi::OsString::from)
        );
        assert!(context.native_tasks().get("run").is_none());
    }

    #[test]
    fn windows_module_executable_contract_uses_exe_suffix() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let module = GoModule {
            module_path: "example.com/api".to_string(),
            manifest_path: root.join_components(&["apps", "api", GO_MOD]),
            relationships: Vec::new(),
            runnable_target: Some(".".to_string()),
        };
        let contract = GoTaskContract::module(&module, "windows", &[]);
        let package = task_context(
            &root,
            &module.module_path,
            "apps/api",
            native_tasks_for_module(&module, "windows"),
            crate::package_graph::PackageTaskContextKind::Package,
            crate::task_contracts::ScopeTaskContract::go(contract.clone()),
        );

        let build = resolve_go_cmd(&package, "build", None, None);
        assert_eq!(
            build.args,
            ["build", "-o", "dist/api.exe", "."].map(std::ffi::OsString::from)
        );

        let environment = toolchain::TaskIOEnvironment::default();
        let context = toolchain::TaskIOContext {
            task_args: None,
            environment: &environment,
        };
        let io = contract
            .derived_task_io(&package, "build", "../..", &[], true, &context)
            .expect("Windows executable build derives IO");
        assert_eq!(
            io.outputs,
            DerivedOutputs::Resolved(vec!["dist/api.exe".to_string()])
        );
    }

    #[test]
    fn ambiguous_main_packages_do_not_register_dev_default() {
        if !go_available() {
            return;
        }

        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        write_workspace(&root, &[("apps/api", "example.com/api", "")]);
        for command in ["first", "second"] {
            join_relative_path(&root, &format!("apps/api/cmd/{command}"))
                .unwrap()
                .create_dir_all()
                .unwrap();
            join_relative_path(&root, &format!("apps/api/cmd/{command}/main.go"))
                .unwrap()
                .create_with_contents("package main\nfunc main() {}\n")
                .unwrap();
        }

        let workspace = discover_workspace(&root).expect("workspace discovery succeeds");
        let module = &workspace.modules[0];
        assert_eq!(module.runnable_target, None);
        let tasks = native_tasks_for_module(module, "linux");
        assert!(!tasks.iter().any(|task| task.name() == "dev"));
        assert!(!tasks.iter().any(|task| task.name() == "run"));
        assert_eq!(
            tasks
                .iter()
                .find(|task| task.name() == "build")
                .unwrap()
                .contract()
                .entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::Candidate)
        );
    }

    #[test]
    fn task_contracts_derive_module_executable_and_workspace_io() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let executable = GoModule {
            module_path: "example.com/api".to_string(),
            manifest_path: root.join_components(&["apps", "api", GO_MOD]),
            relationships: Vec::new(),
            runnable_target: Some(".".to_string()),
        };
        let library = GoModule {
            module_path: "example.com/lib".to_string(),
            manifest_path: root.join_components(&["packages", "lib", GO_MOD]),
            relationships: Vec::new(),
            runnable_target: None,
        };
        let cache_prefixes = vec![".cache/go-build".to_string(), ".cache/go-mod".to_string()];
        let executable_contract = GoTaskContract::module(&executable, "linux", &cache_prefixes);
        let library_contract = GoTaskContract::module(&library, "linux", &cache_prefixes);
        let package = task_context(
            &root,
            &executable.module_path,
            "apps/api",
            native_tasks_for_module(&executable, "linux"),
            crate::package_graph::PackageTaskContextKind::Package,
            crate::task_contracts::ScopeTaskContract::go(executable_contract.clone()),
        );
        assert!(package.task_contract().env_vars().contains(&"GOCACHE"));
        let dependency = task_context(
            &root,
            &library.module_path,
            "packages/lib",
            native_tasks_for_module(&library, "linux"),
            crate::package_graph::PackageTaskContextKind::Package,
            crate::task_contracts::ScopeTaskContract::go(library_contract.clone()),
        );
        let environment = toolchain::TaskIOEnvironment::default();
        let context = toolchain::TaskIOContext {
            task_args: None,
            environment: &environment,
        };

        let executable_io = executable_contract
            .derived_task_io(&package, "build", "../..", &[dependency], true, &context)
            .expect("executable build derives IO");
        assert_eq!(executable_io.package_default_inputs, Some(true));
        for input in [
            "../../go.work",
            "../../packages/lib/**",
            "!../../.cache/go-build/**",
        ] {
            assert!(executable_io.input_globs.iter().any(|glob| glob == input));
        }
        assert!(executable_io.env.contains(&"GOOS".to_string()));
        assert!(!executable_io.env.contains(&"GOCACHE".to_string()));
        assert!(!executable_io.env.contains(&"GOPROXY".to_string()));
        assert_eq!(
            executable_io.outputs,
            DerivedOutputs::Resolved(vec!["dist/api".to_string()])
        );
        assert!(
            executable_io
                .forbidden_output_prefixes
                .contains(&"../../.cache/go-build".to_string())
        );
        assert_eq!(task_cache(&package, "build"), None);

        let library_context = task_context(
            &root,
            &library.module_path,
            "packages/lib",
            native_tasks_for_module(&library, "linux"),
            crate::package_graph::PackageTaskContextKind::Package,
            crate::task_contracts::ScopeTaskContract::go(library_contract.clone()),
        );
        let library_io = library_contract
            .derived_task_io(&library_context, "build", "../..", &[], true, &context)
            .expect("library build derives IO");
        assert_eq!(library_io.outputs, DerivedOutputs::Unavailable);
        assert_eq!(task_cache(&library_context, "build"), Some(false));

        let workspace_contract = GoTaskContract::workspace("linux", &cache_prefixes);
        let workspace = task_context(
            &root,
            GO_WORKSPACE_NAME,
            "",
            native_tasks_for_workspace(&[
                "./apps/api/...".to_string(),
                "./packages/lib/...".to_string(),
            ]),
            crate::package_graph::PackageTaskContextKind::Aggregate,
            crate::task_contracts::ScopeTaskContract::go(workspace_contract.clone()),
        );
        let aggregate_io = workspace_contract
            .derived_task_io(
                &workspace,
                "lint",
                "",
                &[package, library_context],
                true,
                &context,
            )
            .expect("workspace lint derives IO");
        assert_eq!(aggregate_io.package_default_inputs, Some(false));
        for input in ["apps/api/**", "packages/lib/**"] {
            assert!(aggregate_io.input_globs.iter().any(|glob| glob == input));
        }
        assert_eq!(aggregate_io.outputs, DerivedOutputs::Resolved(Vec::new()));
        assert_eq!(task_cache(&workspace, "lint"), None);
    }

    #[test]
    fn cache_prefixes_remain_inside_the_repository() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let build_cache = root.join_components(&[".cache", "go-build"]).to_string();
        let module_cache_tempdir = tempfile::tempdir().unwrap();
        let module_cache = AbsoluteSystemPathBuf::try_from(module_cache_tempdir.path())
            .unwrap()
            .join_components(&["go", "pkg", "mod"])
            .to_string();
        let environment = GoEnvironment {
            target_os: "linux".to_string(),
            build_cache,
            module_cache,
            fingerprint_values: BTreeMap::new(),
        };
        assert_eq!(
            go_cache_prefixes(&root, &environment),
            vec![".cache/go-build".to_string()]
        );
    }

    #[test]
    fn change_observation_covers_workspace_modules_sums_and_caches() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let module = GoModule {
            module_path: "example.com/api".to_string(),
            manifest_path: root.join_components(&["apps", "api", GO_MOD]),
            relationships: Vec::new(),
            runnable_target: None,
        };

        let observation =
            go_change_observation(&root, &[module], &[".cache/go-build".to_string()]).unwrap();

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
