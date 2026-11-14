//! A Dockerfile parser: parser directives (`# escape=`), line continuations,
//! comments, heredocs (`RUN <<EOF`, `COPY <<a <<b /dst/`), `--flags`, JSON
//! (exec) and shell forms, and Docker's variable substitution (`$VAR`,
//! `${VAR}`, `${VAR:-default}`, `${VAR:+alt}`, …).
//!
//! Never panics; instructions it cannot split are kept as shell-form text.

use std::collections::BTreeMap;

/// Maximum Dockerfile size read.
pub const MAX_FILE: usize = 4 * 1024 * 1024;

/// A here-document attached to an instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heredoc {
    /// The delimiter (`EOF`).
    pub name: String,
    /// The body (without the delimiter line), lines joined with `\n`.
    pub body: String,
}

/// Instruction arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Args {
    /// Exec form: `["npm", "run", "build"]`.
    Json(Vec<String>),
    /// Shell form: the text.
    Shell(String),
}

impl Args {
    /// Shell-form words (split on whitespace) or the JSON array.
    pub fn words(&self) -> Vec<String> {
        match self {
            Args::Json(v) => v.clone(),
            Args::Shell(s) => s.split_whitespace().map(str::to_string).collect(),
        }
    }
}

/// One instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    /// Uppercased keyword (`COPY`).
    pub keyword: String,
    /// Leading `--name[=value]` flags.
    pub flags: Vec<(String, Option<String>)>,
    /// Arguments after the flags.
    pub args: Args,
    /// Here-documents, in order.
    pub heredocs: Vec<Heredoc>,
    /// 1-based line of the instruction.
    pub line: usize,
}

impl Instruction {
    /// The value of flag `--name` (the last occurrence).
    pub fn flag(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    /// Every value of flag `--name`.
    pub fn flags_named(&self, name: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(n, _)| n == name)
            .filter_map(|(_, v)| v.as_deref())
            .collect()
    }
}

/// A parsed Dockerfile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dockerfile {
    /// Instructions in order.
    pub instructions: Vec<Instruction>,
    /// The escape character (`\` or `` ` ``).
    pub escape: char,
}

/// Parses Dockerfile text.
pub fn parse(src: &str) -> Dockerfile {
    let src = if src.len() > MAX_FILE {
        let mut end = MAX_FILE;
        while !src.is_char_boundary(end) {
            end -= 1;
        }
        &src[..end]
    } else {
        src
    };
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let lines: Vec<&str> = src
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut escape = '\\';
    let mut i = 0;
    // Parser directives: `# key=value` lines at the very top.
    while i < lines.len() {
        let t = lines[i].trim();
        let Some(rest) = t.strip_prefix('#') else {
            break;
        };
        let Some((k, v)) = rest.split_once('=') else {
            break;
        };
        let k = k.trim().to_ascii_lowercase();
        if k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric()) {
            break;
        }
        if k == "escape" {
            match v.trim() {
                "`" => escape = '`',
                "\\" => escape = '\\',
                _ => {}
            }
        }
        i += 1;
    }
    let mut out = Vec::new();
    while i < lines.len() {
        let first = lines[i];
        i += 1;
        let t = first.trim_start();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let line_no = i;
        // Join continuations (comment and empty lines inside are dropped).
        let mut logical = String::new();
        let mut cur = first.to_string();
        loop {
            let trimmed = cur.trim_end();
            if let Some(stripped) = trimmed.strip_suffix(escape) {
                logical.push_str(stripped);
                let mut next = None;
                while i < lines.len() {
                    let nx = lines[i];
                    i += 1;
                    let nt = nx.trim_start();
                    if nt.is_empty() || nt.starts_with('#') {
                        continue;
                    }
                    next = Some(nx.to_string());
                    break;
                }
                match next {
                    Some(n) => cur = n,
                    None => break,
                }
            } else {
                logical.push_str(&cur);
                break;
            }
        }
        let logical = logical.trim();
        let (kw, rest) = match logical.find(char::is_whitespace) {
            Some(p) => (&logical[..p], logical[p..].trim_start()),
            None => (logical, ""),
        };
        let keyword = kw.to_ascii_uppercase();
        // Flags.
        let mut flags = Vec::new();
        let mut rest = rest;
        while let Some(r) = rest.strip_prefix("--") {
            let end = r.find(char::is_whitespace).unwrap_or(r.len());
            let tok = &r[..end];
            match tok.split_once('=') {
                Some((n, v)) => flags.push((n.to_string(), Some(unquote(v)))),
                None => flags.push((tok.to_string(), None)),
            }
            rest = r[end..].trim_start();
        }
        // Heredocs.
        let mut heredocs = Vec::new();
        if matches!(keyword.as_str(), "RUN" | "COPY" | "ADD") {
            for (name, strip_tabs) in heredoc_markers(rest) {
                let mut body = Vec::new();
                while i < lines.len() {
                    let l = lines[i];
                    i += 1;
                    let cmp = if strip_tabs {
                        l.trim_start_matches('\t')
                    } else {
                        l
                    };
                    if cmp == name {
                        break;
                    }
                    body.push(if strip_tabs { cmp } else { l });
                }
                heredocs.push(Heredoc {
                    name,
                    body: body.join("\n"),
                });
            }
        }
        let args = if rest.starts_with('[') {
            match serde_json::from_str::<Vec<String>>(rest) {
                Ok(v) => Args::Json(v),
                Err(_) => Args::Shell(rest.to_string()),
            }
        } else {
            Args::Shell(rest.to_string())
        };
        out.push(Instruction {
            keyword,
            flags,
            args,
            heredocs,
            line: line_no,
        });
    }
    Dockerfile {
        instructions: out,
        escape,
    }
}

