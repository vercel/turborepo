//! Workspace discovery and monorepo hostname inference.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::auto::{ProjectNameError, infer_project_name, sanitize_for_hostname, truncate_label};

/// Metadata read from a workspace package's `package.json`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePackage {
    pub dir: PathBuf,
    /// Package name with an npm scope removed.
    pub name: Option<String>,
    /// Npm scope without the leading `@`.
    pub scope: Option<String>,
    pub scripts: BTreeMap<String, String>,
}

#[derive(Clone, Copy)]
enum WorkspaceSource {
    Pnpm,
    PackageJson,
}

/// Walk upward to find a pnpm, npm, Yarn, or Bun workspace root.
#[must_use]
pub fn find_workspace_root(cwd: impl AsRef<Path>) -> Option<PathBuf> {
    for dir in cwd.as_ref().ancestors() {
        if dir.join("pnpm-workspace.yaml").exists()
            || read_workspaces_from_package_json(dir).is_some()
        {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// Find a workspace root from the process's current directory.
#[must_use]
pub fn find_workspace_root_from_current_dir() -> Option<PathBuf> {
    std::env::current_dir().ok().and_then(find_workspace_root)
}

/// Read array or Yarn-classic object-form workspaces from `package.json`.
#[must_use]
pub fn read_workspaces_from_package_json(dir: impl AsRef<Path>) -> Option<Vec<String>> {
    let package: Value =
        serde_json::from_str(&fs::read_to_string(dir.as_ref().join("package.json")).ok()?).ok()?;
    let object = package.as_object()?;
    let workspaces = object.get("workspaces")?;
    let values = workspaces.as_array().or_else(|| {
        workspaces
            .as_object()
            .and_then(|object| object.get("packages"))
            .and_then(Value::as_array)
    })?;
    Some(
        values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    )
}

/// Discover package metadata from the workspace's supported glob subset.
#[must_use]
pub fn discover_workspace_packages(workspace_root: impl AsRef<Path>) -> Vec<WorkspacePackage> {
    let root = workspace_root.as_ref();
    let Some(source) = detect_workspace_source(root) else {
        return Vec::new();
    };
    let globs = match source {
        WorkspaceSource::Pnpm => fs::read_to_string(root.join("pnpm-workspace.yaml"))
            .ok()
            .map(|content| parse_pnpm_workspace_yaml(&content))
            .unwrap_or_default(),
        WorkspaceSource::PackageJson => read_workspaces_from_package_json(root).unwrap_or_default(),
    };

    expand_package_globs(root, &globs)
        .into_iter()
        .filter_map(read_workspace_package)
        .collect()
}

/// Parse the block-list or one-line flow-sequence subset used by pnpm
/// workspaces.
#[must_use]
pub fn parse_pnpm_workspace_yaml(content: &str) -> Vec<String> {
    let mut globs = Vec::new();
    let mut in_packages = false;

    for raw_line in content.lines() {
        let line = raw_line.trim_end();
        if let Some(rest) = line.strip_prefix("packages")
            && let Some(rest) = rest.trim_start().strip_prefix(':')
        {
            let rest = rest.trim();
            if rest.starts_with('[') {
                return parse_flow_sequence(rest);
            }
            in_packages = true;
            continue;
        }

        if !in_packages {
            continue;
        }
        if !line.is_empty() && !line.starts_with(char::is_whitespace) && !line.starts_with('-') {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(item) = trimmed.strip_prefix('-') else {
            continue;
        };
        if !item.starts_with(char::is_whitespace) {
            continue;
        }
        let item = strip_yaml_comment(item.trim());
        let item = strip_matching_quotes(item.trim()).trim();
        if !item.is_empty() {
            globs.push(item.to_owned());
        }
    }
    globs
}

/// Expand literal paths and single-segment `*` patterns, then apply exclusions.
#[must_use]
pub fn expand_package_globs<S: AsRef<str>>(root: impl AsRef<Path>, globs: &[S]) -> Vec<PathBuf> {
    let root = root.as_ref();
    let mut included = Vec::new();
    let mut excluded = Vec::new();

    for glob in globs {
        let glob = glob.as_ref();
        if let Some(negated) = glob.strip_prefix('!') {
            excluded.extend(expand_single_glob(root, negated));
        } else {
            for path in expand_single_glob(root, glob) {
                if !included.contains(&path) {
                    included.push(path);
                }
            }
        }
    }
    included.retain(|path| !excluded.contains(path));
    included.sort();
    included
}

/// Infer the monorepo base name: config, common npm scope, then root inference.
pub fn infer_monorepo_project_name(
    workspace_root: impl AsRef<Path>,
    packages: &[WorkspacePackage],
    configured_name: Option<&str>,
) -> Result<String, ProjectNameError> {
    if let Some(name) = configured_name {
        return Ok(name
            .split('.')
            .map(truncate_label)
            .collect::<Vec<_>>()
            .join("."));
    }

    let mut counts: Vec<(&str, usize)> = Vec::new();
    for scope in packages
        .iter()
        .filter_map(|package| package.scope.as_deref())
    {
        if let Some((_, count)) = counts.iter_mut().find(|(candidate, _)| *candidate == scope) {
            *count += 1;
        } else {
            counts.push((scope, 1));
        }
    }
    let mut common_scope = None;
    let mut max_count = 0;
    for (scope, count) in counts {
        if count > max_count {
            common_scope = Some(scope);
            max_count = count;
        }
    }
    if let Some(scope) = common_scope {
        let sanitized = sanitize_for_hostname(scope);
        if !sanitized.is_empty() {
            return Ok(sanitized);
        }
    }
    infer_project_name(workspace_root).map(|inferred| inferred.name)
}

/// Infer `<package>.<project>`, omitting a duplicate package label.
#[must_use]
pub fn infer_monorepo_hostname(
    package: &WorkspacePackage,
    workspace_root: impl AsRef<Path>,
    project_name: &str,
) -> String {
    let relative = package
        .dir
        .strip_prefix(workspace_root.as_ref())
        .map(normalize_path)
        .unwrap_or_else(|_| normalize_path(&package.dir));
    let package_label = package
        .name
        .as_deref()
        .map(sanitize_for_hostname)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| relative.replace('/', "-"));

    if package_label == project_name {
        project_name.to_owned()
    } else {
        format!("{package_label}.{project_name}")
    }
}

fn detect_workspace_source(root: &Path) -> Option<WorkspaceSource> {
    if root.join("pnpm-workspace.yaml").exists() {
        Some(WorkspaceSource::Pnpm)
    } else if read_workspaces_from_package_json(root).is_some() {
        Some(WorkspaceSource::PackageJson)
    } else {
        None
    }
}

fn read_workspace_package(dir: PathBuf) -> Option<WorkspacePackage> {
    let package: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("package.json")).ok()?).ok()?;
    let raw_name = package.get("name").and_then(Value::as_str);
    let (name, scope) = raw_name.map_or((None, None), |raw_name| {
        if raw_name.is_empty() {
            return (None, None);
        }
        if let Some(scoped) = raw_name.strip_prefix('@')
            && let Some((scope, name)) = scoped.split_once('/')
        {
            return (Some(name.to_owned()), Some(scope.to_owned()));
        }
        (Some(raw_name.to_owned()), None)
    });
    let scripts = package
        .get("scripts")
        .and_then(Value::as_object)
        .map(|scripts| {
            scripts
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .as_str()
                        .map(|command| (name.clone(), command.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(WorkspacePackage {
        dir,
        name,
        scope,
        scripts,
    })
}

fn parse_flow_sequence(input: &str) -> Vec<String> {
    input
        .strip_prefix('[')
        .unwrap_or(input)
        .strip_suffix(']')
        .unwrap_or_else(|| input.strip_prefix('[').unwrap_or(input))
        .split(',')
        .map(str::trim)
        .map(strip_matching_quotes)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn strip_yaml_comment(input: &str) -> &str {
    input.split_once('#').map_or(input, |(value, _)| value)
}

fn strip_matching_quotes(input: &str) -> &str {
    if input.len() >= 2 {
        let bytes = input.as_bytes();
        if (bytes[0] == b'\'' && bytes[input.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[input.len() - 1] == b'"')
        {
            return &input[1..input.len() - 1];
        }
    }
    input
}

fn expand_single_glob(root: &Path, glob: &str) -> Vec<PathBuf> {
    let segments: Vec<_> = glob.split('/').collect();
    expand_segments(root, &segments)
}

fn expand_segments(base: &Path, segments: &[&str]) -> Vec<PathBuf> {
    let Some((current, rest)) = segments.split_first() else {
        return if base.is_dir() {
            vec![base.to_path_buf()]
        } else {
            Vec::new()
        };
    };
    if !current.contains('*') {
        return expand_segments(&base.join(current), rest);
    }

    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };
    let mut matching: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| segment_matches(current, &entry.file_name().to_string_lossy()))
        .collect();
    matching.sort_by_key(fs::DirEntry::file_name);
    if rest.is_empty() {
        return matching.into_iter().map(|entry| entry.path()).collect();
    }
    matching
        .into_iter()
        .flat_map(|entry| expand_segments(&entry.path(), rest))
        .collect()
}

fn segment_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == "**" {
        return true;
    }
    let Some(star) = pattern.find('*') else {
        return pattern == name;
    };
    let prefix = &pattern[..star];
    let suffix = pattern[star + 1..].trim_end_matches('*');
    name.starts_with(prefix) && name.ends_with(suffix)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn parses_pnpm_block_and_flow_forms() {
        assert_eq!(
            parse_pnpm_workspace_yaml(
                "packages:\n  - 'apps/*' # apps\n  - '!apps/legacy'\ncatalog:\n  react: 19\n"
            ),
            ["apps/*", "!apps/legacy"]
        );
        assert_eq!(
            parse_pnpm_workspace_yaml("packages: [apps/*, \"packages/*\"]"),
            ["apps/*", "packages/*"]
        );
    }

    #[test]
    fn expands_limited_globs_and_exclusions() {
        let temp = TempDir::new().expect("temp dir");
        for dir in ["apps/team-web/src", "apps/team-api/src", "apps/legacy/src"] {
            fs::create_dir_all(temp.path().join(dir)).expect("directory");
        }
        assert_eq!(
            expand_package_globs(temp.path(), &["apps/*/src", "!apps/legacy/src"]),
            vec![
                temp.path().join("apps/team-api/src"),
                temp.path().join("apps/team-web/src")
            ]
        );
    }

    #[test]
    fn discovers_packages_and_infers_monorepo_names() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("package.json"),
            r#"{"workspaces":["apps/*"]}"#,
        )
        .expect("root package");
        for (dir, name) in [("web", "@acme/web"), ("acme", "@acme/acme")] {
            let package_dir = temp.path().join("apps").join(dir);
            fs::create_dir_all(&package_dir).expect("package dir");
            fs::write(
                package_dir.join("package.json"),
                format!(r#"{{"name":"{name}","scripts":{{"dev":"next dev"}}}}"#),
            )
            .expect("package");
        }

        let packages = discover_workspace_packages(temp.path());
        let project =
            infer_monorepo_project_name(temp.path(), &packages, None).expect("project name");
        assert_eq!(project, "acme");
        assert_eq!(
            infer_monorepo_hostname(&packages[0], temp.path(), &project),
            "acme"
        );
        assert_eq!(
            infer_monorepo_hostname(&packages[1], temp.path(), &project),
            "web.acme"
        );
    }
}
