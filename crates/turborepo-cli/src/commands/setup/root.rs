//! Read-only setup discovery, independent of package managers and tool probes.

use miette::Diagnostic;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};
use turborepo_turbo_json::{FutureFlags, RawTurboJson};

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
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] turborepo_config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJson(#[from] turborepo_turbo_json::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

pub(super) struct SetupRoot {
    pub path: AbsoluteSystemPathBuf,
    pub flags: FutureFlags,
}

/// `--cwd` is a discovery starting point. An exact root selected with it is
/// authoritative; a nested starting point still needs unambiguous inference.
/// Explicit config paths retain the global option's invocation-relative
/// meaning.
pub(super) fn infer(
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

    let (path, raw) = if let Some(config) = config {
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
        (root.to_owned(), raw)
    } else {
        let mut selected: Option<(AbsoluteSystemPathBuf, RawTurboJson)> = None;
        for dir in &directories {
            let config = turborepo_config::resolve_turbo_config_path(dir)?;
            let Some(raw) = read_root_config(&cwd, &config)? else {
                continue;
            };
            if let Some((near, _)) = &selected {
                return Err(Error::Ambiguous {
                    near: near.clone(),
                    outer: dir.clone(),
                });
            }
            selected = Some((dir.clone(), raw));
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
            (path, RawTurboJson::default())
        })
    };
    let flags = raw
        .future_flags
        .map(|flags| *flags.as_inner())
        .unwrap_or_default();
    if flags.experimental_setup {
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
            check_nested_markers(dir, &path, flags)?;
        }
    }
    Ok(SetupRoot { path, flags })
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

fn toml_marker(dir: &AbsoluteSystemPath, name: &str) -> Result<Option<toml::Value>, Error> {
    let path = dir.join_component(name);
    path.read_existing_to_string()
        .map_err(|e| marker_error(&path, e))?
        .map(|contents| toml::from_str(&contents).map_err(|e| marker_error(&path, e)))
        .transpose()
}

fn check_nested_markers(
    dir: &AbsoluteSystemPath,
    root: &AbsoluteSystemPath,
    flags: FutureFlags,
) -> Result<(), Error> {
    let nested = |name| Error::Nested {
        marker: dir.join_component(name),
        root: root.to_owned(),
    };
    if dir.join_component("pnpm-workspace.yaml").exists() {
        return Err(nested("pnpm-workspace.yaml"));
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
        let Some(manager) = manager else {
            return Err(nested("package.json"));
        };
        let source = manager.workspace_glob_source(root);
        let globs = manager
            .get_workspace_globs(root)
            .map_err(|e| marker_error(&source, e))?;
        if !globs.target_is_workspace(root, dir)? {
            return Err(nested("package.json"));
        }
    }
    if flags.experimental_cargo_workspaces
        && let Some(cargo) = toml_marker(dir, "Cargo.toml")?
        && cargo.get("workspace").is_some()
    {
        return Err(nested("Cargo.toml"));
    }
    if flags.experimental_python_workspaces
        && let Some(python) = toml_marker(dir, "pyproject.toml")?
        && python
            .get("tool")
            .and_then(|tool| tool.get("uv"))
            .and_then(|uv| uv.get("workspace"))
            .is_some()
    {
        return Err(nested("pyproject.toml"));
    }
    if flags.experimental_go_workspaces && dir.join_component("go.work").exists() {
        return Err(nested("go.work"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
