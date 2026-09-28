//! Edits to the file where the package manager lists workspaces. Every edit
//! is textual so comments, catalogs, and formatting outside the edited list
//! survive.

use std::ops::Range;

use serde_json::Value;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::package_manager::PackageManager;

use super::Error;

/// Where the package manager lists workspaces.
#[derive(Debug)]
pub(super) enum WorkspaceDefinition {
    /// A `packages:` block sequence in `pnpm-workspace.yaml` or
    /// `aube-workspace.yaml`.
    Yaml(AbsoluteSystemPathBuf),
    /// The `workspaces` field of the root `package.json`, either an array or
    /// an object with a `packages` array.
    PackageJson(AbsoluteSystemPathBuf),
}

impl WorkspaceDefinition {
    pub(super) fn for_package_manager(
        package_manager: &PackageManager,
        repo_root: &AbsoluteSystemPath,
    ) -> Self {
        match package_manager.workspace_configuration_path() {
            Some(path) => Self::Yaml(repo_root.join_component(path)),
            None => Self::PackageJson(repo_root.join_component("package.json")),
        }
    }

    pub(super) fn from_file(file: AbsoluteSystemPathBuf) -> Self {
        if file.extension() == Some("json") {
            Self::PackageJson(file)
        } else {
            Self::Yaml(file)
        }
    }

    pub(super) fn path(&self) -> &AbsoluteSystemPath {
        match self {
            Self::Yaml(path) | Self::PackageJson(path) => path,
        }
    }

    /// The literal entry naming `package_path`, exactly as written. Glob
    /// entries never match.
    pub(super) fn find_entry(&self, package_path: &str) -> Result<Option<String>, Error> {
        let contents = self.read()?;
        let entries = match self {
            Self::Yaml(_) => yaml_entries(&contents)
                .map_err(|reason| self.unsupported(reason))?
                .into_iter()
                .map(|item| item.value.to_owned())
                .collect(),
            Self::PackageJson(_) => json_entries(&contents)?
                .map(|workspaces| string_entries(&workspaces.entries))
                .unwrap_or_default(),
        };
        Ok(entries
            .into_iter()
            .find(|entry| normalize(entry) == normalize(package_path)))
    }

    pub(super) fn remove_entry(&self, entry: &str) -> Result<(), Error> {
        let contents = self.read()?;
        let updated = match self {
            Self::Yaml(_) => remove_yaml_entry(&contents, entry),
            Self::PackageJson(_) => remove_json_entry(&contents, entry),
        }
        .map_err(|error| self.lift(error))?;
        self.write(&contents, &updated)
    }

    /// Adds `entry` unless an equivalent entry is already listed, so a retried
    /// unarchive does not duplicate it.
    pub(super) fn add_entry(&self, entry: &str) -> Result<(), Error> {
        let contents = self.read()?;
        let updated = match self {
            Self::Yaml(_) => add_yaml_entry(&contents, entry),
            Self::PackageJson(_) => add_json_entry(&contents, entry),
        }
        .map_err(|error| self.lift(error))?;
        self.write(&contents, &updated)
    }

    fn read(&self) -> Result<String, Error> {
        Ok(self.path().read_to_string()?)
    }

    fn write(&self, original: &str, updated: &str) -> Result<(), Error> {
        if original != updated {
            self.path().create_with_contents(updated)?;
        }
        Ok(())
    }

    fn unsupported(&self, reason: String) -> Error {
        Error::UnsupportedWorkspaceDefinition {
            file: self.path().to_string(),
            reason,
        }
    }

    fn lift(&self, error: EditError) -> Error {
        match error {
            EditError::Unsupported(reason) => self.unsupported(reason),
            EditError::Other(error) => error,
        }
    }
}

enum EditError {
    Unsupported(String),
    Other(Error),
}

impl From<Error> for EditError {
    fn from(error: Error) -> Self {
        Self::Other(error)
    }
}

impl From<turborepo_json_rewrite::RewriteError> for EditError {
    fn from(error: turborepo_json_rewrite::RewriteError) -> Self {
        Self::Other(error.into())
    }
}

