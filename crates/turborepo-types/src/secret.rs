use std::fmt;

use secrecy::{ExposeSecret, SecretBox, zeroize::Zeroize};
use serde::Deserialize;

/// A wrapper around a secret string that prevents accidental exposure through
/// `Debug`, `Display`, or `Serialize` formatting.
///
/// The inner value is only accessible through [`expose()`](Self::expose).
///
/// Backed by `secrecy::SecretBox<str>`, so the inner value is zeroized on drop.
#[derive(Clone)]
pub struct SecretString(SecretBox<str>);

pub const REDACTED: &str = "***";

impl SecretString {
    pub fn new(mut value: String) -> Self {
        let secret = Self(SecretBox::new(Box::from(value.as_str())));
        value.zeroize();
        secret
    }

    /// Returns a reference to the underlying secret value.
    ///
    /// # Security
    /// The caller is responsible for ensuring the returned value is not
    /// logged, serialized, or otherwise exposed unintentionally.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for SecretString {
    fn from(s: &str) -> Self {
        Self(SecretBox::new(Box::from(s)))
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        self.0.expose_secret() == other.0.expose_secret()
    }
}

impl Eq for SecretString {}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_secret() {
        let secret = SecretString::new("super-secret-token".to_string());
        let debug_output = format!("{:?}", secret);
        assert_eq!(debug_output, REDACTED);
        assert!(!debug_output.contains("super-secret-token"));
    }

    #[test]
    fn display_does_not_leak_secret() {
        let secret = SecretString::new("super-secret-token".to_string());
        let display_output = format!("{}", secret);
        assert_eq!(display_output, REDACTED);
        assert!(!display_output.contains("super-secret-token"));
    }

    #[test]
    fn expose_returns_inner_value() {
        let secret = SecretString::new("my-token".to_string());
        assert_eq!(secret.expose(), "my-token");
    }

    #[test]
    fn deserialize_captures_value() {
        let json = "\"my-secret-token\"";
        let secret: SecretString = serde_json::from_str(json).unwrap();
        assert_eq!(secret.expose(), "my-secret-token");
    }

    #[test]
    fn equality_compares_inner_values() {
        let a = SecretString::new("token".to_string());
        let b = SecretString::new("token".to_string());
        let c = SecretString::new("other".to_string());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_string_works() {
        let secret: SecretString = "my-token".to_string().into();
        assert_eq!(secret.expose(), "my-token");
    }

    #[test]
    fn from_str_works() {
        let secret: SecretString = "my-token".into();
        assert_eq!(secret.expose(), "my-token");
    }
}
