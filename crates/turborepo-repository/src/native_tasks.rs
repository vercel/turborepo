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

/// Declarative command template resolved into a [`TaskCommand`] at execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCommandTemplate {
    /// `<package-manager> run <task>` with package-manager arg separators.
    JavaScriptPackageManagerRun { task: String },
    /// `cargo <subcommand> <scope>` with optional locking and serial grouping.
    Cargo {
        subcommand: String,
        /// `--package=<name>`, `--workspace`, or `--all`.
        scope_arg: String,
        locked: bool,
        serial_group: Option<String>,
        pass_through_uses_separator: bool,
    },
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

    /// Construct a Cargo-synthesized native task (not package-authored).
    pub fn cargo(
        name: impl Into<String>,
        display: String,
        subcommand: impl Into<String>,
        scope_arg: impl Into<String>,
        locked: bool,
        serial_group: Option<String>,
        pass_through_uses_separator: bool,
    ) -> Self {
        let name = name.into();
        Self {
            name: name.clone(),
            authored: false,
            registered: true,
            executable: true,
            display: Some(display),
            script: None,
            command: Some(NativeCommandTemplate::Cargo {
                subcommand: subcommand.into(),
                scope_arg: scope_arg.into(),
                locked,
                serial_group,
                pass_through_uses_separator,
            }),
            cwd_policy: WorkingDirectoryPolicy::RepositoryRoot,
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
        let invokes_cargo = override_command.first().map(String::as_str) == Some("cargo");
        (invokes_cargo
            && self
                .tasks()
                .iter()
                .any(|task| matches!(task.command(), Some(NativeCommandTemplate::Cargo { .. }))))
        .then(|| "cargo".to_string())
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
            command: executable
                .then(|| NativeCommandTemplate::JavaScriptPackageManagerRun { task: name.clone() }),
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
    cargo_binary: Option<&std::path::Path>,
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

    match template {
        NativeCommandTemplate::JavaScriptPackageManagerRun { task: task_name } => {
            let package_manager =
                package_manager.ok_or(ResolveNativeCommandError::MissingPackageManager)?;
            let package_manager_binary = package_manager_binary
                .ok_or(ResolveNativeCommandError::MissingPackageManagerBinary)?;
            let (program, mut args) =
                package_manager_command(package_manager, package_manager_binary);
            args.extend([OsString::from("run"), OsString::from(task_name)]);
            if let Some(pass_through_args) = pass_through_args {
                args.extend(
                    package_manager
                        .arg_separator(pass_through_args)
                        .map(OsString::from),
                );
                args.extend(pass_through_args.iter().map(OsString::from));
            }
            Ok(Some(TaskCommand {
                program,
                args,
                cwd,
                serial_group: None,
            }))
        }
        NativeCommandTemplate::Cargo {
            subcommand,
            scope_arg,
            locked,
            serial_group,
            pass_through_uses_separator,
        } => {
            let cargo_binary = cargo_binary.ok_or(ResolveNativeCommandError::MissingCargoBinary)?;
            let mut args: Vec<OsString> = vec![subcommand.into(), scope_arg.into()];
            if *locked {
                args.push("--locked".into());
            }
            if let Some(pass_through_args) = pass_through_args {
                if *pass_through_uses_separator {
                    args.push("--".into());
                }
                args.extend(pass_through_args.iter().map(OsString::from));
            }
            Ok(Some(TaskCommand {
                program: cargo_binary.as_os_str().to_owned(),
                args,
                cwd,
                serial_group: serial_group.clone(),
            }))
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveNativeCommandError {
    #[error("JavaScript package manager is not available for native task resolution")]
    MissingPackageManager,
    #[error("JavaScript package manager binary is not available for native task resolution")]
    MissingPackageManagerBinary,
    #[error("Cargo binary is not available for native task resolution")]
    MissingCargoBinary,
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::{
        knowledge::{
            PackageScopeObservation, RepositoryKnowledge, ScopeKind, WorkspaceRootObservation,
        },
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
        let task = NativeTask::cargo(
            "build",
            "cargo build --package=app --locked".into(),
            "build",
            "--package=app",
            true,
            Some("cargo".into()),
            false,
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
}
