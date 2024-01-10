use std::{
    ops::{Deref, Range},
    sync::Arc,
};

use biome_deserialize::{Deserializable, DeserializableValue, DeserializationDiagnostic};
use serde::Serialize;

#[derive(Debug, Default, Clone, Serialize)]
#[serde(transparent)]
pub struct Spanned<T> {
    pub value: T,
    #[serde(skip)]
    pub range: Option<Range<usize>>,
    #[serde(skip)]
    pub path: Option<Arc<str>>,
    #[serde(skip)]
    pub text: Option<Arc<str>>,
}

impl<T: Deserializable> Deserializable for Spanned<T> {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let range = value.range();
        let value = T::deserialize(value, name, diagnostics)?;
        Some(Spanned {
            value,
            range: Some(range.into()),
            path: None,
            text: None,
        })
    }
}

// We do *not* check for the range equality because that's too finicky
// to get right in tests.
impl<T: PartialEq> PartialEq for Spanned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T> Spanned<T> {
    pub fn new(t: T) -> Self {
        Self {
            value: t,
            range: None,
            path: None,
            text: None,
        }
    }

    pub fn with_range(self, range: impl Into<Range<usize>>) -> Self {
        Self {
            range: Some(range.into()),
            ..self
        }
    }

    pub fn with_path(self, path: Arc<str>) -> Self {
        Self {
            path: Some(path),
            ..self
        }
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
