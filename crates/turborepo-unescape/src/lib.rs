use std::{fmt, fmt::Display, ops::Deref};

use biome_deserialize::{Deserializable, DeserializableValue, DeserializationDiagnostic};

// We're using a newtype here because biome currently doesn't
// handle escapes and we can't override the String deserializer
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct UnescapedString(String);

impl Display for UnescapedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for UnescapedString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for UnescapedString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
fn unescape_str(s: String) -> Result<String, serde_json::Error> {
    let wrapped_s = format!("\"{}\"", s);

    serde_json::from_str(&wrapped_s)
}

impl Deserializable for UnescapedString {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let str = String::deserialize(value, name, diagnostics)?;

        match unescape_str(str) {
            Ok(s) => Some(Self(s)),
            Err(e) => {
                diagnostics.push(DeserializationDiagnostic::new(format!("{}", e)));
                None
            }
        }
    }
}

impl From<UnescapedString> for String {
    fn from(value: UnescapedString) -> Self {
        value.0
    }
}

// For testing purposes
impl From<&'static str> for UnescapedString {
    fn from(value: &'static str) -> Self {
        Self(value.to_owned())
    }
}