fn unquote(v: &str) -> String {
    let b = v.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// `<<EOF`, `<<-EOF`, `<<"EOF"`, `<<'EOF'` markers in an instruction line:
/// (name, strip leading tabs).
pub fn heredoc_markers(s: &str) -> Vec<(String, bool)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'<' && b[i + 1] == b'<' {
            if b.get(i + 2) == Some(&b'<') {
                i += 3;
                continue;
            }
            let mut j = i + 2;
            let strip = b.get(j) == Some(&b'-');
            if strip {
                j += 1;
            }
            let quote = match b.get(j) {
                Some(&q @ (b'"' | b'\'')) => {
                    j += 1;
                    Some(q)
                }
                _ => None,
            };
            let start = j;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            if j > start {
                let name = s[start..j].to_string();
                if let Some(q) = quote
                    && b.get(j) == Some(&q)
                {
                    j += 1;
                }
                out.push((name, strip));
            }
            i = j.max(i + 2);
            continue;
        }
        i += 1;
    }
    out
}

/// Known build variables: `Some(value)`, or `None` for a declared `ARG` without
/// a value.
pub type BuildVars = BTreeMap<String, Option<String>>;

/// Docker variable substitution. Returns the text and the names of variables
/// whose value is unknown (kept as `${NAME}` in the text).
pub fn substitute(s: &str, vars: &BuildVars, escape: char) -> (String, Vec<String>) {
    let mut unknown = Vec::new();
    let out = subst_inner(s, vars, escape, &mut unknown, 0);
    (out, unknown)
}

