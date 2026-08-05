//! Parser-neutral native task and command knowledge.
//!
//! Ecosystems contribute observations once; core consumes an immutable catalog.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
};

use turborepo_errors::Spanned;

use crate::{
    knowledge::RepositoryKnowledge,
    package_graph::PackageTaskContext,
    package_json::PackageJson,
    package_manager::PackageManager,
    toolchain::{TaskCommand, override_task_command, package_manager_command},
};

/// How a native command chooses its working directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkingDirectoryPolicy {
    /// Run in the package/aggregate source directory.
    PackageDirectory,
    /// Run at the repository root.
    RepositoryRoot,
}

/// Where pass-through arguments are placed relative to fixed command arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassThroughPlacement {
    BeforeSuffix,
    AfterSuffix,
}

/// Separator inserted before pass-through arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassThroughSeparator {
    Fixed(String),
    PackageManager,
}

/// General argument layout for a native command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCommandArguments {
    pub prefix: Vec<String>,
    pub pass_through_placement: PassThroughPlacement,
    pub pass_through_separator: Option<PassThroughSeparator>,
    pub suffix: Vec<String>,
}

impl NativeCommandArguments {
    pub fn new(prefix: Vec<String>) -> Self {
        Self {
            prefix,
            pass_through_placement: PassThroughPlacement::AfterSuffix,
            pass_through_separator: None,
            suffix: Vec::new(),
        }
    }
}

/// How the executable for a native command is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCommandProgram {
    PackageManager,
    Tool(String),
}

/// Declarative command template resolved into a [`TaskCommand`] at execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCommandTemplate {
    pub program: NativeCommandProgram,
    pub arguments: NativeCommandArguments,
    pub serial_group: Option<String>,
}

/// One native task observed for an authoritative scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTask {
    name: String,
    /// Package-authored definition (e.g. package.json script).
    authored: bool,
    /// Appears in toolchain registered-task tables without turbo.json.
    registered: bool,
    /// Has a concrete executable command (non-empty script / known Cargo verb).
    executable: bool,
    display: Option<String>,
    /// Source span for authored script text when available.
    script: Option<Spanned<String>>,
    command: Option<NativeCommandTemplate>,
    cwd_policy: WorkingDirectoryPolicy,
}

impl NativeTask {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn authored(&self) -> bool {
        self.authored
    }

    pub fn registered(&self) -> bool {
        self.registered
    }

    pub fn executable(&self) -> bool {
        self.executable
    }

    pub fn display(&self) -> Option<&str> {
        self.display.as_deref()
    }

    pub fn script(&self) -> Option<&Spanned<String>> {
        self.script.as_ref()
    }

    pub fn command(&self) -> Option<&NativeCommandTemplate> {
        self.command.as_ref()
    }

    pub fn cwd_policy(&self) -> WorkingDirectoryPolicy {
        self.cwd_policy
    }

    /// Construct a synthesized native command task (not package-authored).
    pub fn command_task(
        name: impl Into<String>,
        display: String,
        program: NativeCommandProgram,
        arguments: NativeCommandArguments,
        serial_group: Option<String>,
        cwd_policy: WorkingDirectoryPolicy,
    ) -> Self {
        let name = name.into();
        Self {
            name: name.clone(),
            authored: false,
            registered: true,
            executable: true,
            display: Some(display),
            script: None,
            command: Some(NativeCommandTemplate {
                program,
                arguments,
                serial_group,
            }),
            cwd_policy,
        }
    }
}

/// Observation state for one authoritative scope's native tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeNativeTasks {
    /// Scope identity is not present in repository knowledge.
    UnknownScope,
    /// Scope exists but no native-task producer contributed observations.
    Unobserved,
    /// Producer observed the scope and found no tasks.
    Empty,
    /// Producer observed one or more tasks.
    Available(Box<[NativeTask]>),
}

