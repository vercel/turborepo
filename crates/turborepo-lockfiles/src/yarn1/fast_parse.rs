//! Fast-path parser for yarn v1 lockfiles.
//!
//! The nom-based `de::parse_syml` builds a full `serde_json::Value` tree
//! (cloning every entry per key in multi-key lines and regex-trimming every
//! unquoted token) which `serde_json::from_value` then re-walks into
//! [`Entry`] structs. This module parses the machine-generated yarn v1
//! subset directly into the target map in a single pass.
//!
//! Correctness strategy mirrors the pnpm fast parsers: anything outside the
//! modeled subset returns `None` and the caller falls back to the nom +
//! serde path — including inputs that path would *reject*, so the fast
//! path never fabricates a lockfile from input the parser of record treats
//! as an error. Differential tests enforce agreement on everything the
//! fast path accepts.

use super::{Entry, Map};

/// Attempt to parse a yarn v1 lockfile. `None` means "outside the fast
/// subset": the caller must use the nom parser, which either parses it or
/// reports the proper error.
pub(super) fn parse(input: &str) -> Option<Map<String, Entry>> {
    let mut parser = Parser {
        rest: input,
        line: None,
    };
    parser.parse_document()
}

struct Parser<'a> {
    rest: &'a str,
    /// One-line lookahead: (indent, content).
    line: Option<(usize, &'a str)>,
}

/// Characters that cannot begin an unquoted (pseudo)string.
const PSEUDO_FIRST_EXCLUDED: &[u8] = b"\r\n\t ?:,][{}#&*!|>'\"%@`-";
/// Characters that end an unquoted (pseudo)string anywhere.
const PSEUDO_TAIL_EXCLUDED: &[u8] = b"\r\n\t,][{}:#\"'";

impl<'a> Parser<'a> {
    fn parse_document(&mut self) -> Option<Map<String, Entry>> {
        let mut entries = Map::new();
        while let Some((indent, _)) = self.peek_line()? {
            if indent != 0 {
                return None;
            }
            self.parse_top_entry(&mut entries)?;
        }
        Some(entries)
    }

    /// One top-level `key(, key)*:` line plus its indented body.
    fn parse_top_entry(&mut self, entries: &mut Map<String, Entry>) -> Option<()> {
        let (_, content) = self.take_line()??;
        let keys = scan_entry_keys(content)?;

        let entry = self.parse_entry_body()?;
        let (last, firsts) = keys.split_last()?;
        for key in firsts {
            // Multi-key lines assign the same entry to every key,
            // mirroring the value clone in the nom path.
            entries.insert(key.clone(), entry.clone());
        }
        entries.insert(last.clone(), entry);
        Some(())
    }

    /// Fields at indent 2 until the next top-level line.
    fn parse_entry_body(&mut self) -> Option<Entry> {
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
            match parse_pair_line(content)? {
                Pair::Nested(key) => {
                    // Nested map (dependencies / optionalDependencies /
                    // unknown blocks serde would ignore).
                    let map = self.parse_scalar_map(4)?;
                    match key.as_str() {
                        "dependencies" => entry.dependencies = Some(map),
                        "optionalDependencies" => entry.optional_dependencies = Some(map),
                        _ => {}
                    }
                }
                Pair::Scalar(key, value) => assign_field(&mut entry, &mut version, &key, value),
            }
        }
        entry.version = version?;
        Some(entry)
    }

    /// A block of `key value` / `key: value` scalar pairs at the given
    /// indent. Deeper nesting and arrays leave the fast subset.
    fn parse_scalar_map(&mut self, indent_want: usize) -> Option<Map<String, String>> {
        let mut map = Map::new();
        while let Some((indent, content)) = self.peek_line()? {
            if indent < indent_want {
                break;
            }
            if indent != indent_want || content.starts_with('-') {
                return None;
            }
            let (_, content) = self.take_line()??;
            match parse_pair_line(content)? {
                Pair::Scalar(key, value) => {
                    map.insert(key, value);
                }
                Pair::Nested(_) => return None,
            }
        }
        Some(map)
    }

    /// Next content line as (indent, content), skipping blank lines and
    /// comments. `None` in the outer option means "bail"; the inner `None`
    /// is end of input.
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
                // CRLF input is valid for the nom parser; keep the fast
                // subset LF-only.
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

fn assign_field(entry: &mut Entry, version: &mut Option<String>, key: &str, value: String) {
    match key {
        "name" => entry.name = Some(value),
        "version" => *version = Some(value),
        "uid" => entry.uid = Some(value),
        "resolved" => entry.resolved = Some(value),
        "integrity" => entry.integrity = Some(value),
        "registry" => entry.registry = Some(value),
        // Unknown scalar fields are ignored, as serde does.
        _ => {}
    }
}

