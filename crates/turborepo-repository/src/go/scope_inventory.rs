//! Subprocess-free scope inventory for Go workspaces.
//!
//! [`super::discover_workspace`] — the authoritative path — delegates module
//! membership, module paths, replacements, and internal edges to the `go`
//! command (`go work edit -json`, `go mod edit -json`, `go mod graph`,
//! `go list -m all`). That is the only path that produces tasks, edges,
//! contracts, external resolution, and prune facts.
//!
//! Lazy core discovery needs far less to route a selection: *which* Go scopes
//! exist — their identities, their manifests, the `go-workspace` aggregate,
//! and the workspace root. It must learn that without spawning `go`, so
//! selections that never touch Go never invoke the Go toolchain at all. This
//! module supplies exactly that inventory by parsing the repository-root
//! `go.work` member list and each member's `go.mod` module directive.
//! Quotes, comments, and grouped blocks are honored, so the identities match
//! the authoritative observation: same member names, same aggregate name,
//! same workspace root.
//!
//! Replacements, requirements, and build tags are deliberately *not*
//! interpreted here. Their resolution is version- and toolchain-sensitive —
//! only `go` can decide it — so the inventory refuses to guess, and the
//! version-sensitive validations stay where they already are, unchanged, in
//! native discovery. Structural failures — a missing or malformed `go.mod`,
//! a duplicate module identity, a vendored or out-of-repository member, a
//! secondary workspace — fail the inventory the same way they fail native
//! discovery, because even the scope list would be untrustworthy.

use std::collections::HashMap;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use super::{
    Error, GO_MOD, GO_WORK, GO_WORKSPACE_NAME, package_name, resolve_member_dir,
    validate_single_workspace,
};
use crate::toolchain::{DiscoveredPackageScope, DiscoveredPackageScopes, WorkspaceRoot};

/// The directives the `go` command accepts in a `go.work` (verified against
/// Go 1.26: `go`, `toolchain`, `use`, `replace`, and `godebug`; directives
/// like `module`, `require`, `exclude`, `retract`, `env`, or typos such as
/// `unsupported` are rejected with "unknown directive"). Keep this list in
/// sync with the native parser when Go grows new workspace directives.
const GO_WORK_DIRECTIVES: [&str; 5] = ["go", "godebug", "replace", "toolchain", "use"];

/// The `use` entries of a parsed `go.work`, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedGoWork {
    uses: Vec<String>,
}

/// The `module` directive of a parsed `go.mod`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedGoMod {
    module_path: Option<String>,
}

/// One member of the workspace inventory.
struct Member {
    module_path: String,
    manifest_path: AbsoluteSystemPathBuf,
}

/// Collect the workspace's membership inventory: identities and manifests.
///
/// Broken workspaces — a missing or malformed `go.mod`, a duplicate identity,
/// a vendored member, a member outside the repository — fail here, exactly as
/// they fail native discovery, because the inventory would be untrustworthy.
fn collect_members(
    repo_root: &AbsoluteSystemPath,
    uses: &[String],
    root_module_path: Option<&str>,
) -> Result<Vec<Member>, Error> {
    let mut members = Vec::new();
    let mut identities: HashMap<String, String> = HashMap::new();

    for disk_path in uses {
        let member_dir = resolve_member_dir(repo_root, disk_path)?;
        let manifest = member_dir.join_component(GO_MOD);
        if !manifest.exists() {
            return Err(Error::MissingGoMod {
                path: member_dir.to_string(),
            });
        }
        if member_dir.join_component("vendor").exists() {
            return Err(Error::VendoredModule {
                path: member_dir.join_component("vendor").to_string(),
            });
        }

        let parsed = parse_go_mod(&read_manifest(&manifest)?, &manifest)?;
        let module_path = required_module_path(&parsed, &manifest)?.to_string();
        if package_name(&module_path) == GO_WORKSPACE_NAME {
            return Err(Error::WorkspaceNameCollision {
                name: GO_WORKSPACE_NAME.to_string(),
            });
        }
        if let Some(root_path) = root_module_path
            && root_path == module_path
        {
            return Err(Error::RootDefinitionCollision {
                path: member_dir.to_string(),
                module_path,
            });
        }
        if let Some(other_manifest) = identities.get(&module_path) {
            return Err(Error::DuplicateModuleIdentity {
                module_path,
                other_manifest: other_manifest.clone(),
            });
        }
        identities.insert(module_path.clone(), manifest.to_string());

        members.push(Member {
            module_path,
            manifest_path: manifest,
        });
    }

    Ok(members)
}