impl ScopeNativeTasks {
    pub fn tasks(&self) -> &[NativeTask] {
        match self {
            Self::Available(tasks) => tasks,
            Self::UnknownScope | Self::Unobserved | Self::Empty => &[],
        }
    }

    pub fn get(&self, name: &str) -> Option<&NativeTask> {
        self.tasks().iter().find(|task| task.name() == name)
    }

    pub fn defines(&self, name: &str) -> bool {
        self.get(name).is_some_and(NativeTask::executable)
    }

    pub fn authors(&self, name: &str) -> bool {
        self.get(name).is_some_and(NativeTask::authored)
    }

    pub fn registers(&self, name: &str) -> bool {
        self.get(name).is_some_and(NativeTask::registered)
    }

    pub fn registered_names(&self) -> Vec<String> {
        self.tasks()
            .iter()
            .filter(|task| task.registered())
            .map(|task| task.name().to_string())
            .collect()
    }

    /// Names contributed by a native script manifest, including empty scripts.
    pub fn script_names(&self) -> Vec<String> {
        self.tasks()
            .iter()
            .filter(|task| task.script().is_some())
            .map(|task| task.name().to_string())
            .collect()
    }

    /// Resource group retained when an override uses this scope's native
    /// command family. This also applies to override-only task names that have
    /// no catalog entry.
    pub fn override_serial_group(&self, override_command: &[String]) -> Option<String> {
        let program = override_command.first()?;
        self.tasks().iter().find_map(|task| {
            let command = task.command()?;
            match &command.program {
                NativeCommandProgram::Tool(tool) if tool == program => command.serial_group.clone(),
                NativeCommandProgram::PackageManager | NativeCommandProgram::Tool(_) => None,
            }
        })
    }
}

/// Producer-supplied native task facts for one scope, before catalog
/// validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTaskObservation {
    pub scope: String,
    pub tasks: Vec<NativeTask>,
    pub task_contract: crate::task_contracts::ScopeTaskContract,
}

/// Immutable native-task catalog for one repository generation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NativeTaskKnowledge {
    by_scope: HashMap<String, ScopeNativeTasks>,
}

impl NativeTaskKnowledge {
    pub fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn build(
        repository: &RepositoryKnowledge,
        observations: Vec<NativeTaskObservation>,
    ) -> Result<Self, NativeTaskError> {
        let mut by_scope: HashMap<String, ScopeNativeTasks> = HashMap::new();

        for scope in repository.scopes() {
            by_scope.insert(scope.identity().to_string(), ScopeNativeTasks::Unobserved);
        }
        if repository.root_javascript_scope().is_some() {
            by_scope
                .entry("//".to_string())
                .or_insert(ScopeNativeTasks::Unobserved);
        }

        for observation in observations {
            let known = if observation.scope == "//" {
                repository.root_javascript_scope().is_some()
            } else {
                repository.scope(&observation.scope).is_some()
            };
            if !known {
                return Err(NativeTaskError::UnknownScope {
                    identity: observation.scope,
                });
            }

            let mut tasks = observation.tasks;
            tasks.sort_by(|left, right| left.name.cmp(&right.name));
            if let Some(duplicate) = tasks.windows(2).find(|pair| pair[0].name == pair[1].name) {
                return Err(NativeTaskError::DuplicateTask {
                    scope: observation.scope,
                    task: duplicate[0].name.clone(),
                });
            }

            let state = if tasks.is_empty() {
                ScopeNativeTasks::Empty
            } else {
                ScopeNativeTasks::Available(tasks.into_boxed_slice())
            };
            by_scope.insert(observation.scope, state);
        }

        Ok(Self { by_scope })
    }

    pub fn for_scope(&self, scope: &str) -> &ScopeNativeTasks {
        static UNKNOWN: ScopeNativeTasks = ScopeNativeTasks::UnknownScope;
        self.by_scope.get(scope).unwrap_or(&UNKNOWN)
    }

