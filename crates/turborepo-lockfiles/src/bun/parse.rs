use std::{borrow::Cow, collections::HashMap, str::FromStr};

use biome_json_parser::JsonParserOptions;
use biome_json_syntax::{JsonSyntaxKind, JsonSyntaxNode};
use itertools::Itertools as _;
use turborepo_errors::ParseDiagnostic;

use super::{BunLockfile, BunLockfileData, Error, LockfileVersion, PackageIndex};

// Biome has already validated the document, including the positions of trailing
// commas. Keep the original bytes (and avoid an allocation for strict JSON)
// while omitting only comma tokens immediately before a closing array or
// object.
fn strip_trailing_commas<'a>(input: &'a str, syntax: &JsonSyntaxNode) -> Cow<'a, str> {
    let mut output: Option<String> = None;
    let mut last_end = 0;
    let mut token = syntax.first_token();

    while let Some(current) = token {
        let next = current.next_token();
        if current.kind() == JsonSyntaxKind::COMMA
            && next.as_ref().is_some_and(|next| {
                matches!(
                    next.kind(),
                    JsonSyntaxKind::R_BRACK | JsonSyntaxKind::R_CURLY
                )
            })
        {
            let range = current.text_trimmed_range();
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            output
                .get_or_insert_with(|| String::with_capacity(input.len()))
                .push_str(&input[last_end..start]);
            last_end = end;
        }
        token = next;
    }

    if let Some(mut output) = output {
        output.push_str(&input[last_end..]);
        Cow::Owned(output)
    } else {
        Cow::Borrowed(input)
    }
}

/// Removes the same trailing commas as [`strip_trailing_commas`] without
/// building a syntax tree: a comma is dropped only when the previous
/// significant token ends a value and the next one closes an array or
/// object. On any input biome accepts, that is exactly the set of comma
/// tokens biome's tree places before `]`/`}`, so the output is identical.
/// Inputs biome would reject are never fixed up into valid JSON by this
/// (e.g. `[,]` keeps its comma), so the caller can treat a strict-JSON parse
/// failure of the output as "use the biome path". Returns `None` for an
/// unterminated string.
fn strip_trailing_commas_fast(input: &str) -> Option<Cow<'_, str>> {
    let bytes = input.as_bytes();
    let mut removed: Vec<usize> = Vec::new();
    // Whether the last significant token ended a value (scalar, string,
    // `]` or `}`), and the offset of a comma that directly followed one.
    let mut after_value = false;
    let mut candidate: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => {}
            b'"' => {
                i += 1;
                loop {
                    match memchr::memchr2(b'"', b'\\', bytes.get(i..)?) {
                        Some(offset) if bytes[i + offset] == b'\\' => i += offset + 2,
                        Some(offset) => {
                            i += offset;
                            break;
                        }
                        None => return None,
                    }
                }
                candidate = None;
                after_value = true;
            }
            b',' => {
                candidate = after_value.then_some(i);
                after_value = false;
            }
            b']' | b'}' => {
                removed.extend(candidate.take());
                after_value = true;
            }
            b'[' | b'{' | b':' => {
                candidate = None;
                after_value = false;
            }
            _ => {
                candidate = None;
                after_value = true;
            }
        }
        i += 1;
    }
    if removed.is_empty() {
        return Some(Cow::Borrowed(input));
    }
    let mut output = String::with_capacity(input.len());
    let mut last_end = 0;
    for comma in removed {
        output.push_str(&input[last_end..comma]);
        last_end = comma + 1;
    }
    output.push_str(&input[last_end..]);
    Some(Cow::Owned(output))
}

impl BunLockfile {
    pub fn from_bytes(input: &[u8]) -> Result<Self, crate::Error> {
        let s = std::str::from_utf8(input).map_err(Error::from)?;
        Self::from_str(s)
    }

