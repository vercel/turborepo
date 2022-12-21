use serde::{Deserialize, Serialize};
use turbo_tasks::trace::TraceRawVcs;
use turbo_tasks_fs::FileSystemPath;

#[derive(Debug, Clone, Serialize, Deserialize, TraceRawVcs, PartialEq, Eq)]
pub enum ContextCondition {
    InDirectory(String),
}

impl ContextCondition {
    pub fn matches(&self, context: &FileSystemPath) -> bool {
        match self {
            ContextCondition::InDirectory(dir) => {
                context.path.starts_with(&format!("{dir}/"))
                    || context.path.contains(&format!("/{dir}/"))
            }
        }
    }
}
