use std::fmt;

use reqwest::header::HeaderValue;

/// A wrapper around `String` that prevents accidental exposure of sensitive
/// values through `Debug` or `Display` formatting. The inner value is only
/// accessible through explicit method calls.
#[derive(Clone)]
pub struct SecretString(String);

const REDACTED: &str = "***";

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Constructs the `Authorization: Bearer <token>` header value.
    /// This is the only intended way to use the token in HTTP requests,
    /// keeping the raw value out of format strings in application code.
    pub fn bearer_header(&self) -> HeaderValue {
        HeaderValue::from_str(&format!("Bearer {}", self.0))
            .expect("token contained invalid header characters")
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
        self.0 == other.0
    }
}

impl Eq for SecretString {}

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
    fn bearer_header_contains_token() {
        let secret = SecretString::new("my-token".to_string());
        let header = secret.bearer_header();
        assert_eq!(header.to_str().unwrap(), "Bearer my-token");
    }

    #[test]
    fn equality_compares_inner_values() {
        let a = SecretString::new("token".to_string());
        let b = SecretString::new("token".to_string());
        let c = SecretString::new("other".to_string());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
