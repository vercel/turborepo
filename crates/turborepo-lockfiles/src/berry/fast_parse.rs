//! Fast-path parser for Berry (yarn 2+) lockfiles.
//!
//! Berry lockfiles are machine-generated, line-oriented YAML with a rigid
//! shape: double-quoted descriptor keys, one entry body indent level, at
//! most two levels of nested string/bool maps, and plain scalars for the
//! remaining fields. `serde_yaml_ng` routes all of it through a general
//! YAML scanner; this module parses the subset directly into the
//! `Map<String, Entry>` the existing `TryFrom<Map<String, Entry>>` for
//! [`LockfileData`](super::super::LockfileData) consumes, so the semantic
//! layer (metadata split, resolution requirement) is shared.
//!
//! Correctness strategy mirrors the pnpm and yarn v1 fast parsers:
//! anything outside the modeled subset returns `None` and the caller falls
//! back to the serde path — including scalar forms whose YAML typing could
//! diverge from raw text (floats, exotic radixes, booleans and nulls in
//! string positions), so the fast path never fabricates a value the parser
//! of record would type differently or reject. Differential tests enforce
//! agreement on everything the fast path accepts.

use std::collections::BTreeMap;

use super::{DependencyMeta, Entry};

/// Attempt to parse a Berry lockfile into raw entries. `None` means the
/// input is outside the fast subset and the serde path must decide.
pub(crate) fn parse(input: &str) -> Option<BTreeMap<String, Entry>> {
    let mut parser = Parser {
        rest: input,
        line: None,
    };
    let mut entries = BTreeMap::new();
    while let Some((indent, _)) = parser.peek_line()? {
        if indent != 0 {
            return None;
        }
        let (key, entry) = parser.parse_entry()?;
        entries.insert(key, entry);
    }
    Some(entries)
}

struct Parser<'a> {
    rest: &'a str,
    /// One-line lookahead: (indent, content).
    line: Option<(usize, &'a str)>,
}

impl<'a> Parser<'a> {
    fn parse_entry(&mut self) -> Option<(String, Entry)> {
        let (_, content) = self.take_line()??;
        let (key, rest) = scan_key(content)?;
        if rest != ":" {
            return None;
        }

        let mut entry = Entry::default();
        let mut version = None;
        while let Some((indent, _)) = self.peek_line()? {
            if indent == 0 {
                break;
            }
            if indent != 2 {
                return None;
            }
            let (_, content) = self.take_line()??;
            let (field, rest) = scan_key(content)?;
            let rest = rest.strip_prefix(':')?;
            let value_text = rest.strip_prefix(' ').unwrap_or(rest);

            if value_text.is_empty() {
                // Nested block.
                match field.as_str() {
                    "dependencies" => entry.dependencies = Some(self.parse_string_map(4)?),
                    "peerDependencies" => entry.peer_dependencies = Some(self.parse_string_map(4)?),
                    "bin" => entry.bin = Some(self.parse_string_map(4)?),
                    "dependenciesMeta" => entry.dependencies_meta = Some(self.parse_meta_map()?),
                    "peerDependenciesMeta" => {
                        entry.peer_dependencies_meta = Some(self.parse_meta_map()?)
                    }
                    // Unknown nested blocks would be ignored by serde, but
                    // validating their shape isn't worth it; bail.
                    _ => return None,
                }
                continue;
            }

            let value = scan_string_value(value_text)?;
            match field.as_str() {
                "version" => version = Some(value),
                "languageName" => entry.language_name = Some(value),
                "linkType" => entry.link_type = Some(value),
                "resolution" => entry.resolution = Some(value),
                "checksum" => entry.checksum = Some(value),
                "conditions" => entry.conditions = Some(value),
                "cacheKey" => entry.cache_key = Some(value),
                // Unknown scalar fields are ignored, as serde does.
                _ => {}
            }
        }

        // `version` is the only required Entry field; missing means the
        // serde path reports the error.
        entry.version = version?;
        Some((key, entry))
    }