enum Pair {
    /// `key:` opening an indented block.
    Nested(String),
    /// `key: value` or legacy `key value`.
    Scalar(String, String),
}

/// One `key`/`value` line, mirroring the nom alternation: greedy
/// (pseudo)string key with `:` first; on failure the legacy branch —
/// single restricted token, then either `:` or a space-separated legacy
/// literal.
fn parse_pair_line(content: &str) -> Option<Pair> {
    // Branch 1: quoted or pseudostring key followed by (spaced) colon. The
    // pseudostring is greedy across internal spaces, so `version "1"` falls
    // through to the legacy branch just like the nom parser.
    if let Some((key, rest)) = scan_key(content) {
        let rest = rest.trim_start_matches(' ');
        if let Some(after_colon) = rest.strip_prefix(':') {
            let after_colon = after_colon.trim_start_matches(' ');
            if after_colon.is_empty() {
                return Some(Pair::Nested(key));
            }
            return Some(Pair::Scalar(key, scan_colon_value(after_colon)?));
        }
    }
    // Branch 2/3: legacy token key.
    let (key, rest) = scan_legacy_token(content)?;
    let rest_trimmed = rest.trim_start_matches(' ');
    if let Some(after_colon) = rest_trimmed.strip_prefix(':') {
        let after_colon = after_colon.trim_start_matches(' ');
        if after_colon.is_empty() {
            return Some(Pair::Nested(key));
        }
        return Some(Pair::Scalar(key, scan_colon_value(after_colon)?));
    }
    if rest.starts_with(' ') && !rest_trimmed.is_empty() {
        return Some(Pair::Scalar(key, scan_legacy_value(rest_trimmed)?));
    }
    None
}

/// A key: quoted string or unquoted pseudostring (trimmed). Returns the
/// key and the unconsumed remainder.
fn scan_key(s: &str) -> Option<(String, &str)> {
    if s.as_bytes().first() == Some(&b'"') {
        return scan_quoted(s);
    }
    scan_pseudostring(s)
}

/// The keys of a top-level entry line, which must end with `:` and no
/// inline value. Mirrors the nom alternation: a single key may be any
/// (pseudo)string, but the multi-key branch admits only `legacy_name`
/// keys — so `_a@1, b@2:` is an error there (the bare `_a@1` token is not
/// a legal legacy name) and must not parse here either.
fn scan_entry_keys(content: &str) -> Option<Vec<String>> {
    let ends_clean = |rest: &str| -> Option<()> {
        let rest = rest.trim_start_matches(' ').strip_prefix(':')?;
        // Inline values for whole entries don't appear in yarn v1
        // lockfiles; leave them to the general parser.
        rest.trim_start_matches(' ').is_empty().then_some(())
    };

    // Single (pseudo)string key.
    if let Some((key, rest)) = scan_key(content)
        && ends_clean(rest).is_some()
    {
        return Some(vec![key]);
    }

    // Legacy key list: `legacy(, legacy)*:`.
    let mut keys = Vec::new();
    let mut rest = content;
    loop {
        let (key, after) = scan_legacy_token(rest)?;
        keys.push(key);
        let after = after.trim_start_matches(' ');
        if let Some(after_comma) = after.strip_prefix(',') {
            rest = after_comma.trim_start_matches(' ');
            continue;
        }
        ends_clean(after)?;
        return Some(keys);
    }
}

/// Unquoted values whose first token is `null`/`true`/`false` are typed
/// (or hit the nom parser's commit-then-fail behavior on longer tokens
/// with those prefixes); all of it bails to the parser of record.
fn is_typed_literal_prefix(s: &str) -> bool {
    s.starts_with("null") || s.starts_with("true") || s.starts_with("false")
}

/// Value after `key:` — quoted string or greedy pseudostring to end of
/// line.
fn scan_colon_value(s: &str) -> Option<String> {
    if s.as_bytes().first() == Some(&b'"') {
        let (value, rest) = scan_quoted(s)?;
        if !rest.is_empty() {
            return None;
        }
        return Some(value);
    }
    if is_typed_literal_prefix(s) {
        return None;
    }
    let (value, rest) = scan_pseudostring(s)?;
    if !rest.is_empty() {
        return None;
    }
    Some(value)
}

