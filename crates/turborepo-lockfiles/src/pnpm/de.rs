use serde::Deserialize;

use super::{LockfileVersion, VersionFormat};

impl From<f32> for LockfileVersion {
    fn from(value: f32) -> Self {
        Self {
            version: value.to_string(),
            format: VersionFormat::Float,
        }
    }
}

impl From<String> for LockfileVersion {
    fn from(value: String) -> Self {
        Self {
            version: value,
            format: VersionFormat::String,
        }
    }
}

impl<'de> Deserialize<'de> for LockfileVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StringOrNum {
            Str(String),
            Num(f32),
        }

        Ok(match StringOrNum::deserialize(deserializer)? {
            StringOrNum::Num(x) => LockfileVersion::from(x),
            StringOrNum::Str(s) => LockfileVersion::from(s),
        })
    }
}