fn normalize(entry: &str) -> &str {
    let entry = entry.strip_prefix("./").unwrap_or(entry);
    entry.trim_end_matches('/')
}

#[derive(Debug, PartialEq)]
struct YamlItem<'a> {
    /// Byte range of the whole line, including its line ending.
    line: Range<usize>,
    indent: &'a str,
    quote: Option<char>,
    value: &'a str,
}

struct PackagesBlock<'a> {
    /// Byte offset just past the `packages:` line.
    body_start: usize,
    items: Vec<YamlItem<'a>>,
}

fn yaml_entries(contents: &str) -> Result<Vec<YamlItem<'_>>, String> {
    Ok(packages_block(contents)?
        .map(|block| block.items)
        .unwrap_or_default())
}

fn packages_block(contents: &str) -> Result<Option<PackagesBlock<'_>>, String> {
    let mut offset = 0;
    let mut lines = contents.split_inclusive('\n');
    let body_start = loop {
        let Some(line) = lines.next() else {
            return Ok(None);
        };
        offset += line.len();
        if let Some(rest) = line.strip_prefix("packages:") {
            let rest = rest.trim();
            if !rest.is_empty() && !rest.starts_with('#') {
                return Err(
                    "the `packages` list must be a block sequence (`- entry` per line)".into(),
                );
            }
            break offset;
        }
    };

    let mut items = Vec::new();
    for line in lines {
        let start = offset;
        offset += line.len();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let content = line.trim_start();
        let Some(item) = content.strip_prefix('-') else {
            break;
        };
        let (quote, value) = unquote(strip_yaml_comment(item.trim()));
        items.push(YamlItem {
            line: start..offset,
            indent: &line[..line.len() - content.len()],
            quote,
            value,
        });
    }
    Ok(Some(PackagesBlock { body_start, items }))
}

fn strip_yaml_comment(value: &str) -> &str {
    if value.starts_with(['"', '\'']) {
        return value;
    }
    value.split(" #").next().unwrap_or(value).trim_end()
}

fn unquote(value: &str) -> (Option<char>, &str) {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.split(quote).next())
        {
            return (Some(quote), inner);
        }
    }
    (None, value)
}

fn remove_yaml_entry(contents: &str, entry: &str) -> Result<String, EditError> {
    let items = yaml_entries(contents).map_err(EditError::Unsupported)?;
    let Some(item) = items.iter().find(|item| item.value == entry) else {
        return Ok(contents.to_owned());
    };
    let mut updated = contents.to_owned();
    updated.replace_range(item.line.clone(), "");
    Ok(updated)
}

fn add_yaml_entry(contents: &str, entry: &str) -> Result<String, EditError> {
    let Some(block) = packages_block(contents).map_err(EditError::Unsupported)? else {
        return Err(EditError::Unsupported("it has no `packages` list".into()));
    };
    if block
        .items
        .iter()
        .any(|item| normalize(item.value) == normalize(entry))
    {
        return Ok(contents.to_owned());
    }
    let (indent, quote) = block
        .items
        .first()
        .map_or(("  ", Some('"')), |item| (item.indent, item.quote));
    let quote = quote.map(String::from).unwrap_or_default();
    let newline = if contents.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };

    let mut line = format!("{indent}- {quote}{entry}{quote}{newline}");
    if !contents[..block.body_start].ends_with('\n') {
        line.insert_str(0, newline);
    }
    let mut updated = contents.to_owned();
    updated.insert_str(block.body_start, &line);
    Ok(updated)
}

/// The workspace list in package.json and where it sits in the document.
struct JsonWorkspaces {
    path: &'static [&'static str],
    entries: Vec<Value>,
}

fn json_entries(contents: &str) -> Result<Option<JsonWorkspaces>, Error> {
    let package_json: Value = serde_json::from_str(contents)?;
    let (path, entries): (&'static [&'static str], _) = match package_json.get("workspaces") {
        Some(Value::Array(entries)) => (&["workspaces"], entries),
        Some(Value::Object(workspaces)) => match workspaces.get("packages") {
            Some(Value::Array(entries)) => (&["workspaces", "packages"], entries),
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some(JsonWorkspaces {
        path,
        entries: entries.clone(),
    }))
}

