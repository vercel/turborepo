mod npm;
mod pnpm;
mod yarn;

use std::{
    backtrace,
    fmt::{self, Display},
    fs,
};

use globwalk::fix_glob_pattern;
use itertools::{Either, Itertools};
use lazy_regex::{lazy_regex, Lazy};
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPath};
use turborepo_lockfiles::Lockfile;
use turborepo_ui::{UI, UNDERLINE};
use wax::{Any, Glob, Pattern};

use crate::{
    package_json::PackageJson,
    package_manager::{npm::NpmDetector, pnpm::PnpmDetector, yarn::YarnDetector},
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

#[derive(Debug, Serialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Berry,
    Npm,
    Pnpm,
    Pnpm6,
    Yarn,
}

impl PackageManager {
    pub fn lockfile_path(&self, turbo_root: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
        let file_name = match self {
            PackageManager::Berry => yarn::LOCKFILE,
            PackageManager::Npm => npm::LOCKFILE,
            PackageManager::Pnpm => pnpm::LOCKFILE,
            PackageManager::Pnpm6 => pnpm::LOCKFILE,
            PackageManager::Yarn => yarn::LOCKFILE,
        };

        turbo_root.join_component(file_name)
    }
}

impl fmt::Display for PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Do not change these without also changing `GetPackageManager` in
        // packagemanager.go
        match self {
            PackageManager::Berry => write!(f, "berry"),
            PackageManager::Npm => write!(f, "npm"),
            PackageManager::Pnpm => write!(f, "pnpm"),
            PackageManager::Pnpm6 => write!(f, "pnpm6"),
            PackageManager::Yarn => write!(f, "yarn"),
        }
    }
}

// WorkspaceGlobs is suitable for finding package.json files via globwalk
#[derive(Debug)]
pub struct WorkspaceGlobs {
    directory_inclusions: Any<'static>,
    directory_exclusions: Any<'static>,
    package_json_inclusions: Vec<String>,
    raw_exclusions: Vec<String>,
}

impl PartialEq for WorkspaceGlobs {
    fn eq(&self, other: &Self) -> bool {
        // Use the literals for comparison, not the compiled globs
        self.package_json_inclusions == other.package_json_inclusions
            && self.raw_exclusions == other.raw_exclusions
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
                s.push_str("/package.json");
                s
            })
            .collect::<Vec<_>>();
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
        Ok(Self {
            directory_inclusions: any_with_contextual_error(inclusion_globs, raw_inclusions)?,
            directory_exclusions: any_with_contextual_error(
                exclusion_globs,
                raw_exclusions.clone(),
            )?,
            package_json_inclusions,
            raw_exclusions,
        })
    }

    pub fn target_is_workspace(
        &self,
        root: &AbsoluteSystemPath,
        target: &AbsoluteSystemPath,
    ) -> Result<bool, Error> {
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

impl NoPackageManager {
    // TODO: determine how to thread through user-friendly error message and apply
    // our UI
    #[allow(dead_code)]
    pub fn ui_display(&self, ui: &UI) -> String {
        let url =
            ui.apply(UNDERLINE.apply_to("https://nodejs.org/api/packages.html#packagemanager"));
        format!(
            "We did not find a package manager specified in your root package.json. Please set \
             the \"packageManager\" property in your root package.json ({url}) or run `npx \
             @turbo/codemod add-package-manager` in the root of your monorepo."
        )
    }
}

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
        };
        write!(f, "{}", err)
    }
}

impl From<&PackageManager> for MissingWorkspaceError {
    fn from(value: &PackageManager) -> Self {
        Self {
            package_manager: value.clone(),
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
    Glob(String, Box<wax::BuildError>),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::Error),
}

static PACKAGE_MANAGER_PATTERN: Lazy<Regex> =
    lazy_regex!(r"(?P<manager>npm|pnpm|yarn)@(?P<version>\d+\.\d+\.\d+(-.+)?)");

impl PackageManager {
    /// Returns the set of globs for the workspace.
    pub fn get_workspace_globs(
        &self,
        root_path: &AbsoluteSystemPath,
    ) -> Result<WorkspaceGlobs, Error> {
        let (mut inclusions, mut exclusions) = self.get_configured_workspace_globs(root_path)?;
        exclusions.extend(self.get_default_exclusions());

        // Yarn appends node_modules to every other glob specified
        if *self == PackageManager::Yarn {
            inclusions
                .iter_mut()
                .for_each(|inclusion| exclusions.push(format!("{inclusion}/node_modules/**")));
        }
        let globs = WorkspaceGlobs::new(inclusions, exclusions)?;
        Ok(globs)
    }

