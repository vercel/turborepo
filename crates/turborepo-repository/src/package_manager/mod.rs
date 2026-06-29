pub mod berry;
pub mod bun;
pub mod npm;
pub mod npmrc;
pub mod nub;
pub mod pnpm;
pub mod yarn;
pub mod yarnrc;

use std::{
    fmt::{self, Display},
    fs,
    ops::Range,
};

use bun::BunDetector;
use itertools::{Either, Itertools};
use lazy_regex::{Lazy, lazy_regex};
use miette::{Diagnostic, NamedSource, SourceSpan};
use node_semver::{SemverError, Version};
use npm::NpmDetector;
use regex::Regex;
use serde::Deserialize;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPath};
use turborepo_errors::Spanned;
use turborepo_lockfiles::Lockfile;

use crate::{
    discovery,
    package_json::{self, PackageJson},
    package_manager::{pnpm::PnpmDetector, yarn::YarnDetector},
    workspaces::WorkspaceGlobs,
};

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

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PackageManager {
    Berry,
    Npm,
    Pnpm9,
    Pnpm,
    Pnpm6,
    Yarn,
    Bun,
    /// nub (<https://nub.dev>). nub has no lockfile format of its own; it is
    /// lockfile-compatible with whatever the project already uses. `lockfile`
    /// holds the concrete package manager whose lockfile is present in the
    /// repository, which lockfile operations delegate to. See [`nub`].
    Nub {
        lockfile: Box<PackageManager>,
    },
}

#[derive(Debug)]
pub struct MissingWorkspaceError {
    package_manager: PackageManager,
}

impl std::error::Error for MissingWorkspaceError {}

#[derive(Debug)]
pub struct NoPackageManager;

impl std::error::Error for NoPackageManager {}

impl Display for NoPackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "We did not find a package manager specified in your root package.json. \
        Please set the \"devEngines.packageManager\" property in your root package.json \
        or the legacy \"packageManager\" property (https://nodejs.org/api/packages.html#packagemanager) \
        or run `npx @turbo/codemod add-package-manager` in the root of your monorepo.")
    }
}

impl Display for MissingWorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let err = match self.package_manager {
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => {
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
            PackageManager::Nub { .. } => {
                "package.json: no workspaces found. Turborepo requires workspaces to be defined in \
                 the root package.json"
            }
        };
        write!(f, "{err}")
    }
}

impl From<PackageManager> for MissingWorkspaceError {
    fn from(value: PackageManager) -> Self {
        Self {
            package_manager: value,
        }
    }
}

