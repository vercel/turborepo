mod bun;
mod npm;
mod pnpm;
mod yarn;

use std::{
    backtrace,
    fmt::{self, Display},
    fs,
    process::Command,
    str::FromStr,
};

use globwalk::{fix_glob_pattern, ValidatedGlob};
use itertools::{Either, Itertools};
use lazy_regex::{lazy_regex, Lazy};
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError, RelativeUnixPath};
use turborepo_lockfiles::Lockfile;
use wax::{Any, Glob, Program};
use which::which;

use crate::{
    discovery,
    package_json::PackageJson,
    package_manager::{bun::BunDetector, npm::NpmDetector, pnpm::PnpmDetector, yarn::YarnDetector},
    util::IsLast,
};

#[derive(Debug, Deserialize)]
struct PnpmWorkspace {
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackageJsonWorkspaces {
    workspaces: Workspaces,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
#[serde(untagged)]
enum Workspaces {
    TopLevel(Vec<String>),
    Nested { packages: Vec<String> },
}

impl AsRef<[String]> for Workspaces {
    fn as_ref(&self) -> &[String] {
        match self {
            Workspaces::TopLevel(packages) => packages.as_slice(),
            Workspaces::Nested { packages } => packages.as_slice(),
        }
    }
}

impl From<Workspaces> for Vec<String> {
    fn from(value: Workspaces) -> Self {
        match value {
            Workspaces::TopLevel(packages) => packages,
            Workspaces::Nested { packages } => packages,
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Berry,
    Npm,
    Pnpm,
    Pnpm6,
    Yarn,
    Bun,
}

impl Display for PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Do not change these without also changing `GetPackageManager` in
        // packagemanager.go
        match self {
            PackageManager::Berry => write!(f, "berry"),
            PackageManager::Npm => write!(f, "npm"),
            PackageManager::Pnpm => write!(f, "pnpm"),
            PackageManager::Pnpm6 => write!(f, "pnpm6"),
            PackageManager::Yarn => write!(f, "yarn"),
            PackageManager::Bun => write!(f, "bun"),
        }
    }
}

// WorkspaceGlobs is suitable for finding package.json files via globwalk
#[derive(Debug, Clone)]
pub struct WorkspaceGlobs {
    directory_inclusions: Any<'static>,
    directory_exclusions: Any<'static>,
    package_json_inclusions: Vec<ValidatedGlob>,
    pub raw_inclusions: Vec<String>,
    pub raw_exclusions: Vec<String>,
    validated_exclusions: Vec<ValidatedGlob>,
}

impl PartialEq for WorkspaceGlobs {
    fn eq(&self, other: &Self) -> bool {
        // Use the literals for comparison, not the compiled globs
        self.raw_inclusions == other.raw_inclusions && self.raw_exclusions == other.raw_exclusions
    }
}

impl Eq for WorkspaceGlobs {}

fn glob_with_contextual_error<S: AsRef<str>>(raw: S) -> Result<Glob<'static>, Error> {
    let raw = raw.as_ref();
    let fixed = fix_glob_pattern(raw);
    Glob::new(&fixed)
        .map(|g| g.into_owned())
        .map_err(|e| Error::Glob(fixed, Box::new(e)))
}

fn any_with_contextual_error(
    precompiled: Vec<Glob<'static>>,
    text: Vec<String>,
) -> Result<wax::Any<'static>, Error> {
    wax::any(precompiled).map_err(|e| {
        let text = text.iter().join(",");
        Error::Glob(text, Box::new(e))
    })
}

