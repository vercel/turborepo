use std::{collections::HashSet, sync::Arc};

use async_graphql::{Any, Enum, InputObject, Json, Object, SimpleObject, Value};
use turborepo_errors::Spanned;
use turborepo_types::TaskCommandOverride;

use crate::{Array, Error, QueryRun, QueryTaskId, package::Package};

/// Configured environment patterns, without expanding names or reading values.
#[derive(Default, SimpleObject)]
pub struct Environment {
    pub env: Vec<String>,
    pub pass_through_env: Vec<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub(crate) enum TaskFields {
    Name,
    FullName,
    /// Matches a label on either the resolved task or its package.
    Tag,
    /// Compares resolved experimentalCI JSON: a boolean, an object, or null
    /// when unset.
    ExperimentalCi,
}

#[derive(InputObject)]
pub(crate) struct TaskFieldValuePair {
    field: TaskFields,
    value: Option<Any>,
}

impl TaskFieldValuePair {
    fn value(&self) -> &Value {
        self.value.as_ref().map_or(&Value::Null, |value| &value.0)
    }
}

/// Predicates on tasks. Multiple predicates are combined using AND; use `or`
/// for alternatives. `has: {field: TAG, value: "label"}` matches an exact,
/// case-sensitive label in the union of task and package labels, like the CLI
/// `tag:<label>` filter. Name equality only accepts string values.
#[derive(InputObject)]
pub(crate) struct TaskPredicate {
    and: Option<Vec<TaskPredicate>>,
    or: Option<Vec<TaskPredicate>>,
    equal: Option<TaskFieldValuePair>,
    not_equal: Option<TaskFieldValuePair>,
    not: Option<Box<TaskPredicate>>,
    has: Option<TaskFieldValuePair>,
}

impl TaskPredicate {
    fn check_equals(task: &RepositoryTask, pair: &TaskFieldValuePair) -> bool {
        match (&pair.field, pair.value()) {
            (TaskFields::Name, Value::String(name)) => &task.name == name,
            (TaskFields::FullName, Value::String(name)) => task.task_id().full_name() == *name,
            (TaskFields::ExperimentalCi, value) => {
                let Ok(value) = value.clone().into_json() else {
                    return false;
                };
                task.get_experimental_ci().is_ok_and(|config| {
                    config.map_or(serde_json::Value::Null, |config| config.0) == value
                })
            }
            _ => false,
        }
    }

    fn check_has(task: &RepositoryTask, pair: &TaskFieldValuePair) -> bool {
        match (&pair.field, pair.value()) {
            (TaskFields::Tag, Value::String(tag)) => {
                task.get_tags().contains(tag) || task.package.get_tags().contains(tag)
            }
            _ => Self::check_equals(task, pair),
        }
    }

    pub(crate) fn check(&self, task: &RepositoryTask) -> bool {
        self.and
            .as_ref()
            .is_none_or(|predicates| predicates.iter().all(|p| p.check(task)))
            && self
                .or
                .as_ref()
                .is_none_or(|predicates| predicates.iter().any(|p| p.check(task)))
            && self
                .equal
                .as_ref()
                .is_none_or(|pair| Self::check_equals(task, pair))
            && self
                .not_equal
                .as_ref()
                .is_none_or(|pair| !Self::check_equals(task, pair))
            && self
                .not
                .as_ref()
                .is_none_or(|predicate| !predicate.check(task))
            && self
                .has
                .as_ref()
                .is_none_or(|pair| Self::check_has(task, pair))
    }
}

pub struct RepositoryTask {
    pub name: String,
    pub package: Package,
    pub script: Option<Spanned<String>>,
}

impl RepositoryTask {
    pub fn new(task_id: &QueryTaskId, run: &Arc<dyn QueryRun>) -> Result<Self, Error> {
        let package = Package::for_task(run.clone(), task_id.package.clone().into())?;
        let script = package.get_tasks().get(&task_id.task).cloned();

        Ok(RepositoryTask {
            name: task_id.task.clone(),
            package,
            script,
        })
    }

    pub fn get_tags(&self) -> Vec<String> {
        self.package
            .run()
            .task_definition(&self.task_id())
            .map(|definition| definition.tags.clone())
            .unwrap_or_default()
    }

    fn get_experimental_ci(&self) -> Result<Option<Json<serde_json::Value>>, Error> {
        self.package
            .run()
            .task_definition(&self.task_id())
            .and_then(|definition| definition.experimental_ci.as_ref())
            .map(|config| serde_json::to_value(config).map(Json).map_err(Error::from))
            .transpose()
    }

    fn task_id(&self) -> QueryTaskId {
        QueryTaskId::new(self.package.get_name().to_string(), self.name.clone())
    }

    pub fn executes(&self) -> bool {
        match self
            .package
            .run()
            .task_definition(&self.task_id())
            .and_then(|definition| definition.command.as_ref())
        {
            Some(TaskCommandOverride::Argv(_)) => true,
            Some(TaskCommandOverride::OptOut) => false,
            None => self
                .package
                .run()
                .repo_context()
                .pkg_dep_graph()
                .package_task_context(self.package.get_name())
                .is_some_and(|context| context.native_tasks().defines(&self.name)),
        }
    }

