use std::ops::Deref;

use auto_hash_map::AutoSet;
use turbo_tasks_macros::primitive;

use crate::{self as turbo_tasks, RawVc};

primitive!((), "unit");
primitive!(String);
primitive!(Option<String>, "option_string");
primitive!(Vec<String>, "vec_string");

primitive!(Option<u16>, "option_u16");

primitive!(bool);

primitive!(u8);
primitive!(u16);
primitive!(u32);
primitive!(u64);
primitive!(u128);
primitive!(i8);
primitive!(i16);
primitive!(i32);
primitive!(i64);
primitive!(i128);
primitive!(usize);
primitive!(isize);
primitive!(AutoSet<RawVc>, "auto_set_raw_vc");
primitive!(serde_json::Value, "json_value");

primitive!(Vec<u8>, "vec_u8");

#[turbo_tasks::value(transparent, eq = "manual")]
#[derive(Debug, Clone)]
pub struct Regex(
    #[turbo_tasks(trace_ignore)]
    #[serde(with = "serde_regex")]
    pub regex::Regex,
);

impl Deref for Regex {
    type Target = regex::Regex;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq for Regex {
    fn eq(&self, other: &Regex) -> bool {
        // Context: https://github.com/rust-lang/regex/issues/313#issuecomment-269898900
        self.0.as_str() == other.0.as_str()
    }
}
impl Eq for Regex {}
