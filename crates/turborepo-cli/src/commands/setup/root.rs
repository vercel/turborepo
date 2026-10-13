//! Read-only setup discovery, independent of package managers and tool probes.

use miette::Diagnostic;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::{
    bootstrap::{BootstrapError, BootstrapWorkspace, Registry},
    package_json::PackageJson,
    package_manager::PackageManager,
};
use turborepo_turbo_json::{FutureFlags, RawTurboJson};

mod discovery;
pub(super) use discovery::Discovery;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("ambiguous setup roots: {near} and {outer}")]
    #[diagnostic(
        code(turbo::setup::ambiguous_root),
        help(
            "Select the intended repository explicitly with turbo setup --cwd=<root>. No \
             neighboring .turbo/tools directory will be used."
        )
    )]
    Ambiguous {
        near: AbsoluteSystemPathBuf,
        outer: AbsoluteSystemPathBuf,
    },
    #[error("nested setup root marker {marker} conflicts with repository root {root}")]
    #[diagnostic(
        code(turbo::setup::nested_root),
        help(
            "Run turbo setup --cwd=<root> for the intended repository, or consolidate the nested \
             workspace into that root's workspace declaration and remove the secondary root \
             marker."
        )
    )]
    Nested {
        marker: AbsoluteSystemPathBuf,
        root: AbsoluteSystemPathBuf,
    },
    #[error("setup directory {cwd} is outside the selected root configuration {config}")]
    #[diagnostic(help(
        "Select a directory inside that repository with --cwd, or select its root configuration \
         with --root-turbo-json."
    ))]
    Outside {
        cwd: AbsoluteSystemPathBuf,
        config: AbsoluteSystemPathBuf,
    },
    #[error("cannot read setup root marker {path}: {reason}")]
    #[diagnostic(help(
        "Repair the indicated manifest or workspace declaration, then retry turbo setup. \
         Discovery does not invoke package managers or language tools."
    ))]
    Marker {
        path: AbsoluteSystemPathBuf,
        reason: String,
    },
    #[error("setup discovery cannot guard a custom root configuration outside snapshot scope")]
    #[diagnostic(
        code(turbo::setup::unsupported_config),
        help("Use a root turbo.json or turbo.jsonc before provisioning with a snapshot guard.")
    )]
    UnsupportedConfig,
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] turborepo_config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJson(#[from] turborepo_turbo_json::Error),
    #[error(transparent)]
    Bootstrap(#[from] BootstrapError),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

#[derive(PartialEq, Eq)]
struct SetupRoot {
    path: AbsoluteSystemPathBuf,
    flags: FutureFlags,
    config: Option<AbsoluteSystemPathBuf>,
}

/// `--cwd` is a discovery starting point. An exact root selected with it is
/// authoritative; a nested starting point still needs unambiguous inference.
/// Explicit config paths retain the global option's invocation-relative
/// meaning.
fn infer(
    cwd: &AbsoluteSystemPath,
    explicit_cwd: bool,
    config: Option<&AbsoluteSystemPath>,
) -> Result<SetupRoot, Error> {
    let cwd = cwd.to_realpath()?;
    let directories: Vec<_> = cwd
        .ancestors()
        .scan(false, |stopped, dir| {
            if *stopped {
                return None;
            }
            // A nested clone/worktree must never borrow its enclosing repo's tools.
            *stopped = dir.join_component(".git").exists();
            Some(dir.to_owned())
        })
        .collect();

    let (path, raw, selected_config) = if let Some(config) = config {
        let config = config.to_realpath()?;
        let root = config.parent().ok_or_else(|| {
            marker_error(
                &config,
                "expected a root configuration file, not a filesystem root",
            )
        })?;
        if !cwd.starts_with(root) {
            return Err(Error::Outside { cwd, config });
        }
        let raw = RawTurboJson::read(&cwd, &config, true)?.unwrap_or_default();
        (root.to_owned(), raw, Some(config))
    } else {
        let mut selected: Option<(
            AbsoluteSystemPathBuf,
            RawTurboJson,
            Option<AbsoluteSystemPathBuf>,
        )> = None;
        for dir in &directories {
            let config = turborepo_config::resolve_turbo_config_path(dir)?;
            let Some(raw) = read_root_config(&cwd, &config)? else {
                continue;
            };
            if let Some((near, _, _)) = &selected {
                return Err(Error::Ambiguous {
                    near: near.clone(),
                    outer: dir.clone(),
                });
            }
            selected = Some((dir.clone(), raw, Some(config)));
            if explicit_cwd && dir == &cwd {
                break;
            }
        }
        // Without root flags, non-JS markers are ignored. Keep the gate's
        // actionable diagnostic for repositories that have not opted in.
        selected.unwrap_or_else(|| {
            let path = directories
                .iter()
                .find(|dir| dir.join_component("package.json").exists())
                .unwrap_or(&cwd)
                .clone();
            (path, RawTurboJson::default(), None)
        })
    };
    let flags = raw
        .future_flags
        .map(|flags| *flags.as_inner())
        .unwrap_or_default();
    if flags.experimental_setup {
        let Ok(serde_json::Value::Object(serialized_flags)) = serde_json::to_value(flags) else {
            unreachable!("FutureFlags serializes to a JSON object");
        };
        let registry = Registry::from_flags(&serialized_flags);
        // Root recognition shares graph/inference diagnostics, but setup still
        // rejects nested markers rather than borrowing another root's tools.
        let root_workspaces = registry.probe(&path)?;
        for dir in &directories {
            if dir == &path {
                break;
            }
            // An explicit config must not cross a nested Git boundary either.
            if dir.join_component(".git").exists() {
                return Err(Error::Nested {
                    marker: dir.join_component(".git"),
                    root: path,
                });
            }
            if config.is_some() {
                let nested_config = turborepo_config::resolve_turbo_config_path(dir)?;
                if read_root_config(&cwd, &nested_config)?.is_some() {
                    return Err(Error::Nested {
                        marker: nested_config,
                        root: path,
                    });
                }
            }
            check_nested_markers(dir, &path, &registry, &root_workspaces)?;
        }
    }
    Ok(SetupRoot {
        path,
        flags,
        config: selected_config,
    })
}

fn read_root_config(
    cwd: &AbsoluteSystemPath,
    config: &AbsoluteSystemPath,
) -> Result<Option<RawTurboJson>, Error> {
    // Anchor diagnostics at the invocation directory, not each ancestor. Errors
    // outside that directory retain their full path instead of just turbo.json.
    // Package configurations inherit a root, rather than defining one.
    if RawTurboJson::read(cwd, config, false)
        .ok()
        .flatten()
        .is_some_and(|raw| raw.extends.is_some())
    {
        return Ok(None);
    }
    Ok(RawTurboJson::read(cwd, config, true)?)
}

fn marker_error(path: &AbsoluteSystemPath, reason: impl std::fmt::Display) -> Error {
    Error::Marker {
        path: path.to_owned(),
        reason: reason.to_string(),
    }
}

fn check_nested_markers(
    dir: &AbsoluteSystemPath,
    root: &AbsoluteSystemPath,
    registry: &Registry,
    root_workspaces: &[BootstrapWorkspace],
) -> Result<(), Error> {
    let nested = |name| Error::Nested {
        marker: dir.join_component(name),
        root: root.to_owned(),
    };
    if dir.join_component("pnpm-workspace.yaml").exists() {
        return Err(nested("pnpm-workspace.yaml"));
    }
    // A declared member may still introduce an independent native workspace.
    // Check that boundary before membership can trigger its scope inventory.
    if let Some(workspace) = registry.probe(dir)?.first() {
        return Err(Error::Nested {
            marker: workspace.manifest_path().to_owned(),
            root: root.to_owned(),
        });
    }
    let package = dir.join_component("package.json");
    if package.exists() {
        let package_json = PackageJson::load(&package).map_err(|e| marker_error(&package, e))?;
        if package_json.other.contains_key("workspaces") {
            return Err(nested("package.json"));
        }
        // Read native JS globs without detecting or executing a package manager.
        let manager = if root.join_component("pnpm-workspace.yaml").exists() {
            Some(PackageManager::Pnpm)
        } else if root.join_component("package.json").exists() {
            let path = root.join_component("package.json");
            let package = PackageJson::load(&path).map_err(|e| marker_error(&path, e))?;
            package
                .other
                .contains_key("workspaces")
                .then_some(PackageManager::Npm)
        } else {
            None
        };
        let js_owned = if let Some(manager) = manager {
            let source = manager.workspace_glob_source(root);
            let globs = manager
                .get_workspace_globs(root)
                .map_err(|e| marker_error(&source, e))?;
            globs.target_is_workspace(root, dir)?
        } else {
            false
        };
        if !js_owned {
            // Reuse the root observations and their cached native inventories;
            // package.json alone does not make a declared native member a root.
            for workspace in root_workspaces {
                if workspace.owns(dir)? {
                    return Ok(());
                }
            }
            return Err(nested("package.json"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    fn native_fixture(root: &AbsoluteSystemPath, ecosystem: &str) -> &'static str {
        root.join_component("turbo.json").create_with_contents(
            r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true,"experimentalPythonWorkspaces":true,"experimentalGoWorkspaces":true}}"#,
        ).unwrap();
        let member = root.join_component("member");
        member.join_component("src").create_dir_all().unwrap();
        let (root_manifest, root_contents, member_manifest, member_contents) = match ecosystem {
            "cargo" => (
                "Cargo.toml",
                "[workspace]\nmembers = [\"member\"]\n[workspace.metadata]\nname = \"workspace\"\n",
                "Cargo.toml",
                "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
            ),
            "uv" => (
                "pyproject.toml",
                "[tool.uv.workspace]\nmembers = [\"member\"]\n[tool.turbo]\nname = \"workspace\"\n",
                "pyproject.toml",
                "[project]\nname = \"member\"\nversion = \"0.1.0\"\n",
            ),
            "go" => (
                "go.work",
                "go 1.22\nuse ./member\n",
                "go.mod",
                "module example.com/member\ngo 1.22\n",
            ),
            _ => unreachable!(),
        };
        root.join_component(root_manifest)
            .create_with_contents(root_contents)
            .unwrap();
        member
            .join_component(member_manifest)
            .create_with_contents(member_contents)
            .unwrap();
        root_manifest
    }

    #[test_case("cargo"; "cargo")]
    #[test_case("uv"; "uv")]
    #[test_case("go"; "go")]
    fn native_roots_without_package_json(ecosystem: &str) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        native_fixture(&root, ecosystem);
        assert!(!root.join_component("package.json").exists());
        assert_eq!(infer(&root, true, None).unwrap().path, root);
        let cwd = root.join_components(&["member", "src"]);
        assert_eq!(infer(&cwd, false, None).unwrap().path, root);
    }

    #[test_case("cargo", false; "cargo_without_js_root")]
    #[test_case("uv", false; "uv_without_js_root")]
    #[test_case("go", false; "go_without_js_root")]
    #[test_case("cargo", true; "cargo_with_unmatched_js_globs")]
    #[test_case("uv", true; "uv_with_unmatched_js_globs")]
    #[test_case("go", true; "go_with_unmatched_js_globs")]
    fn declared_native_members_with_package_json(ecosystem: &str, js_root: bool) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        native_fixture(&root, ecosystem);
        if js_root {
            root.join_component("package.json")
                .create_with_contents(r#"{"workspaces":["js/*"]}"#)
                .unwrap();
        }
        let member = root.join_component("member");
        member
            .join_component("package.json")
            .create_with_contents(r#"{"name":"member"}"#)
            .unwrap();
        // Both markers need the same root inventory during nested inference.
        let cwd = member.join_component("src");
        cwd.join_component("package.json")
            .create_with_contents(r#"{"name":"member-src"}"#)
            .unwrap();
        for cwd in [&member, &cwd] {
            assert_eq!(infer(cwd, false, None).unwrap().path, root);
        }
    }

    #[test_case("cargo"; "cargo")]
    #[test_case("uv"; "uv")]
    #[test_case("go"; "go")]
    fn unrelated_native_package_json_is_rejected(ecosystem: &str) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        let root_manifest = native_fixture(&root, ecosystem);
        let unrelated = root.join_component("member-other");
        unrelated.create_dir_all().unwrap();
        let member_manifest = if ecosystem == "go" {
            "go.mod"
        } else {
            root_manifest
        };
        std::fs::copy(
            root.join_components(&["member", member_manifest]),
            unrelated.join_component(member_manifest),
        )
        .unwrap();
        let package = unrelated.join_component("package.json");
        package
            .create_with_contents(r#"{"name":"unrelated"}"#)
            .unwrap();
        assert!(matches!(
            infer(&unrelated, false, None),
            Err(Error::Nested { marker, root: selected }) if marker == package && selected == root
        ));
    }

    #[test_case("cargo"; "cargo")]
    #[test_case("uv"; "uv")]
    #[test_case("go"; "go")]
    fn declared_native_members_do_not_hide_boundaries(ecosystem: &str) {
        for boundary in ["native", "js", "pnpm", "config", "git"] {
            let temp = tempfile::tempdir().unwrap();
            let root = AbsoluteSystemPathBuf::try_from(temp.path())
                .unwrap()
                .to_realpath()
                .unwrap();
            let native_manifest = native_fixture(&root, ecosystem);
            let member = root.join_component("member");
            member
                .join_component("package.json")
                .create_with_contents(r#"{"name":"member"}"#)
                .unwrap();
            let (name, contents) = match boundary {
                "native" => (
                    native_manifest,
                    match ecosystem {
                        "cargo" => "[workspace]\nmembers = []\n",
                        "uv" => "[tool.uv.workspace]\nmembers = []\n",
                        "go" => "go 1.22\nuse ()\n",
                        _ => unreachable!(),
                    },
                ),
                "js" => ("package.json", r#"{"workspaces":["nested/*"]}"#),
                "pnpm" => ("pnpm-workspace.yaml", "packages: []\n"),
                "config" => ("turbo.json", "{}"),
                "git" => (".git", "gitdir: elsewhere\n"),
                _ => unreachable!(),
            };
            let marker = member.join_component(name);
            marker.create_with_contents(contents).unwrap();
            let config = root.join_component("turbo.json");
            let result = infer(&member, false, Some(&config));
            assert!(
                matches!(
                    result,
                    Err(Error::Nested { marker: found, root: selected })
                        if found == marker && selected == root
                ),
                "{ecosystem}: {boundary}"
            );
        }
    }

    #[test]
    fn root_identity_is_available_without_provisioning() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        root.join_component("turbo.json")
            .create_with_contents(
                r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#,
            )
            .unwrap();
        root.join_component("Cargo.toml")
            .create_with_contents("[workspace]\nmembers = []\n")
            .unwrap();
        let nested = root.join_components(&["crates", "app", "src"]);
        nested.create_dir_all().unwrap();
        let inferred = infer(&nested, false, None).unwrap();
        assert_eq!(inferred.path, root);
        assert!(inferred.flags.experimental_setup && inferred.flags.experimental_cargo_workspaces);
        assert!(
            !inferred.flags.experimental_python_workspaces
                && !inferred.flags.experimental_go_workspaces
        );
    }

    #[test]
    fn registry_probes_keep_setup_nested_root_boundaries() {
        for (manifest, contents) in [
            ("Cargo.toml", "[workspace]\nmembers = []\n"),
            ("pyproject.toml", "[tool.uv.workspace]\nmembers = []\n"),
            ("go.work", "go 1.22\nuse ()\n"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = AbsoluteSystemPathBuf::try_from(temp.path())
                .unwrap()
                .to_realpath()
                .unwrap();
            root.join_component("turbo.json").create_with_contents(
                r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true,"experimentalPythonWorkspaces":true,"experimentalGoWorkspaces":true}}"#,
            ).unwrap();
            let nested = root.join_component("nested");
            nested.create_dir_all().unwrap();
            let path = nested.join_component(manifest);
            path.create_with_contents(contents).unwrap();
            assert!(matches!(
                infer(&nested, false, None),
                Err(Error::Nested { marker, root: selected }) if marker == path && selected == root
            ));
            root.join_component("turbo.json")
                .create_with_contents(r#"{"futureFlags":{"experimentalSetup":true}}"#)
                .unwrap();
            path.create_with_contents("[broken").unwrap();
            assert_eq!(infer(&nested, false, None).unwrap().path, root);
        }
    }

    #[test]
    fn root_and_nested_native_probe_errors_retain_bootstrap_diagnostics() {
        for nested_manifest in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = AbsoluteSystemPathBuf::try_from(temp.path())
                .unwrap()
                .to_realpath()
                .unwrap();
            root.join_component("turbo.json").create_with_contents(
                r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#,
            ).unwrap();
            let nested = root.join_component("nested");
            nested.create_dir_all().unwrap();
            let path = if nested_manifest { &nested } else { &root }.join_component("Cargo.toml");
            path.create_with_contents("[workspace").unwrap();
            let error = infer(&nested, false, None).err().unwrap();
            let Error::Bootstrap(error) = error else {
                panic!("expected bootstrap diagnostic, got {error}");
            };
            assert_eq!(
                error.toolchain,
                turborepo_repository::toolchain::ToolchainId::RUST
            );
            assert_eq!(error.path, path);
            assert!(std::error::Error::source(&error).is_some());
        }
    }

    #[test]
    fn filesystem_root_is_not_a_configuration_file() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = AbsoluteSystemPathBuf::try_from(temp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        let filesystem_root = cwd.ancestors().last().unwrap();
        assert!(matches!(
            infer(&cwd, false, Some(filesystem_root)),
            Err(Error::Marker { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_cwd_uses_the_target_repository_not_the_link_parent() {
        let temp = tempfile::tempdir().unwrap();
        let base = AbsoluteSystemPathBuf::try_from(temp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        let target = base.join_component("target");
        target.create_dir_all().unwrap();
        target
            .join_component("turbo.json")
            .create_with_contents(r#"{"futureFlags":{"experimentalSetup":true}}"#)
            .unwrap();
        target.join_component("src").create_dir_all().unwrap();
        let neighbor = base.join_component("neighbor");
        neighbor.create_dir_all().unwrap();
        neighbor
            .join_component("turbo.json")
            .create_with_contents("{}")
            .unwrap();
        let link = neighbor.join_component("linked-src");
        std::os::unix::fs::symlink(target.join_component("src"), &link).unwrap();
        assert_eq!(infer(&link, true, None).unwrap().path, target);
    }
}