    pub fn resolved_command(&self) -> Option<String> {
        match self
            .package
            .run()
            .task_definition(&self.task_id())
            .and_then(|definition| definition.command.as_ref())
        {
            Some(TaskCommandOverride::Argv(argv)) => Some(argv.join(" ")),
            Some(TaskCommandOverride::OptOut) => None,
            None => self
                .package
                .run()
                .repo_context()
                .pkg_dep_graph()
                .package_task_context(self.package.get_name())
                .and_then(|context| context.native_tasks().get(&self.name))
                .and_then(|task| task.display())
                .map(str::to_string),
        }
    }

    pub fn participates_in_run(&self) -> bool {
        self.executes()
            || !self
                .package
                .run()
                .task_dependencies(&self.task_id())
                .is_empty()
    }

    fn collect_and_sort(
        &self,
        task_id: &QueryTaskId,
        tasks: impl IntoIterator<Item = QueryTaskId>,
        filter: Option<TaskPredicate>,
    ) -> Result<Array<RepositoryTask>, Error> {
        let mut tasks = tasks
            .into_iter()
            .filter(|task| task != task_id)
            .map(|task| RepositoryTask::new(&task, self.package.run()))
            .filter(|task| {
                task.as_ref().map_or(true, |task| {
                    filter
                        .as_ref()
                        .is_none_or(|predicate| predicate.check(task))
                })
            })
            .collect::<Result<Array<_>, _>>()?;
        tasks.sort_by(|a, b| {
            a.package
                .get_name()
                .cmp(b.package.get_name())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(tasks)
    }
}

#[Object]
impl RepositoryTask {
    async fn name(&self) -> String {
        self.name.clone()
    }

    async fn package(&self) -> Package {
        self.package.clone()
    }

    async fn full_name(&self) -> String {
        format!("{}#{}", self.package.get_name(), self.name)
    }

    /// This task's resolved labels, excluding package labels. Package labels
    /// are available separately through `package.tags`.
    async fn tags(&self) -> Vec<String> {
        self.get_tags()
    }

    async fn script(&self) -> Option<String> {
        self.script.as_ref().map(|script| script.value.to_string())
    }

    /// The command Turbo will execute for this task, including implicit native
    /// tasks and command overrides.
    async fn command(&self) -> Option<String> {
        self.resolved_command()
    }

    /// Environment patterns after resolving task configuration inheritance.
    /// Global patterns are exposed separately by `globalEnvironment`.
    async fn environment(&self) -> Environment {
        self.package
            .run()
            .task_definition(&self.task_id())
            .map(|definition| Environment {
                env: definition.env.clone(),
                pass_through_env: definition.pass_through_env.clone().unwrap_or_default(),
            })
            .unwrap_or_default()
    }

    /// The fully resolved `experimentalCI` configuration for this task from
    /// turbo.json, including Package Configuration overrides. Either a
    /// boolean or an object with arbitrary keys. Null if the key is not set.
    #[graphql(name = "experimentalCI")]
    async fn experimental_ci(&self) -> Result<Option<Json<serde_json::Value>>, Error> {
        self.get_experimental_ci()
    }

    async fn direct_dependents(
        &self,
        filter: Option<TaskPredicate>,
    ) -> Result<Array<RepositoryTask>, Error> {
        let task_id = self.task_id();
        self.collect_and_sort(
            &task_id,
            self.package.run().task_dependents(&task_id),
            filter,
        )
    }

    async fn direct_dependencies(
        &self,
        filter: Option<TaskPredicate>,
    ) -> Result<Array<RepositoryTask>, Error> {
        let task_id = self.task_id();
        self.collect_and_sort(
            &task_id,
            self.package.run().task_dependencies(&task_id),
            filter,
        )
    }

    async fn indirect_dependents(
        &self,
        filter: Option<TaskPredicate>,
    ) -> Result<Array<RepositoryTask>, Error> {
        let task_id = self.task_id();
        // Preserve the existing query semantics: this exclusion set is the
        // task's direct dependencies rather than its direct dependents.
        let direct_dependents: HashSet<_> = self
            .package
            .run()
            .task_dependencies(&task_id)
            .into_iter()
            .collect();

        self.collect_and_sort(
            &task_id,
            self.package
                .run()
                .transitive_task_dependents(&task_id)
                .into_iter()
                .filter(|task| !direct_dependents.contains(task)),
            filter,
        )
    }

    async fn indirect_dependencies(
        &self,
        filter: Option<TaskPredicate>,
    ) -> Result<Array<RepositoryTask>, Error> {
        let task_id = self.task_id();
        let direct_dependencies: HashSet<_> = self
            .package
            .run()
            .task_dependencies(&task_id)
            .into_iter()
            .collect();

        self.collect_and_sort(
            &task_id,
            self.package
                .run()
                .transitive_task_dependencies(&task_id)
                .into_iter()
                .filter(|task| !direct_dependencies.contains(task)),
            filter,
        )
    }

    async fn all_dependents(
        &self,
        filter: Option<TaskPredicate>,
    ) -> Result<Array<RepositoryTask>, Error> {
        let task_id = self.task_id();
        self.collect_and_sort(
            &task_id,
            self.package.run().transitive_task_dependents(&task_id),
            filter,
        )
    }

    async fn all_dependencies(
        &self,
        filter: Option<TaskPredicate>,
    ) -> Result<Array<RepositoryTask>, Error> {
        let task_id = self.task_id();
        self.collect_and_sort(
            &task_id,
            self.package.run().transitive_task_dependencies(&task_id),
            filter,
        )
    }
}
