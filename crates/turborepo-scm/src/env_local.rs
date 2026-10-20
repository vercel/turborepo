use std::ops::Range;

const TOKEN_KEY: &[u8] = b"VERCEL_OIDC_TOKEN";
const TOKEN_PLACEHOLDER: &[u8] = b"__TURBOREPO_VERCEL_OIDC_TOKEN__";

/// Normalize one unambiguous literal dotenv token assignment for hashing.
///
/// This deliberately accepts only a small dotenv subset: ASCII identifiers,
/// optional `export`, horizontal whitespace, comments, and literal unquoted or
/// single/double-quoted values. Quoted values on other keys may span lines.
/// Quotes and escapes are scanned, not decoded; nothing is expanded or
/// executed. Duplicate token keys, multiline token values, binary input, and
/// unsupported syntax return `None`, so the caller must hash the original bytes
/// instead. Only the token's value bytes are replaced; its delimiters and all
/// surrounding bytes are retained. The entire input is validated before
/// producing output.
pub(crate) fn normalize_oidc_token(bytes: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
        || bytes
            .iter()
            .enumerate()
            .any(|(i, &b)| b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
    {
        return None;
    }

    let mut cursor = 0;
    let mut token = None;
    while cursor < bytes.len() {
        skip_horizontal(bytes, &mut cursor);
        if matches!(bytes.get(cursor), None | Some(b'#' | b'\r' | b'\n')) {
            finish_line(bytes, &mut cursor)?;
            continue;
        }

        if bytes[cursor..].starts_with(b"export")
            && matches!(bytes.get(cursor + 6), Some(b' ' | b'\t'))
        {
            cursor += 6;
            skip_horizontal(bytes, &mut cursor);
        }

        let key_start = cursor;
        if !matches!(bytes.get(cursor), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_')) {
            return None;
        }
        cursor += 1;
        while matches!(
            bytes.get(cursor),
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
        ) {
            cursor += 1;
        }
        let is_token = &bytes[key_start..cursor] == TOKEN_KEY;
        skip_horizontal(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'=') {
            return None;
        }
        cursor += 1;
        let after_equals = cursor;
        skip_horizontal(bytes, &mut cursor);

        let value = match bytes.get(cursor) {
            Some(b'\'' | b'"') => quoted_value(bytes, &mut cursor)?,
            _ => {
                if bytes.get(cursor) == Some(&b'#') && cursor == after_equals {
                    return None;
                }
                let value = unquoted_value(bytes, &mut cursor)?;
                // For an empty value, insert before its trailing whitespace so
                // an existing comment still has a separating space.
                if value.is_empty() {
                    after_equals..after_equals
                } else {
                    value
                }
            }
        };
        finish_line(bytes, &mut cursor)?;

        if is_token {
            // Different dotenv readers disagree on duplicate precedence. Do not
            // choose a winner, or erase any of the competing assignments.
            if token.is_some() || bytes[value.clone()].contains(&b'\n') {
                return None;
            }
            token = Some(value);
        }
    }

    let value = token?;
    let mut normalized = Vec::with_capacity(bytes.len() - value.len() + TOKEN_PLACEHOLDER.len());
    normalized.extend_from_slice(&bytes[..value.start]);
    normalized.extend_from_slice(TOKEN_PLACEHOLDER);
    normalized.extend_from_slice(&bytes[value.end..]);
    Some(normalized)
}

fn skip_horizontal(bytes: &[u8], cursor: &mut usize) {
    while matches!(bytes.get(*cursor), Some(b' ' | b'\t')) {
        *cursor += 1;
    }
}

/// Consume only whitespace, an optional comment, and a line ending (or EOF).
fn finish_line(bytes: &[u8], cursor: &mut usize) -> Option<()> {
    skip_horizontal(bytes, cursor);
    if bytes.get(*cursor) == Some(&b'#') {
        while !matches!(bytes.get(*cursor), None | Some(b'\r' | b'\n')) {
            *cursor += 1;
        }
    }
    match bytes.get(*cursor) {
        None => Some(()),
        Some(b'\n') => {
            *cursor += 1;
            Some(())
        }
        Some(b'\r') if bytes.get(*cursor + 1) == Some(&b'\n') => {
            *cursor += 2;
            Some(())
        }
        _ => None,
    }
}

fn quoted_value(bytes: &[u8], cursor: &mut usize) -> Option<Range<usize>> {
    let quote = bytes[*cursor];
    *cursor += 1;
    let start = *cursor;
    while let Some(&byte) = bytes.get(*cursor) {
        match byte {
            b'\\' => {
                // A backslash shields the next byte for delimiter scanning.
                // Line continuations have reader-dependent semantics.
                // Readers disagree whether a doubled backslash escapes a
                // subsequent quote. Do not risk interpreting an embedded line
                // in another variable as a token assignment.
                if matches!(bytes.get(*cursor + 1), None | Some(b'\r' | b'\n' | b'\\')) {
                    return None;
                }
                *cursor += 2;
            }
            byte if byte == quote => {
                let end = *cursor;
                *cursor += 1;
                return Some(start..end);
            }
            _ => *cursor += 1,
        }
    }
    None
}

fn unquoted_value(bytes: &[u8], cursor: &mut usize) -> Option<Range<usize>> {
    let start = *cursor;
    while let Some(&byte) = bytes.get(*cursor) {
        match byte {
            b'\r' | b'\n' => break,
            b'#' => {
                // An adjacent '#' may be either a literal or a comment,
                // depending on the dotenv reader. Only accept separated comments.
                if *cursor != start && !matches!(bytes[*cursor - 1], b' ' | b'\t') {
                    return None;
                }
                break;
            }
            b' ' | b'\t' => {
                let end = *cursor;
                skip_horizontal(bytes, cursor);
                if !matches!(bytes.get(*cursor), None | Some(b'#' | b'\r' | b'\n')) {
                    return None;
                }
                return Some(start..end);
            }
            // Reject concatenated quoting, escaping, expansion, and shell syntax.
            b'\'' | b'"' | b'\\' | b'$' | b'`' | b';' | b'&' | b'|' | b'<' | b'>' | b'(' | b')'
            | b'{' | b'}' => return None,
            _ => *cursor += 1,
        }
    }
    Some(start..*cursor)
}

#[cfg(test)]
mod tests {
    use super::{TOKEN_PLACEHOLDER, normalize_oidc_token};

    fn assert_normalized(input: &str, expected: &str) {
        assert_eq!(
            normalize_oidc_token(input.as_bytes()).as_deref(),
            Some(expected.as_bytes()),
            "input: {input:?}"
        );
        assert_eq!(
            normalize_oidc_token(expected.as_bytes()).as_deref(),
            Some(expected.as_bytes()),
            "normalized output must remain valid and idempotent"
        );
    }

    fn assert_fallback(input: &[u8]) {
        assert_eq!(normalize_oidc_token(input), None, "input: {input:?}");
    }

    #[test]
    fn falls_back_for_ambiguous_doubled_backslashes_before_quotes() {
        for quote in ['\'', '"'] {
            let input =
                format!("OTHER={quote}prefix\\\\\\\\{quote}\nVERCEL_OIDC_TOKEN=one\n# {quote}\n");
            assert_fallback(input.as_bytes());
        }
    }

    #[test]
    fn replaces_only_exact_key_and_value() {
        assert_normalized(
            concat!(
                "# VERCEL_OIDC_TOKEN=comment\nOTHER=unchanged\n",
                "VERCEL_OIDC_TOKEN=secret\nVERCEL_OIDC_TOKEN_SUFFIX=keep\n",
                "PREFIX_VERCEL_OIDC_TOKEN=keep\n",
            ),
            concat!(
                "# VERCEL_OIDC_TOKEN=comment\nOTHER=unchanged\n",
                "VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__\n",
                "VERCEL_OIDC_TOKEN_SUFFIX=keep\nPREFIX_VERCEL_OIDC_TOKEN=keep\n",
            ),
        );
    }

    #[test]
    fn preserves_whitespace_export_quotes_comments_and_crlf() {
        assert_normalized(
            " \t# header\r\n\t export\t VERCEL_OIDC_TOKEN \t= \t\"secret\" \t# comment\r\nOTHER = \
             'keep # this'\r\n\t\r\n",
            " \t# header\r\n\t export\t VERCEL_OIDC_TOKEN \t= \
             \t\"__TURBOREPO_VERCEL_OIDC_TOKEN__\" \t# comment\r\nOTHER = 'keep # this'\r\n\t\r\n",
        );
    }

    #[test]
    fn accepts_each_literal_value_style_and_missing_final_newline() {
        for (input, expected) in [
            (
                "VERCEL_OIDC_TOKEN=abc.def-123_+/==",
                "VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__",
            ),
            (
                "VERCEL_OIDC_TOKEN=' abc # def '",
                "VERCEL_OIDC_TOKEN='__TURBOREPO_VERCEL_OIDC_TOKEN__'",
            ),
            (
                "VERCEL_OIDC_TOKEN=\" abc # def \"#comment",
                "VERCEL_OIDC_TOKEN=\"__TURBOREPO_VERCEL_OIDC_TOKEN__\"#comment",
            ),
            (
                "VERCEL_OIDC_TOKEN=abc \t #comment\n",
                "VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__ \t #comment\n",
            ),
        ] {
            assert_normalized(input, expected);
        }
    }

    #[test]
    fn handles_empty_values_without_removing_surrounding_whitespace() {
        for (input, expected) in [
            (
                "VERCEL_OIDC_TOKEN=",
                "VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__",
            ),
            (
                "VERCEL_OIDC_TOKEN= \t#empty\n",
                "VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__ \t#empty\n",
            ),
            (
                "VERCEL_OIDC_TOKEN=''\n",
                "VERCEL_OIDC_TOKEN='__TURBOREPO_VERCEL_OIDC_TOKEN__'\n",
            ),
            (
                "VERCEL_OIDC_TOKEN=\"\"\r\n",
                "VERCEL_OIDC_TOKEN=\"__TURBOREPO_VERCEL_OIDC_TOKEN__\"\r\n",
            ),
        ] {
            assert_normalized(input, expected);
        }
    }

    #[test]
    fn scans_multiline_other_values_without_matching_embedded_assignments() {
        for quote in ['\'', '"'] {
            let other = format!(
                "OTHER={quote}first\nVERCEL_OIDC_TOKEN=fake\r\nexport \
                 VERCEL_OIDC_TOKEN=also_fake\nlast{quote}\t# keep\r\n"
            );
            assert_fallback(other.as_bytes());
            assert_normalized(
                &format!("{other}VERCEL_OIDC_TOKEN=real\n"),
                &format!("{other}VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__\n"),
            );
            assert_normalized(
                &format!("VERCEL_OIDC_TOKEN=real\n{other}"),
                &format!("VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__\n{other}"),
            );
        }
    }

    #[test]
    fn handles_escaped_quotes_conservatively() {
        for input in [
            r#"VERCEL_OIDC_TOKEN="a\\""#,
            r#"VERCEL_OIDC_TOKEN="a\\\"b""#,
        ] {
            assert_fallback(input.as_bytes());
        }
        for input in [r#"VERCEL_OIDC_TOKEN="a\"b""#, r"VERCEL_OIDC_TOKEN='a\'b'"] {
            let quote = if input.as_bytes()[18] == b'"' {
                '"'
            } else {
                '\''
            };
            assert_normalized(
                input,
                &format!("VERCEL_OIDC_TOKEN={quote}__TURBOREPO_VERCEL_OIDC_TOKEN__{quote}"),
            );
        }
        let other = "OTHER=\"escaped \\\"\nVERCEL_OIDC_TOKEN=fake\nend\"\n";
        assert_normalized(
            &format!("{other}VERCEL_OIDC_TOKEN=real"),
            &format!("{other}VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__"),
        );
    }

    #[test]
    fn preserves_unicode_and_mixed_line_endings() {
        assert_normalized(
            "# café\r\nOTHER=\"日本語\"\nVERCEL_OIDC_TOKEN='秘密'\r\nOTHER_2=naïve\n",
            concat!(
                "# café\r\nOTHER=\"日本語\"\n",
                "VERCEL_OIDC_TOKEN='__TURBOREPO_VERCEL_OIDC_TOKEN__'\r\nOTHER_2=naïve\n",
            ),
        );
    }

    #[test]
    fn returns_none_when_no_exact_assignment_exists() {
        for input in [
            "",
            " \t\n\r\n",
            "# VERCEL_OIDC_TOKEN=secret",
            "OTHER=VERCEL_OIDC_TOKEN",
            "VERCEL_OIDC_TOKEN_SUFFIX=secret",
            "PREFIX_VERCEL_OIDC_TOKEN=secret",
            "vercel_oidc_token=secret",
            "exported=VERCEL_OIDC_TOKEN",
            "exportOTHER=value",
            "OTHER='VERCEL_OIDC_TOKEN=secret'",
            "OTHER=\"VERCEL_OIDC_TOKEN=secret\"",
        ] {
            assert_fallback(input.as_bytes());
        }
    }

    #[test]
    fn duplicate_token_assignments_fall_back_regardless_of_style_or_value() {
        for second in [
            "VERCEL_OIDC_TOKEN=one",
            "VERCEL_OIDC_TOKEN=two",
            "export VERCEL_OIDC_TOKEN='two'",
            " \tVERCEL_OIDC_TOKEN = \"two\"",
        ] {
            assert_fallback(format!("VERCEL_OIDC_TOKEN=one\n{second}\n").as_bytes());
        }
        // Duplicate unrelated keys do not introduce token precedence ambiguity.
        assert_normalized(
            "OTHER=one\nOTHER=two\nVERCEL_OIDC_TOKEN=real",
            "OTHER=one\nOTHER=two\nVERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__",
        );
    }

    #[test]
    fn malformed_or_unsupported_syntax_anywhere_falls_back() {
        for malformed in [
            "not an assignment",
            "export VERCEL_OIDC_TOKEN",
            "export",
            "=value",
            "1KEY=value",
            "KEY-WITH-DASH=value",
            "KEY.WITH.DOT=value",
            "KEY:value",
            "KEY='unterminated",
            "KEY=\"unterminated",
            "KEY=\"dangling\\",
            "KEY='one' 'two'",
            "KEY=\"one\"junk",
            "KEY=unquoted\"quote",
            "KEY=one two",
            "KEY=one#ambiguous",
            "KEY=one\\ two",
            "KEY=one\\\ncontinued",
            "KEY=\"one\\\ncontinued\"",
            "KEY='one\\\r\ncontinued'",
            "KEY=$OTHER",
            "KEY=${OTHER}",
            "KEY=$(command)",
            "KEY=`command`",
            "KEY=one;command",
            "KEY=one&command",
            "KEY=one|command",
            "KEY=<file",
            "KEY=>file",
            "KEY=(value)",
            "KEY={value}",
            "KEY=value\rbare",
            "KEY=#ambiguous",
            "\u{feff}KEY=value",
            "export\u{a0}KEY=value",
        ] {
            assert_fallback(format!("{malformed}\nVERCEL_OIDC_TOKEN=secret\n").as_bytes());
            assert_fallback(format!("VERCEL_OIDC_TOKEN=secret\n{malformed}\n").as_bytes());
        }
    }

    #[test]
    fn malformed_token_values_and_unsupported_assignments_fall_back() {
        for input in [
            "VERCEL_OIDC_TOKEN='unterminated",
            "VERCEL_OIDC_TOKEN=\"unterminated",
            "VERCEL_OIDC_TOKEN='one'junk",
            "VERCEL_OIDC_TOKEN=one two",
            "VERCEL_OIDC_TOKEN=one#ambiguous",
            "VERCEL_OIDC_TOKEN=#ambiguous",
            "VERCEL_OIDC_TOKEN=$OTHER",
            "VERCEL_OIDC_TOKEN=$(command)",
            "VERCEL_OIDC_TOKEN=one\\ two",
            "VERCEL_OIDC_TOKEN=\"one\\\ncontinued\"",
            "export export VERCEL_OIDC_TOKEN=one",
            "exportVERCEL_OIDC_TOKEN=one",
            "VERCEL_OIDC_TOKEN:one",
        ] {
            assert_fallback(input.as_bytes());
        }
    }

    #[test]
    fn scans_delimiter_combinations_without_panicking_or_unstable_output() {
        // Exercise short combinations of parser-significant bytes, both before
        // and after a valid token, including incomplete quote/escape sequences.
        let alphabet = b"a='\"\\# \t\r\n$;";
        for &a in alphabet {
            for &b in alphabet {
                for &c in alphabet {
                    let fragment = [a, b, c];
                    for prefix in [true, false] {
                        let mut input = Vec::new();
                        if prefix {
                            input.extend_from_slice(b"VERCEL_OIDC_TOKEN=secret\n");
                        }
                        input.extend_from_slice(b"OTHER=");
                        input.extend_from_slice(&fragment);
                        if !prefix {
                            input.extend_from_slice(b"\nVERCEL_OIDC_TOKEN=secret");
                        }
                        if let Some(normalized) = normalize_oidc_token(&input) {
                            assert_eq!(normalize_oidc_token(&normalized), Some(normalized));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn multiline_tokens_fall_back_instead_of_removing_line_endings() {
        for input in [
            "VERCEL_OIDC_TOKEN='one\ntwo'",
            "VERCEL_OIDC_TOKEN=\"one\r\ntwo\"",
        ] {
            assert_fallback(input.as_bytes());
        }
    }

    #[test]
    fn invalid_utf8_and_binary_data_anywhere_fall_back() {
        for invalid in [
            &b"\xff"[..],
            &b"\xc3\x28"[..],
            &b"\0"[..],
            &b"\x01"[..],
            &b"\x0b"[..],
            &b"\x0c"[..],
            &b"\x7f"[..],
            "\u{85}".as_bytes(),
            &b"\r"[..],
        ] {
            let mut input = b"VERCEL_OIDC_TOKEN=secret\n# ".to_vec();
            input.extend_from_slice(invalid);
            assert_fallback(&input);
            let mut input = b"VERCEL_OIDC_TOKEN='".to_vec();
            input.extend_from_slice(invalid);
            input.extend_from_slice(b"'\n");
            assert_fallback(&input);
        }
    }

    #[test]
    fn quoted_expansion_like_text_is_opaque_and_other_bytes_are_unchanged() {
        assert_normalized(
            "OTHER=\"$VAR ${VAR} $(command) `command`\"\nVERCEL_OIDC_TOKEN='${TOKEN}'\n",
            "OTHER=\"$VAR ${VAR} $(command) \
             `command`\"\nVERCEL_OIDC_TOKEN='__TURBOREPO_VERCEL_OIDC_TOKEN__'\n",
        );
    }

    #[test]
    fn normalization_is_stable_across_rotation_and_idempotent() {
        let first = normalize_oidc_token(b"VERCEL_OIDC_TOKEN=first\nOTHER=keep\n");
        let second = normalize_oidc_token(b"VERCEL_OIDC_TOKEN=second\nOTHER=keep\n");
        assert!(first.is_some());
        assert_eq!(first, second);
        if let Some(normalized) = first {
            assert_eq!(normalize_oidc_token(&normalized), Some(normalized));
        }
        assert_normalized(
            "VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__",
            "VERCEL_OIDC_TOKEN=__TURBOREPO_VERCEL_OIDC_TOKEN__",
        );
        assert_eq!(TOKEN_PLACEHOLDER, b"__TURBOREPO_VERCEL_OIDC_TOKEN__");
        assert_ne!(
            normalize_oidc_token(b"VERCEL_OIDC_TOKEN=first\nOTHER=one"),
            normalize_oidc_token(b"VERCEL_OIDC_TOKEN=first\nOTHER=two"),
        );
    }
}
