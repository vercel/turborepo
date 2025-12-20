//! Diagnostic utilities to preserve source for more actionable error messages
//! Used in conjunction with `miette` to include source snippets in errors.
//! Any parsing of files should attempt to produce value of `Spanned<T>` so if
//! we need to reference where T came from the span is available.

// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]

use std::{
    fmt::Display,
    iter,
    iter::Once,
    ops::{Deref, DerefMut, Range},
    sync::Arc,
};

use biome_deserialize::{Deserializable, DeserializableValue, DeserializationDiagnostic};
use miette::{Diagnostic, NamedSource, SourceSpan};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Base URL for links supplied in error messages. You can use the TURBO_SITE
/// environment variable at compile time to set a base URL for easier debugging.
///
/// When TURBO_SITE is not provided at compile time, the production site will be
/// used.
pub const TURBO_SITE: &str = match option_env!("TURBO_SITE") {
    Some(url) => url,
    None => "https://turborepo.com",
};

/// A little helper to convert from biome's syntax errors to miette.
#[derive(Debug, Error, Diagnostic)]
#[error("{message}")]
pub struct ParseDiagnostic {
    message: String,
    #[source_code]
    source_code: NamedSource<String>,
    #[label]
    label: Option<SourceSpan>,
}

struct BiomeMessage<'a, T: ?Sized>(&'a T);

impl<T: biome_diagnostics::Diagnostic + ?Sized> Display for BiomeMessage<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.description(f)
    }
}

impl<T: biome_diagnostics::Diagnostic + ?Sized> From<&'_ T> for ParseDiagnostic {
    fn from(diagnostic: &T) -> Self {
        let location = diagnostic.location();
        let message = BiomeMessage(diagnostic).to_string();
        let path = location
            .resource
            .and_then(|r| r.as_file().map(|p| p.to_string()))
            .unwrap_or_default();
        Self {
            message,
            source_code: NamedSource::new(
                path,
                location
                    .source_code
                    .map(|s| s.text.to_string())
                    .unwrap_or_default(),
            ),
            label: location.span.map(|span| {
                let start: usize = span.start().into();
                let len: usize = span.len().into();
                (start, len).into()
            }),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, Eq)]
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

impl<T> IntoIterator for Spanned<T> {
    type Item = T;
    type IntoIter = Once<T>;

    fn into_iter(self) -> Self::IntoIter {
        iter::once(self.value)
    }
}

impl<'a, T> IntoIterator for &'a Spanned<T> {
    type Item = &'a T;
    type IntoIter = Once<&'a T>;

    fn into_iter(self) -> Self::IntoIter {
        iter::once(&self.value)
    }
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

impl Spanned<String> {
    pub fn as_str(&self) -> &str {
        self.value.as_str()
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

    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            value: &self.value,
            range: self.range.clone(),
            path: self.path.clone(),
            text: self.text.clone(),
        }
    }

    /// Splits out the span info from the value.
    pub fn split(self) -> (T, Spanned<()>) {
        (
            self.value,
            Spanned {
                value: (),
                range: self.range,
                path: self.path,
                text: self.text,
            },
        )
    }

    /// Gets a ref to the inner value
    pub fn as_inner(&self) -> &T {
        &self.value
    }

    /// Replaces the old value with a new one
    pub fn to<U>(&self, value: U) -> Spanned<U> {
        Spanned {
            value,
            range: self.range.clone(),
            path: self.path.clone(),
            text: self.text.clone(),
        }
    }

    /// Gets the span and the text if both exist. If either doesn't exist, we
    /// return `None` for the span and an empty string for the text, since
    /// miette doesn't accept an `Option<String>` for `#[source_code]`
    pub fn span_and_text(&self, default_path: &str) -> (Option<SourceSpan>, NamedSource<String>) {
        let path = self.path.as_ref().map_or(default_path, |p| p.as_ref());
        match self.range.clone().zip(self.text.as_ref()) {
            Some((range, text)) => (Some(range.into()), NamedSource::new(path, text.to_string())),
            None => (None, NamedSource::new(path, Default::default())),
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            value: f(self.value),
            range: self.range,
            path: self.path,
            text: self.text,
        }
    }

    /// Gets a mutable ref to the inner value
    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.value
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

impl<T> DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

pub trait WithMetadata {
    fn add_text(&mut self, text: Arc<str>);
    fn add_path(&mut self, path: Arc<str>);
}

impl<T> WithMetadata for Spanned<T> {
    fn add_text(&mut self, text: Arc<str>) {
        self.text = Some(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.path = Some(path);
    }
}

impl<T: WithMetadata> WithMetadata for Option<T> {
    fn add_text(&mut self, text: Arc<str>) {
        if let Some(inner) = self {
            inner.add_text(text);
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        if let Some(inner) = self {
            inner.add_path(path);
        }
    }
}

impl<T: WithMetadata> WithMetadata for Vec<T> {
    fn add_text(&mut self, text: Arc<str>) {
        for item in self {
            item.add_text(text.clone());
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        for item in self {
            item.add_path(path.clone());
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
    #[test_case(Spanned { value: json!({ "name": "George", "age": 100 }), range: None, path: None, text: Some(Arc::from("hello world")) }, "{\"name\":\"George\",\"age\":100}")]
    fn test_serialize_spanned<T>(spanned_value: Spanned<T>, expected: &str)
    where
        T: serde::Serialize,
    {
        let actual = serde_json::to_string(&spanned_value).unwrap();
        assert_eq!(actual, expected);
    }
}
