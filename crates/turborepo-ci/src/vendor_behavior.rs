type GroupPrefixFn = fn(group_name: &str) -> String;

#[derive(Clone, Debug, PartialEq)]
pub struct VendorBehavior {
    pub group_prefix: GroupPrefixFn,
    pub group_suffix: GroupPrefixFn,
}
