use std::collections::{HashMap, HashSet};

pub type Pipeline = HashMap<String, BookkeepingTaskDefinition>;

pub struct BookkeepingTaskDefinition {
    pub defined_fields: HashSet<String>,
    pub experimental_fields: HashSet<String>,
    pub task_definition: HashableTaskDefinition,
}

pub struct HashableTaskDefinition {
    pub should_cache: bool,
}
