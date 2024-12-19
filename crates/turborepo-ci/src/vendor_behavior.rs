use std::sync::Arc;

use chrono::{DateTime, Utc};

pub type GroupPrefixFn = Arc<dyn Fn(DateTime<Utc>) -> String + Send + Sync>;
type GroupPrefixFnFactory = fn(group_name: String) -> GroupPrefixFn;

#[derive(Clone, Debug, PartialEq)]
pub struct VendorBehavior {
    pub group_prefix: GroupPrefixFnFactory,
    pub group_suffix: GroupPrefixFnFactory,
    pub error_group_prefix: Option<GroupPrefixFnFactory>,
    pub error_group_suffix: Option<GroupPrefixFnFactory>,
}

impl VendorBehavior {
    pub fn new(prefix: GroupPrefixFnFactory, suffix: GroupPrefixFnFactory) -> Self {
        Self {
            group_prefix: prefix,
            group_suffix: suffix,
            error_group_prefix: None,
            error_group_suffix: None,
        }
    }

    pub fn with_error(
        mut self,
        prefix: GroupPrefixFnFactory,
        suffix: GroupPrefixFnFactory,
    ) -> Self {
        self.error_group_prefix = Some(prefix);
        self.error_group_suffix = Some(suffix);
        self
    }
}