    fn parse_data_with_biome(s: &str) -> Result<BunLockfileData, crate::Error> {
        let parsed_json = biome_json_parser::parse_json(
            s,
            JsonParserOptions::default().with_allow_trailing_commas(),
        );
        if parsed_json.has_errors() {
            let diags = parsed_json
                .into_diagnostics()
                .into_iter()
                .map(|diagnostic| ParseDiagnostic::from(&diagnostic).to_string())
                .join("\n");
            return Err(crate::Error::BiomeJsonError(diags));
        }
        let syntax_tree = parsed_json.syntax();
        let strict_json = strip_trailing_commas(s, &syntax_tree);
        Ok(serde_json::from_str(
            strict_json.strip_prefix('\u{feff}').unwrap_or(&strict_json),
        )?)
    }
}

impl FromStr for BunLockfile {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Fast path: strip trailing commas lexically and parse as strict JSON,
        // skipping biome's syntax tree. Anything the strict parse rejects goes
        // through the biome path, which owns validation and error reporting.
        let fast = strip_trailing_commas_fast(s.strip_prefix('\u{feff}').unwrap_or(s))
            .and_then(|strict_json| serde_json::from_str::<BunLockfileData>(&strict_json).ok());
        let data = match fast {
            Some(data) => data,
            None => Self::parse_data_with_biome(s)?,
        };

        if LockfileVersion::from_i32(data.lockfile_version).is_none() {
            if data.lockfile_version < LockfileVersion::LATEST.as_i32() {
                return Err(crate::Error::UnsupportedBunVersion(data.lockfile_version));
            }
            // Bun lockfile revisions have only ever added to the schema, and the
            // deserialization above already succeeded, so treat a newer version
            // like the latest one we know instead of discarding the lockfile.
            tracing::warn!(
                "bun.lock has lockfileVersion {}, newer than the latest supported version {}; \
                 treating it as version {}",
                data.lockfile_version,
                LockfileVersion::LATEST.as_i32(),
                LockfileVersion::LATEST.as_i32()
            );
        }

        // The ident map and the package index read `packages` independently.
        let (key_to_entry, index) = rayon::join(
            || build_key_to_entry(&data.packages),
            || PackageIndex::new(&data.packages),
        );
        let key_to_entry = key_to_entry?;

        Ok(Self {
            data,
            key_to_entry,
            index,
        })
    }
}

