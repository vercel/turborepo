//! Discovery of toolchain versions a repository declares.
//!
//! Nothing here is new configuration: these are the files each ecosystem
//! already uses to pin its toolchain, which turbo already watches and hashes
//! but never read as a version request until now.

use turbopath::AbsoluteSystemPath;
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};

use crate::{Error, http::Checksum, version::VersionSpec};

/// Package managers `turbo setup` can install from the npm registry or GitHub.
pub const SUPPORTED_PACKAGE_MANAGERS: [&str; 4] = ["npm", "pnpm", "yarn", "bun"];

/// A toolchain version request found in the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    Node {
        version: VersionSpec,
        source: String,
    },
    PackageManager {
        name: String,
        version: VersionSpec,
        /// Corepack-style hash of the registry tarball, when declared.
        checksum: Option<Checksum>,
        source: String,
    },
    Rust {
        channel: String,
        components: Vec<String>,
        targets: Vec<String>,
        profile: Option<String>,
        source: String,
    },
    Uv {
        version: VersionSpec,
        source: String,
    },
    Python {
        /// A uv Python request (`3.12`, `3.12.4`, `cpython@3.13`).
        request: String,
        source: String,
    },
    Go {
        version: VersionSpec,
        source: String,
    },
}

impl Declaration {
    /// Manifest key and display name.
    pub fn tool(&self) -> &str {
        match self {
            Self::Node { .. } => "node",
            Self::PackageManager { name, .. } => name,
            Self::Rust { .. } => "rust",
            Self::Uv { .. } => "uv",
            Self::Python { .. } => "python",
            Self::Go { .. } => "go",
        }
    }

    pub fn source(&self) -> &str {
        match self {
            Self::Node { source, .. }
            | Self::PackageManager { source, .. }
            | Self::Rust { source, .. }
            | Self::Uv { source, .. }
            | Self::Python { source, .. }
            | Self::Go { source, .. } => source,
        }
    }

    /// The request as written, for display.
    pub fn requested(&self) -> String {
        match self {
            Self::Node { version, .. }
            | Self::PackageManager { version, .. }
            | Self::Uv { version, .. }
            | Self::Go { version, .. } => version.to_string(),
            Self::Rust { channel, .. } => channel.clone(),
            Self::Python { request, .. } => request.clone(),
        }
    }
}

/// Finds every toolchain declaration in `repo_root`, in install order: a
/// runtime before the tools that run on it (Node.js before npm, uv before
/// Python).
pub fn discover(repo_root: &AbsoluteSystemPath) -> Result<Vec<Declaration>, Error> {
    let mut declarations = Vec::new();
    let package_json = read_package_json(repo_root)?;

    if let Some(node) = discover_node(repo_root, package_json.as_ref())? {
        declarations.push(node);
    }
    if let Some(package_manager) = package_json
        .as_ref()
        .map(discover_package_manager)
        .transpose()?
        .flatten()
    {
        declarations.push(package_manager);
    }
    if let Some(rust) = discover_rust(repo_root)? {
        declarations.push(rust);
    }
    if let Some(uv) = discover_uv(repo_root)? {
        declarations.push(uv);
    }
    if let Some(python) = discover_python(repo_root)? {
        declarations.push(python);
    }
    if let Some(go) = discover_go(repo_root)? {
        declarations.push(go);
    }
    Ok(declarations)
}

fn read_package_json(repo_root: &AbsoluteSystemPath) -> Result<Option<PackageJson>, Error> {
    let path = repo_root.join_component("package.json");
    if !path.exists() {
        return Ok(None);
    }
    PackageJson::load(&path)
        .map(Some)
        .map_err(|err| Error::Parse {
            path: path.to_string(),
            reason: err.to_string(),
        })
}

fn read_optional(repo_root: &AbsoluteSystemPath, name: &str) -> Result<Option<String>, Error> {
    let path = repo_root.join_component(name);
    path.read_existing_to_string()
        .map_err(|source| Error::io(path.as_str(), source))
}

/// First meaningful line of a `.nvmrc`-style file.
fn first_line(contents: &str) -> Option<&str> {
    contents
        .lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .find(|line| !line.is_empty())
}

// ---------------------------------------------------------------------------
// JavaScript
// ---------------------------------------------------------------------------

