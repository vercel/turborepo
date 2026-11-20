//! Version-manager declarations that setup detects and reports as ignored:
//! asdf/mise `.tool-versions`, mise `mise.toml`/`.mise.toml`, and Volta's root
//! `package.json#volta`. Native declarations alone select tool versions; these
//! never become requests, lock provenance, or a fallback. Root-only, bounded,
//! and best effort: a malformed, oversized, or non-regular file is still
//! reported, never an error. No executables, network, or writes.

use std::io;

use serde_json::Value;

use crate::lock::DeclarationMap;

pub const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_REQUESTS: usize = 16;
const MAX_TOKEN_BYTES: usize = 64;
/// Root-relative files, in report order. `package.json#volta` follows them.
pub const FILES: [(&str, &str); 3] = [
    (".tool-versions", "asdf/mise"),
    ("mise.toml", "mise"),
    (".mise.toml", "mise"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub tool: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    pub source: &'static str,
    pub manager: &'static str,
    /// Printable requests in file order, at most `MAX_REQUESTS`.
    pub requests: Vec<Request>,
    pub omitted: usize,
    /// Present but not safely interpretable; `requests` is then empty.
    pub unreadable: bool,
}

/// `read` returns a root file's bytes, `None` when missing, or an error for an
/// unsafe, oversized, or unreadable file. `manifest` is the root package.json
/// already captured for native discovery.
pub fn discover(
    mut read: impl FnMut(&'static str) -> io::Result<Option<Vec<u8>>>,
    manifest: Option<&[u8]>,
) -> Vec<Ignored> {
    let mut found = Vec::new();
    for (source, manager) in FILES {
        let parsed = match read(source) {
            Ok(None) => continue,
            Ok(Some(bytes)) => String::from_utf8(bytes).ok().and_then(|text| {
                if source == ".tool-versions" {
                    Some(tool_versions(&text))
                } else {
                    mise(&text)
                }
            }),
            Err(_) => None,
        };
        push(&mut found, source, manager, parsed);
    }
    let volta = manifest
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .and_then(|manifest| manifest.get("volta").cloned())
        .map(|volta| {
            volta.as_object().map(|entries| {
                entries
                    .iter()
                    .map(|(tool, version)| (tool.clone(), version_text(version)))
                    .collect()
            })
        });
    if let Some(parsed) = volta {
        push(&mut found, "package.json#volta", "Volta", parsed);
    }
    found
}

fn push(
    found: &mut Vec<Ignored>,
    source: &'static str,
    manager: &'static str,
    parsed: Option<Vec<(String, String)>>,
) {
    let Some(entries) = parsed else {
        found.push(Ignored {
            source,
            manager,
            requests: Vec::new(),
            omitted: 0,
            unreadable: true,
        });
        return;
    };
    // A file without tool requests (only settings or comments) has nothing
    // for setup to ignore.
    if entries.is_empty() {
        return;
    }
    let omitted = entries.len().saturating_sub(MAX_REQUESTS);
    let requests = entries
        .into_iter()
        .take(MAX_REQUESTS)
        .map(|(tool, version)| Request {
            tool: printable(&tool),
            version: printable(&version),
        })
        .collect();
    found.push(Ignored {
        source,
        manager,
        requests,
        omitted,
        unreadable: false,
    });
}

// asdf: `tool version [fallback...]` per line, `#` starts a comment.
fn tool_versions(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split('#').next()?.split_whitespace();
            let tool = words.next()?;
            Some((tool.to_owned(), words.collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

// mise: `[tools]` values are a version, a list of versions, or a table with
// `version`. Anything else is reported without interpreting it.
fn mise(text: &str) -> Option<Vec<(String, String)>> {
    let document: toml::Table = text.parse().ok()?;
    let Some(tools) = document.get("tools") else {
        return Some(Vec::new());
    };
    let tools = tools.as_table()?;
    let version = |value: &toml::Value| match value {
        toml::Value::String(version) => version.clone(),
        toml::Value::Table(table) => table
            .get("version")
            .and_then(toml::Value::as_str)
            .unwrap_or("?")
            .to_owned(),
        _ => "?".into(),
    };
    Some(
        tools
            .iter()
            .map(|(tool, value)| {
                let text = match value {
                    toml::Value::Array(values) => {
                        values.iter().map(version).collect::<Vec<_>>().join(" ")
                    }
                    value => version(value),
                };
                (tool.clone(), text)
            })
            .collect(),
    )
}

fn version_text(value: &Value) -> String {
    value.as_str().unwrap_or("?").to_owned()
}

// Repository text reaches the terminal: printable ASCII only, bounded.
fn printable(text: &str) -> String {
    if text.len() <= MAX_TOKEN_BYTES && text.bytes().all(|b| b == b' ' || b.is_ascii_graphic()) {
        text.to_owned()
    } else {
        "(unprintable)".into()
    }
}

fn native_id(tool: &str) -> Option<&'static str> {
    match tool {
        "node" | "nodejs" => Some("node"),
        "npm" => Some("npm"),
        "pnpm" => Some("pnpm"),
        _ => None,
    }
}

/// A user-facing notice, or `None` when nothing was ignored. Requests for a
/// tool that native files also declare say that the native declaration wins.
pub fn report(ignored: &[Ignored], native: &DeclarationMap) -> Option<String> {
    if ignored.is_empty() {
        return None;
    }
    let mut lines = vec![
        "Ignored version manager declarations; turbo setup reads only native declarations:"
            .to_owned(),
    ];
    for file in ignored {
        let detail = if file.unreadable {
            "present but not read (unreadable, unsafe, or malformed)".to_owned()
        } else {
            let mut requests: Vec<_> = file
                .requests
                .iter()
                .map(|request| {
                    let mut text = request.tool.clone();
                    if !request.version.is_empty() {
                        text = format!("{text} {}", request.version);
                    }
                    match native_id(&request.tool).filter(|id| native.contains_key(*id)) {
                        Some(id) => format!("{text} (native {id} declaration takes precedence)"),
                        None => text,
                    }
                })
                .collect();
            if file.omitted > 0 {
                requests.push(format!("and {} more", file.omitted));
            }
            requests.join(", ")
        };
        lines.push(format!("  {} ({}): {detail}", file.source, file.manager));
    }
    lines.push(
        "Declare versions turbo setup should manage in native files such as .nvmrc, \
         .node-version, or package.json engines, devEngines, and packageManager."
            .into(),
    );
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests;
