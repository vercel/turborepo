use async_graphql::Object;
use turborepo_errors::Spanned;

use crate::{
    engine::TaskNode,
    query::{package::Package, Array},
    run::task_id::TaskId,
};

pub struct RepositoryTask {
    pub name: String,
    pub package: Package,
    pub script: Option<Spanned<String>>,
}

#[Object]
impl RepositoryTask {
    async fn name(&self) -> String {
        self.name.clone()
    }

    async fn package(&self) -> Package {
        self.package.clone()
    }

    async fn script(&self) -> Option<String> {
        self.script.as_ref().map(|script| script.value.to_string())
    }

    async fn direct_dependents(&self) -> Array<RepositoryTask> {
        let task_id = TaskId::from_static(self.package.name.to_string(), self.name.clone());
        self.package
            .run
            .engine()
            .dependents(&task_id)
            .into_iter()
            .flatten()
            .filter_map(|task| match task {
                TaskNode::Root => None,
                TaskNode::Task(task) => Some(RepositoryTask {
                    name: task.task().to_string(),
                    package: Package {
                        run: self.package.run.clone(),
                        name: task.package().to_string().into(),
                    },
                    script: self.package.get_tasks().get(task.task()).cloned(),
                }),
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
                TaskNode::Task(task) => Some(RepositoryTask {
                    name: task.task().to_string(),
                    package: Package {
                        run: self.package.run.clone(),
                        name: task.package().to_string().into(),
                    },
                    script: self.package.get_tasks().get(task.task()).cloned(),
                }),
            })
            .collect()
    }
}