    fn get_default_exclusions(&self) -> impl Iterator<Item = String> {
        let ignores = match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                ["**/node_modules/**", "**/bower_components/**"].as_slice()
            }
            PackageManager::Npm => ["**/node_modules/**"].as_slice(),
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
                let workspace_yaml =
                    fs::read_to_string(root_path.join_component("pnpm-workspace.yaml"))?;
                let pnpm_workspace: PnpmWorkspace = serde_yaml::from_str(&workspace_yaml)?;
                if pnpm_workspace.packages.is_empty() {
                    return Err(MissingWorkspaceError::from(self).into());
                } else {
                    pnpm_workspace.packages
                }
            }
            PackageManager::Berry | PackageManager::Npm | PackageManager::Yarn => {
                let package_json_text =
                    fs::read_to_string(root_path.join_component("package.json"))?;
                let package_json: PackageJsonWorkspaces = serde_json::from_str(&package_json_text)?;

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

    #[allow(dead_code)]
    pub fn get_package_jsons(
        &self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<impl Iterator<Item = AbsoluteSystemPathBuf>, Error> {
        let globs = self.get_workspace_globs(repo_root)?;

        let files = globwalk::globwalk(
            repo_root,
            &globs.package_json_inclusions,
            &globs.raw_exclusions,
            globwalk::WalkType::Files,
        )?;
        Ok(files.into_iter())
    }

    pub fn lockfile_name(&self) -> &'static str {
        match self {
            PackageManager::Npm => npm::LOCKFILE,
            PackageManager::Pnpm | PackageManager::Pnpm6 => pnpm::LOCKFILE,
            PackageManager::Yarn | PackageManager::Berry => yarn::LOCKFILE,
        }
    }

    pub fn workspace_configuration_path(&self) -> Option<&'static str> {
        match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => Some("pnpm-workspace.yaml"),
            PackageManager::Npm | PackageManager::Berry | PackageManager::Yarn => None,
        }
    }

    pub fn read_lockfile(
        &self,
        root_path: &AbsoluteSystemPath,
        root_package_json: &PackageJson,
    ) -> Result<Box<dyn Lockfile>, Error> {
        let contents = root_path.join_component(self.lockfile_name()).read()?;
        self.parse_lockfile(root_package_json, &contents)
    }

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
            PackageManager::Yarn | PackageManager::Npm => {
                unreachable!("npm and yarn 1 don't have a concept of patches")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs::File};

    use tempfile::tempdir;
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

    #[test]
    fn test_get_package_jsons() {
        let root = repo_root();
        let examples = root.join_component("examples");

        let with_yarn = examples.join_component("with-yarn");
        let with_yarn_expected: HashSet<AbsoluteSystemPathBuf> = HashSet::from_iter([
            with_yarn.join_components(&["apps", "docs", "package.json"]),
            with_yarn.join_components(&["apps", "web", "package.json"]),
            with_yarn.join_components(&["packages", "eslint-config-custom", "package.json"]),
            with_yarn.join_components(&["packages", "tsconfig", "package.json"]),
            with_yarn.join_components(&["packages", "ui", "package.json"]),
        ]);
        for mgr in &[
            PackageManager::Berry,
            PackageManager::Yarn,
            PackageManager::Npm,
        ] {
            let found = mgr.get_package_jsons(&with_yarn).unwrap();
            let found: HashSet<AbsoluteSystemPathBuf> = HashSet::from_iter(found);
            assert_eq!(found, with_yarn_expected);
        }

        let basic = examples.join_component("basic");
        let basic_expected: HashSet<AbsoluteSystemPathBuf> = HashSet::from_iter([
            basic.join_components(&["apps", "docs", "package.json"]),
            basic.join_components(&["apps", "web", "package.json"]),
            basic.join_components(&["packages", "eslint-config-custom", "package.json"]),
            basic.join_components(&["packages", "tsconfig", "package.json"]),
            basic.join_components(&["packages", "ui", "package.json"]),
        ]);
        for mgr in &[PackageManager::Pnpm, PackageManager::Pnpm6] {
            let found = mgr.get_package_jsons(&basic).unwrap();
            let found: HashSet<AbsoluteSystemPathBuf> = HashSet::from_iter(found);
            assert_eq!(found, basic_expected);
        }
    }

    #[test]
    fn test_get_workspace_ignores() {
        let root = repo_root();
        let fixtures = root.join_components(&[
            "crates",
            "turborepo-lib",
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
}
