//! pip `requirements*.txt` (pinned, typically by `pip-compile` or `uv pip
//! compile`, optionally with `--hash`) parsed into the neutral [`Lockfile`]
//! model.
//!
//! # Model mapping
//!
//! | requirements.txt | Model |
//! |---|---|
//! | `name[extras]==version ; marker --hash=…` | [`LockedPackage`] `name@version` (canonical name, PEP 503); `integrity` = the sorted `--hash` values joined by spaces |
//! | `name @ url` | [`LockedPackage`] `name@url` with an empty version and `resolution` = the URL |
//! | `name>=1,<2` (unpinned) | [`LockedPackage`] `name` with an empty version; the range is the edge specifier |
//! | `-e path` / `-e url#egg=name` | [`Importer`] at the path (or the URL) |
//! | the file itself | the root [`Importer`] `"."` |
//! | `# via parent` annotations (pip-compile, `uv export`) | [`DepKind::Prod`] edges `parent → name`; requirements annotated `via -r file` / `-c file`, via a name not in the file (the project), or without annotations, are direct dependencies of `"."` |
//!
//! `-r`/`-c` includes are not followed (the parser sees one file); index and
//! install options are ignored. Physical lines ending in `\` continue; ` #`
//! starts a comment.

use std::collections::BTreeMap;

use crate::{
    Error, Result,
    model::{
        DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta, Target,
    },
};

/// PEP 503 name normalization: lowercase, runs of `-`, `_` and `.` become one
/// `-`.
pub fn canonicalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut sep = false;
    for c in name.trim().chars() {
        if matches!(c, '-' | '_' | '.') {
            sep = true;
        } else {
            if sep && !out.is_empty() {
                out.push('-');
            }
            sep = false;
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// One requirement line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Requirement {
    /// The name as written.
    pub name: String,
    /// Extras (`name[a,b]`).
    pub extras: Vec<String>,
    /// The version specifier as written (`==1.0`, `>=1,<2`), empty for direct
    /// references.
    pub specifier: String,
    /// The pinned version (`==V` or `===V` without wildcards).
    pub pinned: Option<String>,
    /// `name @ url`.
    pub url: Option<String>,
    /// The environment marker after `;`.
    pub marker: Option<String>,
    /// `--hash` values.
    pub hashes: Vec<String>,
    /// Parents from `# via` annotations (`-r file` entries included as
    /// written).
    pub via: Vec<String>,
    /// 1-based line.
    pub line: usize,
}

/// A parsed requirements file before mapping into the model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequirementsFile {
    /// Requirements in file order.
    pub requirements: Vec<Requirement>,
    /// `-e` targets.
    pub editables: Vec<String>,
    /// `-r` / `-c` includes (not followed).
    pub includes: Vec<String>,
}

fn syntax(line: usize, message: impl Into<String>) -> Error {
    Error::Syntax {
        format: LockfileFormat::Requirements,
        line,
        message: message.into(),
    }
}

/// Strips a ` #` comment (a `#` at the start or after whitespace).
fn strip_comment(s: &str) -> &str {
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'#' && (i == 0 || b[i - 1].is_ascii_whitespace()) {
            return &s[..i];
        }
    }
    s
}