fn subst_inner(
    s: &str,
    vars: &BuildVars,
    escape: char,
    unknown: &mut Vec<String>,
    depth: usize,
) -> String {
    if !s.contains('$') || depth > 8 {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == escape && chars.get(i + 1) == Some(&'$') {
            out.push('$');
            i += 2;
            continue;
        }
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        match chars.get(i + 1) {
            Some('{') => {
                // Matching `}` (nested `${` in defaults allowed).
                let mut depth_b = 0;
                let mut j = i + 2;
                let mut end = None;
                while j < chars.len() {
                    match chars[j] {
                        '{' => depth_b += 1,
                        '}' if depth_b == 0 => {
                            end = Some(j);
                            break;
                        }
                        '}' => depth_b -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                let Some(end) = end else {
                    out.extend(&chars[i..]);
                    break;
                };
                let inner: String = chars[i + 2..end].iter().collect();
                let name_len = inner
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .count();
                let name = &inner[..name_len];
                let modifier = &inner[name_len..];
                let val = vars.get(name).cloned().flatten();
                let set = val.is_some();
                let nonempty = val.as_deref().is_some_and(|v| !v.is_empty());
                let sub = |w: &str, u: &mut Vec<String>| subst_inner(w, vars, escape, u, depth + 1);
                let r = if let Some(w) = modifier.strip_prefix(":-") {
                    if nonempty {
                        val.unwrap_or_default()
                    } else {
                        sub(w, unknown)
                    }
                } else if let Some(w) = modifier.strip_prefix('-') {
                    if set {
                        val.unwrap_or_default()
                    } else {
                        sub(w, unknown)
                    }
                } else if let Some(w) = modifier.strip_prefix(":+") {
                    if nonempty {
                        sub(w, unknown)
                    } else {
                        String::new()
                    }
                } else if let Some(w) = modifier.strip_prefix('+') {
                    if set { sub(w, unknown) } else { String::new() }
                } else if modifier.is_empty()
                    || modifier.starts_with(":?")
                    || modifier.starts_with('?')
                {
                    match val {
                        Some(v) => v,
                        None => {
                            unknown.push(name.to_string());
                            format!("${{{name}}}")
                        }
                    }
                } else {
                    // `#`, `%`, `/` pattern operators: not modelled.
                    unknown.push(name.to_string());
                    format!("${{{inner}}}")
                };
                out.push_str(&r);
                i = end + 1;
            }
            Some(&n) if n.is_ascii_alphabetic() || n == '_' => {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[i + 1..j].iter().collect();
                match vars.get(&name).cloned().flatten() {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push_str(&format!("${{{name}}}"));
                        unknown.push(name);
                    }
                }
                i = j;
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    out
}

/// Splits `ENV`/`LABEL` arguments into pairs: `K=V K2="v 2"` or the legacy `K
/// value…`.
pub fn env_pairs(s: &str) -> Vec<(String, String)> {
    let s = s.trim();
    let first_ws = s.find(char::is_whitespace).unwrap_or(s.len());
    if !s[..first_ws].contains('=') {
        // Legacy form.
        return vec![(s[..first_ws].to_string(), s[first_ws..].trim().to_string())];
    }
    let mut out = Vec::new();
    for w in shell_words(s) {
        if let Some((k, v)) = w.split_once('=') {
            out.push((k.to_string(), v.to_string()));
        }
    }
    out
}

/// Whitespace-separated words with `"…"`/`'…'` quoting and `\` escapes.
pub fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' | '\'' => {
                have = true;
                for d in it.by_ref() {
                    if d == c {
                        break;
                    }
                    cur.push(d);
                }
            }
            '\\' => {
                have = true;
                if let Some(d) = it.next() {
                    cur.push(d);
                }
            }
            c if c.is_whitespace() => {
                if have {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            c => {
                have = true;
                cur.push(c);
            }
        }
    }
    if have {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_continuations_flags() {
        let d = parse(
            "# syntax=docker/dockerfile:1\n# escape=\\\nFROM --platform=$BUILDPLATFORM node:20 AS \
             build\n# comment\nRUN apt-get update && \\\n  # inner comment\n  apt-get install -y \
             git\nCOPY --from=build --chown=node:node /app/dist ./dist\nCMD [\"node\", \
             \"dist/index.js\"]\n",
        );
        let i = &d.instructions;
        assert_eq!(i.len(), 4);
        assert_eq!(i[0].keyword, "FROM");
        assert_eq!(i[0].flag("platform"), Some("$BUILDPLATFORM"));
        assert_eq!(i[0].args, Args::Shell("node:20 AS build".into()));
        assert_eq!(
            i[1].args,
            Args::Shell("apt-get update &&   apt-get install -y git".into())
        );
        assert_eq!(i[2].flag("from"), Some("build"));
        assert_eq!(
            i[3].args,
            Args::Json(vec!["node".into(), "dist/index.js".into()])
        );
    }

    #[test]
    fn escape_directive() {
        let d = parse("# escape=`\nFROM mcr.microsoft.com/windows\nCOPY a.txt `\n  c:\\app\\\n");
        assert_eq!(d.escape, '`');
        assert_eq!(
            d.instructions[1].args,
            Args::Shell("a.txt   c:\\app\\".into())
        );
    }

    #[test]
    fn heredocs() {
        let d = parse(
            "FROM alpine\nRUN <<EOF\nset -e\npnpm install\nEOF\nCOPY <<-'A' <<B \
             /dst/\n\tone\n\tA\ntwo\nB\nRUN echo done\n",
        );
        let i = &d.instructions;
        assert_eq!(i.len(), 4);
        assert_eq!(
            i[1].heredocs,
            vec![Heredoc {
                name: "EOF".into(),
                body: "set -e\npnpm install".into()
            }]
        );
        assert_eq!(i[2].heredocs.len(), 2);
        assert_eq!(i[2].heredocs[0].body, "one");
        assert_eq!(i[2].heredocs[1].body, "two");
        assert_eq!(i[3].args, Args::Shell("echo done".into()));
    }

    #[test]
    fn substitution() {
        let mut v = BuildVars::new();
        v.insert("APP".into(), Some("web".into()));
        v.insert("EMPTY".into(), Some(String::new()));
        v.insert("NOVAL".into(), None);
        let s = |x: &str| substitute(x, &v, '\\');
        assert_eq!(s("apps/$APP/x").0, "apps/web/x");
        assert_eq!(
            s("${APP}-${EMPTY:-d}-${UNSET:-u}-${APP:+alt}").0,
            "web-d-u-alt"
        );
        assert_eq!(s("\\$APP").0, "$APP");
        let (t, u) = s("$NOVAL/$UNSET");
        assert_eq!(t, "${NOVAL}/${UNSET}");
        assert_eq!(u, ["NOVAL", "UNSET"]);
    }

    #[test]
    fn env_forms() {
        assert_eq!(
            env_pairs("A=1 B=\"two words\" C=x\\ y"),
            vec![
                ("A".into(), "1".into()),
                ("B".into(), "two words".into()),
                ("C".into(), "x y".into())
            ]
        );
        assert_eq!(
            env_pairs("NODE_ENV production"),
            vec![("NODE_ENV".into(), "production".into())]
        );
    }
}