fn string_entries(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| entry.as_str().map(str::to_owned))
        .collect()
}

fn remove_json_entry(contents: &str, entry: &str) -> Result<String, EditError> {
    let Some(JsonWorkspaces { path, mut entries }) = json_entries(contents)? else {
        return Ok(contents.to_owned());
    };
    let before = entries.len();
    entries.retain(|existing| existing.as_str() != Some(entry));
    if entries.len() == before {
        return Ok(contents.to_owned());
    }
    rewrite_json_array(contents, path, &entries)
}

fn add_json_entry(contents: &str, entry: &str) -> Result<String, EditError> {
    let JsonWorkspaces { path, mut entries } = json_entries(contents)?.unwrap_or(JsonWorkspaces {
        path: &["workspaces"],
        entries: Vec::new(),
    });
    if string_entries(&entries)
        .iter()
        .any(|existing| normalize(existing) == normalize(entry))
    {
        return Ok(contents.to_owned());
    }
    entries.push(Value::String(entry.to_owned()));
    rewrite_json_array(contents, path, &entries)
}

fn rewrite_json_array(
    contents: &str,
    path: &[&str],
    entries: &[Value],
) -> Result<String, EditError> {
    let original = json_value_span(contents, path)?.map_or("[]", |span| &contents[span]);
    let array = format_json_array(original, entries);
    Ok(turborepo_json_rewrite::set_path(contents, path, &array)?)
}

/// Byte range of the value at `path`, located by the same traversal
/// `set_path` uses so the formatting read here belongs to the node it
/// replaces.
fn json_value_span(contents: &str, path: &[&str]) -> Result<Option<Range<usize>>, EditError> {
    const MARKER: &str = "\u{1}turbo-archive-marker\u{1}";
    if contents.contains(MARKER) {
        return Ok(None);
    }
    let marked = turborepo_json_rewrite::set_path(contents, path, MARKER)?;
    let Some(start) = marked.find(MARKER) else {
        return Ok(None);
    };
    let suffix = marked.len() - start - MARKER.len();
    Ok(Some(start..contents.len() - suffix))
}