/// Parses a requirements file into its raw form.
pub fn parse_raw(contents: &str) -> Result<RequirementsFile> {
    let mut out = RequirementsFile::default();
    let mut logical = String::new();
    let mut logical_line = 0usize;
    let mut in_via = false;
    let lines: Vec<&str> = contents.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let line_no = i + 1;
        i += 1;
        let trimmed = raw.trim();
        if logical.is_empty() && trimmed.starts_with('#') {
            let c = trimmed.trim_start_matches('#').trim();
            if let Some(rest) = c
                .strip_prefix("via")
                .filter(|r| r.is_empty() || r.starts_with([' ', '\t']))
            {
                let rest = rest.trim();
                in_via = rest.is_empty();
                if let Some(r) = out.requirements.last_mut()
                    && !rest.is_empty()
                {
                    r.via.extend(
                        rest.split(',')
                            .map(|s| s.trim().to_owned())
                            .filter(|s| !s.is_empty()),
                    );
                }
            } else if in_via && raw.starts_with([' ', '\t']) && !c.is_empty() {
                if let Some(r) = out.requirements.last_mut() {
                    // `#   -r requirements.in` or `#   parent` (may carry `(...)` notes).
                    let entry = c.split(" (").next().unwrap_or(c).trim().to_owned();
                    r.via.push(entry);
                }
            } else {
                in_via = false;
            }
            continue;
        }
        let content = strip_comment(raw);
        let (content, continues) = match content.trim_end().strip_suffix('\\') {
            Some(c) => (c, true),
            None => (content, false),
        };
        if logical.is_empty() {
            logical_line = line_no;
        }
        logical.push_str(content);
        logical.push(' ');
        if continues && i < lines.len() {
            continue;
        }
        let text = std::mem::take(&mut logical);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        in_via = false;
        line(text, logical_line, &mut out)?;
    }
    Ok(out)
}

fn line(text: &str, line_no: usize, out: &mut RequirementsFile) -> Result<()> {
    if text.starts_with('-') {
        let mut words = text.split_whitespace();
        let first = words.next().unwrap_or("");
        let (flag, inline) = match first.split_once('=') {
            Some((f, v)) => (f, Some(v.to_owned())),
            None => (first, None),
        };
        let value = || {
            inline
                .clone()
                .or_else(|| text.split_whitespace().nth(1).map(str::to_owned))
        };
        match flag {
            "-e" | "--editable" => {
                if let Some(v) = value() {
                    out.editables.push(v);
                }
            }
            "-r" | "--requirement" | "-c" | "--constraint" => {
                if let Some(v) = value() {
                    out.includes.push(v);
                }
            }
            _ if flag.starts_with("-e") && flag.len() > 2 => {
                out.editables.push(flag[2..].to_owned())
            }
            _ if flag.starts_with("-r") && flag.len() > 2 => {
                out.includes.push(flag[2..].to_owned())
            }
            _ => {}
        }
        return Ok(());
    }
    // Options trailing the requirement: `--hash=algo:hex` / `--hash algo:hex`.
    let mut hashes = Vec::new();
    let mut req_part = text;
    if let Some(p) = text.find(" --") {
        req_part = &text[..p];
        let mut words = text[p..].split_whitespace().peekable();
        while let Some(w) = words.next() {
            if let Some(h) = w.strip_prefix("--hash=") {
                hashes.push(h.to_owned());
            } else if w == "--hash"
                && let Some(h) = words.next()
            {
                hashes.push(h.to_owned());
            }
        }
    }
    let (req_part, marker) = match req_part.split_once(';') {
        // `;` inside a URL is not a marker separator unless followed by whitespace.
        Some((r, m)) if !r.contains("://") || r.ends_with(' ') || m.starts_with(' ') => {
            (r.trim(), Some(m.trim().to_owned()))
        }
        _ => (req_part.trim(), None),
    };
    let b = req_part.as_bytes();
    let mut j = 0;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'.' | b'_' | b'-')) {
        j += 1;
    }
    if j == 0 || !b[0].is_ascii_alphanumeric() || b.get(j).is_some_and(|c| *c == b'/') {
        // A bare URL or path (`./pkg`, `https://…/x.whl`): not a named requirement.
        return Err(syntax(
            line_no,
            format!("unsupported requirement {req_part:?}"),
        ));
    }
    let name = req_part[..j].to_owned();
    let mut rest = req_part[j..].trim_start();
    let mut extras = Vec::new();
    if let Some(r) = rest.strip_prefix('[') {
        let Some(end) = r.find(']') else {
            return Err(syntax(line_no, "unterminated extras"));
        };
        extras = r[..end]
            .split(',')
            .map(|e| e.trim().to_owned())
            .filter(|e| !e.is_empty())
            .collect();
        rest = r[end + 1..].trim_start();
    }
    let mut req = Requirement {
        name,
        extras,
        marker,
        hashes,
        line: line_no,
        ..Requirement::default()
    };
    if let Some(url) = rest.strip_prefix('@') {
        req.url = Some(url.trim().to_owned());
    } else {
        let spec: String = rest
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split_whitespace()
            .collect();
        req.pinned = spec
            .strip_prefix("===")
            .or_else(|| spec.strip_prefix("=="))
            .filter(|v| !v.contains([',', '*', '<', '>', '!', '~']))
            .map(str::to_owned);
        req.specifier = spec;
    }
    out.requirements.push(req);
    Ok(())
}

