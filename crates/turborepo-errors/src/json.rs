//! JSON deserialization helpers that guard against panics in biome.
//!
//! `biome_deserialize::json::deserialize_from_json_str` runs deserialization
//! even when the source failed to parse. On malformed input containing an
//! unterminated string literal (e.g. a dangling `"` at the end of a line or
//! file), deserialization panics inside `biome_json_syntax::inner_string_text`
//! with `assertion failed: start <= end`. No published biome release fixes
//! this, so we check for parse errors *before* deserializing and surface them
//! as diagnostics instead.

use biome_deserialize::{Deserializable, json::deserialize_from_json_ast};
use biome_diagnostics::{DiagnosticExt, Error};
use biome_json_parser::{JsonParserOptions, parse_json};

/// A replacement for `biome_deserialize::json::deserialize_from_json_str`
/// that reports parse errors rather than attempting to deserialize a broken
/// syntax tree (which can panic, see module docs).
///
/// Returns the same `(deserialized, diagnostics)` pair as
/// `Deserialized::consume`. When the source fails to parse, the deserialized
/// value is `None` and the diagnostics contain only the parse errors.
pub fn deserialize_from_json_str<Output: Deserializable>(
    source: &str,
    options: JsonParserOptions,
    name: &str,
) -> (Option<Output>, Vec<Error>) {
    let parse = parse_json(source, options);
    if parse.has_errors() {
        let diagnostics = parse
            .into_diagnostics()
            .into_iter()
            .map(|diagnostic| Error::from(diagnostic).with_file_source_code(source))
            .collect();
        return (None, diagnostics);
    }
    // The tree is known to be error-free at this point, so deserializing it
    // directly is safe. This is the one place allowed to call biome's
    // deserialization entry points (see disallowed-methods in clippy.toml).
    #[allow(clippy::disallowed_methods)]
    let (deserialized, diagnostics) =
        deserialize_from_json_ast::<Output>(&parse.tree(), name).consume();
    let diagnostics = diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.with_file_source_code(source))
        .collect();
    (deserialized, diagnostics)
}

#[cfg(test)]
mod tests {
    use biome_deserialize_macros::Deserializable;

    use super::*;

    #[derive(Debug, Default, Deserializable, PartialEq, Eq)]
    struct TestConfig {
        name: Option<String>,
    }

    #[test]
    fn test_valid_json_deserializes() {
        let (config, diagnostics) = deserialize_from_json_str::<TestConfig>(
            r#"{"name": "turbo"}"#,
            JsonParserOptions::default(),
            "test.json",
        );
        assert_eq!(diagnostics.len(), 0);
        assert_eq!(
            config,
            Some(TestConfig {
                name: Some("turbo".to_owned())
            })
        );
    }

    // Regression tests for https://github.com/vercel/turborepo/issues/13197
    // Unterminated string literals used to panic inside biome instead of
    // producing parse errors.
    #[test_case::test_case("\""; "lone quote")]
    #[test_case::test_case("{\"name\": \""; "quote at eof")]
    #[test_case::test_case("{\"name\": \"\n}"; "quote before newline")]
    #[test_case::test_case("{\"name\": [\""; "quote in array")]
    fn test_unterminated_string_reports_error(source: &str) {
        let (config, diagnostics) = deserialize_from_json_str::<TestConfig>(
            source,
            JsonParserOptions::default(),
            "test.json",
        );
        assert_eq!(config, None);
        assert!(!diagnostics.is_empty());
    }
}