impl WorkspaceGlobs {
    pub fn new<S: Into<String>>(inclusions: Vec<S>, exclusions: Vec<S>) -> Result<Self, Error> {
        // take ownership of the inputs
        let raw_inclusions: Vec<String> = inclusions
            .into_iter()
            .map(|s| s.into())
            .collect::<Vec<String>>();
        let package_json_inclusions = raw_inclusions
            .iter()
            .map(|s| {
                let mut s: String = s.clone();
                if s.ends_with('/') {
                    s.push_str("package.json");
                } else {
                    s.push_str("/package.json");
                }
                ValidatedGlob::from_str(&s)
            })
            .collect::<Result<Vec<ValidatedGlob>, _>>()?;
        let raw_exclusions: Vec<String> = exclusions
            .into_iter()
            .map(|s| s.into())
            .collect::<Vec<String>>();
        let inclusion_globs = raw_inclusions
            .iter()
            .map(glob_with_contextual_error)
            .collect::<Result<Vec<_>, _>>()?;
        let exclusion_globs = raw_exclusions
            .iter()
            .map(glob_with_contextual_error)
            .collect::<Result<Vec<_>, _>>()?;
        let validated_exclusions = raw_exclusions
            .iter()
            .map(|e| ValidatedGlob::from_str(e))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            directory_inclusions: any_with_contextual_error(
                inclusion_globs,
                raw_inclusions.clone(),
            )?,
            directory_exclusions: any_with_contextual_error(
                exclusion_globs,
                raw_exclusions.clone(),
            )?,
            package_json_inclusions,
            validated_exclusions,
            raw_exclusions,
            raw_inclusions,
        })
    }

    /// Checks if the given `target` matches this `WorkspaceGlobs`.
    ///
    /// Errors:
    /// This function returns an Err if `root` is not a valid anchor for
    /// `target`
    pub fn target_is_workspace(
        &self,
        root: &AbsoluteSystemPath,
        target: &AbsoluteSystemPath,
    ) -> Result<bool, PathError> {
        let search_value = root.anchor(target)?;

        let includes = self.directory_inclusions.is_match(&search_value);
        let excludes = self.directory_exclusions.is_match(&search_value);

        Ok(includes && !excludes)
    }
}

#[derive(Debug, Error)]
pub struct MissingWorkspaceError {
    package_manager: PackageManager,
}

#[derive(Debug, Error)]
pub struct NoPackageManager;

impl Display for NoPackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "We did not find a package manager specified in your root package.json. \
        Please set the \"packageManager\" property in your root package.json (https://nodejs.org/api/packages.html#packagemanager) \
        or run `npx @turbo/codemod add-package-manager` in the root of your monorepo.")
    }
}

impl Display for MissingWorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let err = match self.package_manager {
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                "pnpm-workspace.yaml: no packages found. Turborepo requires pnpm workspaces and \
                 thus packages to be defined in the root pnpm-workspace.yaml"
            }
            PackageManager::Yarn | PackageManager::Berry => {
                "package.json: no workspaces found. Turborepo requires yarn workspaces to be \
                 defined in the root package.json"
            }
            PackageManager::Npm => {
                "package.json: no workspaces found. Turborepo requires npm workspaces to be \
                 defined in the root package.json"
            }
            PackageManager::Bun => {
                "package.json: no workspaces found. Turborepo requires bun workspaces to be \
                 defined in the root package.json"
            }
        };
        write!(f, "{}", err)
    }
}

impl From<&PackageManager> for MissingWorkspaceError {
    fn from(value: &PackageManager) -> Self {
        Self {
            package_manager: *value,
        }
    }
}

