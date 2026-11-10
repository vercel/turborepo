//! A small TOML reader for the Python lockfiles (`uv.lock`, `poetry.lock`).
//!
//! Supports what those files use: `[table]` and `[[array.of.tables]]` headers
//! (dotted, quoted keys), bare and quoted keys (dotted keys too), basic,
//! literal and multi-line strings, arrays (multi-line, trailing commas,
//! comments), inline tables, and scalars (integers, floats, booleans, dates),
//! which are kept as their source text. It is lenient: it reports the
//! first structural error with its line, never panics, and runs in linear time.

use crate::{Error, LockfileFormat, Result};

/// A TOML value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Val {
    /// A string.
    Str(String),
    /// A scalar other than a string, as written (`1`, `true`, `1979-05-27`).
    Scalar(String),
    /// An array.
    Arr(Vec<Val>),
    /// A table (keys in insertion order).
    Table(Vec<(String, Val)>),
}

impl Val {
    pub(crate) fn get(&self, key: &str) -> Option<&Val> {
        match self {
            Val::Table(t) => t.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn scalar(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Val::Str(s) | Val::Scalar(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn arr(&self, key: &str) -> &[Val] {
        match self.get(key) {
            Some(Val::Arr(a)) => a,
            _ => &[],
        }
    }

    pub(crate) fn table(&self) -> &[(String, Val)] {
        match self {
            Val::Table(t) => t,
            _ => &[],
        }
    }

    fn table_mut(&mut self) -> Option<&mut Vec<(String, Val)>> {
        match self {
            Val::Table(t) => Some(t),
            _ => None,
        }
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
    format: LockfileFormat,
}

impl P<'_> {
    fn err(&self, msg: impl Into<String>) -> Error {
        Error::Syntax {
            format: self.format,
            line: self.line,
            message: msg.into(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) {
        if self.peek() == Some(b'\n') {
            self.line += 1;
        }
        self.i += 1;
    }

    /// Skips spaces and tabs (and newlines and comments if `newlines`).
    fn ws(&mut self, newlines: bool) {
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\r' => self.bump(),
                b'\n' if newlines => self.bump(),
                b'#' => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn key_part(&mut self) -> Result<String> {
        match self.peek() {
            Some(b'"' | b'\'') => self.string(),
            _ => {
                let start = self.i;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                {
                    self.i += 1;
                }
                if self.i == start {
                    return Err(self.err("expected a key"));
                }
                Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
            }
        }
    }

    /// A dotted key.
    fn key(&mut self) -> Result<Vec<String>> {
        let mut parts = vec![self.key_part()?];
        loop {
            self.ws(false);
            if self.peek() != Some(b'.') {
                return Ok(parts);
            }
            self.i += 1;
            self.ws(false);
            parts.push(self.key_part()?);
        }
    }

    fn string(&mut self) -> Result<String> {
        let q = self.peek().unwrap_or(b'"');
        let triple = self.s[self.i..].starts_with(&[q, q, q]);
        self.i += if triple { 3 } else { 1 };
        if triple && self.peek() == Some(b'\n') {
            self.bump();
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("unterminated string"));
            };
            if c == q && (!triple || self.s[self.i..].starts_with(&[q, q, q])) {
                self.i += if triple { 3 } else { 1 };
                // `""""` ends with an extra quote inside the string.
                while triple && self.peek() == Some(q) {
                    out.push(q);
                    self.i += 1;
                }
                return Ok(String::from_utf8_lossy(&out).into_owned());
            }
            if c == b'\n' && !triple {
                return Err(self.err("newline in string"));
            }
            if c == b'\\' && q == b'"' {
                self.i += 1;
                let e = self.peek().unwrap_or(b'\\');
                self.i += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'u' | b'U' => {
                        let n = if e == b'u' { 4 } else { 8 };
                        let hex = self.s.get(self.i..self.i + n).unwrap_or_default();
                        self.i = (self.i + n).min(self.s.len());
                        let ch = std::str::from_utf8(hex)
                            .ok()
                            .and_then(|h| u32::from_str_radix(h, 16).ok())
                            .and_then(char::from_u32)
                            .unwrap_or('\u{fffd}');
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    b'\n' | b' ' | b'\t' | b'\r' if triple => {
                        // Line-ending backslash: trim whitespace up to the next content.
                        self.i -= 1;
                        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                            self.bump();
                        }
                    }
                    other => out.push(other),
                }
                continue;
            }
            out.push(c);
            self.bump();
        }
    }

    fn value(&mut self, depth: u32) -> Result<Val> {
        if depth > 64 {
            return Err(self.err("nesting too deep"));
        }
        match self.peek() {
            Some(b'"' | b'\'') => self.string().map(Val::Str),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws(true);
                    match self.peek() {
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Val::Arr(items));
                        }
                        Some(b',') => self.i += 1,
                        None => return Err(self.err("unterminated array")),
                        _ => items.push(self.value(depth + 1)?),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut t: Vec<(String, Val)> = Vec::new();
                loop {
                    self.ws(true);
                    match self.peek() {
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Val::Table(t));
                        }
                        Some(b',') => self.i += 1,
                        None => return Err(self.err("unterminated inline table")),
                        _ => {
                            let k = self.key()?;
                            self.ws(false);
                            if self.peek() != Some(b'=') {
                                return Err(self.err("expected `=` in inline table"));
                            }
                            self.i += 1;
                            self.ws(false);
                            let v = self.value(depth + 1)?;
                            insert(&mut t, &k, v);
                        }
                    }
                }
            }
            Some(_) => {
                let start = self.i;
                while self
                    .peek()
                    .is_some_and(|c| !matches!(c, b',' | b']' | b'}' | b'\n' | b'#' | b'\r'))
                {
                    self.i += 1;
                }
                let text = String::from_utf8_lossy(&self.s[start..self.i])
                    .trim()
                    .to_owned();
                if text.is_empty() {
                    return Err(self.err("expected a value"));
                }
                Ok(Val::Scalar(text))
            }
            None => Err(self.err("expected a value")),
        }
    }
}

