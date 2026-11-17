//! Yarn classic (v1) `yarn.lock`: a custom indentation-based format.
//!
//! ```text
//! "@babel/code-frame@^7.0.0", "@babel/code-frame@^7.22.13":
//!   version "7.22.13"
//!   resolved "https://registry.yarnpkg.com/…#sha1"
//!   integrity sha512-…
//!   dependencies:
//!     "@babel/highlight" "^7.22.13"
//! ```
//!
//! Ids are `name@version`. When one name@version appears in several entries
//! with different sources (registry vs git), every member of that group gets
//! `name@version#<source>` so ids stay order independent. The lockfile has no
//! importer information; use [`Lockfile::add_manifest_importers`] with the
//! workspace `package.json` files.

use std::collections::{BTreeMap, HashMap};

use crate::{
    Error, Result,
    model::{DepEdge, DepKind, LockedPackage, Lockfile, LockfileFormat, LockfileMeta, Target},
    util::{split_descriptors, split_name_at},
};

#[derive(Default)]
struct Entry<'a> {
    descriptors: Vec<String>,
    version: &'a str,
    resolved: Option<&'a str>,
    integrity: Option<&'a str>,
    deps: Vec<(String, String, DepKind)>,
}

fn syntax(line: usize, message: impl Into<String>) -> Error {
    Error::Syntax {
        format: LockfileFormat::YarnClassic,
        line,
        message: message.into(),
    }
}

/// Splits `key value` (either may be quoted) into unquoted parts.
fn key_value(s: &str) -> (String, &str) {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('"') {
        let mut key = String::new();
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => {
                    if let Some((_, n)) = chars.next() {
                        key.push(n);
                    }
                }
                '"' => return (key, rest[i + 1..].trim()),
                c => key.push(c),
            }
        }
        (key, "")
    } else {
        match s.find([' ', '\t']) {
            Some(i) => (s[..i].to_string(), s[i..].trim()),
            None => (s.to_string(), ""),
        }
    }
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(v)
}

/// Parses a yarn classic `yarn.lock`.
pub fn parse(src: &str) -> Result<Lockfile> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut entries: Vec<Entry<'_>> = Vec::new();
    let mut section: Option<DepKind> = None;
    let mut ignore_section = false;
    for (n, raw) in src.lines().enumerate() {
        let line_no = n + 1;
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if line.contains('\t') && indent > 0 && line[..indent].contains('\t') {
            return Err(syntax(line_no, "tab in indentation"));
        }
        match indent {
            0 => {
                let Some(keys) = trimmed.strip_suffix(':') else {
                    return Err(syntax(line_no, "expected `descriptor[, descriptor]:`"));
                };
                entries.push(Entry {
                    descriptors: split_descriptors(keys),
                    ..Entry::default()
                });
                section = None;
                ignore_section = false;
            }
            2 => {
                let Some(e) = entries.last_mut() else {
                    return Err(syntax(line_no, "field outside of an entry"));
                };
                section = None;
                ignore_section = false;
                if let Some(k) = trimmed.strip_suffix(':') {
                    match unquote(k) {
                        "dependencies" => section = Some(DepKind::Prod),
                        "optionalDependencies" => section = Some(DepKind::Optional),
                        _ => ignore_section = true,
                    }
                    continue;
                }
                let (key, value) = key_value(trimmed);
                let value = unquote(value);
                match key.as_str() {
                    "version" => e.version = value,
                    "resolved" => e.resolved = Some(value),
                    "integrity" => e.integrity = Some(value),
                    _ => {}
                }
            }
            4 => {
                if ignore_section {
                    continue;
                }
                let (Some(e), Some(kind)) = (entries.last_mut(), section) else {
                    return Err(syntax(line_no, "unexpected nested field"));
                };
                let (name, range) = key_value(trimmed);
                e.deps.push((name, unquote(range).to_string(), kind));
            }
            _ if ignore_section => {}
            _ => return Err(syntax(line_no, format!("unexpected indentation {indent}"))),
        }
    }

    // Assign ids: name@version, disambiguated by source within colliding groups.
    let names: Vec<String> = entries
        .iter()
        .map(|e| {
            let d = e.descriptors.first().map(String::as_str).unwrap_or("");
            let (name, range) = split_name_at(d).unwrap_or((d, ""));
            // Alias (`string-width-cjs@npm:string-width@^4`): the package is the aliased
            // one.
            match range.strip_prefix("npm:").and_then(split_name_at) {
                Some((real, _)) => real.to_string(),
                None => name.to_string(),
            }
        })
        .collect();
    let mut groups: HashMap<(String, &str), Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        groups
            .entry((names[i].clone(), e.version))
            .or_default()
            .push(i);
    }
    let mut ids = vec![String::new(); entries.len()];
    for ((name, version), members) in &groups {
        let source = |i: usize| entries[i].integrity.or(entries[i].resolved).unwrap_or("");
        let distinct = members.iter().any(|&i| source(i) != source(members[0]));
        for &i in members {
            ids[i] = if distinct {
                format!("{name}@{version}#{}", source(i))
            } else {
                format!("{name}@{version}")
            };
        }
    }
    let mut descriptors = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        for d in &e.descriptors {
            descriptors.insert(d.clone(), ids[i].clone());
        }
    }
    let mut packages = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let mut pkg = LockedPackage::new(ids[i].clone(), names[i].clone(), e.version);
        pkg.integrity = e.integrity.map(str::to_string);
        pkg.resolution = e.resolved.map(str::to_string);
        for (name, range, kind) in &e.deps {
            let target = match descriptors.get(&format!("{name}@{range}")) {
                Some(id) => Target::Package(id.clone()),
                None => Target::Unresolved(range.clone()),
            };
            pkg.dependencies.push(DepEdge {
                name: name.clone(),
                kind: *kind,
                target,
                specifier: Some(range.clone()),
            });
        }
        packages.push(pkg);
    }
    let mut lock = Lockfile::from_parts(
        LockfileFormat::YarnClassic,
        "1",
        packages,
        Vec::new(),
        LockfileMeta::default(),
    );
    lock.set_descriptors(descriptors);
    Ok(lock)
}