/// Value in the legacy `key value` form — quoted string or a single
/// legacy token that must reach end of line.
fn scan_legacy_value(s: &str) -> Option<String> {
    if s.as_bytes().first() == Some(&b'"') {
        let (value, rest) = scan_quoted(s)?;
        if !rest.is_empty() {
            return None;
        }
        return Some(value);
    }
    if is_typed_literal_prefix(s) {
        return None;
    }
    let (value, rest) = scan_legacy_token(s)?;
    if !rest.is_empty() {
        return None;
    }
    Some(value)
}

/// The nom `pseudostring_legacy`: optional `--` prefix, alphanumeric or
/// `/` first character, then anything until `\r \n \t space : ,`.
/// Quoted strings are also accepted (the legacy_name alternation includes
/// them).
fn scan_legacy_token(s: &str) -> Option<(String, &str)> {
    if s.as_bytes().first() == Some(&b'"') {
        return scan_quoted(s);
    }
    let bytes = s.as_bytes();
    let body_start = if s.starts_with("--") { 2 } else { 0 };
    let first = *bytes.get(body_start)?;
    if !(first.is_ascii_alphanumeric() || first == b'/') {
        return None;
    }
    let mut end = body_start + 1;
    while end < bytes.len() {
        let b = bytes[end];
        if !b.is_ascii() {
            // Keep byte-wise scanning trivially correct; the parser of
            // record accepts non-ASCII here.
            return None;
        }
        if matches!(b, b'\r' | b'\n' | b'\t' | b' ' | b':' | b',') {
            break;
        }
        end += 1;
    }
    Some((s[..end].to_string(), &s[end..]))
}

/// Double-quoted string with the escape set the nom parser decodes
/// (`\" \\ \/ \n \r \t`). Unknown escapes bail.
fn scan_quoted(s: &str) -> Option<(String, &str)> {
    let inner = &s[1..];
    let bytes = inner.as_bytes();
    let mut out = String::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                out.push_str(&inner[start..i]);
                return Some((out, &inner[i + 1..]));
            }
            b'\\' => {
                out.push_str(&inner[start..i]);
                let decoded = match bytes.get(i + 1)? {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    _ => return None,
                };
                out.push(decoded);
                i += 2;
                start = i;
            }
            _ => i += 1,
        }
    }
    // Unterminated string.
    None
}

