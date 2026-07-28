//! Span-tracking JSON parser for package manifests.
//!
//! Manifest parsing sits on the critical path of every run: one parse per
//! workspace package. The general-purpose biome CST parser this replaces
//! spends ~27µs per manifest building a full syntax tree with error
//! recovery; this parser scans the source once, extracting exactly what
//! [`PackageJson`] needs, in ~1-2µs.
//!
//! Diagnostics keep their fidelity: `Spanned` fields carry the value
//! token's full source range (quotes and braces included), so miette
//! snippets in downstream errors — packageManager mismatches, devEngines
//! validation, recursive turbo invocations — render with highlighted
//! snippets. Parse errors point at the offending token.
//!
//! Acceptance rules: strict JSON syntax, a tolerated leading BOM,
//! duplicate keys last-win, and explicit `null` is a type error in typed
//! fields but a valid `devEngines` value (package-manager resolution
//! reports it with this span). `serde_json` imposes a 128-level nesting
//! limit when materializing unstructured field values.

use std::{collections::BTreeMap, ops::Range, sync::Arc};

use serde::de::DeserializeOwned;
use turborepo_errors::{ParseDiagnostic, Spanned, WithMetadata};

use crate::package_json::{Error, PackageJson, PnpmConfig};

pub(crate) fn parse(contents: &str, path: &str) -> Result<PackageJson, Error> {
    Parser::new(contents)
        .parse_manifest()
        .map_err(|diag| Error::Parse(vec![diag.into_parse_diagnostic(contents, path)]))
        .map(|manifest| manifest.finish(contents, path))
}

/// A parse or type error at a source range.
struct Diag {
    message: String,
    span: Option<Range<usize>>,
}

impl Diag {
    fn at(span: Range<usize>, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: Some(span),
        }
    }

    fn into_parse_diagnostic(self, contents: &str, path: &str) -> ParseDiagnostic {
        ParseDiagnostic::new(self.message, path, contents.to_owned(), self.span)
    }
}

/// Parsed manifest before file text/path metadata is attached.
#[derive(Default)]
struct Manifest {
    package_json: PackageJson,
}

