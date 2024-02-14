type GroupPrefixFn = fn(group_name: &str) -> String;

#[derive(Clone, Debug, PartialEq)]
pub struct VendorBehavior {
    pub group_prefix: GroupPrefixFn,
    pub group_suffix: GroupPrefixFn,
    pub error_group_prefix: Option<GroupPrefixFn>,
    pub error_group_suffix: Option<GroupPrefixFn>,
}

impl VendorBehavior {
    pub fn new(prefix: GroupPrefixFn, suffix: GroupPrefixFn) -> Self {
        Self {
            group_prefix: prefix,
            group_suffix: suffix,
            error_group_prefix: None,
            error_group_suffix: None,
        }
    }

    pub fn with_error(mut self, prefix: GroupPrefixFn, suffix: GroupPrefixFn) -> Self {
        self.error_group_prefix = Some(prefix);
        self.error_group_suffix = Some(suffix);
        self
    }
}
