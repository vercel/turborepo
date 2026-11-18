//! Minimal ini and TOML table readers: what pytest's config files and uv's
//! workspace table need (string values and arrays of strings).

/// A config value: a string (ini, TOML string) or a TOML array of strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IniValue {
    /// A string.
    Str(String),
    /// An array.
    List(Vec<String>),
}

impl IniValue {
    /// Whitespace-separated items of a string, or the array items.
    pub fn list(&self) -> Vec<String> {
        match self {
            IniValue::Str(s) => s.split_whitespace().map(str::to_owned).collect(),
            IniValue::List(v) => v.clone(),
        }
    }

    /// Shell-split arguments (`addopts`).
    pub fn args(&self) -> Vec<String> {
        match self {
            IniValue::Str(s) => shell_split(s),
            IniValue::List(v) => v.iter().flat_map(|s| shell_split(s)).collect(),
        }
    }
}

/// POSIX-ish shell word splitting (quotes and backslashes).
pub(crate) fn shell_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut chars = s.chars();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                has = true;
            }
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    has = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The `key = value` pairs of ini section `name` (configparser style: `=` or
/// `:`, indented continuation lines, whole-line `#`/`;` comments). `None` if
/// the section is absent.
pub fn ini_section(text: &str, name: &str) -> Option<Vec<(String, IniValue)>> {
    let mut in_section = false;
    let mut found = false;
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !line.starts_with([' ', '\t']) {
            in_section = trimmed[1..trimmed.len() - 1].trim() == name;
            found |= in_section;
            continue;
        }
        if !in_section || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, v)) = out.last_mut() {
                v.push('\n');
                v.push_str(trimmed);
            }
            continue;
        }
        let split = line.find(['=', ':']);
        if let Some(i) = split {
            out.push((line[..i].trim().to_owned(), line[i + 1..].trim().to_owned()));
        }
    }
    found.then(|| {
        out.into_iter()
            .map(|(k, v)| (k, IniValue::Str(v)))
            .collect()
    })
}

/// The string and string-array values of TOML table `name` (dotted header).
/// Other value types are kept as their source text. `None` if the table is
/// absent.
pub fn toml_table(text: &str, name: &str) -> Option<Vec<(String, IniValue)>> {
    let mut lines = text.lines().peekable();
    let mut found = false;
    let mut out = Vec::new();
    let mut in_table = false;
    while let Some(line) = lines.next() {
        let t = line.trim();
        if t.starts_with('[') {
            let header = t.trim_start_matches('[').split(']').next().unwrap_or("");
            let header: String = header
                .split('.')
                .map(|p| p.trim().trim_matches('"'))
                .collect::<Vec<_>>()
                .join(".");
            in_table = !t.starts_with("[[") && header == name;
            found |= in_table;
            continue;
        }
        if !in_table || t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some(eq) = t.find('=') else {
            continue;
        };
        let key = t[..eq].trim().trim_matches('"').to_owned();
        let mut value = t[eq + 1..].trim().to_owned();
        // Multi-line arrays and strings.
        let open = |v: &str| -> bool {
            if v.starts_with("\"\"\"") || v.starts_with("'''") {
                let q = &v[..3];
                return v.len() < 6 || !v[3..].contains(q);
            }
            if v.starts_with('[') {
                return bracket_depth(v) > 0;
            }
            false
        };
        while open(&value) {
            let Some(next) = lines.next() else {
                break;
            };
            value.push('\n');
            value.push_str(next);
        }
        out.push((key, toml_value(&value)));
    }
    found.then_some(out)
}

/// Bracket depth at the end of `v`, ignoring brackets inside strings and
/// comments.
fn bracket_depth(v: &str) -> i32 {
    let mut depth = 0;
    let mut quote: Option<char> = None;
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('"'), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
            (None, '#') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    depth
}

fn toml_value(v: &str) -> IniValue {
    if v.starts_with('[') {
        let mut items = Vec::new();
        let mut rest = v;
        while let Some((s, after)) = next_toml_string(rest) {
            items.push(s);
            rest = after;
        }
        return IniValue::List(items);
    }
    match next_toml_string(v) {
        Some((s, _)) if v.starts_with(['"', '\'']) => IniValue::Str(s),
        _ => IniValue::Str(v.split('#').next().unwrap_or("").trim().to_owned()),
    }
}

/// The first TOML string literal in `v` and the text after it.
fn next_toml_string(v: &str) -> Option<(String, &str)> {
    let mut i = 0;
    let b = v.as_bytes();
    while i < b.len() {
        match b[i] {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            q @ (b'"' | b'\'') => {
                let triple = v[i..].starts_with(if q == b'"' { "\"\"\"" } else { "'''" });
                let ql = if triple { 3 } else { 1 };
                let body = i + ql;
                let mut j = body;
                let mut s = String::new();
                while j < b.len() {
                    if q == b'"' && b[j] == b'\\' && j + 1 < b.len() {
                        let n = v[j + 1..].chars().next().unwrap_or('\\');
                        s.push(match n {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                        j += 1 + n.len_utf8();
                        continue;
                    }
                    let close = if triple {
                        v[j..].starts_with(if q == b'"' { "\"\"\"" } else { "'''" })
                    } else {
                        b[j] == q
                    };
                    if close {
                        let text = if triple {
                            s.trim_start_matches('\n').to_owned()
                        } else {
                            s
                        };
                        return Some((text, &v[j + ql..]));
                    }
                    let c = v[j..].chars().next().unwrap_or(' ');
                    s.push(c);
                    j += c.len_utf8();
                }
                return None;
            }
            _ => i += 1,
        }
    }
    None
}
