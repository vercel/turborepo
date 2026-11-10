//! Static config loading and path helpers shared by the code generator
//! recognizers.
//!
//! Every config format is read into the lang-js evaluator's [`ConfigVal`]
//! model, so one set of accessors serves JSON, YAML, TOML and JS/TS configs:
//!
//! | Format | Reader |
//! |---|---|
//! | `.json`, `.jsonc`, rc files | `serde_json`, else the JS config evaluator (comments, trailing commas) |
//! | `.yml`, `.yaml` | `serde_yaml_ng` (merge keys applied) |
//! | `.toml` | `toml` |
//! | `.js`, `.ts`, ... | [`crate::recognizers::util::load_js_config`] (code configs are also JS entries) |
//!
//! Unknown parts (an env lookup, a function call, an open spread) stay
//! [`ConfigVal::Unknown`] and are reported by the caller as uncertainty.

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;
use turborepo_trace_lang_js::extract::config::{ConfigObj, ConfigVal};

pub use crate::scanners::refs::{add_spec, spec_refs};
use crate::{
    recognizers::util::{add_glob, load_js_config},
    spec::{EntryLang, Output, OutputKind, TaskSpec},
};

/// Loads the config file `path` (recorded as a config of `out`; a missing file
/// is a probe). Returns `None` when the file is missing or unreadable as its
/// format.
pub fn load(path: &Path, out: &mut TaskSpec) -> Option<ConfigVal> {
    out.config(path.to_path_buf());
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "js" | "cjs" | "mjs" | "ts" | "cts" | "mts" => {
            if !fsx::is_file(path) {
                return None;
            }
            load_js_config(path, out)
        }
        "yml" | "yaml" => {
            let s = fsx::read_to_string(path).ok()?;
            parse_yaml(&s)
        }
        "toml" => {
            let s = fsx::read_to_string(path).ok()?;
            parse_toml(&s)
        }
        _ => {
            let bytes = fsx::read(path).ok()?;
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                return Some(from_json(&v));
            }
            if ext.is_empty() {
                // rc files may be YAML.
                if let Some(v) = std::str::from_utf8(&bytes).ok().and_then(parse_yaml)
                    && matches!(v, ConfigVal::Obj(_))
                {
                    return Some(v);
                }
            }
            // JSONC: the JS evaluator reads it as an expression.
            load_js_json(path)
        }
    }
}

/// A JSON-with-comments file through the JS evaluator.
fn load_js_json(path: &Path) -> Option<ConfigVal> {
    let l = turborepo_trace_frameworks::jsconfig::load(
        path,
        turborepo_trace_frameworks::jsconfig::LoadOptions::default(),
    );
    l.value
}

/// Parses YAML into a [`ConfigVal`].
pub fn parse_yaml(s: &str) -> Option<ConfigVal> {
    let mut v: serde_yaml_ng::Value = serde_yaml_ng::from_str(s).ok()?;
    let _ = v.apply_merge();
    Some(from_yaml(&v, 0))
}

/// Parses TOML into a [`ConfigVal`].
pub fn parse_toml(s: &str) -> Option<ConfigVal> {
    let v: toml::Table = toml::from_str(s).ok()?;
    Some(from_toml(&toml::Value::Table(v), 0))
}

/// Converts JSON.
pub fn from_json(v: &serde_json::Value) -> ConfigVal {
    use serde_json::Value as J;
    match v {
        J::Null => ConfigVal::Null,
        J::Bool(b) => ConfigVal::Bool(*b),
        J::Number(n) => ConfigVal::Num(n.to_string()),
        J::String(s) => ConfigVal::str(s.clone()),
        J::Array(a) => ConfigVal::Arr(a.iter().map(from_json).collect()),
        J::Object(o) => ConfigVal::Obj(ConfigObj {
            entries: o.iter().map(|(k, v)| (k.clone(), from_json(v))).collect(),
            open: None,
        }),
    }
}

fn from_yaml(v: &serde_yaml_ng::Value, depth: usize) -> ConfigVal {
    use serde_yaml_ng::Value as Y;
    if depth > 64 {
        return ConfigVal::Unknown("nesting too deep".into());
    }
    match v {
        Y::Null => ConfigVal::Null,
        Y::Bool(b) => ConfigVal::Bool(*b),
        Y::Number(n) => ConfigVal::Num(n.to_string()),
        Y::String(s) => ConfigVal::str(s.clone()),
        Y::Sequence(a) => ConfigVal::Arr(a.iter().map(|x| from_yaml(x, depth + 1)).collect()),
        Y::Mapping(m) => ConfigVal::Obj(ConfigObj {
            entries: m
                .iter()
                .map(|(k, v)| (yaml_key(k), from_yaml(v, depth + 1)))
                .collect(),
            open: None,
        }),
        Y::Tagged(t) => from_yaml(&t.value, depth + 1),
    }
}

