use std::sync::Arc;

use async_graphql::Object;
use itertools::Itertools;
use turborepo_errors::Spanned;

use crate::{
    engine::TaskNode,
    query::{package::Package, Array},
    run::{task_id::TaskId, Run},
};

pub struct RepositoryTask {
    pub name: String,
    pub package: Package,
    pub script: Option<Spanned<String>>,
}

impl RepositoryTask {
    pub fn new(task_id: &TaskId, run: &Arc<Run>) -> Self {
        let package = Package {
            name: task_id.package().into(),
            run: run.clone(),
        };
        let script = package.get_tasks().get(task_id.task()).cloned();

        RepositoryTask {
            name: task_id.task().to_string(),
            package,
            script,
        }
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

    async fn direct_dependents(&self) -> Array<RepositoryTask> {
        let task_id = TaskId::from_static(self.package.get_name().to_string(), self.name.clone());
        self.package
            .run()
            .engine()
            .dependents(&task_id)
            .into_iter()
            .flatten()
            .filter_map(|task| match task {
                TaskNode::Root => None,
                TaskNode::Task(task) if task == &task_id => None,
                TaskNode::Task(task) => Some(RepositoryTask::new(task, &self.package.run())),
            })
            .sorted_by(|a, b| {
                a.package
                    .get_name()
                    .cmp(&b.package.get_name())
                    .then_with(|| a.name.cmp(&b.name))
            })
            .collect()
    }

    async fn direct_dependencies(&self) -> Array<RepositoryTask> {
        let task_id = TaskId::new(self.package.name.as_ref(), &self.name);

        self.package
            .run
            .engine()
            .dependencies(&task_id)
            .into_iter()
            .flatten()
            .filter_map(|task| match task {
                TaskNode::Root => None,
                TaskNode::Task(task) if task == &task_id => None,
                TaskNode::Task(task) => Some(RepositoryTask::new(task, &self.package.run)),
            })
            .sorted_by(|a, b| {
                a.package
                    .name
                    .cmp(&b.package.name)
                    .then_with(|| a.name.cmp(&b.name))
            })
            .collect()
    }

    async fn indirect_dependents(&self) -> Array<RepositoryTask> {
        let task_id = TaskId::from_static(self.package.name.to_string(), self.name.clone());
        let direct_dependents = self
            .package
            .run
            .engine()
            .dependencies(&task_id)
            .unwrap_or_default();
        self.package
            .run
            .engine()
            .transitive_dependents(&task_id)
            .into_iter()
            .filter(|node| !direct_dependents.contains(node))
            .filter_map(|node| match node {
                TaskNode::Root => None,
                TaskNode::Task(task) if task == &task_id => None,
                TaskNode::Task(task) => Some(RepositoryTask::new(task, &self.package.run)),
            })
            .sorted_by(|a, b| {
                a.package
                    .get_name()
                    .cmp(&b.package.get_name())
                    .then_with(|| a.name.cmp(&b.name))
            })
            .collect()
    }

    async fn indirect_dependencies(&self) -> Array<RepositoryTask> {
        let task_id = TaskId::from_static(self.package.get_name().to_string(), self.name.clone());
        let direct_dependencies = self
            .package
            .run()
            .engine()
            .dependencies(&task_id)
            .unwrap_or_default();
        self.package
            .run()
            .engine()
            .transitive_dependencies(&task_id)
            .into_iter()
            .filter(|node| !direct_dependencies.contains(node))
            .filter_map(|node| match node {
                TaskNode::Root => None,
                TaskNode::Task(task) if task == &task_id => None,
                TaskNode::Task(task) => Some(RepositoryTask::new(task, &self.package.run)),
            })
            .sorted_by(|a, b| {
                a.package
                    .name
                    .cmp(&b.package.name)
                    .then_with(|| a.name.cmp(&b.name))
            })
            .collect()
    }

    async fn all_dependents(&self) -> Array<RepositoryTask> {
        let task_id = TaskId::from_static(self.package.name.to_string(), self.name.clone());
        self.package
            .run
            .engine()
            .transitive_dependents(&task_id)
            .into_iter()
            .filter_map(|node| match node {
                TaskNode::Root => None,
                TaskNode::Task(task) if task == &task_id => None,
                TaskNode::Task(task) => Some(RepositoryTask::new(task, &self.package.run)),
            })
            .sorted_by(|a, b| {
                a.package
                    .name
                    .cmp(&b.package.name)
                    .then_with(|| a.name.cmp(&b.name))
            })
            .collect()
    }

    async fn all_dependencies(&self) -> Array<RepositoryTask> {
        let task_id = TaskId::from_static(self.package.get_name().to_string(), self.name.clone());
        self.package
            .run()
            .engine()
            .transitive_dependencies(&task_id)
            .into_iter()
            .filter_map(|node| match node {
                TaskNode::Root => None,
                TaskNode::Task(task) if task == &task_id => None,
                TaskNode::Task(task) => Some(RepositoryTask::new(task, self.package.run())),
            })
            .sorted_by(|a, b| {
                a.package
                    .get_name()
                    .cmp(&b.package.get_name())
                    .then_with(|| a.name.cmp(&b.name))
            })
            .collect()
    }
}