fn discover_node(
    repo_root: &AbsoluteSystemPath,
    package_json: Option<&PackageJson>,
) -> Result<Option<Declaration>, Error> {
    if let Some(declaration) = package_json.and_then(dev_engines_runtime).transpose()? {
        return Ok(Some(declaration));
    }
    for file in [".nvmrc", ".node-version"] {
        if let Some(contents) = read_optional(repo_root, file)? {
            let Some(raw) = first_line(&contents) else {
                continue;
            };
            return Ok(Some(Declaration::Node {
                version: node_version_spec(raw, file)?,
                source: file.to_string(),
            }));
        }
    }
    if let Some(range) = package_json
        .and_then(|pkg| pkg.engines())
        .and_then(|engines| engines.get("node").map(|value| value.to_string()))
    {
        let source = "package.json#engines.node";
        return Ok(Some(Declaration::Node {
            version: node_version_spec(&range, source)?,
            source: source.to_string(),
        }));
    }
    Ok(None)
}

fn node_version_spec(raw: &str, source: &str) -> Result<VersionSpec, Error> {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    if lower == "node" || lower == "latest" || lower == "current" || lower.starts_with("lts/") {
        return Ok(VersionSpec::Alias(lower));
    }
    if let Some(lts) = lower.strip_prefix("lts-") {
        return Ok(VersionSpec::Alias(format!("lts/{lts}")));
    }
    VersionSpec::from_semverish(raw).map_err(|reason| Error::InvalidVersion {
        tool: "node".into(),
        version: raw.to_string(),
        declared_in: source.to_string(),
        reason,
    })
}