fn yaml_key(k: &serde_yaml_ng::Value) -> String {
    use serde_yaml_ng::Value as Y;
    match k {
        Y::String(s) => s.clone(),
        Y::Bool(b) => b.to_string(),
        Y::Number(n) => n.to_string(),
        Y::Null => "null".into(),
        _ => serde_yaml_ng::to_string(k)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn from_toml(v: &toml::Value, depth: usize) -> ConfigVal {
    use toml::Value as T;
    if depth > 64 {
        return ConfigVal::Unknown("nesting too deep".into());
    }
    match v {
        T::String(s) => ConfigVal::str(s.clone()),
        T::Integer(i) => ConfigVal::Num(i.to_string()),
        T::Float(f) => ConfigVal::Num(f.to_string()),
        T::Boolean(b) => ConfigVal::Bool(*b),
        T::Datetime(d) => ConfigVal::str(d.to_string()),
        T::Array(a) => ConfigVal::Arr(a.iter().map(|x| from_toml(x, depth + 1)).collect()),
        T::Table(t) => ConfigVal::Obj(ConfigObj {
            entries: t
                .iter()
                .map(|(k, v)| (k.clone(), from_toml(v, depth + 1)))
                .collect(),
            open: None,
        }),
    }
}

/// Property `key` of `v` (see [`ConfigVal::field`]); `None` when certainly
/// missing or null.
pub fn get(v: &ConfigVal, key: &str) -> Option<ConfigVal> {
    v.field(key).filter(|x| !x.is_null())
}

/// Nested property (`get_path(v, &["exec", "filename"])`).
pub fn get_path(v: &ConfigVal, keys: &[&str]) -> Option<ConfigVal> {
    let mut cur = v.clone();
    for k in keys {
        cur = get(&cur, k)?;
    }
    Some(cur)
}

/// A known string (a plain string or a cwd-relative path value).
pub fn as_str(v: &ConfigVal) -> Option<String> {
    match v {
        ConfigVal::CwdPath(x) => x.as_str().map(str::to_string),
        _ => v.as_str().map(str::to_string),
    }
}

/// The strings of a string-or-list value (alternatives flattened). Unknown
/// parts are returned as their reasons in the second list.
pub fn str_list(v: &ConfigVal) -> (Vec<String>, Vec<String>) {
    let mut out = Vec::new();
    let mut unknown = Vec::new();
    collect(v, &mut out, &mut unknown, 0);
    (out, unknown)
}

fn collect(v: &ConfigVal, out: &mut Vec<String>, unknown: &mut Vec<String>, depth: usize) {
    if depth > 8 {
        unknown.push("nesting too deep".into());
        return;
    }
    match v {
        ConfigVal::Null => {}
        ConfigVal::Arr(a) | ConfigVal::Alts(a) => {
            a.iter().for_each(|x| collect(x, out, unknown, depth + 1))
        }
        _ => match as_str(v) {
            Some(s) => out.push(s),
            None => {
                let mut rs = Vec::new();
                v.unknown_reasons(&mut rs);
                if rs.is_empty() {
                    rs.push(describe(v));
                }
                unknown.extend(rs);
            }
        },
    }
}

/// A short description of a value for uncertainty details.
pub fn describe(v: &ConfigVal) -> String {
    match v {
        ConfigVal::Obj(_) => "object".into(),
        ConfigVal::Num(n) => n.clone(),
        ConfigVal::Bool(b) => b.to_string(),
        ConfigVal::Function(_) => "function".into(),
        ConfigVal::Unknown(r) => r.to_string(),
        _ => "non-string value".into(),
    }
}

/// The entries of an object value (alternatives merged); `None` with a reason
/// when the key set is not known.
pub fn entries(v: &ConfigVal) -> Result<Vec<(String, ConfigVal)>, String> {
    match v {
        ConfigVal::Obj(o) => match &o.open {
            Some(r) => Err(format!("...{r}")),
            None => Ok(o.entries.clone()),
        },
        ConfigVal::Alts(vs) => {
            let mut out = Vec::new();
            for x in vs {
                if x.is_null() {
                    continue;
                }
                out.extend(entries(x)?);
            }
            Ok(out)
        }
        ConfigVal::Null => Ok(Vec::new()),
        other => {
            let mut rs = Vec::new();
            other.unknown_reasons(&mut rs);
            Err(rs.into_iter().next().unwrap_or_else(|| describe(other)))
        }
    }
}

/// Whether `s` is a URL (remote schema or spec).
pub fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("git@")
}

/// Records something that is not statically known: an uncertainty, plus every
/// file in `dir` (the config's directory) as a conservative membership.
pub fn unknown(out: &mut TaskSpec, dir: &Path, kind: UncertaintyKind, detail: impl Into<String>) {
    out.uncertain(kind, detail);
    out.membership(
        dir.to_path_buf(),
        "**",
        "code generator config not fully known",
    );
}

/// How a path-like config value is added.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum PathRole {
    /// Data input (schema files): a file is an input.
    Data,
    /// Code input (Drizzle schema modules): a JS/TS file is also an entry, so
    /// what it imports counts.
    Code,
}