impl From<wax::BuildError> for Error {
    fn from(value: wax::BuildError) -> Self {
        Self::Wax(Box::new(value), backtrace::Backtrace::capture())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error, #[backtrace] backtrace::Backtrace),
    #[error(transparent)]
    Workspace(#[from] MissingWorkspaceError),
    #[error("yaml parsing error: {0}")]
    ParsingYaml(#[from] serde_yaml::Error, #[backtrace] backtrace::Backtrace),
    #[error("json parsing error: {0}")]
    ParsingJson(#[from] serde_json::Error, #[backtrace] backtrace::Backtrace),
    #[error("globbing error: {0}")]
    Wax(Box<wax::BuildError>, #[backtrace] backtrace::Backtrace),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error(transparent)]
    NoPackageManager(#[from] NoPackageManager),
    #[error("We detected multiple package managers in your repository: {}. Please remove one \
    of them.", managers.join(", "))]
    MultiplePackageManagers { managers: Vec<String> },
    #[error(transparent)]
    Semver(#[from] node_semver::SemverError),
    #[error(transparent)]
    Which(#[from] which::Error),
    #[error("invalid utf8: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(
        "We could not parse the packageManager field in package.json, expected: {0}, received: {1}"
    )]
    InvalidPackageManager(String, String),
    #[error(transparent)]
    WalkError(#[from] globwalk::WalkError),
    #[error("invalid workspace glob {0}: {1}")]
    Glob(String, #[source] Box<wax::BuildError>),
    #[error("invalid globwalk pattern {0}")]
    Globwalk(#[from] globwalk::GlobError),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::Error),

    #[error("discovering workspace: {0}")]
    WorkspaceDiscovery(#[from] discovery::Error),
}

impl From<std::convert::Infallible> for Error {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
}

static PACKAGE_MANAGER_PATTERN: Lazy<Regex> =
    lazy_regex!(r"(?P<manager>bun|npm|pnpm|yarn)@(?P<version>\d+\.\d+\.\d+(-.+)?)");

impl PackageManager {
    pub fn command(&self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm | PackageManager::Pnpm6 => "pnpm",
            PackageManager::Yarn | PackageManager::Berry => "yarn",
            PackageManager::Bun => "bun",
        }
    }

    /// Returns the set of globs for the workspace.
    pub fn get_workspace_globs(
        &self,
        root_path: &AbsoluteSystemPath,
    ) -> Result<WorkspaceGlobs, Error> {
        let (inclusions, mut exclusions) = self.get_configured_workspace_globs(root_path)?;
        exclusions.extend(self.get_default_exclusions());

        // Yarn appends node_modules to every other glob specified
        if *self == PackageManager::Yarn {
            inclusions
                .iter()
                .for_each(|inclusion| exclusions.push(format!("{inclusion}/node_modules/**")));
        }

        let globs = WorkspaceGlobs::new(inclusions, exclusions)?;
        Ok(globs)
    }

    pub fn get_default_exclusions(&self) -> impl Iterator<Item = String> {
        let ignores = match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                ["**/node_modules/**", "**/bower_components/**"].as_slice()
            }
            PackageManager::Npm => ["**/node_modules/**"].as_slice(),
            PackageManager::Bun => ["**/node_modules", "**/.git"].as_slice(),
            PackageManager::Berry => ["**/node_modules", "**/.git", "**/.yarn"].as_slice(),
            PackageManager::Yarn => [].as_slice(), // yarn does its own handling above
        };
        ignores.iter().map(|s| s.to_string())
    }

    fn get_configured_workspace_globs(
        &self,
        root_path: &AbsoluteSystemPath,
    ) -> Result<(Vec<String>, Vec<String>), Error> {
        let globs = match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                // Make sure to convert this to a missing workspace error
                // so we can catch it in the case of single package mode.
                let source = self.workspace_glob_source(root_path);
                let workspace_yaml = fs::read_to_string(source)
                    .map_err(|_| Error::Workspace(MissingWorkspaceError::from(self)))?;
                let pnpm_workspace: PnpmWorkspace = serde_yaml::from_str(&workspace_yaml)?;
                if pnpm_workspace.packages.is_empty() {
                    return Err(MissingWorkspaceError::from(self).into());
                } else {
                    pnpm_workspace.packages
                }
            }
            PackageManager::Berry
            | PackageManager::Npm
            | PackageManager::Yarn
            | PackageManager::Bun => {
                let package_json_text = fs::read_to_string(self.workspace_glob_source(root_path))?;
                let package_json: PackageJsonWorkspaces = serde_json::from_str(&package_json_text)
                    .map_err(|_| Error::Workspace(MissingWorkspaceError::from(self)))?; // Make sure to convert this to a missing workspace error

                if package_json.workspaces.as_ref().is_empty() {
                    return Err(MissingWorkspaceError::from(self).into());
                } else {
                    package_json.workspaces.into()
                }
            }
        };

        let (inclusions, exclusions) = globs.into_iter().partition_map(|glob| {
            if let Some(exclusion) = glob.strip_prefix('!') {
                Either::Right(exclusion.to_string())
            } else {
                Either::Left(glob)
            }
        });

        Ok((inclusions, exclusions))
    }

    pub fn workspace_glob_source(&self, root_path: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
        root_path.join_component(
            self.workspace_configuration_path()
                .unwrap_or("package.json"),
        )
    }

    /// Try to detect the package manager by inspecting the repository.
    /// This method does not read the package.json, instead looking for
    /// lockfiles and other files that indicate the package manager.
    ///
    /// TODO: consider if this method should not need an Option, and possibly be
    /// a method on PackageJSON
    pub fn get_package_manager(
        repo_root: &AbsoluteSystemPath,
        pkg: Option<&PackageJson>,
    ) -> Result<Self, Error> {
        // We don't surface errors for `read_package_manager` as we can fall back to
        // `detect_package_manager`
        if let Some(package_json) = pkg {
            if let Ok(Some(package_manager)) = Self::read_package_manager(package_json) {
                return Ok(package_manager);
            }
        }

        Self::detect_package_manager(repo_root)
    }

    // Attempts to read the package manager from the package.json
    fn read_package_manager(pkg: &PackageJson) -> Result<Option<Self>, Error> {
        let Some(package_manager) = &pkg.package_manager else {
            return Ok(None);
        };

        let (manager, version) = Self::parse_package_manager_string(package_manager)?;
        let version = version.parse()?;
        let manager = match manager {
            "npm" => Some(PackageManager::Npm),
            "bun" => Some(PackageManager::Bun),
            "yarn" => Some(YarnDetector::detect_berry_or_yarn(&version)?),
            "pnpm" => Some(PnpmDetector::detect_pnpm6_or_pnpm(&version)?),
            _ => None,
        };

        Ok(manager)
    }

    fn detect_package_manager(repo_root: &AbsoluteSystemPath) -> Result<PackageManager, Error> {
        let mut detected_package_managers = PnpmDetector::new(repo_root)
            .chain(NpmDetector::new(repo_root))
            .chain(YarnDetector::new(repo_root))
            .chain(BunDetector::new(repo_root))
            .collect::<Result<Vec<_>, Error>>()?;

        match detected_package_managers.len() {
            0 => Err(NoPackageManager.into()),
            1 => Ok(detected_package_managers.pop().unwrap()),
            _ => {
                let managers = detected_package_managers
                    .iter()
                    .map(|mgr| mgr.to_string())
                    .collect();
                Err(Error::MultiplePackageManagers { managers })
            }
        }
    }

    pub(crate) fn parse_package_manager_string(manager: &str) -> Result<(&str, &str), Error> {
        if let Some(captures) = PACKAGE_MANAGER_PATTERN.captures(manager) {
            let manager = captures.name("manager").unwrap().as_str();
            let version = captures.name("version").unwrap().as_str();
            Ok((manager, version))
        } else {
            Err(Error::InvalidPackageManager(
                PACKAGE_MANAGER_PATTERN.to_string(),
                manager.to_string(),
            ))
        }
    }

    pub fn get_package_jsons(
        &self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<impl Iterator<Item = AbsoluteSystemPathBuf>, Error> {
        let globs = self.get_workspace_globs(repo_root)?;

        let files = globwalk::globwalk(
            repo_root,
            &globs.package_json_inclusions,
            &globs.validated_exclusions,
            globwalk::WalkType::Files,
        )?;

        // we need to remove package.json files that are in subfolders of others so that
        // we don't yield subpackages. sort, keep track of the parent of last
        // json we encountered, and only yield it if it's not a subfolder of it
        //
        // ideally we would do this during traversal, but walkdir doesn't support
        // inorder traversal so we can't
        Ok(filter_subfolder_package_jsons(files))
    }

    pub fn lockfile_name(&self) -> &'static str {
        match self {
            PackageManager::Npm => npm::LOCKFILE,
            PackageManager::Bun => bun::LOCKFILE,
            PackageManager::Pnpm | PackageManager::Pnpm6 => pnpm::LOCKFILE,
            PackageManager::Yarn | PackageManager::Berry => yarn::LOCKFILE,
        }
    }

    pub fn workspace_configuration_path(&self) -> Option<&'static str> {
        match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => Some("pnpm-workspace.yaml"),
            PackageManager::Npm
            | PackageManager::Berry
            | PackageManager::Yarn
            | PackageManager::Bun => None,
        }
    }

    #[tracing::instrument(skip(self, root_package_json))]
    pub fn read_lockfile(
        &self,
        root_path: &AbsoluteSystemPath,
        root_package_json: &PackageJson,
    ) -> Result<Box<dyn Lockfile>, Error> {
        let lockfile_path = self.lockfile_path(root_path);
        let contents = match self {
            PackageManager::Bun => {
                Command::new(which("bun")?)
                    .arg(lockfile_path.to_string())
                    .current_dir(root_path.to_string())
                    .output()?
                    .stdout
            }
            _ => lockfile_path.read()?,
        };
        self.parse_lockfile(root_package_json, &contents)
    }

    #[tracing::instrument(skip(self, root_package_json, contents))]
    pub fn parse_lockfile(
        &self,
        root_package_json: &PackageJson,
        contents: &[u8],
    ) -> Result<Box<dyn Lockfile>, Error> {
        Ok(match self {
            PackageManager::Npm => Box::new(turborepo_lockfiles::NpmLockfile::load(contents)?),
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                Box::new(turborepo_lockfiles::PnpmLockfile::from_bytes(contents)?)
            }
            PackageManager::Yarn => {
                Box::new(turborepo_lockfiles::Yarn1Lockfile::from_bytes(contents)?)
            }
            PackageManager::Bun => {
                Box::new(turborepo_lockfiles::BunLockfile::from_bytes(contents)?)
            }
            PackageManager::Berry => Box::new(turborepo_lockfiles::BerryLockfile::load(
                contents,
                Some(turborepo_lockfiles::BerryManifest::with_resolutions(
                    root_package_json
                        .resolutions
                        .iter()
                        .flatten()
                        .map(|(k, v)| (k.clone(), v.clone())),
                )),
            )?),
        })
    }

    pub fn prune_patched_packages<R: AsRef<RelativeUnixPath>>(
        &self,
        package_json: &PackageJson,
        patches: &[R],
    ) -> PackageJson {
        match self {
            PackageManager::Berry => yarn::prune_patches(package_json, patches),
            PackageManager::Pnpm6 | PackageManager::Pnpm => {
                pnpm::prune_patches(package_json, patches)
            }
            PackageManager::Yarn | PackageManager::Npm | PackageManager::Bun => {
                unreachable!("bun, npm, and yarn 1 don't have a concept of patches")
            }
        }
    }

    pub fn lockfile_path(&self, turbo_root: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
        turbo_root.join_component(self.lockfile_name())
    }

    pub fn arg_separator(&self, user_args: &[String]) -> Option<&str> {
        match self {
            PackageManager::Yarn | PackageManager::Bun => {
                // Yarn and bun warn and swallows a "--" token. If the user is passing "--", we
                // need to prepend our own so that the user's doesn't get
                // swallowed. If they are not passing their own, we don't need
                // the "--" token and can avoid the warning.
                if user_args.iter().any(|arg| arg == "--") {
                    Some("--")
                } else {
                    None
                }
            }
            PackageManager::Npm | PackageManager::Pnpm6 => Some("--"),
            PackageManager::Pnpm | PackageManager::Berry => None,
        }
    }
}

fn filter_subfolder_package_jsons<T: IntoIterator<Item = AbsoluteSystemPathBuf>>(
    map: T,
) -> impl Iterator<Item = AbsoluteSystemPathBuf> {
    let mut last_parent = None;
    map.into_iter()
        .sorted_by(|a, b| {
            // get an iterator of the components of each path, and zip them together
            let mut segments = a.components().with_last().zip(b.components().with_last());

            // find the first pair of components that are different, and compare them
            // if one of the segments is the last, then the other is a subfolder of it.
            // we must always yield 'file-likes' (the last segment of a path) ahead of
            // subfolders (non-last segments) so that we can guarantee we find the
            // package.json before processing its subfolders
            segments
                .find_map(|((a_last, a_cmp), (b_last, b_cmp))| {
                    if a_last == b_last {
                        match a_cmp.cmp(&b_cmp) {
                            std::cmp::Ordering::Equal => None,
                            other => Some(other),
                        }
                    } else if a_last {
                        Some(std::cmp::Ordering::Less)
                    } else {
                        Some(std::cmp::Ordering::Greater)
                    }
                })
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .filter(move |entry| {
            match &last_parent {
                // last_parent is the parent of the last json we yielded. if the current
                // entry is a subfolder of it, we don't want to yield it
                Some(parent) if entry.starts_with(parent) => false,
                // update last_parent to the parent of the current entry
                _ => {
                    last_parent = Some(entry.parent().unwrap().to_owned());
                    true
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, collections::HashSet, fs::File};

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    struct TestCase {
        name: String,
        package_manager: String,
        expected_manager: String,
        expected_version: String,
        expected_error: bool,
    }

    fn repo_root() -> AbsoluteSystemPathBuf {
        let cwd = AbsoluteSystemPathBuf::cwd().unwrap();
        for ancestor in cwd.ancestors() {
            if ancestor.join_component(".git").exists() {
                return ancestor.to_owned();
            }
        }
        panic!("Couldn't find Turborepo root from {}", cwd);
    }

    #[test_case(&[
        "/a/b/package.json",
        "/a/package.json",
    ], &[
        "/a/package.json",
    ] ; "basic")]
    #[test_case(&[
        "/a/package.json",
        "/a/b/package.json",
    ], &[
        "/a/package.json",
    ] ; "order flipped")]
    #[test_case(&[
        "/a/package.json",
        "/b/package.json",
    ], &[
        "/a/package.json",
        "/b/package.json",
    ] ; "disjoint")]
    #[test_case(&[
        "/a/package.json",
        "/z/package.json",
        "/package.json"
    ], &[
        "/package.json",
    ] ; "root")]
    fn lexicographic_file_sort(inc: &[&str], expected: &[&str]) {
        let to_path = |s: &&str| {
            AbsoluteSystemPathBuf::new(if cfg!(windows) {
                Cow::from(format!("C:/{}", s))
            } else {
                (*s).into()
            })
            .unwrap()
        };

        let inc = inc.into_iter().map(to_path).collect::<Vec<_>>();
        let expected = expected.into_iter().map(to_path).collect::<Vec<_>>();
        let sorted = filter_subfolder_package_jsons(inc);
        let sorted = sorted.collect::<Vec<_>>();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn test_get_package_jsons() {
        let root = repo_root();
        let examples = root.join_component("examples");

        let with_yarn = examples.join_component("with-yarn");
        let with_yarn_expected: HashSet<AbsoluteSystemPathBuf> = HashSet::from_iter([
            with_yarn.join_components(&["apps", "docs", "package.json"]),
            with_yarn.join_components(&["apps", "web", "package.json"]),
            with_yarn.join_components(&["packages", "eslint-config", "package.json"]),
            with_yarn.join_components(&["packages", "typescript-config", "package.json"]),
            with_yarn.join_components(&["packages", "ui", "package.json"]),
        ]);
        for mgr in &[
            PackageManager::Berry,
            PackageManager::Yarn,
            PackageManager::Npm,
            PackageManager::Bun,
        ] {
            let found = mgr.get_package_jsons(&with_yarn).unwrap();
            let found: HashSet<AbsoluteSystemPathBuf> = HashSet::from_iter(found);
            assert_eq!(found, with_yarn_expected);
        }

        let basic = examples.join_component("basic");
        let mut basic_expected = Vec::from_iter([
            basic.join_components(&["apps", "docs", "package.json"]),
            basic.join_components(&["apps", "web", "package.json"]),
            basic.join_components(&["packages", "eslint-config", "package.json"]),
            basic.join_components(&["packages", "typescript-config", "package.json"]),
            basic.join_components(&["packages", "ui", "package.json"]),
        ]);
        basic_expected.sort();
        for mgr in &[PackageManager::Pnpm, PackageManager::Pnpm6] {
            let found = mgr.get_package_jsons(&basic).unwrap();
            let mut found = Vec::from_iter(found);
            found.sort();
            assert_eq!(found, basic_expected, "{}", mgr);
        }
    }

    #[test]
    fn test_get_workspace_ignores() {
        let root = repo_root();
        let fixtures = root.join_components(&[
            "crates",
            "turborepo-repository",
            "src",
            "package_manager",
            "fixtures",
        ]);
        for mgr in &[
            PackageManager::Npm,
            PackageManager::Yarn,
            PackageManager::Berry,
            PackageManager::Pnpm,
            PackageManager::Pnpm6,
        ] {
            let globs = mgr.get_workspace_globs(&fixtures).unwrap();
            let ignores: HashSet<String> = HashSet::from_iter(globs.raw_exclusions);
            let expected: &[&str] = match mgr {
                PackageManager::Npm => &["**/node_modules/**"],
                PackageManager::Berry => &["**/node_modules", "**/.git", "**/.yarn"],
                PackageManager::Bun => &["**/node_modules", "**/.git"],
                PackageManager::Yarn => &["apps/*/node_modules/**", "packages/*/node_modules/**"],
                PackageManager::Pnpm | PackageManager::Pnpm6 => &[
                    "**/node_modules/**",
                    "**/bower_components/**",
                    "packages/skip",
                ],
            };
            let expected: HashSet<String> =
                HashSet::from_iter(expected.iter().map(|s| s.to_string()));
            assert_eq!(ignores, expected);
        }
    }

    #[test]
    fn test_parse_package_manager_string() {
        let tests = vec![
            TestCase {
                name: "errors with a tag version".to_owned(),
                package_manager: "npm@latest".to_owned(),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "errors with no version".to_owned(),
                package_manager: "npm".to_owned(),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "requires fully-qualified semver versions (one digit)".to_owned(),
                package_manager: "npm@1".to_owned(),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "requires fully-qualified semver versions (two digits)".to_owned(),
                package_manager: "npm@1.2".to_owned(),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "supports custom labels".to_owned(),
                package_manager: "npm@1.2.3-alpha.1".to_owned(),
                expected_manager: "npm".to_owned(),
                expected_version: "1.2.3-alpha.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "only supports specified package managers".to_owned(),
                package_manager: "pip@1.2.3".to_owned(),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "supports npm".to_owned(),
                package_manager: "npm@0.0.1".to_owned(),
                expected_manager: "npm".to_owned(),
                expected_version: "0.0.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports pnpm".to_owned(),
                package_manager: "pnpm@0.0.1".to_owned(),
                expected_manager: "pnpm".to_owned(),
                expected_version: "0.0.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports yarn".to_owned(),
                package_manager: "yarn@111.0.1".to_owned(),
                expected_manager: "yarn".to_owned(),
                expected_version: "111.0.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports bun".to_owned(),
                package_manager: "bun@1.0.1".to_owned(),
                expected_manager: "bun".to_owned(),
                expected_version: "1.0.1".to_owned(),
                expected_error: false,
            },
        ];

        for case in tests {
            let result = PackageManager::parse_package_manager_string(&case.package_manager);
            let Ok((received_manager, received_version)) = result else {
                assert!(case.expected_error, "{}: received error", case.name);
                continue;
            };

            assert_eq!(received_manager, case.expected_manager);
            assert_eq!(received_version, case.expected_version);
        }
    }

    #[test]
    fn test_read_package_manager() -> Result<(), Error> {
        let mut package_json = PackageJson {
            package_manager: Some("npm@8.19.4".to_string()),
            ..Default::default()
        };
        let package_manager = PackageManager::read_package_manager(&package_json)?;
        assert_eq!(package_manager, Some(PackageManager::Npm));

        package_json.package_manager = Some("yarn@2.0.0".to_string());
        let package_manager = PackageManager::read_package_manager(&package_json)?;
        assert_eq!(package_manager, Some(PackageManager::Berry));

        package_json.package_manager = Some("yarn@1.9.0".to_string());
        let package_manager = PackageManager::read_package_manager(&package_json)?;
        assert_eq!(package_manager, Some(PackageManager::Yarn));

        package_json.package_manager = Some("pnpm@6.0.0".to_string());
        let package_manager = PackageManager::read_package_manager(&package_json)?;
        assert_eq!(package_manager, Some(PackageManager::Pnpm6));

        package_json.package_manager = Some("pnpm@7.2.0".to_string());
        let package_manager = PackageManager::read_package_manager(&package_json)?;
        assert_eq!(package_manager, Some(PackageManager::Pnpm));

        package_json.package_manager = Some("bun@1.0.1".to_string());
        let package_manager = PackageManager::read_package_manager(&package_json)?;
        assert_eq!(package_manager, Some(PackageManager::Bun));

        Ok(())
    }

    #[test]
    fn test_detect_multiple_package_managers() -> Result<(), Error> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;

        let package_lock_json_path = repo_root.path().join(npm::LOCKFILE);
        File::create(&package_lock_json_path)?;
        let pnpm_lock_path = repo_root.path().join(pnpm::LOCKFILE);
        File::create(pnpm_lock_path)?;

        let error = PackageManager::detect_package_manager(&repo_root_path).unwrap_err();
        assert_eq!(
            error.to_string(),
            "We detected multiple package managers in your repository: pnpm, npm. Please remove \
             one of them."
        );

        fs::remove_file(&package_lock_json_path)?;

        let package_manager = PackageManager::detect_package_manager(&repo_root_path)?;
        assert_eq!(package_manager, PackageManager::Pnpm);

        Ok(())
    }

    #[test]
    fn test_globs_test() {
        struct TestCase {
            globs: WorkspaceGlobs,
            root: AbsoluteSystemPathBuf,
            target: AbsoluteSystemPathBuf,
            output: Result<bool, Error>,
        }

        #[cfg(unix)]
        let root = AbsoluteSystemPathBuf::new("/a/b/c").unwrap();
        #[cfg(windows)]
        let root = AbsoluteSystemPathBuf::new("C:\\a\\b\\c").unwrap();

        #[cfg(unix)]
        let target = AbsoluteSystemPathBuf::new("/a/b/c/d/e/f").unwrap();
        #[cfg(windows)]
        let target = AbsoluteSystemPathBuf::new("C:\\a\\b\\c\\d\\e\\f").unwrap();

        let tests = [TestCase {
            globs: WorkspaceGlobs::new(vec!["d/**".to_string()], vec![]).unwrap(),
            root,
            target,
            output: Ok(true),
        }];

        for test in tests {
            match test.globs.target_is_workspace(&test.root, &test.target) {
                Ok(value) => assert_eq!(value, test.output.unwrap()),
                Err(value) => assert_eq!(value.to_string(), test.output.unwrap_err().to_string()),
            };
        }
    }

    #[test]
    fn test_nested_workspace_globs() -> Result<(), Error> {
        let top_level: PackageJsonWorkspaces =
            serde_json::from_str("{ \"workspaces\": [\"packages/**\"]}")?;
        assert_eq!(top_level.workspaces.as_ref(), vec!["packages/**"]);
        let nested: PackageJsonWorkspaces =
            serde_json::from_str("{ \"workspaces\": {\"packages\": [\"packages/**\"]}}")?;
        assert_eq!(nested.workspaces.as_ref(), vec!["packages/**"]);
        Ok(())
    }

    #[test]
    fn test_workspace_globs_trailing_slash() {
        let globs =
            WorkspaceGlobs::new(vec!["scripts/", "packages/**"], vec!["package/template"]).unwrap();
        assert_eq!(
            &globs
                .package_json_inclusions
                .iter()
                .map(|i| i.as_str())
                .collect::<Vec<_>>(),
            &["scripts/package.json", "packages/**/package.json"]
        );
    }
}
