use chrono::{DateTime, Local};

type GroupPrefixFn = fn(group_name: &str, time: &DateTime<Local>) -> String;

#[derive(Clone, Debug, PartialEq)]
pub struct VendorBehavior {
    pub group_prefix: GroupPrefixFn,
    pub group_suffix: GroupPrefixFn,
}