/// Discover Go scopes without invoking the `go` command: one scope per
/// `go.work` member — named by the shared module-path naming rule and pointing
/// at its `go.mod` manifest — plus the `go-workspace` aggregate at the
/// `go.work` manifest, and the workspace root the authoritative observation
/// reports.
///
/// Repositories without a `go.work` contribute no scopes and no root. The
/// result carries no tasks, edges, contracts, external resolution, or prune
/// facts: those come from [`super::GoContributor::discover_packages`] once a
/// selection actually needs Go.
pub(super) fn discover_package_scopes(
    repo_root: &AbsoluteSystemPath,
) -> Result<DiscoveredPackageScopes, Error> {
    let work_path = repo_root.join_component(GO_WORK);
    if !work_path.exists() {
        return Ok(DiscoveredPackageScopes::new(Vec::new(), Vec::new()));
    }

    let work = parse_go_work(&read_manifest(&work_path)?, &work_path)?;
    if work.uses.is_empty() {
        return Err(Error::EmptyWorkspace);
    }
    let member_dirs = work
        .uses
        .iter()
        .map(|disk_path| resolve_member_dir(repo_root, disk_path))
        .collect::<Result<Vec<_>, _>>()?;
    validate_single_workspace(repo_root, &member_dirs)?;

    let root_module_path = if repo_root.join_component(GO_MOD).exists() {
        let manifest = repo_root.join_component(GO_MOD);
        let parsed = parse_go_mod(&read_manifest(&manifest)?, &manifest)?;
        Some(required_module_path(&parsed, &manifest)?.to_string())
    } else {
        None
    };

    // Membership is exact, so the scope identities can be trusted for
    // routing: one scope per member, in `go.work` order.
    let members = collect_members(repo_root, &work.uses, root_module_path.as_deref())?;

    let mut scopes: Vec<DiscoveredPackageScope> = members
        .into_iter()
        .map(|member| {
            DiscoveredPackageScope::new(
                Some(package_name(&member.module_path).to_string()),
                member.manifest_path,
            )
        })
        .collect();
    // The workspace aggregate matches the authoritative observation: the same
    // reserved name, rooted at the `go.work` manifest.
    scopes.push(
        DiscoveredPackageScope::new(Some(GO_WORKSPACE_NAME.to_string()), work_path)
            .into_aggregate(),
    );

    Ok(DiscoveredPackageScopes::new(
        scopes,
        vec![WorkspaceRoot::new("go", repo_root.to_owned())],
    ))
}

fn required_module_path<'a>(
    parsed: &'a ParsedGoMod,
    manifest_path: &AbsoluteSystemPath,
) -> Result<&'a str, Error> {
    parsed
        .module_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| Error::MissingModulePath {
            path: manifest_path.to_string(),
        })
}

fn read_manifest(path: &AbsoluteSystemPath) -> Result<String, Error> {
    path.read_to_string().map_err(|source| Error::ManifestRead {
        path: path.to_string(),
        source,
    })
}

fn parse_go_work(text: &str, work_path: &AbsoluteSystemPath) -> Result<ParsedGoWork, Error> {
    let mut parsed = ParsedGoWork::default();
    let mut go: Option<String> = None;
    let mut toolchain: Option<String> = None;
    for (keyword, tokens) in
        scan_directives(text).map_err(|reason| Error::MalformedGoWork { reason })?
    {
        if !GO_WORK_DIRECTIVES.contains(&keyword.as_str()) {
            // The go command rejects the same file, but this diagnostic comes
            // from the inventory parser: it names the file and the offending
            // directive without pretending any command ran.
            return Err(Error::UnknownGoWorkDirective {
                path: work_path.to_string(),
                directive: keyword,
            });
        }
        match keyword.as_str() {
            "use" => {
                if tokens.len() != 1 {
                    return Err(Error::MalformedGoWork {
                        reason: format!("`use` expects one directory, found {}", tokens.len()),
                    });
                }
                parsed
                    .uses
                    .push(tokens.into_iter().next().unwrap_or_default());
            }
            // Each is a single token and may appear once; the values belong
            // to native discovery, which renders the pruned `go.work`.
            "go" | "toolchain" => {
                let value = match tokens.len() {
                    1 => tokens.into_iter().next().unwrap_or_default(),
                    count => {
                        return Err(Error::MalformedGoWork {
                            reason: format!("`{keyword}` expects one version, found {count}"),
                        });
                    }
                };
                let field = if keyword == "go" {
                    &mut go
                } else {
                    &mut toolchain
                };
                if field.replace(value).is_some() {
                    return Err(Error::MalformedGoWork {
                        reason: format!("duplicate `{keyword}` directive"),
                    });
                }
            }
            // `godebug` tunes runtime defaults and `replace` redirects
            // module resolution; neither changes which modules the workspace
            // contains, and replacement semantics are version-sensitive, so
            // the inventory accepts both without interpreting them.
            "godebug" | "replace" => {}
            _ => {}
        }
    }
    Ok(parsed)
}