impl From<wax::BuildError> for Error {
    fn from(value: wax::BuildError) -> Self {
        Self::Wax(Box::new(value))
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Workspace(#[from] MissingWorkspaceError),
    #[error("YAML parsing error: {0}")]
    ParsingYaml(#[from] serde_yaml_ng::Error),
    #[error("JSON parsing error: {0}")]
    ParsingJson(#[from] serde_json::Error),
    #[error("Globbing error: {0}")]
    Wax(Box<wax::BuildError>),
    #[error(transparent)]
    PackageJson(#[from] package_json::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error(transparent)]
    NoPackageManager(#[from] NoPackageManager),
    #[error("Multiple package managers in your repository: {}. Please use one package manager.", managers.join(", "))]
    MultiplePackageManagers { managers: Vec<String> },
    #[error("Invalid semantic version: {explanation}")]
    #[diagnostic(code(invalid_semantic_version))]
    InvalidVersion {
        explanation: String,
        #[label("version found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Invalid utf8: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(
        "Could not parse the `packageManager` field in package.json, expected to match regular \
         expression `{pattern}`."
    )]
    #[diagnostic(code(invalid_package_manager_field))]
    InvalidPackageManager {
        pattern: String,
        #[label("Invalid `packageManager` field")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Invalid `devEngines.packageManager` field in package.json: {message}")]
    #[diagnostic(code(invalid_dev_engines_package_manager_field))]
    InvalidDevEnginesPackageManager {
        message: String,
        #[label("Invalid `devEngines.packageManager` field")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error(
        "Package manager mismatch: `devEngines.packageManager` declares `{declared}`, but the \
         lockfile indicates `{detected}`."
    )]
    #[diagnostic(code(package_manager_lockfile_mismatch))]
    PackageManagerLockfileMismatch {
        declared: String,
        detected: String,
        #[label("Declared package manager")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error(transparent)]
    WorkspaceGlob(#[from] crate::workspaces::Error),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::Error),
    #[error("Lockfile not found at {0}")]
    LockfileMissing(AbsoluteSystemPathBuf),
    #[error("Discovering workspace: {0}")]
    WorkspaceDiscovery(#[from] discovery::Error),
    #[error("Missing `devEngines.packageManager` or legacy `packageManager` field in package.json")]
    MissingPackageManager,
    #[error(transparent)]
    Yarnrc(#[from] yarnrc::Error),
    #[error("Only found bun.lockb, please run `bun install --save-text-lockfile`")]
    BunBinaryLockfile,
    #[error(
        "Could not determine Yarn lockfile format from {0}. Expected a Yarn v1 header or Berry \
         __metadata block."
    )]
    UnrecognizedYarnLockfile(AbsoluteSystemPathBuf),
}

impl From<std::convert::Infallible> for Error {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
}

static PACKAGE_MANAGER_PATTERN: Lazy<Regex> = lazy_regex!(
    r"\A(?P<manager>bun|npm|nub|pnpm|yarn)@(?P<version>\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?|https?://\S+)\z"
);

static DEV_ENGINES_VERSION_PATTERN: Lazy<Regex> =
    lazy_regex!(r"\A\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?\z");

impl PackageManager {
    /// Returns the package manager responsible for lockfile operations.
    pub fn lockfile_manager(&self) -> &PackageManager {
        match self {
            PackageManager::Nub { lockfile } => lockfile.as_ref(),
            other => other,
        }
    }

    /// Whether this package manager uses a pnpm-family lockfile.
    pub fn is_pnpm_family(&self) -> bool {
        matches!(
            self.lockfile_manager(),
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9
        )
    }

    /// Re-resolves the underlying lockfile manager for [`PackageManager::Nub`]
    /// from disk. No-op for other variants. Call after daemon proto round-trips
    /// that cannot carry the underlying lockfile type.
    pub fn with_resolved_nub_lockfile(self, repo_root: &AbsoluteSystemPath) -> Self {
        match self {
            PackageManager::Nub { .. } => PackageManager::Nub {
                lockfile: Box::new(nub::underlying_lockfile_manager(repo_root)),
            },
            other => other,
        }
    }

    pub fn supported_managers() -> &'static [Self] {
        [
            Self::Npm,
            Self::Pnpm9,
            Self::Pnpm,
            Self::Pnpm6,
            Self::Yarn,
            Self::Berry,
            Self::Bun,
        ]
        .as_slice()
    }

    pub fn name(&self) -> &'static str {
        match self {
            PackageManager::Berry => "berry",
            PackageManager::Npm => "npm",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Pnpm6 => "pnpm6",
            PackageManager::Pnpm9 => "pnpm9",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun",
            PackageManager::Nub { .. } => "nub",
        }
    }

    pub fn command(&self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => "pnpm",
            PackageManager::Yarn | PackageManager::Berry => "yarn",
            PackageManager::Bun => "bun",
            PackageManager::Nub { .. } => "nub",
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
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => {
                pnpm::get_default_exclusions()
            }
            PackageManager::Npm | PackageManager::Nub { .. } => ["**/node_modules/**"].as_slice(),
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
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => {
                // Make sure to convert this to a missing workspace error
                // so we can catch it in the case of single package mode.
                pnpm::get_configured_workspace_globs(root_path)
                    .ok_or_else(|| Error::Workspace(MissingWorkspaceError::from(self.clone())))?
            }
            PackageManager::Berry
            | PackageManager::Npm
            | PackageManager::Yarn
            | PackageManager::Bun => {
                let package_json_text = fs::read_to_string(self.workspace_glob_source(root_path))?;
                let package_json: PackageJsonWorkspaces = serde_json::from_str(&package_json_text)
                    .map_err(|_| Error::Workspace(MissingWorkspaceError::from(self.clone())))?;

                if package_json.workspaces.as_ref().is_empty() {
                    return Err(MissingWorkspaceError::from(self.clone()).into());
                } else {
                    package_json.workspaces.into()
                }
            }
            PackageManager::Nub { lockfile } => {
                if lockfile.is_pnpm_family()
                    && root_path
                        .join_component(pnpm::WORKSPACE_CONFIGURATION_PATH)
                        .exists()
                {
                    pnpm::get_configured_workspace_globs(root_path)
                        .ok_or_else(|| Error::Workspace(MissingWorkspaceError::from(self.clone())))?
                } else {
                    let package_json_text =
                        fs::read_to_string(self.workspace_glob_source(root_path))?;
                    let package_json: PackageJsonWorkspaces =
                        serde_json::from_str(&package_json_text).map_err(|_| {
                            Error::Workspace(MissingWorkspaceError::from(self.clone()))
                        })?;

                    if package_json.workspaces.as_ref().is_empty() {
                        return Err(MissingWorkspaceError::from(self.clone()).into());
                    } else {
                        package_json.workspaces.into()
                    }
                }
            }
        };

        // Normalize globs by stripping leading "./" since paths are relative to the
        // root anyway and other parts of the codebase don't include the "./"
        // prefix. See https://github.com/vercel/turborepo/issues/8599
        let (inclusions, exclusions) = globs.into_iter().partition_map(|glob| {
            if let Some(exclusion) = glob.strip_prefix('!') {
                let exclusion = exclusion.strip_prefix("./").unwrap_or(exclusion);
                Either::Right(exclusion.to_string())
            } else {
                let glob = glob.strip_prefix("./").unwrap_or(&glob);
                Either::Left(glob.to_string())
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

    /// Try to extract the package manager from package.json.
    /// Package Manager will be read from package.json only using the file
    /// system if the version is a URL and we need to invoke the binary it
    /// points to for version information.
    pub fn get_package_manager(
        repo_root: &AbsoluteSystemPath,
        package_json: &PackageJson,
    ) -> Result<Self, Error> {
        Self::read_package_manager(repo_root, package_json)
    }

    // Attempts to read the package manager from the package.json
    fn read_package_manager(
        repo_root: &AbsoluteSystemPath,
        pkg: &PackageJson,
    ) -> Result<Self, Error> {
        let Some(package_manager) = &pkg.package_manager else {
            return Self::read_dev_engines_package_manager(repo_root, pkg);
        };

        let (manager, version) = Self::parse_package_manager_string(package_manager)?;
        // if version is a https attempt to check that instead
        if version.starts_with("http") {
            match manager {
                "npm" => Ok(PackageManager::Npm),
                "bun" => Ok(PackageManager::Bun),
                "nub" => Ok(PackageManager::Nub {
                    lockfile: Box::new(nub::underlying_lockfile_manager(repo_root)),
                }),
                "yarn" => Ok(YarnDetector::new(repo_root)
                    .next()
                    .ok_or_else(|| Error::MissingPackageManager)??),
                "pnpm" => Ok(PnpmDetector::new(repo_root)
                    .next()
                    .ok_or_else(|| Error::MissingPackageManager)??),
                _ => unreachable!(
                    "found invalid package manager even though regex should have caught it"
                ),
            }
        } else {
            let version = version.parse().map_err(|err: SemverError| {
                let (span, text) = package_manager.span_and_text("package.json");
                Error::InvalidVersion {
                    explanation: err.to_string(),
                    span,
                    text,
                }
            })?;
            match manager {
                "npm" => Ok(PackageManager::Npm),
                "bun" => Ok(PackageManager::Bun),
                "nub" => Ok(PackageManager::Nub {
                    lockfile: Box::new(nub::underlying_lockfile_manager(repo_root)),
                }),
                "yarn" => Ok(YarnDetector::detect_berry_or_yarn(&version)?),
                "pnpm" => Ok(PnpmDetector::detect_pnpm6_or_pnpm(&version)?),
                _ => unreachable!(
                    "found invalid package manager even though regex should have caught it"
                ),
            }
        }
    }

    fn read_dev_engines_package_manager(
        repo_root: &AbsoluteSystemPath,
        pkg: &PackageJson,
    ) -> Result<Self, Error> {
        let Some(dev_engines) = &pkg.dev_engines else {
            return Err(Error::MissingPackageManager);
        };
        let Some(dev_engines_obj) = dev_engines.as_object() else {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &[],
                "`devEngines` must be an object containing `packageManager`",
            ));
        };
        let Some(package_manager) = dev_engines_obj.get("packageManager") else {
            return Err(Error::MissingPackageManager);
        };
        let Some(package_manager_obj) = package_manager.as_object() else {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager"],
                "`devEngines.packageManager` must be an object",
            ));
        };

        if package_manager_obj.is_empty() {
            return Err(Self::invalid_dev_engines_package_manager_key_at(
                dev_engines,
                &["packageManager"],
                "expected `{ \"name\": \"pnpm\", \"version\": \"9.12.3\" }`",
            ));
        }

        let Some(name) = package_manager_obj.get("name") else {
            return Err(Self::invalid_dev_engines_package_manager_key_at(
                dev_engines,
                &["packageManager"],
                "`devEngines.packageManager.name` is required",
            ));
        };
        let Some(name) = name.as_str() else {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "name"],
                "`devEngines.packageManager.name` must be a string",
            ));
        };
        if name.is_empty() {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "name"],
                "`devEngines.packageManager.name` must not be empty",
            ));
        }
        if name.trim() != name {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "name"],
                "`devEngines.packageManager.name` must not contain leading or trailing whitespace",
            ));
        }
        if !matches!(name, "npm" | "pnpm" | "yarn" | "bun") {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "name"],
                "`devEngines.packageManager.name` must be one of `npm`, `pnpm`, `yarn`, or `bun`",
            ));
        }

        let Some(version) = package_manager_obj.get("version") else {
            return Err(Self::invalid_dev_engines_package_manager_key_at(
                dev_engines,
                &["packageManager"],
                "`devEngines.packageManager.version` is required",
            ));
        };
        let Some(version) = version.as_str() else {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "version"],
                "`devEngines.packageManager.version` must be a string",
            ));
        };
        if version.is_empty() {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "version"],
                "`devEngines.packageManager.version` must not be empty",
            ));
        }
        if version.trim() != version {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "version"],
                "`devEngines.packageManager.version` must not contain leading or trailing \
                 whitespace",
            ));
        }
        if !DEV_ENGINES_VERSION_PATTERN.is_match(version) {
            return Err(Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "version"],
                "`devEngines.packageManager.version` must be an exact semantic version",
            ));
        }

        let version = version.parse().map_err(|err: SemverError| {
            Self::invalid_dev_engines_package_manager_at(
                dev_engines,
                &["packageManager", "version"],
                format!("invalid semantic version: {err}"),
            )
        })?;
        let declared = Self::package_manager_from_name_and_version(name, &version)?;
        Self::validate_package_manager_lockfile_match(repo_root, &declared, dev_engines)?;

        Ok(declared)
    }

    fn package_manager_from_name_and_version(name: &str, version: &Version) -> Result<Self, Error> {
        match name {
            "npm" => Ok(PackageManager::Npm),
            "bun" => Ok(PackageManager::Bun),
            "yarn" => YarnDetector::detect_berry_or_yarn(version),
            "pnpm" => PnpmDetector::detect_pnpm6_or_pnpm(version),
            _ => unreachable!("devEngines package manager name should have been validated"),
        }
    }

    fn validate_package_manager_lockfile_match(
        repo_root: &AbsoluteSystemPath,
        declared: &Self,
        span_source: &Spanned<serde_json::Value>,
    ) -> Result<(), Error> {
        let detected = match Self::detect_package_manager(repo_root) {
            Ok(detected) => detected,
            Err(Error::NoPackageManager(_)) => return Ok(()),
            Err(err) => return Err(err),
        };

        if Self::same_lockfile_family(declared, &detected) {
            Ok(())
        } else {
            let (span, text) =
                Self::dev_engines_span_and_text(span_source, &["packageManager", "name"]);
            Err(Error::PackageManagerLockfileMismatch {
                declared: declared.command().to_string(),
                detected: detected.command().to_string(),
                span,
                text,
            })
        }
    }

    fn same_lockfile_family(left: &Self, right: &Self) -> bool {
        matches!(
            (left, right),
            (
                PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9,
                PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9
            ) | (PackageManager::Npm, PackageManager::Npm)
                | (PackageManager::Bun, PackageManager::Bun)
                | (PackageManager::Yarn, PackageManager::Yarn)
                | (PackageManager::Berry, PackageManager::Berry)
        )
    }

    fn invalid_dev_engines_package_manager_at(
        span_source: &Spanned<serde_json::Value>,
        path: &[&str],
        message: impl Into<String>,
    ) -> Error {
        let (span, text) = Self::dev_engines_span_and_text(span_source, path);
        Error::InvalidDevEnginesPackageManager {
            message: message.into(),
            span,
            text,
        }
    }

    fn invalid_dev_engines_package_manager_key_at(
        span_source: &Spanned<serde_json::Value>,
        path: &[&str],
        message: impl Into<String>,
    ) -> Error {
        let (span, text) = Self::dev_engines_key_span_and_text(span_source, path);
        Error::InvalidDevEnginesPackageManager {
            message: message.into(),
            span,
            text,
        }
    }

    fn dev_engines_key_span_and_text(
        span_source: &Spanned<serde_json::Value>,
        path: &[&str],
    ) -> (Option<SourceSpan>, NamedSource<String>) {
        let path_name = span_source
            .path
            .as_ref()
            .map_or("package.json", |path| path.as_ref());
        let Some(text) = span_source.text.as_ref() else {
            return span_source.span_and_text("package.json");
        };
        let Some(mut range) = span_source.range.clone() else {
            return span_source.span_and_text("package.json");
        };

        for (index, key) in path.iter().enumerate() {
            let Some(key_range) = Self::json_property_key_range(text, range.clone(), key) else {
                return span_source.span_and_text("package.json");
            };
            if index == path.len() - 1 {
                return (
                    Some(key_range.into()),
                    NamedSource::new(path_name, text.to_string()),
                );
            }

            let Some(value_range) = Self::json_property_value_range(text, range, key) else {
                return span_source.span_and_text("package.json");
            };
            range = value_range;
        }

        span_source.span_and_text("package.json")
    }

    fn dev_engines_span_and_text(
        span_source: &Spanned<serde_json::Value>,
        path: &[&str],
    ) -> (Option<SourceSpan>, NamedSource<String>) {
        let path_name = span_source
            .path
            .as_ref()
            .map_or("package.json", |path| path.as_ref());
        let Some(text) = span_source.text.as_ref() else {
            return span_source.span_and_text("package.json");
        };
        let Some(mut range) = span_source.range.clone() else {
            return span_source.span_and_text("package.json");
        };

        for key in path {
            let Some(nested_range) = Self::json_property_value_range(text, range, key) else {
                return span_source.span_and_text("package.json");
            };
            range = nested_range;
        }

        (
            Some(range.into()),
            NamedSource::new(path_name, text.to_string()),
        )
    }

    fn json_property_value_range(
        text: &str,
        containing_range: Range<usize>,
        key: &str,
    ) -> Option<Range<usize>> {
        let key_range = Self::json_property_key_range(text, containing_range.clone(), key)?;
        let mut cursor = key_range.end;
        let bytes = text.as_bytes();

        while cursor < containing_range.end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b':') {
            return None;
        }
        cursor += 1;
        while cursor < containing_range.end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }

        let value_end = Self::json_value_end(text, cursor, containing_range.end)?;
        Some(cursor..value_end)
    }

    fn json_property_key_range(
        text: &str,
        containing_range: Range<usize>,
        key: &str,
    ) -> Option<Range<usize>> {
        let pattern = format!("\"{key}\"");
        let search_text = text.get(containing_range.clone())?;
        let key_start = containing_range.start + search_text.find(&pattern)?;
        Some(key_start..key_start + pattern.len())
    }

    fn json_value_end(text: &str, start: usize, containing_end: usize) -> Option<usize> {
        let bytes = text.as_bytes();
        match bytes.get(start)? {
            b'"' => Self::json_string_end(bytes, start, containing_end),
            b'{' => Self::json_bracketed_value_end(bytes, start, containing_end, b'{', b'}'),
            b'[' => Self::json_bracketed_value_end(bytes, start, containing_end, b'[', b']'),
            _ => {
                let mut end = start;
                while end < containing_end
                    && !matches!(bytes[end], b',' | b'}' | b']')
                    && !bytes[end].is_ascii_whitespace()
                {
                    end += 1;
                }
                (end > start).then_some(end)
            }
        }
    }

    fn json_string_end(bytes: &[u8], start: usize, containing_end: usize) -> Option<usize> {
        let mut cursor = start + 1;
        let mut escaped = false;
        while cursor < containing_end {
            let byte = bytes[cursor];
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return Some(cursor + 1);
            }
            cursor += 1;
        }
        None
    }

    fn json_bracketed_value_end(
        bytes: &[u8],
        start: usize,
        containing_end: usize,
        open: u8,
        close: u8,
    ) -> Option<usize> {
        let mut cursor = start;
        let mut depth = 0usize;
        while cursor < containing_end {
            match bytes[cursor] {
                b'"' => cursor = Self::json_string_end(bytes, cursor, containing_end)? - 1,
                byte if byte == open => depth += 1,
                byte if byte == close => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(cursor + 1);
                    }
                }
                _ => {}
            }
            cursor += 1;
        }
        None
    }

    /// Try to detect package manager based on configuration files and binaries
    /// installed on the system.
    pub fn detect_package_manager(repo_root: &AbsoluteSystemPath) -> Result<Self, Error> {
        let detected_package_managers = PnpmDetector::new(repo_root)
            .chain(NpmDetector::new(repo_root))
            .chain(YarnDetector::new(repo_root))
            .chain(BunDetector::new(repo_root))
            .collect::<Result<Vec<_>, Error>>()?;

        match detected_package_managers.as_slice() {
            [] => Err(NoPackageManager.into()),
            [package_manager] => Ok(package_manager.clone()),
            _ => {
                let managers = detected_package_managers
                    .iter()
                    .map(|mgr| mgr.name().to_string())
                    .collect();
                Err(Error::MultiplePackageManagers { managers })
            }
        }
    }

    /// Try to extract package manager from package.json, otherwise detect based
    /// on configuration files and binaries installed on the system
    pub fn read_or_detect_package_manager(
        package_json: &PackageJson,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<Self, Error> {
        Self::get_package_manager(repo_root, package_json).or_else(|err| match err {
            Error::NoPackageManager(_)
            | Error::InvalidPackageManager { .. }
            | Error::InvalidVersion { .. } => Self::detect_package_manager(repo_root),
            err => Err(err),
        })
    }

    pub(crate) fn parse_package_manager_string(
        manager: &Spanned<String>,
    ) -> Result<(&str, &str), Error> {
        if let Some(captures) = PACKAGE_MANAGER_PATTERN.captures(manager) {
            let manager = captures.name("manager").unwrap().as_str();
            let version = captures.name("version").unwrap().as_str();
            Ok((manager, version))
        } else {
            let (span, text) = manager.span_and_text("package.json");
            Err(Error::InvalidPackageManager {
                pattern: PACKAGE_MANAGER_PATTERN.to_string(),
                span,
                text,
            })
        }
    }

    pub fn get_package_jsons(
        &self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<impl Iterator<Item = AbsoluteSystemPathBuf> + use<>, Error> {
        let globs = self.get_workspace_globs(repo_root)?;
        Ok(globs.get_package_jsons(repo_root)?)
    }

    pub fn lockfile_name(&self) -> &'static str {
        match self {
            PackageManager::Npm => npm::LOCKFILE,
            PackageManager::Bun => bun::LOCKFILE,
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => pnpm::LOCKFILE,
            PackageManager::Yarn | PackageManager::Berry => yarn::LOCKFILE,
            // nub uses the lockfile of whichever package manager the project
            // already uses; delegate to the resolved underlying manager.
            PackageManager::Nub { lockfile } => lockfile.lockfile_name(),
        }
    }

    pub fn workspace_configuration_path(&self) -> Option<&'static str> {
        match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => {
                Some(pnpm::WORKSPACE_CONFIGURATION_PATH)
            }
            PackageManager::Nub { lockfile } if lockfile.is_pnpm_family() => {
                Some(pnpm::WORKSPACE_CONFIGURATION_PATH)
            }
            PackageManager::Npm
            | PackageManager::Berry
            | PackageManager::Yarn
            | PackageManager::Bun
            // nub with a non-pnpm underlying lockfile reads `workspaces` from package.json.
            | PackageManager::Nub { .. } => None,
        }
    }

    #[tracing::instrument(skip(self, root_package_json))]
    pub fn read_lockfile(
        &self,
        root_path: &AbsoluteSystemPath,
        root_package_json: &PackageJson,
    ) -> Result<Box<dyn Lockfile>, Error> {
        if let PackageManager::Nub { lockfile } = self {
            return lockfile.read_lockfile(root_path, root_package_json);
        }

        // For pnpm, check if per-workspace lockfiles are configured
        if matches!(
            self,
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9
        ) && let Some(lockfile) =
            self.try_read_pnpm_per_workspace_lockfiles(root_path, root_package_json)?
        {
            return Ok(lockfile);
        }

        let lockfile_path = self.lockfile_path(root_path);
        let contents = lockfile_path
            .read()
            .map_err(|_| Error::LockfileMissing(lockfile_path.clone()))?;

        // Read .yarnrc.yml for Berry to get catalog information
        let yarnrc = if matches!(self, PackageManager::Berry) {
            Some(yarnrc::YarnRc::from_file(root_path)?)
        } else {
            None
        };

        self.parse_lockfile(root_package_json, &contents, yarnrc)
    }

    #[tracing::instrument(skip(self, root_package_json, contents, yarnrc))]
    pub fn parse_lockfile(
        &self,
        root_package_json: &PackageJson,
        contents: &[u8],
        yarnrc: Option<yarnrc::YarnRc>,
    ) -> Result<Box<dyn Lockfile>, Error> {
        Ok(match self {
            PackageManager::Npm => Box::new(turborepo_lockfiles::NpmLockfile::load(contents)?),
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => {
                Box::new(turborepo_lockfiles::PnpmLockfile::from_bytes(contents)?)
            }
            PackageManager::Yarn => {
                Box::new(turborepo_lockfiles::Yarn1Lockfile::from_bytes(contents)?)
            }
            PackageManager::Bun => {
                Box::new(turborepo_lockfiles::BunLockfile::from_bytes(contents)?)
            }
            PackageManager::Berry => {
                // Take ownership of yarnrc fields to avoid cloning
                let (catalog, catalogs) = yarnrc
                    .map(|y| (y.catalog, y.catalogs))
                    .unwrap_or((None, None));

                let manifest = turborepo_lockfiles::BerryManifest::new(
                    root_package_json
                        .resolutions
                        .iter()
                        .flatten()
                        .map(|(k, v)| (k.clone(), v.clone())),
                    catalog,
                    catalogs,
                );
                Box::new(turborepo_lockfiles::BerryLockfile::load(
                    contents,
                    Some(manifest),
                )?)
            }
            // nub delegates to the parser of the lockfile actually present.
            PackageManager::Nub { lockfile } => {
                return lockfile.parse_lockfile(root_package_json, contents, yarnrc);
            }
        })
    }

    pub fn prune_patched_packages<R: AsRef<RelativeUnixPath>>(
        &self,
        package_json: &PackageJson,
        patches: &[R],
        repo_root: &AbsoluteSystemPath,
    ) -> PackageJson {
        match self {
            PackageManager::Berry => yarn::prune_patches(package_json, patches),
            PackageManager::Pnpm9 | PackageManager::Pnpm6 | PackageManager::Pnpm => {
                pnpm::prune_patches(package_json, patches, repo_root)
            }
            PackageManager::Bun => bun::prune_patches(package_json, patches),
            PackageManager::Yarn | PackageManager::Npm => {
                unreachable!("npm and yarn 1 don't have a concept of patches")
            }
            // nub delegates patch pruning to the underlying package manager.
            PackageManager::Nub { lockfile } => {
                lockfile.prune_patched_packages(package_json, patches, repo_root)
            }
        }
    }

    /// When pnpm is configured with `shared-workspace-lockfile=false`, each
    /// workspace gets its own `pnpm-lock.yaml`. This method reads and
    /// merges them into a single lockfile. Returns `None` if shared
    /// lockfile mode is active (the default).
    fn try_read_pnpm_per_workspace_lockfiles(
        &self,
        root_path: &AbsoluteSystemPath,
        _root_package_json: &PackageJson,
    ) -> Result<Option<Box<dyn Lockfile>>, Error> {
        let npmrc = npmrc::NpmRc::from_file(root_path)
            .inspect_err(|e| tracing::debug!("unable to read npmrc: {e}"))
            .unwrap_or_default();

        // shared-workspace-lockfile defaults to true
        if npmrc.shared_workspace_lockfile != Some(false) {
            return Ok(None);
        }

        tracing::debug!(
            "shared-workspace-lockfile=false detected, reading per-workspace lockfiles"
        );

        let lockfile_name = self.lockfile_name();
        let root_lockfile_path = root_path.join_component(lockfile_name);
        let root_contents = root_lockfile_path
            .read()
            .map_err(|_| Error::LockfileMissing(root_lockfile_path.clone()))?;

        let mut lockfile = turborepo_lockfiles::PnpmLockfile::from_bytes(&root_contents)?;

        // Discover workspace directories by finding all package.json files
        let globs = self.get_workspace_globs(root_path)?;
        let workspace_package_jsons: Vec<_> = globs.get_package_jsons(root_path)?.collect();

        let mut workspace_lockfile_data: Vec<(String, Vec<u8>)> = Vec::new();
        for pkg_json_path in &workspace_package_jsons {
            let ws_dir = pkg_json_path.parent().expect("package.json has parent dir");
            let ws_lockfile_path = ws_dir.join_component(lockfile_name);
            if ws_lockfile_path.exists() {
                let relative_path = root_path
                    .anchor(ws_dir)
                    .expect("workspace is under repo root");
                let unix_path = relative_path.to_unix();
                let bytes = ws_lockfile_path.read().map_err(|e| {
                    tracing::warn!(
                        "Failed to read per-workspace lockfile at {}: {}",
                        ws_lockfile_path,
                        e
                    );
                    Error::Io(std::io::Error::other(format!(
                        "Failed to read {ws_lockfile_path}"
                    )))
                })?;
                workspace_lockfile_data.push((unix_path.to_string(), bytes));
            }
        }

        let refs: Vec<(&str, &[u8])> = workspace_lockfile_data
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect();
        lockfile.merge_per_workspace_lockfiles(&refs)?;

        Ok(Some(Box::new(lockfile)))
    }

    pub fn lockfile_path(&self, turbo_root: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
        turbo_root.join_component(self.lockfile_name())
    }

    pub fn arg_separator(&self, user_args: &[impl AsRef<str>]) -> Option<&str> {
        match self {
            PackageManager::Yarn | PackageManager::Bun => {
                // Yarn and bun warn and swallows a "--" token. If the user is passing "--", we
                // need to prepend our own so that the user's doesn't get
                // swallowed. If they are not passing their own, we don't need
                // the "--" token and can avoid the warning.
                if user_args.iter().any(|arg| arg.as_ref() == "--") {
                    Some("--")
                } else {
                    None
                }
            }
            PackageManager::Npm | PackageManager::Pnpm6 => Some("--"),
            // nub has a pnpm-compatible CLI, which forwards script arguments
            // without needing a `--` separator.
            PackageManager::Pnpm
            | PackageManager::Pnpm9
            | PackageManager::Berry
            | PackageManager::Nub { .. } => None,
        }
    }

    /// Returns whether or not the package manager will select a package in the
    /// workspace as a dependency if the `workspace:` protocol isn't used.
    /// For example if a package in the workspace has `"lib": "1.2.3"` and
    /// there's a package in the workspace with the name of `lib` and
    /// version `1.2.3` if this is true, then the local `lib` package will
    /// be used where `false` would use a `lib` package from the registry.
    pub fn link_workspace_packages(&self, repo_root: &AbsoluteSystemPath) -> bool {
        match self {
            PackageManager::Berry => berry::link_workspace_packages(repo_root),
            PackageManager::Pnpm9 | PackageManager::Pnpm | PackageManager::Pnpm6 => {
                let pnpm_version = pnpm::PnpmVersion::try_from(self)
                    .expect("attempted to extract pnpm version from non-pnpm package manager");
                pnpm::link_workspace_packages(pnpm_version, repo_root)
            }
            PackageManager::Yarn | PackageManager::Bun | PackageManager::Npm => true,
            // nub links workspace packages by default, delegating to the
            // underlying manager's behavior where it has one.
            PackageManager::Nub { lockfile } => lockfile.link_workspace_packages(repo_root),
        }
    }

    /// Read catalog definitions from the package manager's configuration.
    /// Currently only pnpm supports catalogs in pnpm-workspace.yaml.
    pub fn read_catalogs(&self, repo_root: &AbsoluteSystemPath) -> Option<pnpm::PnpmCatalogs> {
        match self.lockfile_manager() {
            PackageManager::Pnpm9 | PackageManager::Pnpm | PackageManager::Pnpm6 => {
                pnpm::read_catalogs(repo_root)
            }
            // Berry catalogs are handled during lockfile parsing, not here.
            // Bun catalogs are handled during lockfile parsing, not here.
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::TempDir;
    use test_case::test_case;

    use super::*;
    use crate::discovery::{
        LocalPackageDiscoveryBuilder, PackageDiscovery, PackageDiscoveryBuilder,
    };

    struct TestCase {
        name: String,
        package_manager: Spanned<String>,
        expected_manager: String,
        expected_version: String,
        expected_error: bool,
    }

    const COREPACK_HASHED_YARN: &str =
        "yarn@3.2.3+sha224.953c8233f7a92884eee2de69a1b92d1f2ec1655e66d08071ba9a02fa";
    const COREPACK_HASHED_YARN_VERSION: &str =
        "3.2.3+sha224.953c8233f7a92884eee2de69a1b92d1f2ec1655e66d08071ba9a02fa";

    fn repo_root() -> AbsoluteSystemPathBuf {
        let cwd = AbsoluteSystemPathBuf::cwd().unwrap();
        for ancestor in cwd.ancestors() {
            if ancestor.join_component(".git").exists() {
                return ancestor.to_owned();
            }
        }
        panic!("Couldn't find Turborepo root from {cwd}");
    }

    fn package_json(value: serde_json::Value) -> PackageJson {
        PackageJson::from_value(value).unwrap()
    }

    fn dev_engines_package_manager(
        name: serde_json::Value,
        version: serde_json::Value,
    ) -> PackageJson {
        package_json(json!({
            "devEngines": {
                "packageManager": {
                    "name": name,
                    "version": version
                }
            }
        }))
    }

    fn temp_repo_root() -> Result<(TempDir, AbsoluteSystemPathBuf), Error> {
        let dir = TempDir::new()?;
        let repo_root = AbsoluteSystemPath::from_std_path(dir.path())?.to_owned();
        Ok((dir, repo_root))
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
            assert_eq!(found, basic_expected, "{}", mgr.name());
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
                PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => &[
                    "**/node_modules/**",
                    "**/bower_components/**",
                    "packages/skip",
                ],
                PackageManager::Nub { .. } => &["**/node_modules/**"],
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
                package_manager: Spanned::new("npm@latest".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "errors with no version".to_owned(),
                package_manager: Spanned::new("npm".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "requires fully-qualified semver versions (one digit)".to_owned(),
                package_manager: Spanned::new("npm@1".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "requires fully-qualified semver versions (two digits)".to_owned(),
                package_manager: Spanned::new("npm@1.2".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "supports custom labels".to_owned(),
                package_manager: Spanned::new("npm@1.2.3-alpha.1".to_owned()),
                expected_manager: "npm".to_owned(),
                expected_version: "1.2.3-alpha.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports corepack integrity hashes".to_owned(),
                package_manager: Spanned::new(COREPACK_HASHED_YARN.to_owned()),
                expected_manager: "yarn".to_owned(),
                expected_version: COREPACK_HASHED_YARN_VERSION.to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "errors with leading characters before manager".to_owned(),
                package_manager: Spanned::new("prefix npm@1.2.3".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "errors with trailing characters after version".to_owned(),
                package_manager: Spanned::new("npm@1.2.3suffix".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "only supports specified package managers".to_owned(),
                package_manager: Spanned::new("pip@1.2.3".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "supports npm".to_owned(),
                package_manager: Spanned::new("npm@0.0.1".to_owned()),
                expected_manager: "npm".to_owned(),
                expected_version: "0.0.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports pnpm".to_owned(),
                package_manager: Spanned::new("pnpm@0.0.1".to_owned()),
                expected_manager: "pnpm".to_owned(),
                expected_version: "0.0.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports yarn".to_owned(),
                package_manager: Spanned::new("yarn@111.0.1".to_owned()),
                expected_manager: "yarn".to_owned(),
                expected_version: "111.0.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports bun".to_owned(),
                package_manager: Spanned::new("bun@1.0.1".to_owned()),
                expected_manager: "bun".to_owned(),
                expected_version: "1.0.1".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports nub".to_owned(),
                package_manager: Spanned::new("nub@0.1.0".to_owned()),
                expected_manager: "nub".to_owned(),
                expected_version: "0.1.0".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "supports custom URL".to_owned(),
                package_manager: Spanned::new("npm@https://some-npm-fork".to_owned()),
                expected_manager: "npm".to_owned(),
                expected_version: "https://some-npm-fork".to_owned(),
                expected_error: false,
            },
            TestCase {
                name: "errors with leading whitespace before URL manager".to_owned(),
                package_manager: Spanned::new(" npm@https://some-npm-fork".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
            },
            TestCase {
                name: "errors with trailing whitespace after URL".to_owned(),
                package_manager: Spanned::new("npm@https://some-npm-fork ".to_owned()),
                expected_manager: "".to_owned(),
                expected_version: "".to_owned(),
                expected_error: true,
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
        let dir = TempDir::new()?;
        let repo_root = AbsoluteSystemPath::from_std_path(dir.path())?;
        let mut package_json = PackageJson {
            package_manager: Some(Spanned::new("npm@8.19.4".to_string())),
            ..Default::default()
        };
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Npm);

        package_json.package_manager = Some(Spanned::new("yarn@2.0.0".to_string()));
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Berry);

        package_json.package_manager = Some(Spanned::new(COREPACK_HASHED_YARN.to_string()));
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Berry);

        package_json.package_manager = Some(Spanned::new("yarn@1.9.0".to_string()));
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Yarn);

        package_json.package_manager = Some(Spanned::new("pnpm@6.0.0".to_string()));
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Pnpm6);

        package_json.package_manager = Some(Spanned::new("pnpm@7.2.0".to_string()));
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Pnpm);

        package_json.package_manager = Some(Spanned::new("bun@1.0.1".to_string()));
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Bun);

        // nub has no lockfile of its own, so with none present it resolves to the
        // npm-compatible default lockfile.
        package_json.package_manager = Some(Spanned::new("nub@0.1.0".to_string()));
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(
            package_manager,
            PackageManager::Nub {
                lockfile: Box::new(PackageManager::Npm)
            }
        );

        Ok(())
    }

    #[test_case("npm", "10.5.0", PackageManager::Npm ; "npm")]
    #[test_case("bun", "1.1.0", PackageManager::Bun ; "bun")]
    #[test_case("yarn", "1.22.22", PackageManager::Yarn ; "yarn classic")]
    #[test_case("yarn", "4.5.0", PackageManager::Berry ; "yarn berry")]
    #[test_case("pnpm", "6.35.1", PackageManager::Pnpm6 ; "pnpm6")]
    #[test_case("pnpm", "8.15.9", PackageManager::Pnpm ; "pnpm")]
    #[test_case("pnpm", "9.12.3", PackageManager::Pnpm9 ; "pnpm9")]
    #[test_case("pnpm", "9.12.3-alpha.0", PackageManager::Pnpm ; "pnpm prerelease")]
    fn test_read_dev_engines_package_manager(
        name: &str,
        version: &str,
        expected: PackageManager,
    ) -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        let package_json = dev_engines_package_manager(json!(name), json!(version));

        let package_manager = PackageManager::read_package_manager(&repo_root, &package_json)?;

        assert_eq!(package_manager, expected);
        Ok(())
    }

    #[test]
    fn test_top_level_package_manager_takes_precedence_over_dev_engines() -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        let package_json = package_json(json!({
            "packageManager": "npm@10.5.0",
            "devEngines": {
                "packageManager": {
                    "name": "not-supported",
                    "version": 1
                }
            }
        }));

        let package_manager = PackageManager::read_package_manager(&repo_root, &package_json)?;

        assert_eq!(package_manager, PackageManager::Npm);
        Ok(())
    }

    #[test_case(json!({"devEngines": []}), "`devEngines` must be an object" ; "devEngines array")]
    #[test_case(json!({"devEngines": null}), "`devEngines` must be an object" ; "devEngines null")]
    #[test_case(json!({"devEngines": {"packageManager": []}}), "`devEngines.packageManager` must be an object" ; "packageManager array")]
    #[test_case(json!({"devEngines": {"packageManager": null}}), "`devEngines.packageManager` must be an object" ; "packageManager null")]
    #[test_case(json!({"devEngines": {"packageManager": {}}}), "expected" ; "empty object")]
    #[test_case(json!({"devEngines": {"packageManager": {"version": "9.12.3"}}}), "name` is required" ; "missing name")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": 1, "version": "9.12.3"}}}), "name` must be a string" ; "non-string name")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "", "version": "9.12.3"}}}), "name` must not be empty" ; "empty name")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": " pnpm", "version": "9.12.3"}}}), "name` must not contain" ; "name whitespace")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pip", "version": 1}}}), "name` must be one of" ; "unsupported name before version")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pnpm"}}}), "version` is required" ; "missing version")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pnpm", "version": 1}}}), "version` must be a string" ; "non-string version")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pnpm", "version": ""}}}), "version` must not be empty" ; "empty version")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pnpm", "version": " 9.12.3"}}}), "version` must not contain" ; "version whitespace")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pnpm", "version": "^9.0.0"}}}), "exact semantic version" ; "range version")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pnpm", "version": "https://registry.npmjs.org/pnpm/-/pnpm-9.12.3.tgz"}}}), "exact semantic version" ; "url version")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pnpm", "version": "9"}}}), "exact semantic version" ; "short version")]
    #[test_case(json!({"devEngines": {"packageManager": {"name": "pnpm", "version": "9.12.3+sha512.Purxi/Zex=="}}}), "exact semantic version" ; "integrity version")]
    fn test_invalid_dev_engines_package_manager(
        value: serde_json::Value,
        expected_message: &str,
    ) -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        let package_json = package_json(value);

        let err = PackageManager::read_package_manager(&repo_root, &package_json).unwrap_err();

        let Error::InvalidDevEnginesPackageManager { message, .. } = err else {
            panic!("expected InvalidDevEnginesPackageManager, got {err:?}");
        };
        assert!(
            message.contains(expected_message),
            "expected {message:?} to contain {expected_message:?}"
        );
        Ok(())
    }

    #[test]
    fn test_missing_dev_engines_package_manager_version_span() -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        let contents = r#"{
  "devEngines": {
    "packageManager": {
      "name": "pnpm"
    }
  }
}"#;
        let package_json = PackageJson::load_from_str(contents, "package.json").unwrap();

        let err = PackageManager::read_package_manager(&repo_root, &package_json).unwrap_err();

        let Error::InvalidDevEnginesPackageManager {
            span: Some(span), ..
        } = err
        else {
            panic!("expected InvalidDevEnginesPackageManager with span, got {err:?}");
        };
        let snippet = &contents[span.offset()..span.offset() + span.len()];
        assert_eq!(snippet, "\"packageManager\"");
        assert!(!snippet.contains("devEngines"));
        Ok(())
    }

    #[test]
    fn test_missing_declarations_produces_missing_package_manager_error() -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        let package_json = package_json(json!({}));

        let err = PackageManager::read_package_manager(&repo_root, &package_json).unwrap_err();

        assert!(matches!(err, Error::MissingPackageManager));
        assert!(err.to_string().contains("devEngines.packageManager"));
        Ok(())
    }

    #[test]
    fn test_read_or_detect_does_not_infer_missing_declarations() -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        std::fs::write(repo_root.join_component(npm::LOCKFILE).as_std_path(), "{}")?;
        let package_json = package_json(json!({}));

        let err =
            PackageManager::read_or_detect_package_manager(&package_json, &repo_root).unwrap_err();

        assert!(matches!(err, Error::MissingPackageManager));
        Ok(())
    }

    #[test]
    fn test_dev_engines_package_manager_lockfile_mismatch() -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        std::fs::write(repo_root.join_component(npm::LOCKFILE).as_std_path(), "{}")?;
        let package_json = dev_engines_package_manager(json!("pnpm"), json!("9.12.3"));

        let err = PackageManager::read_package_manager(&repo_root, &package_json).unwrap_err();

        assert!(matches!(
            err,
            Error::PackageManagerLockfileMismatch { declared, detected, .. }
                if declared == "pnpm" && detected == "npm"
        ));
        Ok(())
    }

    #[test]
    fn test_dev_engines_package_manager_multiple_lockfiles() -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        std::fs::write(repo_root.join_component(npm::LOCKFILE).as_std_path(), "{}")?;
        std::fs::write(repo_root.join_component(pnpm::LOCKFILE).as_std_path(), "")?;
        let package_json = dev_engines_package_manager(json!("pnpm"), json!("9.12.3"));

        let err = PackageManager::read_package_manager(&repo_root, &package_json).unwrap_err();

        assert!(matches!(err, Error::MultiplePackageManagers { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn test_allow_no_package_manager_bypasses_dev_engines_validation() -> Result<(), Error> {
        let (_dir, repo_root) = temp_repo_root()?;
        std::fs::write(repo_root.join_component(npm::LOCKFILE).as_std_path(), "{}")?;
        std::fs::write(
            repo_root.join_component("package.json").as_std_path(),
            r#"{"workspaces":[]}"#,
        )?;
        let package_json = package_json(json!({
            "devEngines": {
                "packageManager": {
                    "name": "pnpm",
                    "version": "not-semver"
                }
            }
        }));
        let mut builder = LocalPackageDiscoveryBuilder::new(repo_root, None, Some(package_json));
        builder.with_allow_no_package_manager(true);

        let discovery = builder.build()?;
        let response = discovery.discover_packages().await.unwrap();

        assert_eq!(response.package_manager, PackageManager::Npm);
        Ok(())
    }

    #[test]
    fn test_read_yarn_url_package_manager_from_lockfile() -> Result<(), Error> {
        let dir = TempDir::new()?;
        let repo_root = AbsoluteSystemPath::from_std_path(dir.path())?;
        let package_json = PackageJson {
            package_manager: Some(Spanned::new(
                "yarn@https://repo.yarnpkg.com/4.5.0/packages/yarnpkg-cli/bin/yarn.js".to_string(),
            )),
            ..Default::default()
        };
        let lockfile_path = repo_root.join_component(yarn::LOCKFILE);

        std::fs::write(
            lockfile_path.as_std_path(),
            "# This file is generated by running \"yarn install\" inside your \
             project.\n\n__metadata:\n  version: 6\n  cacheKey: 8\n",
        )?;
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Berry);

        std::fs::write(
            lockfile_path.as_std_path(),
            "# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n# yarn lockfile \
             v1\n",
        )?;
        let package_manager = PackageManager::read_package_manager(repo_root, &package_json)?;
        assert_eq!(package_manager, PackageManager::Yarn);

        Ok(())
    }

    #[test]
    fn test_read_yarn_url_package_manager_errors_on_unrecognized_lockfile() -> Result<(), Error> {
        let dir = TempDir::new()?;
        let repo_root = AbsoluteSystemPath::from_std_path(dir.path())?;
        let package_json = PackageJson {
            package_manager: Some(Spanned::new("yarn@https://example.com/yarn.js".to_string())),
            ..Default::default()
        };
        let lockfile_path = repo_root.join_component(yarn::LOCKFILE);
        std::fs::write(lockfile_path.as_std_path(), "not a yarn lockfile\n")?;

        let err = PackageManager::read_package_manager(repo_root, &package_json).unwrap_err();
        assert!(matches!(err, Error::UnrecognizedYarnLockfile(_)));

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

    /// Test that workspace globs with leading "./" are normalized
    /// See https://github.com/vercel/turborepo/issues/8599
    #[test]
    fn test_workspace_globs_leading_dot_slash_normalized() -> Result<(), Error> {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let package_json_path = repo_root.join_component("package.json");

        // Test with leading "./" in workspace globs
        std::fs::write(
            package_json_path.as_std_path(),
            r#"{"workspaces": ["./packages/*", "!./packages/excluded"]}"#,
        )?;

        let pm = PackageManager::Npm;
        let globs = pm.get_workspace_globs(repo_root)?;

        // Verify the leading "./" is stripped from inclusions
        assert_eq!(globs.raw_inclusions, vec!["packages/*"]);
        // Exclusions include both the configured exclusion (normalized) and default
        // exclusions
        assert!(
            globs
                .raw_exclusions
                .contains(&"packages/excluded".to_string())
        );
        // Make sure it's normalized (doesn't have leading "./")
        assert!(
            !globs
                .raw_exclusions
                .contains(&"./packages/excluded".to_string())
        );

        Ok(())
    }

    #[test_case(PackageManager::Npm)]
    #[test_case(PackageManager::Yarn)]
    #[test_case(PackageManager::Bun)]
    fn test_link_workspace_packages_enabled_by_default(pm: PackageManager) {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let actual = pm.link_workspace_packages(repo_root);
        assert!(
            actual,
            "all package managers without a special implementation should use workspace packages"
        );
    }
}