fn build_key_to_entry(
    packages: &super::Map<String, super::PackageEntry>,
) -> Result<HashMap<String, String>, crate::Error> {
    // Build key_to_entry map
    // When there are multiple lockfile keys with the same ident (e.g., nested
    // versions), we pick the FIRST one in sorted order for determinism.
    // Sort keys to ensure deterministic selection: workspace-specific entries (with
    // /) come before hoisted entries (without /) in the sort order.
    let mut sorted_keys: Vec<_> = packages.keys().collect();
    sorted_keys.sort();

    let mut key_to_entry: HashMap<String, String> = HashMap::with_capacity(packages.len());
    for path in sorted_keys {
        let Some(info) = packages.get(path) else {
            continue;
        };

        if let Some(prev_path) = key_to_entry.get(&info.ident) {
            let Some(prev_info) = packages.get(prev_path) else {
                continue;
            };

            // Verify checksums match for duplicate idents
            if prev_info.checksum != info.checksum {
                return Err(Error::MismatchedShas {
                    ident: info.ident.clone(),
                    sha1: prev_info.checksum.clone().unwrap_or_default(),
                    sha2: info.checksum.clone().unwrap_or_default(),
                }
                .into());
            }
            // Skip this entry - we already have one for this ident
        } else {
            // First time seeing this ident
            key_to_entry.insert(info.ident.clone(), path.clone());
        }
    }
    Ok(key_to_entry)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use biome_json_parser::JsonParserOptions;

    use super::{BunLockfile, strip_trailing_commas};

    #[test]
    fn removes_only_trailing_comma_tokens() {
        let input = r#"{"key,]":"value,}", "nested": [1, {"x": "a,b",},],}"#;
        let parsed = biome_json_parser::parse_json(
            input,
            JsonParserOptions::default().with_allow_trailing_commas(),
        );
        assert!(!parsed.has_errors());
        assert_eq!(
            strip_trailing_commas(input, &parsed.syntax()),
            r#"{"key,]":"value,}", "nested": [1, {"x": "a,b"}]}"#
        );
    }

    #[test]
    fn borrows_json_without_trailing_commas() {
        let input = r#"{"lockfileVersion":1,"workspaces":{},"packages":{}}"#;
        let parsed = biome_json_parser::parse_json(input, JsonParserOptions::default());
        assert!(matches!(
            strip_trailing_commas(input, &parsed.syntax()),
            Cow::Borrowed(_)
        ));
        assert!(BunLockfile::from_bytes(input.as_bytes()).is_ok());
    }

    #[test]
    fn parses_bun_lockfile_with_trailing_commas() {
        let input = "{\r\n\"lockfileVersion\":1,\r\n\"workspaces\":{\"\":{\"name\":\"team,}\",},},\
                     \r\n\"packages\":{},\r\n}";
        assert!(BunLockfile::from_bytes(input.as_bytes()).is_ok());
    }

    fn assert_fast_matches_biome(input: &str) {
        let parsed = biome_json_parser::parse_json(
            input,
            JsonParserOptions::default().with_allow_trailing_commas(),
        );
        assert!(!parsed.has_errors(), "biome must accept: {input}");
        assert_eq!(
            super::strip_trailing_commas_fast(input).expect("lexes"),
            strip_trailing_commas(input, &parsed.syntax()),
        );
    }

    #[test]
    fn fast_comma_stripping_matches_biome() {
        for input in [
            r#"{"key,]":"value,}", "nested": [1, {"x": "a,b",},],}"#,
            "{\"esc\\\"aped,]\": \"back\\\\slash\\\\\",  \"arr\": [true, false, null, -1.5e3 ,\n  \
             ] ,\n}",
            r#"{"lockfileVersion":1,"workspaces":{},"packages":{}}"#,
            "{\r\n\"a\":[1,\r\n],\r\n\"b\":{\"c\":[],},\r\n}",
            r#"{"u": "\u00e9,]", "e": [{}, [], "",],}"#,
        ] {
            assert_fast_matches_biome(input);
        }
    }

    #[test]
    fn fast_comma_stripping_matches_biome_on_fixtures() {
        let fixtures =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lockfile-tests/fixtures");
        let mut checked = 0;
        for entry in std::fs::read_dir(&fixtures).expect("fixtures dir") {
            let path = entry.expect("entry").path().join("bun.lock");
            if let Ok(contents) = std::fs::read_to_string(&path) {
                assert_fast_matches_biome(contents.strip_prefix('\u{feff}').unwrap_or(&contents));
                assert!(BunLockfile::from_bytes(contents.as_bytes()).is_ok());
                checked += 1;
            }
        }
        assert!(checked > 0, "expected bun.lock fixtures");
    }

    #[test]
    fn fast_comma_stripping_never_repairs_invalid_json() {
        // Commas biome rejects (not preceded by a value) are kept, so the
        // strict parse fails and the biome path reports the error.
        for invalid in [r#"[,]"#, r#"{,}"#, r#"[1,,]"#, r#"{"a":,}"#, r#"{"a":1},"#] {
            let stripped = super::strip_trailing_commas_fast(invalid).expect("lexes");
            assert!(
                serde_json::from_str::<serde_json::Value>(&stripped).is_err(),
                "{invalid} must stay invalid"
            );
        }
        assert!(super::strip_trailing_commas_fast(r#"{"unterminated"#).is_none());
        assert!(super::strip_trailing_commas_fast("\"trailing backslash\\").is_none());
    }

    #[test]
    fn rejects_invalid_json_before_removing_commas() {
        for invalid in [
            r#"{"lockfileVersion":1,"workspaces":{},"packages":{},// comment
}"#,
            r#"{"lockfileVersion":1,"workspaces":{},"packages":[,]}"#,
        ] {
            assert!(matches!(
                BunLockfile::from_bytes(invalid.as_bytes()),
                Err(crate::Error::BiomeJsonError(_))
            ));
        }
    }
}