fn parse_go_mod(text: &str, manifest_path: &AbsoluteSystemPath) -> Result<ParsedGoMod, Error> {
    let malformed = |reason: String| Error::MalformedGoMod {
        path: manifest_path.to_string(),
        reason,
    };
    let mut parsed = ParsedGoMod::default();
    for (keyword, tokens) in scan_directives(text).map_err(malformed)? {
        // `require`, `replace`, `exclude`, `retract`, `godebug`, `go`,
        // and `toolchain` never decide which module this manifest
        // declares, and their semantics are version- and
        // toolchain-sensitive, so the inventory skips them: native
        // discovery keeps every validation that depends on them.
        if keyword == "module" {
            if tokens.len() != 1 {
                return Err(malformed(format!(
                    "`module` expects one path, found {}",
                    tokens.len()
                )));
            }
            if parsed.module_path.is_some() {
                return Err(malformed(
                    "duplicate `module` directive; each go.mod declares one module".to_string(),
                ));
            }
            parsed.module_path = tokens.into_iter().next();
        }
    }
    Ok(parsed)
}

/// Flatten a `go.work`/`go.mod` file into `(keyword, entry tokens)` pairs,
/// unwrapping single and parenthesized block forms.
fn scan_directives(text: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut directives = Vec::new();
    let mut block: Option<String> = None;

    for line in logical_lines(text)? {
        let tokens = tokenize(&line)?;
        if tokens.is_empty() {
            continue;
        }

        if let Some(keyword) = block.as_ref() {
            if tokens.iter().any(|token| token == "(") {
                return Err(format!("nested `(` in `{}`", line.trim()));
            }
            if let Some(close) = tokens.iter().position(|token| token == ")") {
                if close + 1 != tokens.len() {
                    return Err(format!("unexpected tokens after `)` in `{}`", line.trim()));
                }
                if close > 0 {
                    directives.push((keyword.clone(), tokens[..close].to_vec()));
                }
                block = None;
            } else {
                directives.push((keyword.clone(), tokens));
            }
            continue;
        }

        let Some(open) = tokens.iter().position(|token| token == "(") else {
            if tokens.iter().any(|token| token == ")") {
                return Err(format!("unexpected `)` in `{}`", line.trim()));
            }
            let mut tokens = tokens.into_iter();
            let keyword = tokens.next().unwrap_or_default();
            let entry: Vec<String> = tokens.collect();
            if entry.is_empty() {
                return Err(format!("`{keyword}` is missing its operands"));
            }
            directives.push((keyword, entry));
            continue;
        };

        if open != 1 {
            return Err(format!("unexpected tokens before `(` in `{}`", line.trim()));
        }

        let keyword = tokens[0].clone();
        let inner = &tokens[open + 1..];
        if inner.iter().any(|token| token == "(") {
            return Err(format!("nested `(` in `{}`", line.trim()));
        }

        if let Some(close) = inner.iter().position(|token| token == ")") {
            if close + 1 != inner.len() {
                return Err(format!("unexpected tokens after `)` in `{}`", line.trim()));
            }
            if close > 0 {
                directives.push((keyword, inner[..close].to_vec()));
            }
        } else {
            if !inner.is_empty() {
                directives.push((keyword.clone(), inner.to_vec()));
            }
            block = Some(keyword);
        }
    }

    if let Some(keyword) = block {
        return Err(format!("unterminated `{keyword} (` block"));
    }
    Ok(directives)
}

/// Split a manifest into logical directive lines, keeping quoted and raw
/// strings intact and removing comments that appear outside of them.
///
/// A Go raw string may span newlines, so it is never broken into directive
/// lines, and carriage returns inside one are discarded as Go does. An
/// interpreted string cannot span lines; the newline is kept so tokenizing
/// reports the unterminated literal instead of silently merging directives.
fn logical_lines(text: &str) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut in_double = false;
    let mut in_raw = false;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if in_raw {
            match character {
                '\r' => {}
                '`' => {
                    in_raw = false;
                    current.push(character);
                }
                other => current.push(other),
            }
            continue;
        }
        if in_double {
            match character {
                '\\' => {
                    current.push(character);
                    if let Some(escaped) = chars.next() {
                        current.push(escaped);
                    }
                }
                '"' => {
                    in_double = false;
                    current.push(character);
                }
                other => current.push(other),
            }
            continue;
        }
        match character {
            '\n' => lines.push(std::mem::take(&mut current)),
            '"' => {
                in_double = true;
                current.push(character);
            }
            '`' => {
                in_raw = true;
                current.push(character);
            }
            '/' if chars.peek() == Some(&'/') => {
                while chars.peek().is_some_and(|next| *next != '\n') {
                    chars.next();
                }
            }
            other => current.push(other),
        }
    }
    if in_raw {
        return Err("unterminated raw string".to_string());
    }
    if in_double {
        return Err("unterminated quoted string".to_string());
    }
    lines.push(current);
    Ok(lines)
}

