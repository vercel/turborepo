//! Property tests: formatting-only rewrites of every fixture parse to the same
//! model and diff empty. Rewrites are semantics-preserving text transforms:
//! entry reordering at every mapping level, key and value requoting,
//! flow-to-block expansion, comments and blank lines, re-indentation and CRLF
//! line endings (YAML); key reordering, compact/pretty layout, escaped
//! slashes and trailing commas (JSON); entry and field reordering, requoting
//! and CRLF (yarn v1).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use proptest::prelude::*;
use serde_json::Value;
use turborepo_trace_lockfile::{LockfileKind, diff, parse};

fn fixture(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/lockfile")
        .join(rel);
    std::fs::read_to_string(path).unwrap()
}

/// xorshift64*, seeded by proptest.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

// ---------------------------------------------------------------- YAML

/// Shuffles the entries of the block mapping in `lines` whose keys sit at
/// `indent`, recursively.
fn shuffle_mapping(lines: &[String], indent: usize, rng: &mut Rng) -> Vec<String> {
    let mut prefix = Vec::new();
    let mut entries: Vec<Vec<String>> = Vec::new();
    for line in lines {
        let starts_entry = !is_blank_or_comment(line)
            && indent_of(line) == indent
            && !line.trim_start().starts_with("- ");
        if starts_entry {
            entries.push(vec![line.clone()]);
        } else if let Some(e) = entries.last_mut() {
            e.push(line.clone());
        } else {
            prefix.push(line.clone());
        }
    }
    for e in &mut entries {
        let child_indent = e[1..]
            .iter()
            .find(|l| !is_blank_or_comment(l))
            .map(|l| indent_of(l));
        if let Some(ci) = child_indent
            && ci > indent
            && !e[1..].iter().any(|l| l.trim_start().starts_with("- "))
        {
            let head = e[0].clone();
            let mut body = shuffle_mapping(&e[1..], ci, rng);
            body.insert(0, head);
            *e = body;
        }
        // Keep at least one separator line so shuffled entries stay readable;
        // not required.
    }
    rng.shuffle(&mut entries);
    prefix
        .into_iter()
        .chain(entries.into_iter().flatten())
        .collect()
}

/// Position of the key/value separator in a block-mapping line's content, if
/// any.
fn key_colon(content: &str) -> Option<usize> {
    if content.starts_with(['\'', '"', '{', '[', '-', '#']) {
        return None;
    }
    let b = content.as_bytes();
    (0..b.len()).find(|&i| b[i] == b':' && (i + 1 == b.len() || b[i + 1] == b' '))
}