    pub fn scopes(&self) -> impl Iterator<Item = (&str, &ScopeNativeTasks)> {
        self.by_scope
            .iter()
            .map(|(scope, tasks)| (scope.as_str(), tasks))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NativeTaskError {
    #[error("native task observation for unknown scope {identity}")]
    UnknownScope { identity: String },
    #[error("scope {scope} contributed duplicate native task {task}")]
    DuplicateTask { scope: String, task: String },
}

/// Convert package.json scripts into native-task observations.
pub fn observation_from_scripts(
    scope: impl Into<String>,
    scripts: &BTreeMap<String, Spanned<String>>,
) -> NativeTaskObservation {
    let scope = scope.into();
    let mut tasks = Vec::with_capacity(scripts.len());
    for (name, script) in scripts {
        let executable = !script.is_empty();
        tasks.push(NativeTask {
            name: name.clone(),
            authored: executable,
            registered: false,
            executable,
            display: executable.then(|| script.as_inner().clone()),
            script: Some(script.clone()),
            command: executable.then(|| NativeCommandTemplate {
                program: NativeCommandProgram::PackageManager,
                arguments: NativeCommandArguments {
                    prefix: vec!["run".to_string(), name.clone()],
                    pass_through_placement: PassThroughPlacement::AfterSuffix,
                    pass_through_separator: Some(PassThroughSeparator::PackageManager),
                    suffix: Vec::new(),
                },
                serial_group: None,
            }),
            cwd_policy: WorkingDirectoryPolicy::PackageDirectory,
        });
    }
    NativeTaskObservation {
        scope,
        tasks,
        task_contract: crate::task_contracts::ScopeTaskContract::javascript(),
    }
}

/// Convert a package.json descriptor into a native-task observation.
pub fn observation_from_package_json(
    scope: impl Into<String>,
    package_json: &PackageJson,
) -> NativeTaskObservation {
    observation_from_scripts(scope, &package_json.scripts)
}

/// Resolve a catalog command template into an executable [`TaskCommand`].
pub fn resolve_task_command(
    context: &PackageTaskContext<'_>,
    task: &NativeTask,
    package_manager: Option<&PackageManager>,
    package_manager_binary: Option<&std::path::Path>,
    tool_binary: Option<&std::path::Path>,
    pass_through_args: Option<&[String]>,
    override_command: Option<&[String]>,
) -> Result<Option<TaskCommand>, ResolveNativeCommandError> {
    if let Some(override_command) = override_command {
        let serial_group = context
            .native_tasks()
            .override_serial_group(override_command);
        return Ok(override_task_command(
            context,
            override_command,
            pass_through_args,
            serial_group,
        ));
    }

    let Some(template) = task.command() else {
        return Ok(None);
    };
    if !task.executable() {
        return Ok(None);
    }

    let cwd = match task.cwd_policy() {
        WorkingDirectoryPolicy::PackageDirectory => {
            context.repository_root().resolve(context.directory())
        }
        WorkingDirectoryPolicy::RepositoryRoot => context.repository_root().to_owned(),
    };

    let (program, mut args) = match &template.program {
        NativeCommandProgram::PackageManager => {
            let package_manager =
                package_manager.ok_or(ResolveNativeCommandError::MissingPackageManager)?;
            let package_manager_binary = package_manager_binary
                .ok_or(ResolveNativeCommandError::MissingPackageManagerBinary)?;
            package_manager_command(package_manager, package_manager_binary)
        }
        NativeCommandProgram::Tool(tool) => {
            let binary = tool_binary.ok_or_else(|| {
                ResolveNativeCommandError::MissingToolBinary { tool: tool.clone() }
            })?;
            (binary.as_os_str().to_owned(), Vec::new())
        }
    };
    args.extend(template.arguments.prefix.iter().map(OsString::from));
    if template.arguments.pass_through_placement == PassThroughPlacement::BeforeSuffix {
        append_pass_through(
            &mut args,
            &template.arguments,
            pass_through_args,
            package_manager,
        );
    }
    args.extend(template.arguments.suffix.iter().map(OsString::from));
    if template.arguments.pass_through_placement == PassThroughPlacement::AfterSuffix {
        append_pass_through(
            &mut args,
            &template.arguments,
            pass_through_args,
            package_manager,
        );
    }
    Ok(Some(TaskCommand {
        program,
        args,
        cwd,
        serial_group: template.serial_group.clone(),
    }))
}

fn append_pass_through(
    args: &mut Vec<OsString>,
    layout: &NativeCommandArguments,
    pass_through_args: Option<&[String]>,
    package_manager: Option<&PackageManager>,
) {
    let Some(pass_through_args) = pass_through_args else {
        return;
    };
    match &layout.pass_through_separator {
        Some(PassThroughSeparator::Fixed(separator)) => args.push(separator.into()),
        Some(PassThroughSeparator::PackageManager) => args.extend(
            package_manager
                .into_iter()
                .flat_map(|manager| manager.arg_separator(pass_through_args))
                .map(OsString::from),
        ),
        None => {}
    }
    args.extend(pass_through_args.iter().map(OsString::from));
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveNativeCommandError {
    #[error("JavaScript package manager is not available for native task resolution")]
    MissingPackageManager,
    #[error("JavaScript package manager binary is not available for native task resolution")]
    MissingPackageManagerBinary,
    #[error("The `{tool}` binary is not available for native task resolution")]
    MissingToolBinary { tool: String },
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::{
        knowledge::{
            PackageScopeObservation, RepositoryKnowledge, ScopeKind, WorkspaceRootObservation,
        },
        package_graph::{PackageName, PackageTaskContextKind},
        toolchain::{ToolchainId, WorkspaceRoot},
    };

    fn repository() -> RepositoryKnowledge {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        RepositoryKnowledge::build(
            &root,
            Some(Some("root".to_string())),
            &[PackageScopeObservation {
                identity: Some("web".to_string()),
                name_source: None,
                definition_path: root.join_components(&["apps", "web", "package.json"]),
                toolchain: ToolchainId::JAVASCRIPT,
                scope_kind: ScopeKind::Package,
            }],
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("npm", root.clone()),
                ToolchainId::JAVASCRIPT,
            )],
        )
        .unwrap()
    }

    #[test]
    fn scripts_become_authored_executable_tasks() {
        let mut scripts = BTreeMap::new();
        scripts.insert("build".into(), Spanned::new("next build".into()));
        scripts.insert("empty".into(), Spanned::new(String::new()));
        let observation = observation_from_scripts("web", &scripts);
        assert_eq!(observation.tasks.len(), 2);
        let build = observation
            .tasks
            .iter()
            .find(|task| task.name() == "build")
            .unwrap();
        assert!(build.authored());
        assert!(build.executable());
        assert!(!build.registered());
        assert_eq!(build.display(), Some("next build"));
        let empty = observation
            .tasks
            .iter()
            .find(|task| task.name() == "empty")
            .unwrap();
        assert!(!empty.authored());
        assert!(!empty.executable());
        let tasks = ScopeNativeTasks::Available(observation.tasks.into_boxed_slice());
        assert_eq!(tasks.script_names(), ["build", "empty"]);
    }

    #[test]
    fn catalog_validates_against_repository_knowledge() {
        let repository = repository();
        let observation = observation_from_scripts(
            "web",
            &BTreeMap::from([("build".into(), Spanned::new("tsc".into()))]),
        );
        let root_observation = observation_from_scripts("//", &BTreeMap::new());
        let knowledge =
            NativeTaskKnowledge::build(&repository, vec![observation, root_observation]).unwrap();
        let scope = knowledge.for_scope("web");
        assert!(matches!(scope, ScopeNativeTasks::Available(_)));
        assert!(scope.defines("build"));
        assert!(scope.authors("build"));
        assert!(!scope.registers("build"));
        assert!(matches!(
            knowledge.for_scope("missing"),
            ScopeNativeTasks::UnknownScope
        ));
        assert!(matches!(knowledge.for_scope("//"), ScopeNativeTasks::Empty));
    }

    #[test]
    fn unknown_scope_observation_is_rejected() {
        let repository = repository();
        let observation = observation_from_scripts("other", &BTreeMap::new());
        let error = NativeTaskKnowledge::build(&repository, vec![observation]).unwrap_err();
        assert!(matches!(error, NativeTaskError::UnknownScope { .. }));
    }

    #[test]
    fn cargo_tasks_are_registered_not_authored() {
        let task = NativeTask::command_task(
            "build",
            "cargo build --package=app --locked".into(),
            NativeCommandProgram::Tool("cargo".into()),
            NativeCommandArguments {
                prefix: vec!["build".into(), "--package=app".into()],
                pass_through_placement: PassThroughPlacement::AfterSuffix,
                pass_through_separator: None,
                suffix: vec!["--locked".into()],
            },
            Some("cargo".into()),
            WorkingDirectoryPolicy::RepositoryRoot,
        );
        assert!(!task.authored());
        assert!(task.registered());
        assert!(task.executable());
        assert_eq!(task.cwd_policy(), WorkingDirectoryPolicy::RepositoryRoot);
        assert!(
            ScopeNativeTasks::Available(vec![task].into_boxed_slice())
                .script_names()
                .is_empty()
        );
    }

    #[test]
    fn generalized_command_resolution_supports_prefix_suffix_and_fixed_separator() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let binary = std::path::Path::new(if cfg!(windows) {
            r"C:\bin\tool.exe"
        } else {
            "/bin/tool"
        });
        let pass_through = ["--frozen".to_string()];
        let task = NativeTask::command_task(
            "sync",
            "tool sync --package=app --locked".into(),
            NativeCommandProgram::Tool("tool".into()),
            NativeCommandArguments {
                prefix: vec!["sync".into(), "--package=app".into()],
                pass_through_placement: PassThroughPlacement::BeforeSuffix,
                pass_through_separator: Some(PassThroughSeparator::Fixed("--".into())),
                suffix: vec!["--locked".into()],
            },
            Some("tool".into()),
            WorkingDirectoryPolicy::RepositoryRoot,
        );
        let directory = turbopath::AnchoredSystemPath::new("apps/web").unwrap();
        let context = PackageTaskContext::new_for_test(
            PackageName::from("web"),
            &root,
            directory,
            PackageTaskContextKind::Package,
            None,
        );
        let command = resolve_task_command(
            &context,
            &task,
            None,
            None,
            Some(binary),
            Some(&pass_through),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(command.program, binary.as_os_str());
        assert_eq!(
            command.args,
            ["sync", "--package=app", "--", "--frozen", "--locked"].map(OsString::from)
        );
        assert_eq!(command.cwd, root);
        assert_eq!(command.serial_group.as_deref(), Some("tool"));
    }

    #[test]
    fn generalized_command_resolution_requires_tool_binary() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let directory = turbopath::AnchoredSystemPath::new("").unwrap();
        let context = PackageTaskContext::new_for_test(
            PackageName::from("workspace"),
            &root,
            directory,
            PackageTaskContextKind::Aggregate,
            None,
        );
        let task = NativeTask::command_task(
            "build",
            "uv build".into(),
            NativeCommandProgram::Tool("uv".into()),
            NativeCommandArguments::new(vec!["build".into()]),
            None,
            WorkingDirectoryPolicy::RepositoryRoot,
        );
        assert!(matches!(
            resolve_task_command(&context, &task, None, None, None, None, None),
            Err(ResolveNativeCommandError::MissingToolBinary { tool }) if tool == "uv"
        ));
    }
}