/// Split a directive line into whitespace-delimited tokens, decoding Go
/// interpreted (double-quoted) strings and backtick raw strings.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars
            .peek()
            .is_some_and(|character| character.is_whitespace())
        {
            chars.next();
        }
        let Some(&first) = chars.peek() else {
            break;
        };
        if first == '"' {
            chars.next();
            tokens.push(unquote_interpreted(&mut chars)?);
        } else if first == '`' {
            chars.next();
            let mut token = String::new();
            let mut terminated = false;
            for character in chars.by_ref() {
                if character == '`' {
                    terminated = true;
                    break;
                }
                token.push(character);
            }
            if !terminated {
                return Err("unterminated raw string".to_string());
            }
            tokens.push(token);
        } else {
            let mut token = String::new();
            while let Some(&character) = chars.peek() {
                if character.is_whitespace() {
                    break;
                }
                token.push(character);
                chars.next();
            }
            tokens.push(token);
        }
    }
    Ok(tokens)
}

type CharStream<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// Decode a Go interpreted string literal, starting after the opening quote.
///
/// Go's escape rules are reproduced so valid paths and module paths survive
/// byte-for-byte: `\\x61` must stay `a` rather than becoming `x61`, and unknown
/// escapes or invalid code points must fail instead of corrupting a path.
fn unquote_interpreted(chars: &mut CharStream<'_>) -> Result<String, String> {
    let mut bytes = Vec::new();
    while let Some(character) = chars.next() {
        match character {
            '"' => {
                return String::from_utf8(bytes)
                    .map_err(|_| "invalid UTF-8 in quoted string".to_string());
            }
            '\\' => decode_escape(chars, &mut bytes)?,
            '\n' | '\r' => return Err("newline in interpreted string literal".to_string()),
            '\0' => return Err("invalid NUL character in string literal".to_string()),
            other => {
                let mut encoded = [0; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut encoded).as_bytes());
            }
        }
    }
    Err("unterminated quoted string".to_string())
}

