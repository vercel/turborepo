use crate::{self as turbo_tasks, Vc};

#[turbo_tasks::value_trait]
pub trait ValueToString {
    fn to_string(self: Vc<Self>) -> Vc<String>;
}
