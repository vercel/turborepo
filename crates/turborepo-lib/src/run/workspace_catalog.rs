use std::{collections::HashMap, ffi::OsStr, fs::File};

use anyhow::Result;
use itertools::{Either, Itertools};
use serde::Deserialize;
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;

use crate::commands::CommandBase;

#[derive(Default, Debug)]
pub struct WorkspaceCatalog {
    /// a map between package name and package info
    packages: HashMap<String, Package>,
    /// a map between package name and the turbo.json path
    turbos: HashMap<String, TurboJsonPath>,
}

type TurboJsonPath = AbsoluteSystemPathBuf;

#[derive(Debug)]
struct Package {
    name: String,
    path: AbsoluteSystemPathBuf,
}

#[derive(Deserialize)]
struct PackageJson {
    name: String,
    /// A glob for defining workspaces.
    workspaces: Option<Vec<String>>,
}

#[derive(Error, Debug)]
pub enum DiscoveryError {
    #[error("error when setting up walk: {0}")]
    Globwalk(#[from] globwalk::WalkError),
    #[error("error when walking fs: {0}")]
    WalkDir(#[from] globwalk::WalkDirError),
    #[error("error when reading json: {0}")]
    Io(#[from] std::io::Error),
    #[error("error when parsing json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("unknown file: {0}")]
    UnknownFile(AbsoluteSystemPathBuf),
    #[error("turbo json must have a package.json")]
    MissingPackageJson(AbsoluteSystemPathBuf),
    #[error("error when getting package manager: {0}")]
    PackageManager(anyhow::Error),
    #[error("error when getting workspace: {0}")]
    Workspace(anyhow::Error),
}

impl WorkspaceCatalog {
    /// Discover the turbo context from the given root directory.
    ///
    /// A discovery can only be made if the base_path has a package.json file
    /// inside it. Otherwise, it is not a valid workspace.
    ///
    /// Note: as of now, a package.json file without a workspaces field is not
    ///       considered a valid workspace.
    pub fn discover(base: &CommandBase) -> Result<Self, DiscoveryError> {
        let package_manager =
            crate::package_manager::PackageManager::get_package_manager(base, None)
                .map_err(|e| DiscoveryError::PackageManager(e))?;

        let (include, exclude) = package_manager
            .get_workspace_globs(base.repo_root.as_path())
            .map_err(|e| DiscoveryError::Workspace(e))?
            .map(|g| (g.raw_inclusions, g.raw_exclusions))
            .unwrap_or_else(|| {
                (
                    vec!["package.json".to_string(), "turbo.json".to_string()],
                    vec![],
                )
            });

        let walker = globwalk::globwalk(
            &base.repo_root,
            &include,
            &exclude,
            globwalk::WalkType::Files,
        )?;

        let mut path_to_package_name = HashMap::new();

        walker
            .map(|path| {
                let path = path.map_err(DiscoveryError::Globwalk)?;
                let parent = path.parent().expect("will never return None");
                let file_name = path
                    .as_path()
                    .file_name()
                    .and_then(OsStr::to_str)
                    .ok_or(DiscoveryError::UnknownFile(path.clone()))?;

                let part = if file_name == "package.json" {
                    let reader = File::open(path.as_path())?;
                    let json: PackageJson = serde_json::from_reader(reader)?;
                    let name: &String = path_to_package_name.entry(parent).or_insert(json.name);

                    Either::Left((
                        name.to_owned(),
                        Package {
                            name: name.to_owned(),
                            path,
                        },
                    ))
                } else if file_name == "turbo.json" {
                    // files come in alphabetical order, so turbo will appear _after_ package.json
                    let package_name = path_to_package_name
                        .get(&parent)
                        .ok_or(DiscoveryError::MissingPackageJson(path.clone()))?;
                    Either::Right((package_name.to_owned(), path))
                } else {
                    return Err(DiscoveryError::UnknownFile(path));
                };

                Ok(part)
            })
            .collect::<Result<WorkspaceCatalog, _>>()
    }
}

/// We do this so that we can error early when collecting above.
impl FromIterator<Either<(String, Package), (String, AbsoluteSystemPathBuf)>> for WorkspaceCatalog {
    fn from_iter<
        T: IntoIterator<Item = Either<(String, Package), (String, AbsoluteSystemPathBuf)>>,
    >(
        iter: T,
    ) -> Self {
        let (packages, turbos) = iter.into_iter().partition_map(|p| p);
        Self { packages, turbos }
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, fs::File, io::Write};

    use tempdir::TempDir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::{DiscoveryError, WorkspaceCatalog};
    use crate::{commands::CommandBase, ui::UI};

    fn get_base(
        package_json_contents: Option<&'static str>,
        lockfile_contents: Option<&'static str>,
        turbo_json_contents: Option<&'static str>,
    ) -> (TempDir, CommandBase) {
        let dir = tempdir::TempDir::new("turborepo-test").unwrap();
        let base = CommandBase::new(
            Default::default(),
            AbsoluteSystemPathBuf::new(dir.path()).unwrap(),
            "test",
            UI::new(true),
        )
        .unwrap();

        if let Some(contents) = package_json_contents {
            let mut package = File::create(base.repo_root.as_path().join("package.json")).unwrap();
            writeln!(package, "{}", contents).unwrap();
        }

        if let Some(contents) = lockfile_contents {
            let mut package_lock =
                File::create(base.repo_root.as_path().join("package-lock.json")).unwrap();
            writeln!(package_lock, "{}", contents).unwrap();
        }

        if let Some(contents) = turbo_json_contents {
            let mut _turbo = File::create(base.repo_root.as_path().join("turbo.json")).unwrap();
            writeln!(_turbo, "{}", contents).unwrap();
        }

        (dir, base)
    }

    #[test]
    fn discovers_single_package() {
        let (_dir, base) = get_base(
            Some("{\"name\": \"test_package\", \"workspaces\": []}"),
            Some("{\"name\": \"test_package\"}"),
            Some("{}"),
        );

        let ctx = WorkspaceCatalog::discover(&base).unwrap();

        assert!(ctx.packages.contains_key("test_package"));
        assert!(ctx.turbos.contains_key("test_package"));
    }

    #[test]
    fn rejects_underspecified_package_json() {
        let (_dir, base) = get_base(
            Some("{}"),
            Some("{{\"name\": \"test_package\"}}"),
            Some("{}"),
        );

        let ctx = WorkspaceCatalog::discover(&base);

        assert_matches!(ctx.unwrap_err(), DiscoveryError::Workspace(_));
    }

    #[test]
    fn rejects_malformed_json() {
        let (_dir, base) = get_base(
            Some("{{\"name\": \"test_packag"),
            Some("{{\"name\": \"test_packag"),
            Some("{}"),
        );

        let ctx = WorkspaceCatalog::discover(&base);

        assert_matches!(ctx.unwrap_err(), DiscoveryError::Workspace(_));
    }

    #[test]
    fn discovers_multi_package() {
        let (_dir, base) = get_base(
            Some(
                "{\"name\": \"test_package\", \"workspaces\": [
                \"apps/**\",
                \"packages/**\",
                \"package_a/**\"
            ]}",
            ),
            Some("{\"name\": \"test_package\"}"),
            Some("{}"),
        );

        for path in ["package_a", "packages/package_b", "apps/package_c"] {
            let name = path.rsplit_once('/').map(|s| s.1).unwrap_or(path);

            let dir: std::path::PathBuf = base.repo_root.as_path().join(path);
            std::fs::create_dir_all(&dir).unwrap();

            let mut package = File::create(dir.join("package.json")).unwrap();
            writeln!(package, "{{\"name\": \"{}\"}}", name).unwrap();
        }

        let ctx = WorkspaceCatalog::discover(&base).unwrap();

        assert!(ctx.packages.contains_key("package_a"));
        assert!(ctx.packages.contains_key("package_b"));
        assert!(ctx.packages.contains_key("package_c"));
    }

    #[test]
    fn rejects_missing_package_name() {
        let (_dir, base) = get_base(Some("{}"), Some("{}"), Some("{}"));
        let ctx = WorkspaceCatalog::discover(&base);
        assert_matches!(ctx.unwrap_err(), DiscoveryError::Workspace(_));
    }

    #[test]
    fn rejects_missing_package_json() {
        let (_dir, base) = get_base(None, None, Some("{}"));
        let ctx = WorkspaceCatalog::discover(&base);

        assert_matches!(ctx.unwrap_err(), DiscoveryError::PackageManager(_));
    }

    #[test]
    fn rejects_missing_package_lock() {
        let (_dir, base) = get_base(Some("{}"), None, Some("{}"));
        let ctx = WorkspaceCatalog::discover(&base);

        assert_matches!(ctx.unwrap_err(), DiscoveryError::PackageManager(_));
    }
}
