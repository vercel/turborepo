use std::{collections::HashMap, str::FromStr};

use biome_json_formatter::context::JsonFormatOptions;
use biome_json_parser::JsonParserOptions;
use itertools::Itertools as _;
use turborepo_errors::ParseDiagnostic;

use super::{BunLockfile, BunLockfileData, Error, LockfileVersion, PackageIndex};

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
        let format = biome_json_formatter::format_node(
            JsonFormatOptions::default()
                .with_trailing_commas(biome_json_formatter::context::TrailingCommas::None),
            &syntax_tree,
        )
        .map_err(Error::from)?;
        let strict_json = format.print().map_err(Error::from)?;
        let data: BunLockfileData = serde_json::from_str(strict_json.as_code())?;

        // Validate that we support this lockfile version
        let _version = LockfileVersion::from_i32(data.lockfile_version)
            .ok_or(crate::Error::UnsupportedBunVersion(data.lockfile_version))?;

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