#[expect(
    clippy::expect_used,
    reason = "this match arm accepts only octal digits"
)]
fn decode_escape(chars: &mut CharStream<'_>, bytes: &mut Vec<u8>) -> Result<(), String> {
    let Some(escape) = chars.next() else {
        return Err("unterminated escape in quoted string".to_string());
    };

    match escape {
        'a' => bytes.push(b'\x07'),
        'b' => bytes.push(b'\x08'),
        'f' => bytes.push(b'\x0c'),
        'n' => bytes.push(b'\n'),
        'r' => bytes.push(b'\r'),
        't' => bytes.push(b'\t'),
        'v' => bytes.push(b'\x0b'),
        '\\' => bytes.push(b'\\'),
        '"' => bytes.push(b'"'),
        'x' => bytes.push(read_hex(chars, 2)? as u8),
        'u' => {
            let character = decode_unicode(read_hex(chars, 4)?, "\\u")?;
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
        'U' => {
            let character = decode_unicode(read_hex(chars, 8)?, "\\U")?;
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
        '0'..='7' => {
            let mut value = escape.to_digit(8).expect("octal digit");
            for _ in 0..2 {
                let Some(digit) = chars.next() else {
                    return Err("incomplete octal escape in quoted string".to_string());
                };
                let Some(digit) = digit.to_digit(8) else {
                    return Err("invalid octal escape in quoted string".to_string());
                };
                value = value * 8 + digit;
            }
            if value > 0xFF {
                return Err("octal escape out of range in quoted string".to_string());
            }
            bytes.push(value as u8);
        }
        other => return Err(format!("unknown escape `\\{other}` in quoted string")),
    }

    Ok(())
}

fn decode_unicode(value: u32, escape: &str) -> Result<char, String> {
    char::from_u32(value).ok_or_else(|| format!("invalid `{escape}` escape in quoted string"))
}

/// Read exactly `count` hexadecimal digits.
fn read_hex(chars: &mut CharStream<'_>, count: usize) -> Result<u32, String> {
    let mut value = 0u32;
    for _ in 0..count {
        let Some(digit) = chars.next() else {
            return Err("incomplete hexadecimal escape in quoted string".to_string());
        };
        let Some(digit) = digit.to_digit(16) else {
            return Err("invalid hexadecimal escape in quoted string".to_string());
        };
        value = value * 16 + digit;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a `go.mod` with `contents` at `root/<directory>/go.mod`,
    /// creating any missing parent directories. `directory` is a
    /// slash-separated relative path like `apps/api`; turbopath components
    /// must not contain separators, so it is split here.
    fn write_manifest(root: &AbsoluteSystemPath, directory: &str, contents: &str) {
        let mut segments: Vec<&str> = directory.split('/').collect();
        segments.push(GO_MOD);
        let manifest = root.join_components(&segments);
        manifest
            .parent()
            .expect("member manifest has a parent")
            .create_dir_all()
            .unwrap();
        manifest.create_with_contents(contents).unwrap();
    }

    /// Write a member module — a `go.mod` declaring `module_path` — below
    /// `root`, creating any missing parent directories.
    fn write_member(root: &AbsoluteSystemPath, directory: &str, module_path: &str) {
        write_manifest(
            root,
            directory,
            &format!("module {module_path}\n\ngo 1.22\n"),
        );
    }

    /// Write a repository-root `go.work` listing `directories` as members.
    fn write_work(root: &AbsoluteSystemPath, directories: &[&str]) {
        let mut work = String::from("go 1.22\n\nuse (\n");
        for directory in directories {
            work.push_str(&format!("\t{directory}\n"));
        }
        work.push_str(")\n");
        root.join_component(GO_WORK)
            .create_with_contents(work)
            .unwrap();
    }

    fn temp_root() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        (tempdir, root)
    }

    /// The repository-root `go.work` under test.
    fn work_file() -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::new(if cfg!(windows) {
            r"C:\repo\go.work"
        } else {
            "/repo/go.work"
        })
        .unwrap()
    }

    /// The manifest path used by parser unit tests.
    fn manifest_file() -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::new(if cfg!(windows) {
            r"C:\repo\go.mod"
        } else {
            "/repo/go.mod"
        })
        .unwrap()
    }

    // ---- Inventory, without invoking `go` ----

    #[test]
    fn inventories_member_scopes_aggregate_and_root_without_go() {
        let (_tempdir, root) = temp_root();
        write_work(&root, &["./apps/api", "./packages/lib"]);
        write_member(&root, "apps/api", "example.com/api");
        write_member(&root, "packages/lib", "example.com/lib");

        let scopes = discover_package_scopes(&root).unwrap();

        let roots = scopes.workspace_roots();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].kind(), "go");
        assert_eq!(roots[0].path().as_str(), root.as_str());

        // One scope per member, in `go.work` order, then the aggregate with
        // the same reserved name and manifest the authoritative observation
        // reports.
        let inventory: Vec<(Option<String>, String)> = scopes
            .scopes()
            .iter()
            .map(|scope| {
                (
                    scope.name().map(|name| name.to_string()),
                    scope.manifest_path().to_string(),
                )
            })
            .collect();
        assert_eq!(
            inventory,
            vec![
                (
                    Some("api".to_string()),
                    root.join_components(&["apps", "api", GO_MOD]).to_string()
                ),
                (
                    Some("lib".to_string()),
                    root.join_components(&["packages", "lib", GO_MOD])
                        .to_string()
                ),
                (
                    Some(GO_WORKSPACE_NAME.to_string()),
                    root.join_component(GO_WORK).to_string()
                ),
            ]
        );
    }

    #[test]
    fn inventory_preserves_versioned_names_and_full_module_paths_without_go() {
        let (_tempdir, root) = temp_root();
        let cases = [
            ("apps/api", "example.com/api", "api"),
            ("apps/api-v2", "example.com/team/api/v2", "api/v2"),
            ("apps/api-v10", "example.com/team/api/v10", "api/v10"),
            ("packages/yaml", "gopkg.in/yaml.v3", "yaml.v3"),
        ];
        let directories = cases
            .iter()
            .map(|(directory, _, _)| *directory)
            .collect::<Vec<_>>();
        write_work(&root, &directories);
        for (directory, module_path, _) in cases {
            write_member(&root, directory, module_path);
        }

        // Inventory membership retains Go's identity even though public scope names
        // shorten.
        let uses = directories
            .iter()
            .map(|directory| directory.to_string())
            .collect::<Vec<_>>();
        let members = collect_members(&root, &uses, None).unwrap();
        assert_eq!(members.len(), cases.len());
        for (member, (_, module_path, _)) in members.iter().zip(cases) {
            assert_eq!(member.module_path, module_path);
            let parsed = parse_go_mod(
                &read_manifest(&member.manifest_path).unwrap(),
                &member.manifest_path,
            )
            .unwrap();
            assert_eq!(parsed.module_path.as_deref(), Some(module_path));
        }

        let scopes = discover_package_scopes(&root).unwrap();
        assert_eq!(
            scopes
                .scopes()
                .iter()
                .map(|scope| scope.name())
                .collect::<Vec<_>>(),
            cases
                .iter()
                .map(|(_, _, name)| Some(*name))
                .chain(std::iter::once(Some(GO_WORKSPACE_NAME)))
                .collect::<Vec<_>>()
        );
        for (scope, member) in scopes.scopes().iter().zip(&members) {
            assert_eq!(
                scope.manifest_path().as_str(),
                member.manifest_path.as_str()
            );
        }
    }

    #[test]
    fn inventory_without_go_work_is_empty() {
        let (_tempdir, root) = temp_root();
        // A root go.mod alone is not a Go workspace: no scopes, no root, and
        // no `go` invocation either way.
        root.join_component(GO_MOD)
            .create_with_contents("module example.com/root\n\ngo 1.22\n")
            .unwrap();

        let scopes = discover_package_scopes(&root).unwrap();
        assert!(scopes.scopes().is_empty());
        assert!(scopes.workspace_roots().is_empty());
    }

    #[test]
    fn inventory_ignores_worktree_checked_out_inside_repository() {
        let (_tempdir, root) = temp_root();
        write_work(&root, &["./module"]);
        write_member(&root, "module", "example.com/module");

        // Go never reads this go.work for `module`: it is neither above the
        // member nor inside it.
        let worktree = root.join_components(&[".claude", "worktrees", "copy"]);
        worktree.create_dir_all().unwrap();
        write_work(&worktree, &["./module"]);
        write_member(&worktree, "module", "example.com/module");

        let scopes = discover_package_scopes(&root).unwrap();
        // The member, then the aggregate.
        assert_eq!(scopes.scopes().len(), 2);
    }

    #[test]
    fn inventory_reads_member_lists_with_quotes_comments_and_grouping() {
        let (_tempdir, root) = temp_root();
        write_member(&root, "apps/api", "example.com/api");
        write_member(&root, "packages/lib", "example.com/lib");
        root.join_component(GO_WORK)
            .create_with_contents(
                "go 1.22\n\nuse (\n\t// grouped members\n\t\"./apps/api\"\n\t`./packages/lib` // \
                 trailing\n)\n\ngodebug default=go1.22#1\n",
            )
            .unwrap();

        let scopes = discover_package_scopes(&root).unwrap();
        let names: Vec<Option<String>> = scopes
            .scopes()
            .iter()
            .map(|scope| scope.name().map(|name| name.to_string()))
            .collect();
        assert_eq!(
            names,
            vec![
                Some("api".to_string()),
                Some("lib".to_string()),
                Some(GO_WORKSPACE_NAME.to_string()),
            ]
        );
    }

    #[test]
    fn inventory_skips_require_and_replace_interpretation() {
        // Requirements and replacements — including version-sensitive ones
        // whose resolution only `go` can decide — never change which modules
        // the workspace contains, so the inventory lists scopes without
        // interpreting them; native discovery keeps every validation.
        let (_tempdir, root) = temp_root();
        write_manifest(
            &root,
            "api",
            "module example.com/api\n\ngo 1.22\n\nrequire example.com/alias v1.0.0\n",
        );
        root.join_component(GO_WORK)
            .create_with_contents(
                "go 1.22\n\nuse (\n\t./api\n\t./lib\n)\n\nreplace example.com/alias v1.0.0 => \
                 ./lib\n",
            )
            .unwrap();
        write_member(&root, "lib", "example.com/lib");

        let scopes = discover_package_scopes(&root).unwrap();
        let names: Vec<Option<String>> = scopes
            .scopes()
            .iter()
            .map(|scope| scope.name().map(|name| name.to_string()))
            .collect();
        assert_eq!(
            names,
            vec![
                Some("api".to_string()),
                Some("lib".to_string()),
                Some(GO_WORKSPACE_NAME.to_string()),
            ]
        );
    }

    #[test]
    fn inventory_preserves_membership_validation() {
        // Duplicate module identity.
        let (_tempdir, root) = temp_root();
        write_work(&root, &["./a", "./b"]);
        write_member(&root, "a", "example.com/shared");
        write_member(&root, "b", "example.com/shared");
        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::DuplicateModuleIdentity { .. })
        ));

        // Member outside the repository.
        let (_tempdir, root) = temp_root();
        root.join_component(GO_WORK)
            .create_with_contents("go 1.22\n\nuse ../outside\n")
            .unwrap();
        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::MemberOutsideRepository { .. })
        ));

        // Vendored member.
        let (_tempdir, root) = temp_root();
        write_work(&root, &["./module"]);
        write_member(&root, "module", "example.com/module");
        root.join_components(&["module", "vendor"])
            .create_dir_all()
            .unwrap();
        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::VendoredModule { .. })
        ));

        // Both a bare and a derived package name can collide with the aggregate.
        for module_path in [GO_WORKSPACE_NAME, "example.com/go-workspace"] {
            let (_tempdir, root) = temp_root();
            write_work(&root, &["./module"]);
            write_member(&root, "module", module_path);
            assert!(
                matches!(
                    discover_package_scopes(&root),
                    Err(Error::WorkspaceNameCollision { name }) if name == GO_WORKSPACE_NAME
                ),
                "{module_path} must not claim the reserved aggregate name"
            );
        }

        // Root module definition collision.
        let (_tempdir, root) = temp_root();
        root.join_component(GO_MOD)
            .create_with_contents("module example.com/root\n\ngo 1.22\n")
            .unwrap();
        write_work(&root, &["."]);
        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::RootDefinitionCollision { .. })
        ));

        // Secondary workspace.
        let (_tempdir, root) = temp_root();
        write_work(&root, &["./module"]);
        write_member(&root, "module", "example.com/module");
        root.join_components(&["module", GO_WORK])
            .create_with_contents("go 1.22\n")
            .unwrap();
        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::SecondaryWorkspace { .. })
        ));

        // Missing go.mod for a workspace member.
        let (_tempdir, root) = temp_root();
        root.join_component(GO_WORK)
            .create_with_contents("go 1.22\n\nuse ./missing\n")
            .unwrap();
        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::MissingGoMod { .. })
        ));

        // Unnamed module path.
        let (_tempdir, root) = temp_root();
        write_work(&root, &["./module"]);
        write_manifest(&root, "module", "go 1.22\n");
        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::MissingModulePath { .. })
        ));

        // Empty workspace.
        let (_tempdir, root) = temp_root();
        root.join_component(GO_WORK)
            .create_with_contents("go 1.22\n")
            .unwrap();
        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::EmptyWorkspace)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn inventory_rejects_member_symlink_outside_repository() {
        use std::os::unix::fs::symlink;

        let (tempdir, root) = temp_root();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(
            outside.path().join(GO_MOD),
            "module example.com/outside\n\ngo 1.22\n",
        )
        .unwrap();
        symlink(outside.path(), tempdir.path().join("linked")).unwrap();
        root.join_component(GO_WORK)
            .create_with_contents("go 1.22\n\nuse ./linked\n")
            .unwrap();

        assert!(matches!(
            discover_package_scopes(&root),
            Err(Error::MemberOutsideRepository { .. })
        ));
    }

    // ---- `go.work` and `go.mod` parsing ----

    #[test]
    fn preserves_utf8_bytes_and_all_inline_block_operands() {
        assert_eq!(
            tokenize(r#""\xc3\xa9 \303\251""#).unwrap(),
            vec!["é é".to_string()]
        );
        assert_eq!(
            scan_directives("use ( ./one\n./two\n)\n").unwrap(),
            vec![
                ("use".to_string(), vec!["./one".to_string()]),
                ("use".to_string(), vec!["./two".to_string()]),
            ]
        );
        assert!(scan_directives("use ( ./one ) ./two\n").is_err());
    }

    #[test]
    fn parses_single_and_block_use_directives_with_comments() {
        let work = r#"
go 1.22

toolchain go1.22.0

use ./api
use (
	./lib // trailing comment
	// a whole-line comment
	"./quoted dir"
	`./raw`
)

replace example.com/a => ./api
replace (
	example.com/b v1.0.0 => ./lib
	example.com/c => ./api
)
"#;
        // `replace` never decides membership and its semantics are
        // version-sensitive, so its directives are accepted without being
        // interpreted; only the `use` list is kept.
        let parsed = parse_go_work(work, &work_file()).unwrap();
        assert_eq!(parsed.uses, vec!["./api", "./lib", "./quoted dir", "./raw"]);
    }

    #[test]
    fn mirrors_the_go_commands_go_work_directive_allowlist() {
        // Verified against Go 1.26: `godebug` is accepted in go.work and
        // ignored for topology, while go.mod-only directives and typos are
        // rejected with the directive and file named — never a fake command
        // failure.
        let parsed = parse_go_work(
            "go 1.22\n\nuse ./api\n\ngodebug default=go1.22#1\n",
            &work_file(),
        )
        .unwrap();
        assert_eq!(parsed.uses, vec!["./api".to_string()]);

        for directive in [
            "unsupported",
            "module",
            "require",
            "exclude",
            "retract",
            "env",
        ] {
            let work = format!("go 1.22\n\n{directive} ./apps/api\n");
            match parse_go_work(&work, &work_file()) {
                Err(Error::UnknownGoWorkDirective {
                    path,
                    directive: found,
                }) => {
                    assert!(
                        path.ends_with("go.work"),
                        "diagnostic names the file: {path}"
                    );
                    assert_eq!(found, directive);
                }
                other => panic!("accepted `{directive}`: {other:?}"),
            }
        }
    }

    #[test]
    fn rejects_invalid_escapes_and_malformed_directives() {
        // Unknown escape, incomplete hex, out-of-range octal, and a surrogate
        // code point must all fail instead of corrupting a path.
        for work in [
            "go 1.22\n\nuse \"./a\\q\"\n",
            "go 1.22\n\nuse \"./a\\x6\"\n",
            "go 1.22\n\nuse \"./a\\400\"\n",
            "go 1.22\n\nuse \"./a\\ud800\"\n",
        ] {
            assert!(
                parse_go_work(work, &work_file()).is_err(),
                "accepted {work:?}"
            );
        }
        // An unterminated block and an operand-less directive must fail
        // instead of silently dropping the directive.
        assert!(parse_go_work("go 1.22\n\nuse (\n./a\n", &work_file()).is_err());
        assert!(parse_go_work("go 1.22\n\nuse\n", &work_file()).is_err());
        assert!(parse_go_work("go 1.22\n\nuse ./a ./b\n", &work_file()).is_err());
        assert!(parse_go_work("go 1.22\ngo 1.23\n", &work_file()).is_err());
        assert!(
            parse_go_work(
                "go 1.22\n\ntoolchain go1.22.0\ntoolchain go1.23.0\n",
                &work_file()
            )
            .is_err()
        );
        // A second `module` directive is rejected rather than silently
        // winning.
        assert!(
            parse_go_mod(
                "module example.com/a\nmodule example.com/b\n",
                &manifest_file()
            )
            .is_err()
        );
        assert!(parse_go_mod("module\n", &manifest_file()).is_err());
        assert!(parse_go_mod("module a b\n", &manifest_file()).is_err());
    }

    #[test]
    fn decodes_go_escapes_in_use_directives() {
        let work =
            "go 1.22\n\nuse \"./\\x61\\u0062\\U00000063\"\nuse \"./oct\\141\"\nuse `./raw dir`\n";
        let parsed = parse_go_work(work, &work_file()).unwrap();
        assert_eq!(parsed.uses, vec!["./abc", "./octa", "./raw dir"]);
    }

    #[test]
    fn keeps_multiline_raw_strings_and_ignores_comments_outside_strings() {
        // A raw string may span lines and is never split into directive
        // lines; carriage returns inside one are discarded as Go does.
        let work = "go 1.22\n\nuse `./multi\r\nline`\nuse \"./kept//not-comment\"\n// whole line \
                    comment\nuse ./after\n";
        let parsed = parse_go_work(work, &work_file()).unwrap();
        assert_eq!(
            parsed.uses,
            vec!["./multi\nline", "./kept//not-comment", "./after"]
        );
    }

    #[test]
    fn parses_module_directives_from_go_mod() {
        let go_mod = r#"module example.com/api

go 1.22

require example.com/one v1.0.0
require (
	example.com/two v2.0.0 // indirect
	example.com/three v0.1.0
)

exclude example.com/bad v1.0.0

replace example.com/one => ../one
replace (
	example.com/two v2.0.0 => example.com/fork v2.1.0
	example.com/three => `../three dir`
)
"#;
        // Only the module identity matters to the inventory; require and
        // replace forms are accepted without interpretation, and native
        // discovery keeps their version-sensitive validations.
        let parsed = parse_go_mod(go_mod, &manifest_file()).unwrap();
        assert_eq!(parsed.module_path.as_deref(), Some("example.com/api"));

        // Quoted module paths decode like Go string literals.
        let parsed = parse_go_mod("module \"example.com/quoted\"\n", &manifest_file()).unwrap();
        assert_eq!(parsed.module_path.as_deref(), Some("example.com/quoted"));
    }
}
