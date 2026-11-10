//! Reference closure of API specs and schemas: OpenAPI / JSON Schema `$ref`s
//! (JSON and YAML) and GraphQL `#import` lines, followed transitively through
//! local files.

use std::path::Path;

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::{
        codegen::config::{file_name, is_url, unknown},
        util::add_glob,
    },
    spec::TaskSpec,
};

/// Adds an API spec file (OpenAPI, JSON Schema, GraphQL SDL) and every local
/// file its `$ref`s / `#import`s reach (transitively) as inputs. URLs are
/// uncertainties (with a membership of `cfg_dir`), a glob is a membership, a
/// directory a `**` membership. Returns false when something was not statically
/// known.
pub fn add_spec(base: &Path, s: &str, cfg_dir: &Path, out: &mut TaskSpec) -> bool {
    if is_url(s) {
        unknown(
            out,
            cfg_dir,
            UncertaintyKind::EnvDependent,
            format!("remote spec `{s}`"),
        );
        return false;
    }
    if crate::glob::is_glob(s) || s.starts_with('!') {
        add_glob(base, s, "API spec", out);
        return true;
    }
    let p = fsx::join(base, s);
    if fsx::is_dir(&p) {
        out.membership(p, "**", "API spec directory");
        return true;
    }
    let mut todo = vec![p];
    let mut seen = std::collections::BTreeSet::new();
    let mut complete = true;
    while let Some(f) = todo.pop() {
        if !seen.insert(f.clone()) || seen.len() > 512 {
            continue;
        }
        out.input(f.clone());
        let Ok(text) = fsx::read_to_string(&f) else {
            continue;
        };
        let dir = f.parent().unwrap_or(base).to_path_buf();
        for r in spec_refs(&text) {
            if is_url(&r) {
                out.uncertain(
                    UncertaintyKind::EnvDependent,
                    format!("remote $ref `{r}` in {}", file_name(&f)),
                );
                complete = false;
                continue;
            }
            todo.push(fsx::join(&dir, &r));
        }
    }
    complete
}

/// Local file targets of `$ref: "file#/pointer"` (JSON and YAML) and GraphQL
/// `#import` lines in `text` (the part before `#`; internal `#/...` refs
/// skipped).
pub fn spec_refs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    for (i, _) in text.match_indices("$ref") {
        let mut j = i + 4;
        // `"$ref": "x"` or `$ref: x` or `$ref: 'x'`
        while j < b.len() && matches!(b[j], b'"' | b'\'' | b' ' | b'\t') {
            j += 1;
        }
        if j >= b.len() || b[j] != b':' {
            continue;
        }
        j += 1;
        while j < b.len() && matches!(b[j], b' ' | b'\t') {
            j += 1;
        }
        let quote = b.get(j).copied().filter(|c| matches!(c, b'"' | b'\''));
        if quote.is_some() {
            j += 1;
        }
        let start = j;
        while j < b.len() {
            let c = b[j];
            if Some(c) == quote
                || (quote.is_none() && matches!(c, b' ' | b'\n' | b'\r' | b',' | b'}' | b']'))
            {
                break;
            }
            if c == b'\n' {
                break;
            }
            j += 1;
        }
        let v = text[start..j].trim();
        let file = v.split('#').next().unwrap_or("");
        if !file.is_empty() {
            out.push(file.to_string());
        }
    }
    for line in text.lines() {
        // graphql-import style: `# import * from "x.graphql"` / `#import "./f.graphql"`
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix('#')
            && rest.trim_start().starts_with("import")
            && let Some(q) = rest.find(['"', '\''])
        {
            let r = &rest[q + 1..];
            if let Some(e) = r.find(['"', '\'']) {
                out.push(r[..e].to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs() {
        let y = "a:\n  $ref: './common.yaml#/X'\n  b:\n    \"$ref\": \"#/components/x\"\n  c: \
                 {$ref: other.json}\n";
        assert_eq!(spec_refs(y), ["./common.yaml", "other.json"]);
        assert_eq!(
            spec_refs("#import \"./frag.graphql\"\ntype A"),
            ["./frag.graphql"]
        );
    }
}