/// Unquoted pseudostring: mirrors `pseudostring_inner` (first-char and
/// tail exclusion sets, internal spaces allowed when followed by an
/// allowed character) including the leading/trailing space trim. Non-ASCII
/// input bails: the nom parser accepts it, but machine-generated lockfile
/// tokens are ASCII and byte-wise scanning stays trivially correct.
fn scan_pseudostring(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    let first = *bytes.first()?;
    if !first.is_ascii() || PSEUDO_FIRST_EXCLUDED.contains(&first) {
        return None;
    }
    let mut end = 1;
    while end < bytes.len() {
        let b = bytes[end];
        if !b.is_ascii() {
            return None;
        }
        if b == b' ' {
            // Spaces stay inside the token only when followed by another
            // allowed character.
            let mut probe = end;
            while probe < bytes.len() && bytes[probe] == b' ' {
                probe += 1;
            }
            match bytes.get(probe) {
                Some(&next) if !next.is_ascii() => return None,
                Some(&next) if !PSEUDO_TAIL_EXCLUDED.contains(&next) => end = probe + 1,
                _ => break,
            }
            continue;
        }
        if PSEUDO_TAIL_EXCLUDED.contains(&b) {
            break;
        }
        end += 1;
    }
    let raw = &s[..end];
    Some((raw.trim_matches(' ').to_string(), &s[end..]))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Both parsers must accept and agree, and the fast path must accept
    /// (so mainline shapes can't silently regress onto the slow path).
    fn assert_fast_matches_nom(input: &str) {
        let fast = parse(input).expect("fast path must accept this input");
        let value = super::super::de::parse_syml(input).expect("nom accepts");
        let nom: Map<String, Entry> = serde_json::from_value(value).expect("serde accepts");
        assert_eq!(fast, nom);
    }

    fn assert_falls_back(input: &str) {
        assert!(
            parse(input).is_none(),
            "input should be outside the fast subset"
        );
    }

    #[test]
    fn test_basic_lockfile_differential() {
        assert_fast_matches_nom(
            r#"# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.
# yarn lockfile v1


"@babel/code-frame@^7.0.0", "@babel/code-frame@^7.10.4":
  version "7.24.2"
  resolved "https://registry.yarnpkg.com/@babel/code-frame/-/code-frame-7.24.2.tgz#718b4b19841809a58b29b68cde80bc5e1aa6d9ae"
  integrity sha512-y5+tLQyV8pg3fsiln67BVLD1P13Eg4lh5RW9mF0zUuvLrv9uIQ4MCL+CRT+FTsBlBjcIan6PGsLcBN0m3ClUyQ==
  dependencies:
    "@babel/highlight" "^7.24.2"
    picocolors "^1.0.0"

lodash@^4.17.21:
  version "4.17.21"
  resolved "https://registry.yarnpkg.com/lodash/-/lodash-4.17.21.tgz"
  integrity sha512-v2kDEe57lecTulaDIuNTPy3Ry4gLGJ6Z1O3vE1krgXZNrsQ+LFTGHVxVjcXPs17LhbZVGedAJv8XZ1tvj5FvSg==

fsevents@^2.3.2:
  version "2.3.3"
  resolved "https://registry.yarnpkg.com/fsevents/-/fsevents-2.3.3.tgz"
  integrity sha512-abc
  optionalDependencies:
    node-gyp "^9.0.0"
"#,
        );
    }

    #[test]
    fn test_file_deps_and_unquoted_values_differential() {
        assert_fast_matches_nom(
            r#""@repo/eslint-config@file:./packages/eslint-config":
  version "0.0.0"
  dependencies:
    eslint-config-prettier "8.6.0"

eslint-config-prettier@8.6.0:
  version "8.6.0"
  uid abc123
  registry npm
"#,
        );
    }

    #[test]
    fn test_multikey_legacy_key_rules() {
        // Multi-key lines admit only legacy names under the parser of
        // record: a leading `_` (or `.`, `+`, `~`, ...) key is an error
        // there, so the fast path must not fabricate entries for it.
        assert_falls_back("_a@1, b@1:\n  version \"1.0.0\"\n");
        assert_falls_back("a@1, .b@1:\n  version \"1.0.0\"\n");
        assert!(super::super::de::parse_syml("_a@1, b@1:\n  version \"1.0.0\"\n").is_err());
        // ...while a single pseudostring key with those leads is legal.
        assert_fast_matches_nom("_a@1:\n  version \"1.0.0\"\n");
        // `--`-prefixed legacy names are accepted in multi-key position.
        assert_fast_matches_nom("--flag@1, b@1:\n  version \"1.0.0\"\n");
    }

    #[test]
    fn test_multikey_and_duplicate_keys_differential() {
        // Duplicate top-level keys are last-wins in both parsers.
        assert_fast_matches_nom(
            "a@^1, b@^2, c@^3:\n  version \"1.0.0\"\n\na@^1:\n  version \"2.0.0\"\n",
        );
    }

    #[test]
    fn test_escapes_differential() {
        assert_fast_matches_nom(
            "\"weird\\\"key\\\\name@^1\":\n  version \"1.0.0\"\n  resolved \"https://x.com/a\\nb\"\n",
        );
    }

    #[test]
    fn test_colon_field_form_differential() {
        // `key: value` fields (colon form) parse identically to the legacy
        // space form.
        assert_fast_matches_nom("foo@^1:\n  version: \"1.0.0\"\n  resolved: \"https://x\"\n");
    }

    #[test]
    fn test_unknown_fields_ignored_differential() {
        assert_fast_matches_nom(
            "foo@^1:\n  version \"1.0.0\"\n  languageName node\n  extra:\n    nested \"x\"\n",
        );
    }

    #[test]
    fn test_empty_and_comment_only_differential() {
        assert_fast_matches_nom("");
        assert_fast_matches_nom("# just a comment\n\n");
    }

    #[test]
    fn test_bails_outside_subset() {
        // Typed literals in string positions: serde rejects what nom types.
        assert_falls_back("foo@^1:\n  version null\n");
        // Arrays leave the fast subset.
        assert_falls_back("foo@^1:\n  version \"1\"\n  os:\n    - linux\n");
        // CRLF input is nom's problem.
        assert_falls_back("foo@^1:\r\n  version \"1.0.0\"\r\n");
        // Unknown escape sequences.
        assert_falls_back("\"a\\qb@^1\":\n  version \"1.0.0\"\n");
        // Deeper nesting than dependency maps.
        assert_falls_back("foo@^1:\n  version \"1\"\n  a:\n    b:\n      c \"d\"\n");
    }

    #[test]
    fn test_missing_version_falls_back() {
        // The serde layer requires `version`; the fast path must let the
        // parser of record produce that error.
        assert_falls_back("foo@^1:\n  resolved \"https://x\"\n");
    }
}