    /// `name: value` scalar pairs at the given indent.
    fn parse_string_map(&mut self, indent_want: usize) -> Option<BTreeMap<String, String>> {
        let mut map = BTreeMap::new();
        while let Some((indent, _)) = self.peek_line()? {
            if indent < indent_want {
                break;
            }
            if indent != indent_want {
                return None;
            }
            let (_, content) = self.take_line()??;
            let (key, rest) = scan_key(content)?;
            let value_text = rest.strip_prefix(':')?.strip_prefix(' ')?;
            if value_text.is_empty() {
                return None;
            }
            map.insert(key, scan_string_value(value_text)?);
        }
        if map.is_empty() {
            // An empty block means the header's value is YAML null, which
            // the serde path types as `None`/an error, not an empty map.
            return None;
        }
        Some(map)
    }

    /// `name:` blocks of boolean fields (dependenciesMeta shape).
    fn parse_meta_map(&mut self) -> Option<BTreeMap<String, DependencyMeta>> {
        let mut map = BTreeMap::new();
        while let Some((indent, _)) = self.peek_line()? {
            if indent < 4 {
                break;
            }
            if indent != 4 {
                return None;
            }
            let (_, content) = self.take_line()??;
            let (key, rest) = scan_key(content)?;
            if rest != ":" {
                return None;
            }
            let mut meta = DependencyMeta::default();
            let mut saw_field = false;
            while let Some((indent, _)) = self.peek_line()? {
                if indent < 6 {
                    break;
                }
                if indent != 6 {
                    return None;
                }
                let (_, content) = self.take_line()??;
                let (field, rest) = scan_key(content)?;
                let value = match rest.strip_prefix(':')?.strip_prefix(' ')? {
                    "true" => true,
                    "false" => false,
                    _ => return None,
                };
                match field.as_str() {
                    "optional" => meta.optional = Some(value),
                    "unplugged" => meta.unplugged = Some(value),
                    "built" => meta.built = Some(value),
                    _ => return None,
                }
                saw_field = true;
            }
            if !saw_field {
                // Empty block: the key's value is YAML null, which serde
                // rejects for the `DependencyMeta` struct.
                return None;
            }
            map.insert(key, meta);
        }
        if map.is_empty() {
            // See `parse_string_map`: an empty block is YAML null.
            return None;
        }
        Some(map)
    }

    /// Next content line as (indent, content), skipping blanks and
    /// comments. Outer `None` = bail, inner `None` = end of input.
    #[allow(clippy::option_option)]
    fn peek_line(&mut self) -> Option<Option<(usize, &'a str)>> {
        if self.line.is_some() {
            return Some(self.line);
        }
        loop {
            if self.rest.is_empty() {
                return Some(None);
            }
            let (line, rest) = match memchr::memchr(b'\n', self.rest.as_bytes()) {
                Some(i) => (&self.rest[..i], &self.rest[i + 1..]),
                None => (self.rest, ""),
            };
            self.rest = rest;
            if line.contains('\r') || line.contains('\t') {
                return None;
            }
            let indent = line.len() - line.trim_start_matches(' ').len();
            let content = &line[indent..];
            if content.is_empty() {
                continue;
            }
            if content.starts_with('#') {
                continue;
            }
            if content.ends_with(' ') {
                // Trailing whitespace interacts with YAML scalar rules;
                // machine-generated files never have it.
                return None;
            }
            self.line = Some((indent, content));
            return Some(self.line);
        }
    }

    fn take_line(&mut self) -> Option<Option<(usize, &'a str)>> {
        let line = self.peek_line()?;
        self.line = None;
        Some(line)
    }
}

/// A map key: double-quoted (no escapes in the subset) or a plain scalar
/// ending at the first colon. Returns the key and the rest starting at the
/// colon.
fn scan_key(s: &str) -> Option<(String, &str)> {
    if let Some(inner) = s.strip_prefix('"') {
        let end = memchr::memchr2(b'"', b'\\', inner.as_bytes())?;
        if inner.as_bytes()[end] == b'\\' {
            return None;
        }
        return Some((inner[..end].to_string(), &inner[end + 1..]));
    }
    let end = memchr::memchr(b':', s.as_bytes())?;
    let key = &s[..end];
    plain_scalar_ok(key)?;
    Some((key.to_string(), &s[end..]))
}

/// A scalar in a `String` position occupying the rest of the line.
fn scan_string_value(s: &str) -> Option<String> {
    if let Some(inner) = s.strip_prefix('"') {
        let end = memchr::memchr2(b'"', b'\\', inner.as_bytes())?;
        if inner.as_bytes()[end] == b'\\' || !inner[end + 1..].is_empty() {
            return None;
        }
        return Some(inner[..end].to_string());
    }
    plain_scalar_ok(s)?;
    Some(s.to_string())
}