/// Adds a schema/document location from a config: a URL is an uncertainty
/// (network), a glob a membership, a directory a `**` membership (or
/// `dir_pattern`), a file an input. Returns false when something was not
/// statically known.
pub fn add_location(
    base: &Path,
    s: &str,
    role: PathRole,
    dir_pattern: &str,
    reason: &str,
    cfg_dir: &Path,
    out: &mut TaskSpec,
) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return true;
    }
    if is_url(s) {
        unknown(
            out,
            cfg_dir,
            UncertaintyKind::EnvDependent,
            format!("remote {reason} `{s}`"),
        );
        return false;
    }
    if s.contains("${") || s.contains("$(") {
        unknown(
            out,
            cfg_dir,
            UncertaintyKind::EnvDependent,
            format!("{reason} `{s}` uses a variable"),
        );
        return false;
    }
    if let Some(neg) = s.strip_prefix('!') {
        add_glob(base, &format!("!{neg}"), reason, out);
        return true;
    }
    if crate::glob::is_glob(s) {
        add_glob(base, s, reason, out);
        if role == PathRole::Code {
            // The matching modules are loaded: what they import counts too.
            for f in expand_glob(base, s) {
                if EntryLang::of(&f) == EntryLang::Js {
                    out.entry(f, EntryLang::Js);
                }
            }
        }
        return true;
    }
    let p = fsx::join(base, s);
    if fsx::is_dir(&p) {
        out.membership(p.clone(), dir_pattern, reason);
        if role == PathRole::Code {
            for f in expand_glob(&p, dir_pattern) {
                if EntryLang::of(&f) == EntryLang::Js {
                    out.entry(f, EntryLang::Js);
                }
            }
        }
    } else {
        if role == PathRole::Code && EntryLang::of(&p) == EntryLang::Js {
            out.entry(p.clone(), EntryLang::Js);
        }
        out.input(p);
    }
    true
}

/// The existing files matching `pattern` (relative to `base`), at most 2000,
/// sorted. Walks only below the pattern's static prefix, skipping
/// `node_modules` and VCS directories.
pub fn expand_glob(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let (prefix, rest) = crate::glob::split_static_prefix(pattern);
    let root = fsx::join(base, prefix);
    let mut res = Vec::new();
    if rest.is_empty() {
        if fsx::is_file(&root) {
            res.push(root);
        }
        return res;
    }
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        for (p, is_dir) in fsx::read_dir(&d).unwrap_or_default() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_dir {
                if depth < 16 && !fsx::skip_dir(name) {
                    stack.push((p, depth + 1));
                }
            } else if let Ok(r) = p.strip_prefix(&root)
                && crate::glob::matches(rest, &r.to_string_lossy())
            {
                res.push(p);
                if res.len() >= 2000 {
                    res.sort();
                    return res;
                }
            }
        }
    }
    res.sort();
    res
}

/// Adds a generated output ([`OutputKind::Generated`]) with a glob below `dir`.
pub fn output_pattern(out: &mut TaskSpec, dir: PathBuf, pattern: &str, exclude: &[String]) {
    out.outputs.push(Output {
        path: dir,
        pattern: Some(pattern.to_string()),
        exclude: exclude.to_vec(),
        kind: OutputKind::Generated,
        part: None,
    });
}

/// The file name of `p` for messages.
pub fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The directory of a config file.
pub fn dir_of(p: &Path) -> PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_and_toml() {
        let v = parse_yaml("a: [x, y]\nb:\n  c: 1\n").unwrap();
        assert_eq!(str_list(&get(&v, "a").unwrap()).0, ["x", "y"]);
        let t = parse_toml("[tool.x]\ninput = \"s.json\"\n").unwrap();
        assert_eq!(
            as_str(&get_path(&t, &["tool", "x", "input"]).unwrap()).as_deref(),
            Some("s.json")
        );
    }
}
