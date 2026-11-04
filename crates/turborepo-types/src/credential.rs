use serde::{Deserialize, Serialize};

use crate::SecretString;

/// A credential and its inseparable runtime destination restriction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    Generic(SecretString),
    SavedVercel(SecretString),
}

impl Credential {
    pub fn generic(secret: SecretString) -> Self {
        Self::Generic(secret)
    }

    pub fn saved_vercel(secret: SecretString) -> Self {
        Self::SavedVercel(secret)
    }

    pub fn is_saved_vercel(&self) -> bool {
        matches!(self, Self::SavedVercel(_))
    }

    /// Secret access for persistence and validated transport boundaries.
    pub fn expose(&self) -> &str {
        match self {
            Self::Generic(secret) | Self::SavedVercel(secret) => secret.expose(),
        }
    }
}

impl<'de> Deserialize<'de> for Credential {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SecretString::deserialize(deserializer).map(Self::Generic)
    }
}

impl Serialize for Credential {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(crate::secret::REDACTED)
    }
}

impl From<String> for Credential {
    fn from(value: String) -> Self {
        Self::Generic(value.into())
    }
}

impl From<&str> for Credential {
    fn from(value: &str) -> Self {
        Self::Generic(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_and_serialization_redact_both_variants() {
        for credential in [
            Credential::generic("sentinel".into()),
            Credential::saved_vercel("sentinel".into()),
        ] {
            assert!(!format!("{credential:?}").contains("sentinel"));
            assert_eq!(serde_json::to_string(&credential).unwrap(), "\"***\"");
        }
        assert!(serde_json::from_str::<Credential>(r#"{"SavedVercel":"sentinel"}"#).is_err());
        assert!(
            !serde_json::from_str::<Credential>("\"sentinel\"")
                .unwrap()
                .is_saved_vercel()
        );
    }
}