/// Serializes `entries` in the layout of `original`: one line when it was on
/// one line, otherwise one element per line at its element indentation.
fn format_json_array(original: &str, entries: &[Value]) -> String {
    let rendered: Vec<String> = entries.iter().map(Value::to_string).collect();
    if !original.contains('\n') {
        let compact = original.contains(',') && !original.contains(", ");
        let separator = if compact { "," } else { ", " };
        return format!("[{}]", rendered.join(separator));
    }
    if rendered.is_empty() {
        return "[]".to_owned();
    }
    let newline = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let indent_of = |line: &str| line[..line.len() - line.trim_start().len()].to_owned();
    let closing_indent = original.lines().last().map(indent_of).unwrap_or_default();
    let element_indent = original
        .lines()
        .skip(1)
        .find(|line| {
            let line = line.trim();
            !line.is_empty() && line != "]"
        })
        .map(indent_of)
        .unwrap_or_else(|| format!("{closing_indent}  "));
    let body: Vec<String> = rendered
        .iter()
        .map(|entry| format!("{element_indent}{entry}"))
        .collect();
    format!(
        "[{newline}{}{newline}{closing_indent}]",
        body.join(&format!(",{newline}"))
    )
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    const PNPM_WORKSPACE: &str = "# workspace packages
packages:
  # apps
  - \"apps/*\"
  - 'packages/util' # keep
  - ./packages/another/

catalog:
  react: ^19.0.0
";

    fn values(contents: &str) -> Vec<&str> {
        yaml_entries(contents)
            .unwrap()
            .into_iter()
            .map(|item| item.value)
            .collect()
    }

    fn ok(result: Result<String, EditError>) -> String {
        match result {
            Ok(contents) => contents,
            Err(EditError::Unsupported(reason)) => panic!("unsupported: {reason}"),
            Err(EditError::Other(error)) => panic!("{error}"),
        }
    }

    #[test]
    fn yaml_entries_read_every_quote_style_and_stop_at_the_next_key() {
        assert_eq!(
            values(PNPM_WORKSPACE),
            ["apps/*", "packages/util", "./packages/another/"]
        );
        assert_eq!(values("packages:\n- a\n- b"), ["a", "b"]);
        assert!(values("catalog:\n  a: 1\n").is_empty());
    }

    #[test]
    fn yaml_flow_sequence_is_unsupported() {
        assert!(yaml_entries("packages: [apps/*, packages/*]\n").is_err());
    }

    #[test]
    fn remove_yaml_entry_removes_only_that_line() {
        let updated = ok(remove_yaml_entry(PNPM_WORKSPACE, "packages/util"));
        assert_eq!(
            updated,
            "# workspace packages\npackages:\n  # apps\n  - \"apps/*\"\n  - \
             ./packages/another/\n\ncatalog:\n  react: ^19.0.0\n"
        );
    }

    #[test]
    fn add_yaml_entry_follows_the_first_item_style() {
        let removed = ok(remove_yaml_entry(PNPM_WORKSPACE, "packages/util"));
        assert_eq!(
            ok(add_yaml_entry(&removed, "packages/util")),
            "# workspace packages\npackages:\n  - \"packages/util\"\n  # apps\n  - \"apps/*\"\n  \
             - ./packages/another/\n\ncatalog:\n  react: ^19.0.0\n"
        );
        assert_eq!(
            ok(add_yaml_entry("packages:\n- 'a'\n", "b")),
            "packages:\n- 'b'\n- 'a'\n"
        );
        assert_eq!(
            ok(add_yaml_entry("packages:", "b")),
            "packages:\n  - \"b\"\n"
        );
    }

    #[test]
    fn add_yaml_entry_skips_an_equivalent_entry() {
        assert_eq!(
            ok(add_yaml_entry(PNPM_WORKSPACE, "packages/another")),
            PNPM_WORKSPACE
        );
    }

    #[test]
    fn remove_json_entry_keeps_single_line_layout() {
        let contents = "{\n  \"name\": \"root\",\n  \"workspaces\": [\"apps/**\", \
                        \"packages/util\", \"packages/another\"]\n}\n";
        assert_eq!(
            ok(remove_json_entry(contents, "packages/another")),
            "{\n  \"name\": \"root\",\n  \"workspaces\": [\"apps/**\", \"packages/util\"]\n}\n"
        );
    }

    #[test]
    fn remove_json_entry_keeps_multi_line_layout() {
        let contents = "{\n  \"workspaces\": [\n    \"apps/**\",\n    \"packages/util\"\n  ],\n  \
                        \"private\": true\n}\n";
        assert_eq!(
            ok(remove_json_entry(contents, "packages/util")),
            "{\n  \"workspaces\": [\n    \"apps/**\"\n  ],\n  \"private\": true\n}\n"
        );
    }

    #[test]
    fn json_entries_under_workspaces_packages_round_trip() {
        let contents = "{\n  \"workspaces\": {\n    \"packages\": [\"a\", \"b\"],\n    \
                        \"nohoist\": []\n  }\n}\n";
        let removed = ok(remove_json_entry(contents, "b"));
        assert_eq!(
            removed,
            "{\n  \"workspaces\": {\n    \"packages\": [\"a\"],\n    \"nohoist\": []\n  }\n}\n"
        );
        assert_eq!(ok(add_json_entry(&removed, "b")), contents);
    }

    #[test]
    fn add_json_entry_skips_an_equivalent_entry() {
        let contents = "{\"workspaces\":[\"./packages/util/\"]}";
        assert_eq!(ok(add_json_entry(contents, "packages/util")), contents);
    }

    #[test]
    fn normalize_ignores_dot_prefix_and_trailing_slash() {
        assert_eq!(normalize("./packages/util/"), "packages/util");
        assert_eq!(normalize("packages/util"), "packages/util");
    }
}
