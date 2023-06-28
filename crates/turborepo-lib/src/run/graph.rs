use std::collections::BTreeMap;

use anyhow::Result;
use turbopath::AbsoluteSystemPath;

use crate::{
    config::TurboJson,
    package_graph::PackageGraph,
    task_graph::{Pipeline, TaskDefinition},
};

pub struct CompleteGraph<'run> {
    // Expresses the dependencies between packages
    package_graph: &'run PackageGraph,
    // Config from turbo.json
    pipeline: Pipeline,
    // Hash of all global dependencies
    global_hash: Option<String>,

    task_definitions: BTreeMap<String, TaskDefinition>,
    repo_root: &'run AbsoluteSystemPath,

    task_hash_tracker: TaskHashTracker,
}

impl<'run> CompleteGraph<'run> {
    pub fn new(package_graph: &'run PackageGraph, repo_root: &'run AbsoluteSystemPath) -> Self {
        Self {
            package_graph,
            pipeline: Pipeline::default(),
            repo_root,
            global_hash: None,
            task_definitions: BTreeMap::new(),
            task_hash_tracker: TaskHashTracker::default(),
        }
    }

    pub fn get_turbo_config_from_workspace(
        &self,
        _workspace_name: &str,
        _is_single_package: bool,
    ) -> Result<TurboJson> {
        // TODO
        Ok(TurboJson::default())
    }
}

#[derive(Default)]
pub struct TaskHashTracker {}