impl Manifest {
    /// Attach source text and path to the fields whose spans feed
    /// diagnostics.
    fn finish(mut self, contents: &str, path: &str) -> PackageJson {
        let text: Arc<str> = contents.into();
        let path: Arc<str> = path.into();
        if let Some(package_manager) = self.package_json.package_manager.as_mut() {
            package_manager.add_text(text.clone());
            package_manager.add_path(path.clone());
        }
        if let Some(dev_engines) = self.package_json.dev_engines.as_mut() {
            dev_engines.add_text(text.clone());
            dev_engines.add_path(path.clone());
        }
        for script in self.package_json.scripts.values_mut() {
            script.add_text(text.clone());
            script.add_path(path.clone());
        }
        self.package_json
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            text,
            pos: 0,
        };
        // A leading BOM is tolerated, matching the biome parser.
        if text.as_bytes().starts_with(b"\xef\xbb\xbf") {
            parser.pos = 3;
        }
        parser
    }

    fn parse_manifest(&mut self) -> Result<Manifest, Diag> {
        let mut manifest = Manifest::default();
        self.skip_ws();
        if self.peek() != Some(b'{') {
            return Err(self.error_here("expected `{` at the start of the manifest"));
        }
        self.parse_object_members(|parser, key, key_range| {
            manifest.member(parser, key, key_range)
        })?;
        self.skip_ws();
        if self.pos != self.bytes.len() {
            return Err(self.error_here("unexpected trailing content after the manifest object"));
        }
        Ok(manifest)
    }

    /// Parse the object starting at `self.pos` (which must be `{`), calling
    /// `member` for each key. `member` must consume the member's value.
    fn parse_object_members(
        &mut self,
        mut member: impl FnMut(&mut Self, String, Range<usize>) -> Result<(), Diag>,
    ) -> Result<(), Diag> {
        debug_assert_eq!(self.peek(), Some(b'{'));
        self.pos += 1;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(());
        }
        loop {
            self.skip_ws();
            let key_range = self.scan_string()?;
            let key: String = self.deserialize_slice(key_range.clone(), "an object key")?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.error_here("expected `:` after object key"));
            }
            self.pos += 1;
            self.skip_ws();
            member(self, key, key_range)?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err(self.error_here("expected `,` or `}` in object")),
            }
        }
    }

    /// Deserialize the source slice at `range` with serde_json. The slice is
    /// a single JSON value already validated structurally by the scanner;
    /// serde performs content validation (escapes, number precision) and
    /// unescaping.
    fn deserialize_slice<T: DeserializeOwned>(
        &self,
        range: Range<usize>,
        expected: &str,
    ) -> Result<T, Diag> {
        serde_json::from_str(&self.text[range.clone()])
            .map_err(|_| Diag::at(range, format!("expected {expected}")))
    }

    fn spanned<T: DeserializeOwned>(
        &self,
        range: Range<usize>,
        expected: &str,
    ) -> Result<Spanned<T>, Diag> {
        let value: T = self.deserialize_slice(range.clone(), expected)?;
        Ok(Spanned::new(value).with_range(range))
    }

    /// Parse an object of string values, keeping each value's span.
    fn parse_string_map_spanned(
        &mut self,
        field: &str,
    ) -> Result<BTreeMap<String, Spanned<String>>, Diag> {
        if self.peek() != Some(b'{') {
            let range = self.scan_value()?;
            return Err(Diag::at(
                range,
                format!("expected `{field}` to be an object"),
            ));
        }
        let mut map = BTreeMap::new();
        self.parse_object_members(|parser, key, _| {
            let value_range = parser.scan_value()?;
            let value = parser.spanned(value_range, "a string")?;
            map.insert(key, value);
            Ok(())
        })?;
        Ok(map)
    }

    // -- scanning ---------------------------------------------------------

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn error_here(&self, message: impl Into<String>) -> Diag {
        let end = (self.pos + 1).min(self.bytes.len());
        Diag {
            message: message.into(),
            span: Some(self.pos.min(self.bytes.len())..end),
        }
    }

    /// Scan one JSON value, returning its full source range. Validates
    /// syntax structurally with an explicit container stack, so nesting
    /// depth is unbounded here (materialization of unstructured values via
    /// serde_json is what imposes a depth limit).
    fn scan_value(&mut self) -> Result<Range<usize>, Diag> {
        let start = self.pos;
        let mut stack: Vec<u8> = Vec::new();
        'value: loop {
            self.skip_ws();
            match self.peek() {
                Some(b'{') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        // Empty object: a complete value; unwind below.
                    } else {
                        stack.push(b'{');
                        self.scan_string()?;
                        self.skip_ws();
                        if self.peek() != Some(b':') {
                            return Err(self.error_here("expected `:` after object key"));
                        }
                        self.pos += 1;
                        continue 'value;
                    }
                }
                Some(b'[') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                    } else {
                        stack.push(b'[');
                        continue 'value;
                    }
                }
                Some(b'"') => {
                    self.scan_string()?;
                }
                Some(b't') => self.scan_keyword(b"true")?,
                Some(b'f') => self.scan_keyword(b"false")?,
                Some(b'n') => self.scan_keyword(b"null")?,
                Some(b'-' | b'0'..=b'9') => self.scan_number()?,
                _ => return Err(self.error_here("expected a JSON value")),
            }
            // A complete value has been consumed; unwind commas and
            // container closers until another value is expected.
            loop {
                let Some(&container) = stack.last() else {
                    return Ok(start..self.pos);
                };
                self.skip_ws();
                match self.peek() {
                    Some(b',') => {
                        self.pos += 1;
                        self.skip_ws();
                        if container == b'{' {
                            self.scan_string()?;
                            self.skip_ws();
                            if self.peek() != Some(b':') {
                                return Err(self.error_here("expected `:` after object key"));
                            }
                            self.pos += 1;
                        }
                        continue 'value;
                    }
                    Some(b'}') if container == b'{' => {
                        self.pos += 1;
                        stack.pop();
                    }
                    Some(b']') if container == b'[' => {
                        self.pos += 1;
                        stack.pop();
                    }
                    _ => {
                        return Err(self.error_here("expected `,` or a closing delimiter"));
                    }
                }
            }
        }
    }

    /// Scan a string token (opening quote at `self.pos`), returning the
    /// range including quotes. Validates escape structure; full escape
    /// content validation happens in serde when the slice is deserialized.
    fn scan_string(&mut self) -> Result<Range<usize>, Diag> {
        let start = self.pos;
        if self.peek() != Some(b'"') {
            return Err(self.error_here("expected a string"));
        }
        self.pos += 1;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(start..self.pos);
                }
                b'\\' => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                            self.pos += 1;
                        }
                        Some(b'u') => {
                            self.pos += 1;
                            for _ in 0..4 {
                                if !matches!(
                                    self.peek(),
                                    Some(b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F')
                                ) {
                                    return Err(self.error_here("invalid unicode escape in string"));
                                }
                                self.pos += 1;
                            }
                        }
                        _ => return Err(self.error_here("invalid escape in string")),
                    }
                }
                0x00..=0x1f => {
                    return Err(self.error_here("unescaped control character in string"));
                }
                _ => self.pos += 1,
            }
        }
        Err(Diag::at(
            start..self.bytes.len(),
            "unterminated string".to_owned(),
        ))
    }

    fn scan_keyword(&mut self, keyword: &[u8]) -> Result<(), Diag> {
        if self.bytes[self.pos..].starts_with(keyword) {
            self.pos += keyword.len();
            Ok(())
        } else {
            Err(self.error_here("expected a JSON value"))
        }
    }

    fn scan_number(&mut self) -> Result<(), Diag> {
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error_here("invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error_here("invalid number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error_here("invalid number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        Ok(())
    }
}

impl Manifest {
    /// Handle one top-level manifest member. Duplicate keys last-win,
    /// matching the biome deserializer.
    fn member(
        &mut self,
        parser: &mut Parser,
        key: String,
        _key_range: Range<usize>,
    ) -> Result<(), Diag> {
        let pkg = &mut self.package_json;
        match key.as_str() {
            "name" => {
                let range = parser.scan_value()?;
                pkg.name = Some(parser.spanned(range, "`name` to be a string")?);
            }
            "version" => {
                let range = parser.scan_value()?;
                pkg.version = Some(parser.deserialize_slice(range, "`version` to be a string")?);
            }
            "packageManager" => {
                let range = parser.scan_value()?;
                pkg.package_manager =
                    Some(parser.spanned(range, "`packageManager` to be a string")?);
            }
            "devEngines" => {
                // Any JSON value is accepted here (validation happens in
                // package-manager resolution, with this span).
                let range = parser.scan_value()?;
                pkg.dev_engines = Some(parser.spanned(range, "`devEngines` to be valid JSON")?);
            }
            "dependencies" => {
                let range = parser.scan_value()?;
                pkg.dependencies = Some(parser.deserialize_slice(
                    range,
                    "`dependencies` to be a map of package names to version specifiers",
                )?);
            }
            "devDependencies" => {
                let range = parser.scan_value()?;
                pkg.dev_dependencies = Some(parser.deserialize_slice(
                    range,
                    "`devDependencies` to be a map of package names to version specifiers",
                )?);
            }
            "optionalDependencies" => {
                let range = parser.scan_value()?;
                pkg.optional_dependencies = Some(parser.deserialize_slice(
                    range,
                    "`optionalDependencies` to be a map of package names to version specifiers",
                )?);
            }
            "peerDependencies" => {
                let range = parser.scan_value()?;
                pkg.peer_dependencies = Some(parser.deserialize_slice(
                    range,
                    "`peerDependencies` to be a map of package names to version specifiers",
                )?);
            }
            "scripts" => {
                pkg.scripts = parser.parse_string_map_spanned("scripts")?;
            }
            "resolutions" => {
                let range = parser.scan_value()?;
                pkg.resolutions = Some(parser.deserialize_slice(
                    range,
                    "`resolutions` to be a map of package names to version specifiers",
                )?);
            }
            "pnpm" => {
                pkg.pnpm = Some(Self::parse_pnpm(parser)?);
            }
            "patchedDependencies" => {
                let range = parser.scan_value()?;
                pkg.patched_dependencies = Some(parser.deserialize_slice(
                    range,
                    "`patchedDependencies` to be a map of package specifiers to relative patch \
                     paths",
                )?);
            }
            _ => {
                let range = parser.scan_value()?;
                let value = parser.deserialize_slice(range, "valid JSON")?;
                pkg.other.insert(key, value);
            }
        }
        Ok(())
    }

    fn parse_pnpm(parser: &mut Parser) -> Result<PnpmConfig, Diag> {
        if parser.peek() != Some(b'{') {
            let range = parser.scan_value()?;
            return Err(Diag::at(
                range,
                "expected `pnpm` to be an object".to_owned(),
            ));
        }
        let mut config = PnpmConfig {
            patched_dependencies: None,
            other: BTreeMap::new(),
        };
        parser.parse_object_members(|parser, key, _| {
            let range = parser.scan_value()?;
            if key == "patchedDependencies" {
                config.patched_dependencies = Some(parser.deserialize_slice(
                    range,
                    "`pnpm.patchedDependencies` to be a map of package specifiers to relative \
                     patch paths",
                )?);
            } else {
                let value = parser.deserialize_slice(range, "valid JSON")?;
                config.other.insert(key, value);
            }
            Ok(())
        })?;
        Ok(config)
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use test_case::test_case;

    use super::*;

    fn parse_ok(contents: &str) -> PackageJson {
        parse(contents, "pkg/package.json").expect("parser must accept")
    }

    /// The source range of `token`'s first occurrence: the span convention
    /// is the value token's full source text, quotes and braces included.
    fn range_of(contents: &str, token: &str) -> Range<usize> {
        let start = contents.find(token).expect("token present");
        start..start + token.len()
    }

    #[test]
    fn test_every_field() {
        let contents = r#"{
            "name": "full",
            "version": "0.1.0",
            "packageManager": "pnpm@9.0.0",
            "devEngines": {"packageManager": {"name": "pnpm", "version": "9.12.3"}},
            "dependencies": {"a": "^1.0.0", "b": "workspace:*"},
            "devDependencies": {"c": "~2.0.0"},
            "optionalDependencies": {"d": "3.0.0"},
            "peerDependencies": {"e": ">=4"},
            "scripts": {"build": "turbo build", "test": "jest --ci"},
            "resolutions": {"f": "5.0.0"},
            "pnpm": {"patchedDependencies": {"g@1.0.0": "patches/g.patch"}, "overrides": {"h": "6.0.0"}},
            "patchedDependencies": {"i@2.0.0": "patches/i.patch"},
            "private": true,
            "workspaces": ["packages/*"],
            "exports": {".": {"import": "./dist/index.mjs"}, "./sub": null}
        }"#;
        let pkg = parse_ok(contents);

        let name = pkg.name.as_ref().expect("name");
        assert_eq!(name.value, "full");
        assert_eq!(name.range, Some(range_of(contents, "\"full\"")));

        assert_eq!(pkg.version.as_deref(), Some("0.1.0"));

        let pm = pkg.package_manager.as_ref().expect("packageManager");
        assert_eq!(pm.value, "pnpm@9.0.0");
        assert_eq!(pm.range, Some(range_of(contents, "\"pnpm@9.0.0\"")));

        let dev_engines = pkg.dev_engines.as_ref().expect("devEngines");
        assert_eq!(
            dev_engines.value,
            json!({"packageManager": {"name": "pnpm", "version": "9.12.3"}})
        );
        assert_eq!(
            dev_engines.range,
            Some(range_of(
                contents,
                r#"{"packageManager": {"name": "pnpm", "version": "9.12.3"}}"#
            ))
        );

        assert_eq!(
            pkg.dependencies,
            Some(BTreeMap::from([
                ("a".into(), "^1.0.0".into()),
                ("b".into(), "workspace:*".into()),
            ]))
        );
        assert_eq!(
            pkg.dev_dependencies,
            Some(BTreeMap::from([("c".into(), "~2.0.0".into())]))
        );
        assert_eq!(
            pkg.optional_dependencies,
            Some(BTreeMap::from([("d".into(), "3.0.0".into())]))
        );
        assert_eq!(
            pkg.peer_dependencies,
            Some(BTreeMap::from([("e".into(), ">=4".into())]))
        );
        assert_eq!(
            pkg.resolutions,
            Some(BTreeMap::from([("f".into(), "5.0.0".into())]))
        );

        let build = pkg.scripts.get("build").expect("build script");
        assert_eq!(build.value, "turbo build");
        assert_eq!(build.range, Some(range_of(contents, "\"turbo build\"")));
        assert_eq!(pkg.scripts.get("test").unwrap().value, "jest --ci");

        let pnpm = pkg.pnpm.as_ref().expect("pnpm");
        assert_eq!(
            pnpm.patched_dependencies
                .as_ref()
                .unwrap()
                .get("g@1.0.0")
                .unwrap()
                .as_str(),
            "patches/g.patch"
        );
        assert_eq!(pnpm.other.get("overrides"), Some(&json!({"h": "6.0.0"})));

        assert_eq!(
            pkg.patched_dependencies
                .as_ref()
                .unwrap()
                .get("i@2.0.0")
                .unwrap()
                .as_str(),
            "patches/i.patch"
        );

        assert_eq!(pkg.other.get("private"), Some(&json!(true)));
        assert_eq!(pkg.other.get("workspaces"), Some(&json!(["packages/*"])));
        assert_eq!(
            pkg.other.get("exports"),
            Some(&json!({".": {"import": "./dist/index.mjs"}, "./sub": null}))
        );
    }

    #[test]
    fn test_metadata_attachment() {
        let contents = r#"{"name": "a", "packageManager": "pnpm@9.0.0", "devEngines": {}, "scripts": {"build": "turbo build"}}"#;
        let pkg = parse_ok(contents);
        for (label, text, path) in [
            (
                "packageManager",
                pkg.package_manager.as_ref().unwrap().text.as_deref(),
                pkg.package_manager.as_ref().unwrap().path.as_deref(),
            ),
            (
                "devEngines",
                pkg.dev_engines.as_ref().unwrap().text.as_deref(),
                pkg.dev_engines.as_ref().unwrap().path.as_deref(),
            ),
            (
                "script",
                pkg.scripts.get("build").unwrap().text.as_deref(),
                pkg.scripts.get("build").unwrap().path.as_deref(),
            ),
        ] {
            assert_eq!(text, Some(contents), "{label} text");
            assert_eq!(path, Some("pkg/package.json"), "{label} path");
        }
    }

    #[test]
    fn test_escapes_are_unescaped() {
        let contents = r#"{"name": "esc\u0041pe", "scripts": {"x": "echo \"hi\"\n\t\\"}}"#;
        let pkg = parse_ok(contents);
        assert_eq!(pkg.name.unwrap().value, "escApe");
        assert_eq!(pkg.scripts.get("x").unwrap().value, "echo \"hi\"\n\t\\");
    }

    #[test]
    fn test_escaped_keys_are_unescaped() {
        let pkg = parse_ok(r#"{"na\u006de": "k"}"#);
        assert_eq!(pkg.name.unwrap().value, "k");
    }

    #[test]
    fn test_duplicate_keys_last_win() {
        let pkg = parse_ok(r#"{"name": "first", "name": "second"}"#);
        assert_eq!(pkg.name.unwrap().value, "second");

        let pkg = parse_ok(r#"{"dependencies": {"x": "1", "x": "2"}}"#);
        assert_eq!(pkg.dependencies.unwrap()["x"], "2");

        let pkg = parse_ok(r#"{"scripts": {"b": "one", "b": "two"}}"#);
        assert_eq!(pkg.scripts["b"].value, "two");

        let pkg = parse_ok(r#"{"custom": 1, "custom": 2}"#);
        assert_eq!(pkg.other.get("custom"), Some(&json!(2)));
    }

    #[test]
    fn test_leading_bom_tolerated_and_ranges_stay_aligned() {
        let contents = "\u{feff}{\"name\": \"bom\"}";
        let pkg = parse_ok(contents);
        let name = pkg.name.expect("name");
        assert_eq!(name.value, "bom");
        // Ranges index the original text, BOM included, so snippet
        // extraction stays aligned.
        let range = name.range.expect("range");
        assert_eq!(&contents[range], "\"bom\"");
    }

    #[test]
    fn test_null_dev_engines_is_kept() {
        // `devEngines: null` is a declaration; package-manager resolution
        // reports it as invalid, pointing at this span.
        let contents = r#"{"devEngines": null}"#;
        let pkg = parse_ok(contents);
        let dev_engines = pkg.dev_engines.expect("devEngines");
        assert_eq!(dev_engines.value, serde_json::Value::Null);
        assert_eq!(dev_engines.range, Some(range_of(contents, "null")));
    }

    #[test_case("{}" ; "empty object")]
    #[test_case("  {  }  " ; "whitespace around empty object")]
    #[test_case("{\n  \"name\"\n  :\n  \"spread\"\n}\n" ; "whitespace everywhere")]
    #[test_case(r#"{"pnpm": {}}"# ; "empty pnpm")]
    #[test_case(r#"{"scripts": {}}"# ; "empty scripts")]
    #[test_case(r#"{"devEngines": "string"}"# ; "devEngines accepts any json")]
    #[test_case(r#"{"patchedDependencies": {"x": "../up.patch"}}"# ; "parent-relative patch path")]
    #[test_case(r#"{"custom": [1, -2.5, 0, 1e3, 2E-2, null, {"k": "v"}, []]}"# ; "numbers arrays and nesting in unknown fields")]
    fn test_accepts(contents: &str) {
        parse_ok(contents);
    }

    #[test]
    fn test_unknown_field_values_roundtrip() {
        let contents = r#"{"custom": [1, -2.5, 0, 1e3, 2E-2, null, {"k": "v"}, []]}"#;
        let pkg = parse_ok(contents);
        assert_eq!(
            pkg.other.get("custom"),
            Some(&json!([1, -2.5, 0, 1e3, 2E-2, null, {"k": "v"}, []]))
        );
    }

    #[test_case(r#"{"name": "a",}"# ; "trailing comma")]
    #[test_case(r#"{"name": "a"} // comment"# ; "line comment")]
    #[test_case(r#"{/* c */ "name": "a"}"# ; "block comment")]
    #[test_case(r#"{"name": 5}"# ; "name not a string")]
    #[test_case(r#"{"name": null}"# ; "name null")]
    #[test_case(r#"{"dependencies": null}"# ; "dependencies null")]
    #[test_case(r#"{"dependencies": {"x": 1}}"# ; "dependency version not a string")]
    #[test_case(r#"{"dependencies": ["x"]}"# ; "dependencies an array")]
    #[test_case(r#"{"scripts": "build"}"# ; "scripts not an object")]
    #[test_case(r#"{"scripts": {"b": 1}}"# ; "script not a string")]
    #[test_case(r#"{"pnpm": null}"# ; "pnpm null")]
    #[test_case(r#"{"pnpm": "config"}"# ; "pnpm not an object")]
    #[test_case(r#"{"patchedDependencies": {"x": "/abs/path"}}"# ; "absolute patch path")]
    #[test_case("not json" ; "not json")]
    #[test_case("" ; "empty input")]
    #[test_case("   " ; "whitespace only")]
    #[test_case("[]" ; "array root")]
    #[test_case("\"str\"" ; "string root")]
    #[test_case("{\"a\": 1} tail" ; "trailing garbage")]
    #[test_case(r#"{"a": "unterminated"# ; "unterminated string")]
    #[test_case("{\"name\": \"\n}" ; "unescaped newline in string")]
    #[test_case(r#"{"a": 01}"# ; "leading zero number")]
    #[test_case(r#"{"a": 1.}"# ; "bare decimal point")]
    #[test_case(r#"{"a": .5}"# ; "leading decimal point")]
    #[test_case(r#"{"a": +1}"# ; "plus sign number")]
    #[test_case(r#"{"a": 1e}"# ; "empty exponent")]
    #[test_case(r#"{"a": tru}"# ; "bad keyword")]
    #[test_case(r#"{"a": "bad \x escape"}"# ; "invalid escape")]
    #[test_case(r#"{"a": "bad \u00zz escape"}"# ; "invalid unicode escape")]
    #[test_case("{\"a\": \"ctrl \u{0001} char\"}" ; "unescaped control character")]
    #[test_case(r#"{"a": {"b": 1]}"# ; "mismatched delimiters")]
    #[test_case(r#"{"a" "b"}"# ; "missing colon")]
    #[test_case(r#"{"a": 1 "b": 2}"# ; "missing comma")]
    #[test_case(r#"{"exports": {"a": 1]}"# ; "mismatched delimiters in unknown field")]
    #[test_case(r#"{"exports": {"a": 1,}}"# ; "trailing comma in unknown field")]
    fn test_rejects(contents: &str) {
        assert!(
            parse(contents, "package.json").is_err(),
            "parser must reject: {contents}"
        );
    }

    #[test]
    fn test_type_errors_produce_diagnostics() {
        let err = parse(r#"{"name": "a", "dependencies": {"x": 1}}"#, "package.json").unwrap_err();
        let Error::Parse(diagnostics) = err else {
            panic!("expected a parse error");
        };
        assert!(!diagnostics.is_empty());
    }
}
