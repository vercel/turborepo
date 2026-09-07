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
    collections::{HashMap, HashSet},
    io,
    path::Path,
    process::Command,
};

use serde::Deserialize;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::{
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
struct GoEnvironment {
    #[serde(rename = "GOOS")]
    target_os: String,
    #[serde(rename = "GOCACHE")]
    build_cache: String,
    #[serde(rename = "GOMODCACHE")]
    module_cache: String,
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

fn go_environment(repo_root: &AbsoluteSystemPath) -> Result<GoEnvironment, Error> {
    const COMMAND: &str = "go env -json GOOS GOCACHE GOMODCACHE";
    let output = run_go(
        repo_root,
        &["env", "-json", "GOOS", "GOCACHE", "GOMODCACHE"],
        COMMAND,
    )?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: COMMAND,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|source| Error::CommandParse {
        command: COMMAND,
        source,
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

    Ok(DiscoveredWorkspace { modules })
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

/// Go variables that alter compilation, package selection, tests, or target
/// platform. Credentials, proxy settings, and machine-local cache paths are
/// deliberately excluded.
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

/// Machine-local Go variables required by task execution but excluded from
/// task hashes.
pub(crate) const PROJECTED_ONLY_ENV_VARS: &[&str] = &["GOCACHE"];

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

            let workspace_roots = self
                .repo_root
                .join_component(GO_WORK)
                .exists()
                .then(|| WorkspaceRoot::new("go", self.repo_root.clone()))
                .into_iter()
                .collect();

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
            if !packages.is_empty() {
                packages.push(
                    DiscoveredPackage::aggregate(
                        GO_WORKSPACE_NAME.to_string(),
                        PackageJson::default(),
                        self.repo_root.join_component(GO_WORK),
                    )
                    .with_native_relationships(workspace_relationships)
                    .with_native_tasks(native_tasks_for_workspace(&module_patterns))
                    .with_task_contract(
                        crate::task_contracts::ScopeTaskContract::go(GoTaskContract::workspace(
                            &environment.target_os,
                            &cache_prefixes,
                        )),
                    ),
                );
            }

            Ok(DiscoveredPackages::new(packages, workspace_roots))
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
        };
        assert_eq!(
            go_cache_prefixes(&root, &environment),
            vec![".cache/go-build".to_string()]
        );
    }
}
