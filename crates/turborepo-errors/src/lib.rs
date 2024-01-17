use std::{
    ops::{Deref, Range},
    sync::Arc,
};

use biome_deserialize::{Deserializable, DeserializableValue, DeserializationDiagnostic};
use serde::Serialize;

//pub const TURBO_SITE: &str = "https://turbo.build";
pub const TURBO_SITE: &str = "http://localhost:3000";

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
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

impl<T> Spanned<T> {
    pub fn new(t: T) -> Self {
        Self {
            value: t,
            range: None,
            path: None,
            text: None,
        }
    }

    pub fn with_text(self, text: impl Into<Arc<str>>) -> Self {
        Self {
            text: Some(text.into()),
            ..self
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

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            value: f(self.value),
            range: self.range,
            path: self.path,
            text: self.text,
        }
    }
}

impl<T> Spanned<Option<T>> {
    pub fn is_none(&self) -> bool {
        self.value.is_none()
    }
}

impl<T> Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

pub trait WithText {
    fn add_text(&mut self, text: Arc<str>);
}

impl<T> WithText for Spanned<T> {
    fn add_text(&mut self, text: Arc<str>) {
        self.text = Some(text);
    }
}

impl<T: WithText> WithText for Option<T> {
    fn add_text(&mut self, text: Arc<str>) {
        if let Some(inner) = self {
            inner.add_text(text);
        }
    }
}

impl<T: WithText> WithText for Vec<T> {
    fn add_text(&mut self, text: Arc<str>) {
        for item in self {
            item.add_text(text.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use test_case::test_case;

    use crate::Spanned;

    #[test_case(Spanned { value: 10, range: Some(0..2), path: None, text: None }, "10")]
    #[test_case(Spanned { value: "hello world", range: None, path: None, text: Some(Arc::from("hello world")) }, "\"hello world\"")]
    #[test_case(Spanned { value: json!({ "name": "George", "age": 100 }), range: None, path: None, text: Some(Arc::from("hello world")) }, "{\"age\":100,\"name\":\"George\"}")]
    fn test_serialize_spanned<T>(spanned_value: Spanned<T>, expected: &str)
    where
        T: serde::Serialize,
    {
        let actual = serde_json::to_string(&spanned_value).unwrap();
        assert_eq!(actual, expected);
    }
}
