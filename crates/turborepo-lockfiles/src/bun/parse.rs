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

impl BunLockfile {
    pub fn from_bytes(input: &[u8]) -> Result<Self, crate::Error> {
        let s = std::str::from_utf8(input).map_err(Error::from)?;
        Self::from_str(s)
    }
}

impl FromStr for BunLockfile {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
        let data: BunLockfileData =
            serde_json::from_str(strict_json.strip_prefix('\u{feff}').unwrap_or(&strict_json))?;

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

        // Build key_to_entry map
        // When there are multiple lockfile keys with the same ident (e.g., nested
        // versions), we pick the FIRST one in sorted order for determinism.
        // Sort keys to ensure deterministic selection: workspace-specific entries (with
        // /) come before hoisted entries (without /) in the sort order.
        let mut sorted_keys: Vec<_> = data.packages.keys().collect();
        sorted_keys.sort();

        let mut key_to_entry: HashMap<String, String> = HashMap::with_capacity(data.packages.len());
        for path in sorted_keys {
            let Some(info) = data.packages.get(path) else {
                continue;
            };

            if let Some(prev_path) = key_to_entry.get(&info.ident) {
                let Some(prev_info) = data.packages.get(prev_path) else {
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
        // Build package index
        let index = PackageIndex::new(&data.packages);

        Ok(Self {
            data,
            key_to_entry,
            index,
        })
    }
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
