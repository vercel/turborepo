use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbo_tasks::trace::TraceRawVcs;

use crate::{environment::EnvironmentVc, number::ConstantNumber};

// TODO stringify split map collect could be optimized with a marco
#[macro_export]
macro_rules! compile_time_defines_internal {
    ($map:ident, $($name:ident).+ = $value:expr) => {
        $map.insert(
            $crate::compile_time_defines_internal!($($name).+).into(),
            $value.into()
        );
    };
    ($map:ident, $($name:ident).+ = $value:expr,) => {
        $map.insert(
            $crate::compile_time_defines_internal!($($name).+).into(),
            $value.into()
        );
    };
    ($map:ident, $($name:ident).+ = $value:expr, $($more:tt)+) => {
        $crate::compile_time_defines_internal!($map, $($name).+ = $value);
        $crate::compile_time_defines_internal!($map, $($more)+);
    };
    ($name:ident) => {
        [stringify!($name).to_string()]
    };
    ($name:ident . $($more:ident).+) => {
        $crate::compile_time_defines_internal!($($more).+, [stringify!($name).to_string()])
    };
    ($name:ident, [$($array:expr),+]) => {
        [$($array),+, stringify!($name).to_string()]
    };
    ($name:ident . $($more:ident).+, [$($array:expr),+]) => {
        $crate::compile_time_defines_internal!($($more).+, [$($array),+, stringify!($name).to_string()])
    };
}

// TODO stringify split map collect could be optimized with a marco
#[macro_export]
macro_rules! compile_time_defines {
    ($($more:tt)+) => {
        {
            let mut map = std::collections::HashMap::new();
            $crate::compile_time_defines_internal!(map, $($more)+);
            $crate::compile_time_info::CompileTimeDefines(map)
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TraceRawVcs)]
#[serde(untagged)]
pub enum CompileTimeDefineValue {
    Bool(bool),
    String(String),
    Number(ConstantNumber),
}

impl From<bool> for CompileTimeDefineValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<String> for CompileTimeDefineValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for CompileTimeDefineValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl From<f64> for CompileTimeDefineValue {
    fn from(value: f64) -> Self {
        Self::Number(ConstantNumber(value))
    }
}

impl From<ConstantNumber> for CompileTimeDefineValue {
    fn from(value: ConstantNumber) -> Self {
        Self::Number(value)
    }
}

#[turbo_tasks::value(transparent)]
pub struct CompileTimeDefines(pub HashMap<Vec<String>, CompileTimeDefineValue>);

#[turbo_tasks::value_impl]
impl CompileTimeDefinesVc {
    #[turbo_tasks::function]
    pub fn empty() -> Self {
        Self::cell(HashMap::new())
    }
}

#[turbo_tasks::value(shared)]
pub struct CompileTimeInfo {
    pub environment: EnvironmentVc,
    pub defines: CompileTimeDefinesVc,
}

#[turbo_tasks::value_impl]
impl CompileTimeInfoVc {
    #[turbo_tasks::function]
    pub fn new(environment: EnvironmentVc) -> Self {
        CompileTimeInfo {
            environment,
            defines: CompileTimeDefinesVc::empty(),
        }
        .cell()
    }

    #[turbo_tasks::function]
    pub async fn environment(self) -> Result<EnvironmentVc> {
        Ok(self.await?.environment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialization() {
        use indexmap::{indexmap, IndexMap};
        let values: IndexMap<&str, CompileTimeDefineValue> = serde_json::from_str(
            r#"
            {
                "true": true,
                "false": false,
                "str": "hello",
                "empty": "",
                "number": 12.25
            }
            "#,
        )
        .unwrap();
        assert_eq!(
            values,
            indexmap! {
                "true" => CompileTimeDefineValue::Bool(true),
                "false" => CompileTimeDefineValue::Bool(false),
                "str" => CompileTimeDefineValue::String("hello".to_string()),
                "empty" => CompileTimeDefineValue::String("".to_string()),
                "number" => CompileTimeDefineValue::Number(ConstantNumber(12.25)),
            }
        );
    }
}