/// `devEngines.runtime` (object or array of objects) naming `node`.
fn dev_engines_runtime(package_json: &PackageJson) -> Option<Result<Declaration, Error>> {
    let runtime = package_json.dev_engines.as_ref()?.get("runtime")?;
    let entries: Vec<&serde_json::Value> = match runtime {
        serde_json::Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let node = entries
        .into_iter()
        .find(|entry| entry.get("name").and_then(|name| name.as_str()) == Some("node"))?;
    let source = "package.json#devEngines.runtime";
    let version = node.get("version")?.as_str()?;
    Some(
        node_version_spec(version, source).map(|version| Declaration::Node {
            version,
            source: source.to_string(),
        }),
    )
}

fn discover_package_manager(package_json: &PackageJson) -> Result<Option<Declaration>, Error> {
    if let Some(field) = &package_json.package_manager {
        let source = "package.json#packageManager";
        let (name, version) =
            PackageManager::parse_package_manager_string(field).map_err(|err| {
                Error::InvalidVersion {
                    tool: "packageManager".into(),
                    version: field.value.clone(),
                    declared_in: source.into(),
                    reason: err.to_string(),
                }
            })?;
        if version.starts_with("http") {
            return Err(Error::Unsupported {
                tool: name.to_string(),
                reason: format!(
                    "`packageManager` points at a URL ({version}); turbo setup only installs \
                     versions published to the registry"
                ),
            });
        }
        let (version, metadata) = match version.split_once('+') {
            Some((version, metadata)) => (version, Some(metadata)),
            None => (version, None),
        };
        let checksum = metadata
            .map(Checksum::from_build_metadata)
            .transpose()?
            .flatten();
        return Ok(Some(package_manager_declaration(
            name,
            VersionSpec::Exact(version.to_string()),
            checksum,
            source,
        )?));
    }

    let Some(dev_engines) = package_json.dev_engines.as_ref() else {
        return Ok(None);
    };
    let Some(package_manager) = dev_engines.get("packageManager") else {
        return Ok(None);
    };
    let source = "package.json#devEngines.packageManager";
    let (Some(name), Some(version)) = (
        package_manager.get("name").and_then(|v| v.as_str()),
        package_manager.get("version").and_then(|v| v.as_str()),
    ) else {
        return Ok(None);
    };
    let version = VersionSpec::from_semverish(version).map_err(|reason| Error::InvalidVersion {
        tool: name.to_string(),
        version: version.to_string(),
        declared_in: source.into(),
        reason,
    })?;
    Ok(Some(package_manager_declaration(
        name, version, None, source,
    )?))
}

fn package_manager_declaration(
    name: &str,
    version: VersionSpec,
    checksum: Option<Checksum>,
    source: &str,
) -> Result<Declaration, Error> {
    if !SUPPORTED_PACKAGE_MANAGERS.contains(&name) {
        return Err(Error::Unsupported {
            tool: name.to_string(),
            reason: format!(
                "turbo setup can install {}; install `{name}` manually and make it available on \
                 PATH",
                SUPPORTED_PACKAGE_MANAGERS.join(", ")
            ),
        });
    }
    Ok(Declaration::PackageManager {
        name: name.to_string(),
        version,
        checksum,
        source: source.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Rust
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct RustToolchainFile {
    toolchain: Option<RustToolchainSection>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RustToolchainSection {
    channel: Option<String>,
    path: Option<String>,
    #[serde(default)]
    components: Vec<String>,
    #[serde(default)]
    targets: Vec<String>,
    profile: Option<String>,
}

fn discover_rust(repo_root: &AbsoluteSystemPath) -> Result<Option<Declaration>, Error> {
    if let Some(contents) = read_optional(repo_root, "rust-toolchain.toml")? {
        let source = "rust-toolchain.toml";
        let file: RustToolchainFile = toml::from_str(&contents).map_err(|err| Error::Parse {
            path: source.into(),
            reason: err.to_string(),
        })?;
        let section = file.toolchain.unwrap_or_default();
        if section.path.is_some() {
            return Err(Error::Unsupported {
                tool: "rust".into(),
                reason: "rust-toolchain.toml uses a custom `path` toolchain, which turbo setup \
                         cannot install"
                    .into(),
            });
        }
        let Some(channel) = section.channel.filter(|channel| !channel.trim().is_empty()) else {
            return Err(Error::Parse {
                path: source.into(),
                reason: "`[toolchain] channel` is required".into(),
            });
        };
        return Ok(Some(Declaration::Rust {
            channel: channel.trim().to_string(),
            components: section.components,
            targets: section.targets,
            profile: section.profile,
            source: source.into(),
        }));
    }
    if let Some(contents) = read_optional(repo_root, "rust-toolchain")? {
        // The legacy file is either a bare channel name or TOML.
        let trimmed = contents.trim();
        if trimmed.contains('[') {
            let file: RustToolchainFile = toml::from_str(trimmed).map_err(|err| Error::Parse {
                path: "rust-toolchain".into(),
                reason: err.to_string(),
            })?;
            let section = file.toolchain.unwrap_or_default();
            if let Some(channel) = section.channel {
                return Ok(Some(Declaration::Rust {
                    channel,
                    components: section.components,
                    targets: section.targets,
                    profile: section.profile,
                    source: "rust-toolchain".into(),
                }));
            }
            return Ok(None);
        }
        if let Some(channel) = first_line(trimmed) {
            return Ok(Some(Declaration::Rust {
                channel: channel.to_string(),
                components: Vec::new(),
                targets: Vec::new(),
                profile: None,
                source: "rust-toolchain".into(),
            }));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Python / uv
// ---------------------------------------------------------------------------

fn discover_uv(repo_root: &AbsoluteSystemPath) -> Result<Option<Declaration>, Error> {
    if let Some(contents) = read_optional(repo_root, "pyproject.toml")? {
        let value: toml::Value = toml::from_str(&contents).map_err(|err| Error::Parse {
            path: "pyproject.toml".into(),
            reason: err.to_string(),
        })?;
        if let Some(required) = value
            .get("tool")
            .and_then(|tool| tool.get("uv"))
            .and_then(|uv| uv.get("required-version"))
            .and_then(|v| v.as_str())
        {
            return uv_declaration(required, "pyproject.toml#tool.uv.required-version").map(Some);
        }
    }
    if let Some(contents) = read_optional(repo_root, "uv.toml")? {
        let value: toml::Value = toml::from_str(&contents).map_err(|err| Error::Parse {
            path: "uv.toml".into(),
            reason: err.to_string(),
        })?;
        if let Some(required) = value.get("required-version").and_then(|v| v.as_str()) {
            return uv_declaration(required, "uv.toml#required-version").map(Some);
        }
    }
    Ok(None)
}

fn uv_declaration(required: &str, source: &str) -> Result<Declaration, Error> {
    let semver = crate::version::pep440_to_semver(required);
    let version = VersionSpec::from_semverish(&semver).map_err(|reason| Error::InvalidVersion {
        tool: "uv".into(),
        version: required.to_string(),
        declared_in: source.into(),
        reason,
    })?;
    // `=0.5.1` is a pin; treat it as exact so no index lookup is needed.
    let version = match version {
        VersionSpec::Range(range) if range.starts_with('=') && !range.contains(' ') => {
            VersionSpec::from_semverish(range.trim_start_matches('='))
                .unwrap_or(VersionSpec::Range(range))
        }
        other => other,
    };
    Ok(Declaration::Uv {
        version,
        source: source.into(),
    })
}

fn discover_python(repo_root: &AbsoluteSystemPath) -> Result<Option<Declaration>, Error> {
    let Some(contents) = read_optional(repo_root, ".python-version")? else {
        return Ok(None);
    };
    let Some(request) = first_line(&contents) else {
        return Ok(None);
    };
    Ok(Some(Declaration::Python {
        request: request.to_string(),
        source: ".python-version".into(),
    }))
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

fn discover_go(repo_root: &AbsoluteSystemPath) -> Result<Option<Declaration>, Error> {
    for file in ["go.work", "go.mod"] {
        let Some(contents) = read_optional(repo_root, file)? else {
            continue;
        };
        if let Some(declaration) = parse_go_directives(&contents, file)? {
            return Ok(Some(declaration));
        }
    }
    Ok(None)
}

/// Reads the `toolchain` and `go` directives of a go.mod / go.work file. The
/// `toolchain` directive names the exact release to use; the `go` directive
/// is the minimum language version, which resolves to its newest patch.
fn parse_go_directives(contents: &str, file: &str) -> Result<Option<Declaration>, Error> {
    let mut go_directive = None;
    let mut toolchain_directive = None;
    for line in contents.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("toolchain"), Some(value)) => toolchain_directive = Some(value.to_string()),
            (Some("go"), Some(value)) => go_directive = Some(value.to_string()),
            _ => {}
        }
    }
    if let Some(toolchain) = toolchain_directive {
        if toolchain == "default" || toolchain == "local" {
            // Fall through to the go directive.
        } else {
            let source = format!("{file}#toolchain");
            let version = toolchain
                .strip_prefix("go")
                .ok_or_else(|| Error::InvalidVersion {
                    tool: "go".into(),
                    version: toolchain.clone(),
                    declared_in: source.clone(),
                    reason: "expected `toolchain goX.Y.Z`".into(),
                })?;
            return Ok(Some(Declaration::Go {
                version: go_version_spec(version),
                source,
            }));
        }
    }
    Ok(go_directive.map(|version| Declaration::Go {
        version: go_version_spec(&version),
        source: format!("{file}#go"),
    }))
}

/// `1.22` is a language version (newest patch wins); anything more specific
/// (`1.22.3`, `1.23rc1`) is an exact release.
fn go_version_spec(raw: &str) -> VersionSpec {
    let raw = raw.trim();
    let is_language_version = raw.split('.').count() == 2
        && raw
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    if is_language_version {
        VersionSpec::Range(raw.to_string())
    } else {
        VersionSpec::Exact(raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    fn repo() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        (tmp, root)
    }

    fn write(root: &AbsoluteSystemPath, name: &str, contents: &str) {
        root.join_component(name)
            .create_with_contents(contents)
            .unwrap();
    }

    #[test]
    fn empty_repository_has_no_declarations() {
        let (_tmp, root) = repo();
        assert!(discover(&root).unwrap().is_empty());
    }

    #[test]
    fn package_manager_field_with_corepack_hash() {
        let (_tmp, root) = repo();
        let digest = "0".repeat(56);
        write(
            &root,
            "package.json",
            &format!(r#"{{"packageManager": "pnpm@9.1.0+sha224.{digest}"}}"#),
        );
        let declarations = discover(&root).unwrap();
        assert_eq!(declarations.len(), 1);
        match &declarations[0] {
            Declaration::PackageManager {
                name,
                version,
                checksum,
                source,
            } => {
                assert_eq!(name, "pnpm");
                assert_eq!(version, &VersionSpec::Exact("9.1.0".into()));
                assert!(checksum.is_some());
                assert_eq!(source, "package.json#packageManager");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn dev_engines_declares_node_and_package_manager() {
        let (_tmp, root) = repo();
        write(
            &root,
            "package.json",
            r#"{"devEngines": {"runtime": {"name": "node", "version": "22.1.0"}, "packageManager": {"name": "npm", "version": "^10.5.0"}}}"#,
        );
        let declarations = discover(&root).unwrap();
        assert_eq!(
            declarations,
            vec![
                Declaration::Node {
                    version: VersionSpec::Exact("22.1.0".into()),
                    source: "package.json#devEngines.runtime".into(),
                },
                Declaration::PackageManager {
                    name: "npm".into(),
                    version: VersionSpec::Range("^10.5.0".into()),
                    checksum: None,
                    source: "package.json#devEngines.packageManager".into(),
                },
            ]
        );
    }

    #[test]
    fn nvmrc_wins_over_engines() {
        let (_tmp, root) = repo();
        write(&root, "package.json", r#"{"engines": {"node": ">=18"}}"#);
        write(&root, ".nvmrc", "# pinned\nv20.11.1\n");
        let declarations = discover(&root).unwrap();
        assert_eq!(
            declarations,
            vec![Declaration::Node {
                version: VersionSpec::Exact("20.11.1".into()),
                source: ".nvmrc".into(),
            }]
        );
    }

    #[test]
    fn engines_node_is_a_range_and_lts_is_an_alias() {
        let (_tmp, root) = repo();
        write(&root, "package.json", r#"{"engines": {"node": ">=18"}}"#);
        assert_eq!(
            discover(&root).unwrap(),
            vec![Declaration::Node {
                version: VersionSpec::Range(">=18".into()),
                source: "package.json#engines.node".into(),
            }]
        );
        write(&root, ".node-version", "lts/*");
        assert_eq!(
            discover(&root).unwrap()[0],
            Declaration::Node {
                version: VersionSpec::Alias("lts/*".into()),
                source: ".node-version".into(),
            }
        );
    }

    #[test]
    fn unsupported_package_manager_errors() {
        let (_tmp, root) = repo();
        write(&root, "package.json", r#"{"packageManager": "nub@1.0.0"}"#);
        assert!(matches!(
            discover(&root),
            Err(Error::Unsupported { tool, .. }) if tool == "nub"
        ));
        write(
            &root,
            "package.json",
            r#"{"packageManager": "pnpm@https://example.com/pnpm.tgz"}"#,
        );
        assert!(matches!(discover(&root), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn rust_toolchain_toml() {
        let (_tmp, root) = repo();
        write(
            &root,
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"nightly-2026-07-03\"\ncomponents = [\"rustfmt\", \
             \"clippy\"]\nprofile = \"minimal\"\n",
        );
        assert_eq!(
            discover(&root).unwrap(),
            vec![Declaration::Rust {
                channel: "nightly-2026-07-03".into(),
                components: vec!["rustfmt".into(), "clippy".into()],
                targets: vec![],
                profile: Some("minimal".into()),
                source: "rust-toolchain.toml".into(),
            }]
        );
    }

    #[test]
    fn legacy_rust_toolchain_file() {
        let (_tmp, root) = repo();
        write(&root, "rust-toolchain", "1.80.0\n");
        assert_eq!(discover(&root).unwrap()[0].requested(), "1.80.0");
    }

    #[test]
    fn uv_and_python() {
        let (_tmp, root) = repo();
        write(
            &root,
            "pyproject.toml",
            "[project]\nname = \"x\"\n[tool.uv]\nrequired-version = \">=0.5.0,<0.6\"\n",
        );
        write(&root, ".python-version", "3.12\n");
        assert_eq!(
            discover(&root).unwrap(),
            vec![
                Declaration::Uv {
                    version: VersionSpec::Range(">=0.5.0 <0.6".into()),
                    source: "pyproject.toml#tool.uv.required-version".into(),
                },
                Declaration::Python {
                    request: "3.12".into(),
                    source: ".python-version".into(),
                },
            ]
        );
        write(&root, "uv.toml", "required-version = \"==0.5.1\"\n");
        write(&root, "pyproject.toml", "[project]\nname = \"x\"\n");
        assert_eq!(
            discover(&root).unwrap()[0],
            Declaration::Uv {
                version: VersionSpec::Exact("0.5.1".into()),
                source: "uv.toml#required-version".into(),
            }
        );
    }

    #[test]
    fn go_work_toolchain_beats_go_directive() {
        let (_tmp, root) = repo();
        write(&root, "go.mod", "module example.com/m\n\ngo 1.21\n");
        write(
            &root,
            "go.work",
            "go 1.22 // comment\n\ntoolchain go1.22.5\n\nuse ./a\n",
        );
        assert_eq!(
            discover(&root).unwrap(),
            vec![Declaration::Go {
                version: VersionSpec::Exact("1.22.5".into()),
                source: "go.work#toolchain".into(),
            }]
        );
        root.join_component("go.work").remove_file().unwrap();
        assert_eq!(
            discover(&root).unwrap(),
            vec![Declaration::Go {
                version: VersionSpec::Range("1.21".into()),
                source: "go.mod#go".into(),
            }]
        );
    }
}