/// Accept a plain scalar only when its raw text is exactly what
/// `serde_yaml_ng` produces for a `String` field: no YAML typing surprises
/// (bools and nulls error, floats and exotic radixes may normalize), no
/// comment/flow/indicator ambiguity. Integers without leading zeros
/// stringify as their raw text, so they pass (`__metadata.version: 8`).
fn plain_scalar_ok(s: &str) -> Option<()> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    if !bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        return None;
    }
    match bytes[0] {
        b'!' | b'&' | b'*' | b'|' | b'>' | b'%' | b'@' | b'`' | b'\'' | b'"' | b'{' | b'}'
        | b'[' | b']' | b',' | b'#' | b'?' => return None,
        // `~` is null only as the entire scalar; `~16` (semver range) is a
        // plain string.
        b'~' if bytes.len() == 1 => return None,
        b'-' | b':' if bytes.len() == 1 || bytes[1] == b' ' => return None,
        _ => {}
    }
    if s.contains(": ") || s.ends_with(':') || s.contains(" #") {
        return None;
    }
    if matches!(
        s,
        "null" | "Null" | "NULL" | "true" | "True" | "TRUE" | "false" | "False" | "FALSE"
    ) {
        return None;
    }
    // Scalars YAML types as numbers can normalize away from their raw
    // text; only those bail. Anything non-numeric is a string whose raw
    // text is preserved.
    let core = s.strip_prefix('-').unwrap_or(s);
    if s.starts_with('+')
        || core.starts_with("0x")
        || core.starts_with("0o")
        || core.starts_with("0b")
    {
        return None;
    }
    if matches!(
        s,
        ".inf" | ".Inf" | ".INF" | "-.inf" | "-.Inf" | "-.INF" | ".nan" | ".NaN" | ".NAN"
    ) {
        return None;
    }
    let all_digits = !core.is_empty() && core.bytes().all(|b| b.is_ascii_digit());
    if all_digits {
        // Ints without leading zeros round-trip as raw text; leading-zero
        // forms are strings under YAML 1.2. 64-bit overflow could take a
        // different path, so huge ints bail.
        if core.len() > 18 && !core.starts_with('0') {
            return None;
        }
        return Some(());
    }
    // Float-like forms (`1.5`, `1e3`, `.5`) may normalize. Rust's float
    // parser also accepts words like `nan` and `inf` that YAML types as
    // strings (the npm package `nan` is a real dependency key), so only
    // digit- or dot-led scalars are candidates.
    let yaml_numeric_lead = core.starts_with(|c: char| c.is_ascii_digit()) || core.starts_with('.');
    if yaml_numeric_lead && s.parse::<f64>().is_ok() {
        return None;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::berry::LockfileData;

    /// The fast path must accept the input and produce the same
    /// `LockfileData` as the serde path.
    fn assert_fast_matches_serde(input: &str) {
        let fast = parse(input).expect("fast path must accept this input");
        let fast_data = LockfileData::try_from(fast).expect("fast entries convert");
        let serde_data: LockfileData =
            serde_yaml_ng::from_str(input).expect("serde path must accept this input");
        assert_eq!(fast_data.metadata, serde_data.metadata);
        assert_eq!(fast_data.packages, serde_data.packages);
    }

    fn assert_falls_back(input: &str) {
        assert!(
            parse(input).is_none(),
            "input should be outside the fast subset"
        );
    }

    const HEADER: &str = "# This file is generated by running \"yarn install\".\n\n__metadata:\n  \
                          version: 8\n  cacheKey: 10c0\n\n";

    #[test]
    fn test_basic_lockfile_differential() {
        assert_fast_matches_serde(&format!(
            "{HEADER}\"@scope/pkg@npm:^1.0.0, @scope/pkg@npm:^1.2.0\":\n  version: 1.2.6\n  \
             resolution: \"@scope/pkg@npm:1.2.6\"\n  dependencies:\n    lodash: \
             \"npm:^4.17.21\"\n    nan: \"npm:^2.13.2\"\n  checksum: 10c0/e93016ab\n  \
             languageName: node\n  linkType: hard\n"
        ));
    }

    #[test]
    fn test_plain_scalar_typing_differential() {
        // Semver ranges and version-ish strings that flirt with YAML
        // numeric/null typing but are strings: `~16`, `3 || 4 || 5`,
        // leading-zero ints, package name `nan`.
        assert_fast_matches_serde(&format!(
            "{HEADER}\"a@npm:^1\":\n  version: 1.2.3\n  resolution: \"a@npm:1.2.3\"\n  \
             peerDependencies:\n    react: ~16\n    webpack: 3 || 4 || 5\n    nan: 2\n    zero: \
             012\n  languageName: node\n  linkType: hard\n"
        ));
    }

    #[test]
    fn test_meta_maps_and_bin_differential() {
        assert_fast_matches_serde(&format!(
            "{HEADER}\"b@npm:^2\":\n  version: 2.0.0\n  resolution: \"b@npm:2.0.0\"\n  \
             dependenciesMeta:\n    fsevents:\n      optional: true\n      built: false\n  \
             peerDependenciesMeta:\n    react:\n      optional: true\n  bin:\n    cmd: \
             ./bin/cmd.js\n  conditions: os=darwin & cpu=x64\n  languageName: node\n  linkType: \
             hard\n"
        ));
    }

    #[test]
    fn test_patch_protocol_keys_differential() {
        assert_fast_matches_serde(&format!(
            "{HEADER}\"c@patch:c@npm%3A1.0.0#~/.yarn/patches/c-npm-1.0.0-abc.patch::version=1.0.0&\
             hash=cafe\":\n  version: 1.0.0\n  resolution: \
             \"c@patch:c@npm%3A1.0.0#~/.yarn/patches/c-npm-1.0.0-abc.patch::version=1.0.0&\
             hash=cafe\"\n  languageName: node\n  linkType: hard\n"
        ));
    }

    #[test]
    fn test_bails_outside_subset() {
        // Typed scalars in string positions.
        assert_falls_back(&format!(
            "{HEADER}\"a@npm:^1\":\n  version: 1.5\n  resolution: \"a@npm:1.5\"\n"
        ));
        assert_falls_back(&format!(
            "{HEADER}\"a@npm:^1\":\n  version: true\n  resolution: \"a@npm:x\"\n"
        ));
        // Escapes in quoted strings.
        assert_falls_back(&format!(
            "{HEADER}\"a\\\"b@npm:^1\":\n  version: 1.0.0\n  resolution: \"x\"\n"
        ));
        // Unknown nested blocks.
        assert_falls_back(&format!(
            "{HEADER}\"a@npm:^1\":\n  version: 1.0.0\n  resolution: \"x\"\n  unknownBlock:\n    \
             key: value\n"
        ));
        // Empty nested blocks are YAML null, not empty maps: the serde
        // path yields `None` for map fields and errors for meta structs.
        assert_falls_back(&format!(
            "{HEADER}\"a@npm:^1\":\n  version: 1.0.0\n  resolution: \"x\"\n  dependencies:\n  \
             languageName: node\n"
        ));
        assert_falls_back(&format!(
            "{HEADER}\"a@npm:^1\":\n  version: 1.0.0\n  resolution: \"x\"\n  dependencies:\n"
        ));
        assert_falls_back(&format!(
            "{HEADER}\"a@npm:^1\":\n  version: 1.0.0\n  resolution: \"x\"\n  dependenciesMeta:\n  \
             languageName: node\n"
        ));
        assert_falls_back(&format!(
            "{HEADER}\"a@npm:^1\":\n  version: 1.0.0\n  resolution: \"x\"\n  \
             peerDependenciesMeta:\n    react:\n  languageName: node\n"
        ));
        // CRLF input.
        assert_falls_back("__metadata:\r\n  version: 8\r\n");
    }

    #[test]
    fn test_semantic_errors_use_serde_path() {
        // Structurally fine but semantically wrong (missing metadata /
        // missing resolution): the fast path parses the entries, but
        // `from_bytes` must fall back so the serde path reports its own
        // error.
        let missing_metadata = "\"a@npm:^1\":\n  version: 1.0.0\n  resolution: \"x\"\n";
        assert!(parse(missing_metadata).is_some());
        assert!(LockfileData::from_bytes(missing_metadata.as_bytes()).is_err());

        let missing_resolution = &format!("{HEADER}\"a@npm:^1\":\n  version: 1.0.0\n");
        assert!(parse(missing_resolution).is_some());
        assert!(LockfileData::from_bytes(missing_resolution.as_bytes()).is_err());
    }
}