/// Parses a requirements file.
pub fn parse(contents: &str) -> Result<Lockfile> {
    Ok(to_lockfile(&parse_raw(contents)?))
}

fn id_of(r: &Requirement) -> (String, String) {
    let canon = canonicalize_name(&r.name);
    match (&r.pinned, &r.url) {
        (Some(v), _) => (format!("{canon}@{v}"), v.clone()),
        (None, Some(u)) => (format!("{canon}@{u}"), String::new()),
        (None, None) => (canon, String::new()),
    }
}

/// Maps a parsed file into the model.
pub fn to_lockfile(file: &RequirementsFile) -> Lockfile {
    let mut by_id: BTreeMap<String, LockedPackage> = BTreeMap::new();
    let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in &file.requirements {
        let (id, version) = id_of(r);
        let canon = canonicalize_name(&r.name);
        let p = by_id
            .entry(id.clone())
            .or_insert_with(|| LockedPackage::new(id.clone(), canon.clone(), version));
        let mut hashes: Vec<String> = p
            .integrity
            .iter()
            .flat_map(|h| h.split(' ').map(str::to_owned))
            .chain(r.hashes.iter().cloned())
            .collect();
        hashes.sort();
        hashes.dedup();
        p.integrity = (!hashes.is_empty()).then(|| hashes.join(" "));
        p.resolution = r.url.clone();
        let ids = by_name.entry(canon).or_default();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let annotated = file.requirements.iter().any(|r| !r.via.is_empty());
    let mut root = Importer::new(".");
    for r in &file.requirements {
        let (id, _) = id_of(r);
        let canon = canonicalize_name(&r.name);
        let mut spec = r.specifier.clone();
        if let Some(u) = &r.url {
            spec = format!("@ {u}");
        }
        if let Some(m) = &r.marker {
            spec.push_str(&format!("; {m}"));
        }
        let edge = DepEdge {
            name: canon,
            kind: DepKind::Prod,
            target: Target::Package(id),
            specifier: (!spec.is_empty()).then_some(spec),
        };
        // A parent that is not in the file is the project itself (`uv export` writes
        // `# via <project>`) or an input file: a direct dependency.
        let direct = !annotated
            || r.via.is_empty()
            || r.via.iter().any(|v| {
                v.starts_with("-r")
                    || v.starts_with("-c")
                    || !by_name.contains_key(&canonicalize_name(v))
            });
        for parent in r.via.iter().filter(|v| !v.starts_with('-')) {
            let pc = canonicalize_name(parent);
            for pid in by_name.get(&pc).cloned().unwrap_or_default() {
                if let Some(p) = by_id.get_mut(&pid)
                    && !p
                        .dependencies
                        .iter()
                        .any(|e| e.name == edge.name && e.target == edge.target)
                {
                    p.dependencies.push(edge.clone());
                }
            }
        }
        if direct && !root.dependencies.iter().any(|e| e.target == edge.target) {
            root.dependencies.push(edge);
        }
    }
    let mut importers = vec![root];
    for e in &file.editables {
        let mut imp = Importer::new(e.split('#').next().unwrap_or(e));
        imp.name = e
            .split_once("#egg=")
            .map(|(_, n)| canonicalize_name(n.split('&').next().unwrap_or(n)));
        importers.push(imp);
    }
    Lockfile::from_parts(
        LockfileFormat::Requirements,
        "1",
        by_id.into_values().collect(),
        importers,
        LockfileMeta::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Resolved, diff};

    const COMPILED: &str = r#"#
# This file is autogenerated by pip-compile with Python 3.12
#
--index-url https://pypi.org/simple

certifi==2024.8.30 \
    --hash=sha256:bbb \
    --hash=sha256:aaa
    # via requests
charset-normalizer==3.4.0 \
    --hash=sha256:ccc
    # via requests
Django==5.1.2 ; python_version >= "3.10" \
    --hash=sha256:ddd
    # via -r requirements.in
idna==3.10 --hash=sha256:iii
    # via requests
requests[socks]==2.32.3 \
    --hash=sha256:rrr
    # via
    #   -r requirements.in
    #   django-extra
urllib3==2.2.3  # pinned for CVE
    # via requests
-e ./libs/core#egg=Core_Lib
"#;

    #[test]
    fn names() {
        assert_eq!(canonicalize_name("Friendly-Bard"), "friendly-bard");
        assert_eq!(canonicalize_name("FRIENDLY._.bard"), "friendly-bard");
        assert_eq!(canonicalize_name("typing_extensions"), "typing-extensions");
    }

    #[test]
    fn parses_pip_compile_output() {
        let raw = parse_raw(COMPILED).unwrap();
        let r = &raw.requirements[4];
        assert_eq!(r.name, "requests");
        assert_eq!(r.extras, ["socks"]);
        assert_eq!(r.pinned.as_deref(), Some("2.32.3"));
        assert_eq!(r.via, ["-r requirements.in", "django-extra"]);
        assert_eq!(
            raw.requirements[2].marker.as_deref(),
            Some("python_version >= \"3.10\"")
        );
        let lock = to_lockfile(&raw);
        assert_eq!(lock.format(), LockfileFormat::Requirements);
        let ids: Vec<_> = lock.packages().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "certifi@2024.8.30",
                "charset-normalizer@3.4.0",
                "django@5.1.2",
                "idna@3.10",
                "requests@2.32.3",
                "urllib3@2.2.3"
            ]
        );
        assert_eq!(
            lock.package("certifi@2024.8.30")
                .unwrap()
                .integrity
                .as_deref(),
            Some("sha256:aaa sha256:bbb")
        );
        let root: Vec<_> = lock
            .importer(".")
            .unwrap()
            .dependencies
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(root, ["django", "requests"]);
        match lock.resolve_from_package("requests@2.32.3", "urllib3") {
            Some(Resolved::Package(p)) => assert_eq!(p.version, "2.2.3"),
            other => panic!("{other:?}"),
        }
        let closure: Vec<_> = lock
            .importer_closure(".", false)
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert_eq!(closure.len(), 6);
        let core = lock.importer("libs/core").unwrap();
        assert_eq!(core.name.as_deref(), Some("core-lib"));
    }

    #[test]
    fn unpinned_and_urls() {
        let lock =
            parse("flask>=2,<4\npkg @ https://x/pkg-1.0.whl\nold (==1.0)\n-r base.txt\n").unwrap();
        let ids: Vec<_> = lock.packages().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["flask", "old@1.0", "pkg@https://x/pkg-1.0.whl"]);
        let flask = lock.resolve(".", "flask");
        assert!(matches!(flask, Some(Resolved::Package(p)) if p.version.is_empty()));
        assert!(parse("./local/path\n").is_err());
        assert!(parse("x[a\n").is_err());
    }

    #[test]
    fn uv_export_via_project() {
        let lock = parse(
            "-e .\nanyio==4.6.0 \\\n    --hash=sha256:a\n    # via starlette\nfastapi==0.115.0 \
             \\\n    --hash=sha256:f\n    # via myproj\nstarlette==0.38.6 \\\n    \
             --hash=sha256:s\n    # via fastapi\n",
        )
        .unwrap();
        let root: Vec<_> = lock
            .importer(".")
            .unwrap()
            .dependencies
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(root, ["fastapi"]);
        assert_eq!(lock.importer_closure(".", false).len(), 3);
    }

    #[test]
    fn hash_change_is_an_identity_change() {
        let old = parse(COMPILED).unwrap();
        let new = parse(&COMPILED.replace("sha256:iii", "sha256:jjj")).unwrap();
        let d = diff(&old, &new);
        let changed: Vec<_> = d.changed.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(changed, ["idna@3.10"]);
    }
}
