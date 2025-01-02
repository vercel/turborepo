use std::sync::Arc;

use async_graphql::Object;
use turborepo_errors::Spanned;

use crate::{
    engine::TaskNode,
    query::{package::Package, Array, Error},
    run::{task_id::TaskId, Run},
};

pub struct RepositoryTask {
    pub name: String,
    pub package: Package,
    pub script: Option<Spanned<String>>,
}

impl RepositoryTask {
    pub fn new(task_id: &TaskId, run: &Arc<Run>) -> Result<Self, Error> {
        let package = Package::new(run.clone(), task_id.package().into())?;
        let script = package.get_tasks().get(task_id.task()).cloned();

        Ok(RepositoryTask {
            name: task_id.task().to_string(),
            package,
            script,
        })
    }

    fn collect_and_sort<'a>(
        &self,
        task_id: &TaskId<'a>,
        tasks: impl IntoIterator<Item = &'a TaskNode>,
    ) -> Result<Array<RepositoryTask>, Error> {
        let mut tasks = tasks
            .into_iter()
            .filter_map(|task| match task {
                TaskNode::Root => None,
                TaskNode::Task(task) if task == task_id => None,
                TaskNode::Task(task) => Some(RepositoryTask::new(task, self.package.run())),
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

    async fn script(&self) -> Option<String> {
        self.script.as_ref().map(|script| script.value.to_string())
    }

    async fn direct_dependents(&self) -> Result<Array<RepositoryTask>, Error> {
        let task_id = TaskId::from_static(self.package.get_name().to_string(), self.name.clone());

        self.collect_and_sort(
            &task_id,
            self.package
                .run()
                .engine()
                .dependents(&task_id)
                .into_iter()
                .flatten(),
        )
    }

    async fn direct_dependencies(&self) -> Result<Array<RepositoryTask>, Error> {
        let task_id = TaskId::new(self.package.get_name().as_ref(), &self.name);

        self.collect_and_sort(
            &task_id,
            self.package
                .run()
                .engine()
                .dependencies(&task_id)
                .into_iter()
                .flatten(),
        )
    }

    async fn indirect_dependents(&self) -> Result<Array<RepositoryTask>, Error> {
        let task_id = TaskId::from_static(self.package.get_name().to_string(), self.name.clone());
        let direct_dependents = self
            .package
            .run()
            .engine()
            .dependencies(&task_id)
            .unwrap_or_default();

        self.collect_and_sort(
            &task_id,
            self.package
                .run()
                .engine()
                .transitive_dependents(&task_id)
                .into_iter()
                .filter(|node| !direct_dependents.contains(node)),
        )
    }

    async fn indirect_dependencies(&self) -> Result<Array<RepositoryTask>, Error> {
        let task_id = TaskId::from_static(self.package.get_name().to_string(), self.name.clone());
        let direct_dependencies = self
            .package
            .run()
            .engine()
            .dependencies(&task_id)
            .unwrap_or_default();
        let mut dependencies = self
            .package
            .run()
            .engine()
            .transitive_dependencies(&task_id)
            .into_iter()
            .filter(|node| !direct_dependencies.contains(node))
            .filter_map(|node| match node {
                TaskNode::Root => None,
                TaskNode::Task(task) if task == &task_id => None,
                TaskNode::Task(task) => Some(RepositoryTask::new(task, self.package.run())),
            })
            .collect::<Result<Array<_>, _>>()?;

        dependencies.sort_by(|a, b| {
            a.package
                .get_name()
                .cmp(b.package.get_name())
                .then_with(|| a.name.cmp(&b.name))
        });

        Ok(dependencies)
    }

    async fn all_dependents(&self) -> Result<Array<RepositoryTask>, Error> {
        let task_id = TaskId::from_static(self.package.get_name().to_string(), self.name.clone());
        self.collect_and_sort(
            &task_id,
            self.package.run().engine().transitive_dependents(&task_id),
        )
    }

    async fn all_dependencies(&self) -> Result<Array<RepositoryTask>, Error> {
        let task_id = TaskId::from_static(self.package.get_name().to_string(), self.name.clone());
        self.collect_and_sort(
            &task_id,
            self.package
                .run()
                .engine()
                .transitive_dependencies(&task_id),
        )
    }
}
