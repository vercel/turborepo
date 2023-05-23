use std::{collections::HashMap, ffi::OsStr, fs::File, io::ErrorKind};

use itertools::{Either, Itertools};
use serde::Deserialize;
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;

#[derive(Debug)]
struct Package {
    name: String,
    // package: NpmPackage,
    path: AbsoluteSystemPathBuf,
}

#[derive(Deserialize)]
struct PackageJson {
    name: String,

    /// A glob for defining workspaces.
    workspaces: Option<Vec<String>>,
}

/// A list of packages and turbos corresponding to workspaces in the current
/// monorepo.
#[derive(Debug)]
pub struct WorkspaceContext {
    packages: HashMap<String, Package>,
    turbos: HashMap<String, TurboJsonPath>,
}

type TurboJsonPath = AbsoluteSystemPathBuf;

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
}

fn join_globs(a: &str, b: &str) -> String {
    let insert_sep = !a.ends_with('/');
    format!("{}{}{}", a, if insert_sep { "/" } else { "" }, b)
}

impl WorkspaceContext {
    fn get_workspace_globs(
        base_path: &AbsoluteSystemPathBuf,
    ) -> Result<Vec<String>, DiscoveryError> {
        let default_globs = ["package.json".to_string(), "turbo.json".to_string()];
        let package_json = {
            let package_json = base_path.as_path().join("package.json");
            let reader = File::open(package_json).map_err(|e| {
                if e.kind() == ErrorKind::NotFound {
                    DiscoveryError::MissingPackageJson(base_path.to_owned())
                } else {
                    DiscoveryError::Io(e)
                }
            })?;
            let json: PackageJson = serde_json::from_reader(reader)?;
            json
        };

        // if no workspaces are defined, simply return the default globs (single package
        // workspace)
        let globs = if let Some(workspaces) = package_json.workspaces {
            workspaces
                .iter()
                .flat_map(|workspace| default_globs.iter().map(|dg| join_globs(workspace, dg)))
                .collect()
        } else {
            default_globs.into_iter().collect()
        };

        Ok(globs)
    }

    /// Discover the turbo context from the given root directory.
    ///
    /// A discovery can only be made if the base_path has a package.json file
    /// inside it. Otherwise, it is not a valid workspace.
    pub fn discover(base_path: &AbsoluteSystemPathBuf) -> Result<Self, DiscoveryError> {
        let include = Self::get_workspace_globs(base_path)?;
        let walker = globwalk::globwalk(base_path, &include, &[], globwalk::WalkType::Files)?;

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
            .collect::<Result<WorkspaceContext, _>>()
    }
}

/// We do this so that we can error early when collecting above.
impl FromIterator<Either<(String, Package), (String, AbsoluteSystemPathBuf)>> for WorkspaceContext {
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

    use turbopath::AbsoluteSystemPathBuf;

    use super::WorkspaceContext;
    use crate::context::DiscoveryError;

    #[test]
    fn discovers_single_package() {
        let dir = tempdir::TempDir::new("turborepo-test").unwrap();
        let mut package = File::create(dir.path().join("package.json")).unwrap();
        writeln!(package, "{{\"name\": \"test_package\"}}").unwrap();
        let _turbo = File::create(dir.path().join("turbo.json")).unwrap();
        let ctx =
            WorkspaceContext::discover(&AbsoluteSystemPathBuf::new(dir.path()).unwrap()).unwrap();
        assert!(ctx.packages.contains_key("test_package"));
        assert!(ctx.turbos.contains_key("test_package"));
    }

    #[test]
    fn discovers_multi_package() {
        let dir = tempdir::TempDir::new("turborepo-test").unwrap();

        let mut package = File::create(dir.path().join("package.json")).unwrap();
        writeln!(
            package,
            "{{\"name\": \"root\", \"workspaces\": [
            \"apps/**\",
            \"packages/**\",
            \"package_a/**\"
        ]}}"
        )
        .unwrap();
        let _turbo = File::create(dir.path().join("turbo.json")).unwrap();

        for path in ["package_a", "packages/package_b", "apps/package_c"] {
            let name = path.rsplit_once('/').map(|s| s.1).unwrap_or(path);

            let dir: std::path::PathBuf = dir.path().join(path);
            std::fs::create_dir_all(&dir).unwrap();

            let mut package = File::create(dir.join("package.json")).unwrap();
            writeln!(package, "{{\"name\": \"{}\"}}", name).unwrap();
        }

        let ctx =
            WorkspaceContext::discover(&AbsoluteSystemPathBuf::new(dir.path()).unwrap()).unwrap();

        assert!(ctx.packages.contains_key("package_a"));
        assert!(ctx.packages.contains_key("package_b"));
        assert!(ctx.packages.contains_key("package_c"));
    }

    #[test]
    fn handles_missing_package_name() {
        let dir = tempdir::TempDir::new("turborepo-test").unwrap();
        let mut package = File::create(dir.path().join("package.json")).unwrap();
        writeln!(package, "{{}}").unwrap();
        let _turbo = File::create(dir.path().join("turbo.json")).unwrap();
        assert_matches!(
            WorkspaceContext::discover(&AbsoluteSystemPathBuf::new(dir.path()).unwrap())
                .unwrap_err(),
            DiscoveryError::Parse(_)
        );
    }

    #[test]
    fn handles_missing_package_json() {
        let dir = tempdir::TempDir::new("turborepo-test").unwrap();
        let _turbo = File::create(dir.path().join("turbo.json")).unwrap();
        assert_matches!(
            WorkspaceContext::discover(&AbsoluteSystemPathBuf::new(dir.path()).unwrap())
                .unwrap_err(),
            DiscoveryError::MissingPackageJson(_)
        );
    }
}