/// Inserts `v` at dotted key `k` inside table `t`, creating intermediate
/// tables.
fn insert(t: &mut Vec<(String, Val)>, k: &[String], v: Val) {
    let Some((last, path)) = k.split_last() else {
        return;
    };
    let mut cur = t;
    for p in path {
        let idx = match cur.iter().position(|(n, _)| n == p) {
            Some(i) => i,
            None => {
                cur.push((p.clone(), Val::Table(Vec::new())));
                cur.len() - 1
            }
        };
        let Some(next) = cur[idx].1.table_mut() else {
            return;
        };
        cur = next;
    }
    match cur.iter_mut().find(|(n, _)| n == last) {
        Some(slot) => slot.1 = v,
        None => cur.push((last.clone(), v)),
    }
}

/// The table at header path `path`, creating it (and appending to the last
/// element of arrays of tables along the way, as TOML does).
fn table_at<'t>(root: &'t mut Vec<(String, Val)>, path: &[String]) -> &'t mut Vec<(String, Val)> {
    let mut cur = root;
    for p in path {
        let idx = match cur.iter().position(|(n, _)| n == p) {
            Some(i) => i,
            None => {
                cur.push((p.clone(), Val::Table(Vec::new())));
                cur.len() - 1
            }
        };
        let slot = &mut cur[idx].1;
        if let Val::Arr(a) = slot {
            if !matches!(a.last(), Some(Val::Table(_))) {
                a.push(Val::Table(Vec::new()));
            }
            let Some(Val::Table(t)) = a.last_mut() else {
                unreachable!("pushed a table above");
            };
            cur = t;
        } else {
            if !matches!(slot, Val::Table(_)) {
                *slot = Val::Table(Vec::new());
            }
            let Val::Table(t) = slot else {
                unreachable!("made a table above");
            };
            cur = t;
        }
    }
    cur
}

/// Parses a TOML document into its root table.
// Invariant: `key()` returns at least one part.
#[allow(clippy::expect_used)]
pub(crate) fn parse(text: &str, format: LockfileFormat) -> Result<Val> {
    let mut p = P {
        s: text.as_bytes(),
        i: 0,
        line: 1,
        format,
    };
    let mut root: Vec<(String, Val)> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    loop {
        p.ws(true);
        let Some(c) = p.peek() else {
            break;
        };
        if c == b'[' {
            let array = p.s[p.i..].starts_with(b"[[");
            p.i += if array { 2 } else { 1 };
            p.ws(false);
            let path = p.key()?;
            p.ws(false);
            let close: &[u8] = if array { b"]]" } else { b"]" };
            if !p.s[p.i..].starts_with(close) {
                return Err(p.err("unterminated table header"));
            }
            p.i += close.len();
            if array {
                let (last, parent) = path.split_last().expect("key() returns a part");
                let t = table_at(&mut root, parent);
                match t.iter_mut().find(|(n, _)| n == last) {
                    Some((_, Val::Arr(a))) => a.push(Val::Table(Vec::new())),
                    Some(slot) => slot.1 = Val::Arr(vec![Val::Table(Vec::new())]),
                    None => t.push((last.clone(), Val::Arr(vec![Val::Table(Vec::new())]))),
                }
            } else {
                table_at(&mut root, &path);
            }
            current = path;
            continue;
        }
        let key = p.key()?;
        p.ws(false);
        if p.peek() != Some(b'=') {
            return Err(p.err("expected `=`"));
        }
        p.i += 1;
        p.ws(false);
        let v = p.value(0)?;
        let t = table_at(&mut root, &current);
        insert(t, &key, v);
        p.ws(false);
        match p.peek() {
            None | Some(b'\n') => {}
            Some(_) => return Err(p.err("trailing characters after value")),
        }
    }
    Ok(Val::Table(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents() {
        let doc = r#"
version = 1 # c
[[package]]
name = "a"
source = { registry = "https://x", "q k" = 'lit' }
deps = [
  { name = "b" }, # c
  { name = "c", marker = "python_version < \"3.11\"" },
]
[package.extras]
x = ["y (>=1)"]
[[package]]
name = "b"
description = """
multi
line"""
[metadata]
files.a = [{file = "a.whl", hash = "sha256:1"}]
"#;
        let v = parse(doc, LockfileFormat::Uv).unwrap();
        assert_eq!(v.scalar("version"), Some("1"));
        let pk = v.arr("package");
        assert_eq!(pk.len(), 2);
        assert_eq!(pk[0].get("source").unwrap().str("q k"), Some("lit"));
        assert_eq!(
            pk[0].arr("deps")[1].str("marker"),
            Some("python_version < \"3.11\"")
        );
        assert_eq!(
            pk[0].get("extras").unwrap().arr("x"),
            [Val::Str("y (>=1)".into())]
        );
        assert_eq!(pk[1].str("description"), Some("multi\nline"));
        let files = v.get("metadata").unwrap().get("files").unwrap();
        assert_eq!(files.arr("a")[0].str("hash"), Some("sha256:1"));
        for bad in [
            "x = ",
            "[a",
            "x = \"",
            "x = [1",
            "x = {a = 1",
            "= 1",
            "x = \"a\" b",
        ] {
            assert!(parse(bad, LockfileFormat::Uv).is_err(), "{bad}");
        }
    }
}