fn quote(s: &str, rng: &mut Rng) -> String {
    if rng.coin() {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn requote_line(line: &str, keys: bool, values: bool, rng: &mut Rng) -> String {
    let ind = indent_of(line);
    let content = &line[ind..];
    let Some(c) = key_colon(content) else {
        return line.to_string();
    };
    let (key, rest) = (&content[..c], &content[c + 1..]);
    let key = if keys && rng.coin() {
        quote(key, rng)
    } else {
        key.to_string()
    };
    let value = rest.trim_start();
    let plain_value = !value.is_empty()
        && !value.starts_with(['{', '[', '\'', '"', '|', '>', '&', '*', '!', '#']);
    let value = if values && plain_value && rng.coin() {
        quote(value, rng)
    } else {
        value.to_string()
    };
    if value.is_empty() {
        format!("{}{key}:", " ".repeat(ind))
    } else {
        format!("{}{key}: {value}", " ".repeat(ind))
    }
}

/// `key: {a: b, c: d}` → block mapping; `key: [a, b]` → block sequence.
fn expand_flow_line(line: &str) -> Vec<String> {
    let ind = indent_of(line);
    let content = &line[ind..];
    let Some(c) = key_colon(content) else {
        return vec![line.to_string()];
    };
    let value = content[c + 1..].trim();
    let pad = " ".repeat(ind + 2);
    let inner = |v: &str| v[1..v.len() - 1].trim().to_string();
    if value.starts_with('{')
        && value.ends_with('}')
        && value.len() > 2
        && !value[1..].contains(['{', '['])
    {
        let mut out = vec![format!("{}{}:", " ".repeat(ind), &content[..c])];
        out.extend(
            inner(value)
                .split(", ")
                .map(|kv| format!("{pad}{}", kv.trim())),
        );
        out
    } else if value.starts_with('[')
        && value.ends_with(']')
        && value.len() > 2
        && !value[1..].contains(['{', '['])
    {
        let mut out = vec![format!("{}{}:", " ".repeat(ind), &content[..c])];
        out.extend(
            inner(value)
                .split(", ")
                .map(|item| format!("{pad}- {}", item.trim())),
        );
        out
    } else {
        vec![line.to_string()]
    }
}

#[derive(Debug, Clone)]
struct YamlStyle {
    shuffle: bool,
    expand_flow: bool,
    quote_keys: bool,
    quote_values: bool,
    comments: bool,
    reindent: bool,
    crlf: bool,
}

fn restyle_yaml(src: &str, style: &YamlStyle, rng: &mut Rng) -> String {
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
    if style.shuffle {
        lines = shuffle_mapping(&lines, 0, rng);
    }
    if style.expand_flow {
        lines = lines.iter().flat_map(|l| expand_flow_line(l)).collect();
    }
    if style.quote_keys || style.quote_values {
        lines = lines
            .iter()
            .map(|l| requote_line(l, style.quote_keys, style.quote_values, rng))
            .collect();
    }
    if style.comments {
        let mut out = Vec::new();
        for l in lines {
            match rng.next() % 8 {
                0 => out.push(format!("{}# noise", " ".repeat(indent_of(&l)))),
                1 => out.push(String::new()),
                _ => {}
            }
            out.push(l);
        }
        lines = out;
    }
    if style.reindent {
        lines = lines
            .iter()
            .map(|l| format!("{}{}", " ".repeat(indent_of(l)), l))
            .collect();
    }
    let nl = if style.crlf { "\r\n" } else { "\n" };
    let mut s = lines.join(nl);
    s.push_str(nl);
    s
}

fn yaml_style() -> impl Strategy<Value = YamlStyle> {
    (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(shuffle, expand_flow, quote_keys, quote_values, comments, reindent, crlf)| {
                YamlStyle {
                    shuffle,
                    expand_flow,
                    quote_keys,
                    quote_values,
                    comments,
                    reindent,
                    crlf,
                }
            },
        )
}

const YAML_FIXTURES: [(&str, LockfileKind); 5] = [
    ("pnpm/v9/pnpm-lock.yaml", LockfileKind::Pnpm),
    ("pnpm/v6/pnpm-lock.yaml", LockfileKind::Pnpm),
    ("pnpm/v6-single/pnpm-lock.yaml", LockfileKind::Pnpm),
    ("yarn/berry-v8/yarn.lock", LockfileKind::YarnBerry),
    ("yarn/berry-v6/yarn.lock", LockfileKind::YarnBerry),
];

// ---------------------------------------------------------------- JSON

fn emit_json(
    v: &Value,
    rng: &mut Rng,
    pretty: bool,
    trailing: bool,
    escape_slash: bool,
    depth: usize,
    out: &mut String,
) {
    let nl = |out: &mut String, d: usize| {
        if pretty {
            out.push('\n');
            out.push_str(&"  ".repeat(d));
        }
    };
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            rng.shuffle(&mut keys);
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, depth + 1);
                emit_json(
                    &Value::String((*k).clone()),
                    rng,
                    pretty,
                    trailing,
                    escape_slash,
                    depth + 1,
                    out,
                );
                out.push_str(if pretty { ": " } else { ":" });
                emit_json(
                    &map[*k],
                    rng,
                    pretty,
                    trailing,
                    escape_slash,
                    depth + 1,
                    out,
                );
            }
            if trailing && !keys.is_empty() {
                out.push(',');
            }
            if !keys.is_empty() {
                nl(out, depth);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(if pretty { ", " } else { "," });
                }
                emit_json(item, rng, pretty, trailing, escape_slash, depth + 1, out);
            }
            if trailing && !items.is_empty() {
                out.push(',');
            }
            out.push(']');
        }
        Value::String(s) => {
            let mut enc = serde_json::to_string(s).unwrap();
            if escape_slash {
                enc = enc.replace('/', "\\/");
            }
            out.push_str(&enc);
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Removes trailing commas (bun.lock is JSONC) so serde_json can read the
/// fixture.
fn strip_trailing_commas(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if in_str {
            out.push(c);
            if c == '\\' {
                out.push(b[i + 1] as char);
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == ',' {
            let rest = s[i + 1..].trim_start();
            if !(rest.starts_with('}') || rest.starts_with(']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------- yarn classic

fn restyle_yarn_classic(src: &str, rng: &mut Rng, quote_flip: bool, crlf: bool) -> String {
    let mut header = Vec::new();
    let mut entries: Vec<Vec<String>> = Vec::new();
    for line in src.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('#') && entries.is_empty() {
            header.push(line.to_string());
        } else if indent_of(line) == 0 {
            entries.push(vec![line.to_string()]);
        } else {
            entries.last_mut().unwrap().push(line.to_string());
        }
    }
    rng.shuffle(&mut entries);
    let mut out = header;
    out.push(String::new());
    for e in entries {
        out.push(String::new());
        let mut head = e[0].clone();
        if quote_flip && !head.contains('"') {
            let keys = head.trim_end_matches(':');
            head = format!(
                "{}:",
                keys.split(", ")
                    .map(|k| format!("\"{k}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        out.push(head);
        // Fields (indent 2) with their nested blocks, shuffled; nested dependency lines
        // too.
        let mut fields: Vec<Vec<String>> = Vec::new();
        for l in &e[1..] {
            if indent_of(l) == 2 {
                fields.push(vec![l.clone()]);
            } else {
                fields.last_mut().unwrap().push(l.clone());
            }
        }
        rng.shuffle(&mut fields);
        for mut f in fields {
            let mut nested = f.split_off(1);
            rng.shuffle(&mut nested);
            for l in f.into_iter().chain(nested) {
                out.push(if quote_flip { flip_yarn_value(&l) } else { l });
            }
        }
    }
    let nl = if crlf { "\r\n" } else { "\n" };
    let mut s = out.join(nl);
    s.push_str(nl);
    s
}

/// `key "value"` ↔ `key value` (only when the value has no spaces); bare names
/// get quoted.
fn flip_yarn_value(line: &str) -> String {
    let ind = indent_of(line);
    let content = &line[ind..];
    if content.ends_with(':') || content.starts_with('"') {
        return line.to_string();
    }
    let Some((key, value)) = content.split_once(' ') else {
        return line.to_string();
    };
    let value = if let Some(v) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        if v.contains(' ') || v.is_empty() {
            value.to_string()
        } else {
            v.to_string()
        }
    } else {
        format!("\"{value}\"")
    };
    format!("{}\"{key}\" {value}", " ".repeat(ind))
}

#[test]
fn rewrites_really_change_the_text() {
    let all = YamlStyle {
        shuffle: true,
        expand_flow: true,
        quote_keys: true,
        quote_values: true,
        comments: true,
        reindent: true,
        crlf: true,
    };
    let src = fixture("pnpm/v9/pnpm-lock.yaml");
    let out = restyle_yaml(&src, &all, &mut Rng(7));
    assert_ne!(out, src);
    assert!(out.contains("\r\n"), "CRLF");
    assert!(out.contains("# noise"), "comments");
    assert!(
        out.contains("\r\n    ") && !out.contains("\r\n  '@fx"),
        "reindented to 4 spaces"
    );
    assert!(
        !out.contains("resolution: {integrity"),
        "flow mappings were expanded"
    );
    assert!(out.contains('\'') && out.contains('"'));
    let first_keys = |s: &str| {
        s.lines()
            .filter(|l| indent_of(l) == 4 && !is_blank_or_comment(l))
            .take(3)
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    assert_ne!(
        first_keys(&out),
        first_keys(&restyle_yaml(&src, &all, &mut Rng(8))),
        "seeds shuffle differently"
    );
    assert_eq!(
        parse(LockfileKind::Pnpm, &out).unwrap(),
        parse(LockfileKind::Pnpm, &src).unwrap()
    );

    let yarn = fixture("yarn/classic/yarn.lock");
    let out = restyle_yarn_classic(&yarn, &mut Rng(7), true, true);
    assert_ne!(out, yarn);
    assert!(
        out.contains("\"debug@4.3.4\":") && out.contains("\"version\" 4.3.4"),
        "{out}"
    );
}

// ---------------------------------------------------------------- properties

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    #[test]
    fn yaml_rewrites_are_stable(seed in 1..u64::MAX, style in yaml_style(), which in 0..YAML_FIXTURES.len()) {
        let (rel, kind) = YAML_FIXTURES[which];
        let src = fixture(rel);
        let restyled = restyle_yaml(&src, &style, &mut Rng(seed));
        let a = parse(kind, &src).unwrap();
        let b = parse(kind, &restyled).map_err(|e| TestCaseError::fail(format!("{e}\n---\n{restyled}")))?;
        prop_assert_eq!(&a, &b, "{}", restyled);
        prop_assert!(diff(&a, &b).is_empty());
    }

    #[test]
    fn json_rewrites_are_stable(seed in 1..u64::MAX, pretty: bool, escape_slash: bool, trailing: bool, which in 0..3usize) {
        let (rel, kind) = [
            ("npm/v3/package-lock.json", LockfileKind::Npm),
            ("npm/v2/package-lock.json", LockfileKind::Npm),
            ("bun/v1/bun.lock", LockfileKind::Bun),
        ][which];
        let src = fixture(rel);
        let value: Value = serde_json::from_str(&strip_trailing_commas(&src)).unwrap();
        let mut restyled = String::new();
        // Trailing commas are JSONC; npm's parser is strict JSON.
        let trailing = trailing && kind == LockfileKind::Bun;
        emit_json(&value, &mut Rng(seed), pretty, trailing, escape_slash, 0, &mut restyled);
        let a = parse(kind, &src).unwrap();
        let b = parse(kind, &restyled).map_err(|e| TestCaseError::fail(format!("{e}\n---\n{restyled}")))?;
        prop_assert_eq!(&a, &b, "{}", restyled);
        prop_assert!(diff(&a, &b).is_empty());
    }

    #[test]
    fn yarn_classic_rewrites_are_stable(seed in 1..u64::MAX, quote_flip: bool, crlf: bool) {
        let src = fixture("yarn/classic/yarn.lock");
        let restyled = restyle_yarn_classic(&src, &mut Rng(seed), quote_flip, crlf);
        let a = parse(LockfileKind::YarnClassic, &src).unwrap();
        let b = parse(LockfileKind::YarnClassic, &restyled).map_err(|e| TestCaseError::fail(format!("{e}\n---\n{restyled}")))?;
        prop_assert_eq!(&a, &b, "{}", restyled);
        prop_assert!(diff(&a, &b).is_empty());
    }
}
