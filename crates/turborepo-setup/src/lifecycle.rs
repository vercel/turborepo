//! Native lifecycle-script policy, decided before any dependency installer
//! runs. Installers evaluate a [`Policy`], print [`Policy::report`] before
//! starting the package manager, and append exactly [`Policy::args`].
//!
//! The default is native policy: setup passes nothing, so each manager's own
//! defaults and committed allowlists or trust settings stay authoritative and
//! setup never broadens them. A skip request maps to the manager's own skip
//! flag, and a manager without one is an error rather than a claimed skip.
//! Reports name the committed repository settings that shape native policy;
//! where a setting's effect is uncertain, they assume the broader execution.
//! User, global, and environment configuration still apply natively but are
//! not read here. Setup itself never runs Turborepo tasks.

use std::{collections::BTreeMap, fmt, fs, io::Read, path::Path};

use semver::Version;
use serde_json::Value;
use turborepo_package_manager::Family;

const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
const PNPM_WORKSPACE: &str = "pnpm-workspace.yaml";
const DISCLOSURE: &str = "Permitted lifecycle scripts run package code that can start other \
                          commands. turbo setup runs no Turborepo tasks.";

/// The dependency installer, with the exact locked version where native
/// defaults depend on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installer {
    Npm(Version),
    Pnpm(Version),
    /// Yarn classic below 2, modern Yarn from 2.
    Yarn(Version),
    Bun(Version),
    /// `uv sync` runs no install hooks, but source builds run build backends.
    Uv,
    /// `cargo fetch` downloads crates without running build scripts.
    Cargo,
    /// `go mod download` downloads modules without running module code.
    Go,
}

impl Installer {
    /// The installer for a declared JavaScript package manager, if supported.
    pub fn javascript(family: Family, version: Version) -> Option<Self> {
        match family {
            Family::Npm => Some(Self::Npm(version)),
            Family::Pnpm => Some(Self::Pnpm(version)),
            Family::Yarn => Some(Self::Yarn(version)),
            Family::Bun => Some(Self::Bun(version)),
            Family::Aube | Family::Nub => None,
        }
    }

    fn command(&self) -> &'static str {
        match self {
            Self::Npm(_) => "npm",
            Self::Pnpm(_) => "pnpm",
            Self::Yarn(_) => "yarn",
            Self::Bun(_) => "bun",
            Self::Uv => "uv",
            Self::Cargo => "cargo",
            Self::Go => "go",
        }
    }
}

impl fmt::Display for Installer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Npm(v) | Self::Pnpm(v) | Self::Yarn(v) | Self::Bun(v) => {
                write!(f, "{} {v}", self.command())
            }
            _ => f.write_str(self.command()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Native,
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Default,
    /// Root `setup` policy, including a toolchain's own value.
    Repository,
    /// `turbo setup --ignore-scripts`.
    Cli,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub intent: Intent,
    pub origin: Origin,
}

impl Request {
    /// CLI skip > repository policy > native default.
    pub fn new(cli_skip: bool, repository: Option<Intent>) -> Self {
        let (intent, origin) = match (cli_skip, repository) {
            (true, _) => (Intent::Skip, Origin::Cli),
            (false, Some(intent)) => (intent, Origin::Repository),
            (false, None) => (Intent::Native, Origin::Default),
        };
        Self { intent, origin }
    }
}

/// Which scripts of one kind may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scripts {
    Run,
    /// Only for packages the manager allows: a committed allowlist or its
    /// built-in list.
    Allowed,
    Blocked,
    /// Setup passes the manager's skip flag.
    Skipped,
}

/// A committed repository setting that shapes native policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setting {
    pub file: &'static str,
    pub key: &'static str,
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} in {}", self.key, self.file)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(
        "{installer} cannot skip lifecycle scripts: {reason}. Allow native lifecycle scripts or \
         skip this toolchain's dependencies"
    )]
    UnsupportedSkip {
        installer: String,
        reason: &'static str,
    },
    #[error("cannot read lifecycle settings from {0}; fix or remove it and retry")]
    Settings(&'static str),
}

/// The effective lifecycle policy for one installer in one repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    installer: Installer,
    request: Request,
    /// Root and workspace package scripts; `None` when the installer has none.
    project: Option<Scripts>,
    /// Dependency scripts; for uv, source-distribution build backends.
    dependencies: Option<Scripts>,
    settings: Vec<Setting>,
    args: &'static [&'static str],
}

impl Policy {
    /// Reads each committed settings file at the repository `root` once.
    /// Malformed or unsafe files fail closed rather than counting as absent.
    pub fn evaluate(root: &Path, installer: Installer, request: Request) -> Result<Self, Error> {
        let mut found = Found {
            root,
            texts: BTreeMap::new(),
            settings: Vec::new(),
        };
        let (mut project, mut dependencies) = native(&installer, &mut found)?;
        let mut args: &'static [&'static str] = &[];
        if request.intent == Intent::Skip {
            args = skip_args(&installer)?;
            for scripts in [&mut project, &mut dependencies].into_iter().flatten() {
                *scripts = Scripts::Skipped;
            }
        }
        Ok(Self {
            installer,
            request,
            project,
            dependencies,
            settings: found.settings,
            args,
        })
    }

    pub fn project(&self) -> Option<Scripts> {
        self.project
    }
    pub fn dependencies(&self) -> Option<Scripts> {
        self.dependencies
    }
    pub fn settings(&self) -> &[Setting] {
        &self.settings
    }

    /// Exact arguments to append to the native install command. Native
    /// policy adds none, and no policy needs environment changes.
    pub fn args(&self) -> &'static [&'static str] {
        self.args
    }

    /// Whether permitted scripts may run package code.
    pub fn runs_code(&self) -> bool {
        [self.project, self.dependencies]
            .into_iter()
            .flatten()
            .any(|scripts| matches!(scripts, Scripts::Run | Scripts::Allowed))
    }

    /// Lines to show before the installer starts.
    pub fn report(&self) -> Vec<String> {
        let installer = &self.installer;
        let command = installer.command();
        let uv = *installer == Installer::Uv;
        let mut line = if self.project.is_none() && self.dependencies.is_none() {
            format!("{installer}: dependency setup runs no package code")
        } else if self.request.intent == Intent::Skip {
            let origin = match self.request.origin {
                Origin::Cli => "--ignore-scripts",
                _ => "turbo.json setup policy",
            };
            let (what, consequence) = if uv {
                (
                    "source builds refused",
                    ", so packages without a compatible wheel fail",
                )
            } else {
                ("lifecycle scripts skipped", "")
            };
            let args = self.args.join(" ");
            format!("{installer}: {what} ({origin}); passing {args} to {command}{consequence}")
        } else {
            let scopes = [("project", self.project), ("dependency", self.dependencies)];
            let parts: Vec<_> = scopes
                .into_iter()
                .filter_map(|(scope, scripts)| match (scripts?, installer) {
                    (_, Installer::Uv) => Some(
                        "source distributions without a compatible wheel run their build backends"
                            .to_owned(),
                    ),
                    (Scripts::Allowed, Installer::Bun(_)) => {
                        Some(format!("{scope} scripts run only for trusted packages"))
                    }
                    (scripts, _) => Some(format!("{scope} scripts {}", describe(scripts))),
                })
                .collect();
            format!("{installer}: native {command} policy; {}", parts.join("; "))
        };
        if self.request.intent == Intent::Native && !self.settings.is_empty() {
            let settings: Vec<_> = self.settings.iter().map(ToString::to_string).collect();
            line.push_str(&format!(" (committed: {})", settings.join(", ")));
        }
        let mut lines = vec![line];
        if self.runs_code() {
            lines.push(DISCLOSURE.to_owned());
        }
        lines
    }
}

/// Whether the committed `.yarnrc.yml` sets `enableNetwork` for any host in
/// `networkSettings`. Modern Yarn prefers that over the global setting, so it
/// could re-enable the network that offline setup disables. Read with the
/// same bounds as lifecycle settings; a malformed file fails closed.
pub fn yarn_host_network(root: &Path) -> Result<bool, Error> {
    let mut found = Found {
        root,
        texts: BTreeMap::new(),
        settings: Vec::new(),
    };
    found.yaml(".yarnrc.yml", "networkSettings", |hosts| {
        hosts.as_mapping().is_some_and(|hosts| {
            hosts.values().any(|host| {
                host.get("enableNetwork")
                    .is_some_and(|enabled| !enabled.is_null() && enabled.as_bool() != Some(false))
            })
        })
    })
}

fn describe(scripts: Scripts) -> &'static str {
    match scripts {
        Scripts::Run => "run",
        Scripts::Allowed => "run only for allowed packages",
        Scripts::Blocked => "do not run",
        Scripts::Skipped => "skipped",
    }
}

fn skip_args(installer: &Installer) -> Result<&'static [&'static str], Error> {
    Ok(match installer {
        Installer::Yarn(v) if v.major == 2 => {
            return Err(Error::UnsupportedSkip {
                installer: installer.to_string(),
                reason: "Yarn 2 has no --mode=skip-build; upgrade to Yarn 3 or later",
            });
        }
        Installer::Yarn(v) if v.major > 2 => &["--mode=skip-build"],
        Installer::Npm(_) | Installer::Pnpm(_) | Installer::Yarn(_) | Installer::Bun(_) => {
            &["--ignore-scripts"]
        }
        Installer::Uv => &["--no-build"],
        Installer::Cargo | Installer::Go => &[],
    })
}

type Scopes = (Option<Scripts>, Option<Scripts>);

/// Native defaults for the exact version, adjusted by committed settings.
fn native(installer: &Installer, found: &mut Found) -> Result<Scopes, Error> {
    use Scripts::{Allowed, Blocked, Run};
    let blocked = (Some(Blocked), Some(Blocked));
    let mut deps = match installer {
        Installer::Npm(v) if v.major >= 12 => Allowed,
        Installer::Pnpm(v) if v.major >= 10 => Allowed,
        Installer::Bun(_) => Allowed,
        Installer::Uv => return Ok((None, Some(Run))),
        Installer::Cargo | Installer::Go => return Ok((None, None)),
        _ => Run,
    };
    let is_true = |value: &serde_yaml_ng::Value| value.as_bool() == Some(true);
    match installer {
        Installer::Npm(v) => {
            if found.rc(".npmrc", "ignore-scripts")? {
                return Ok(blocked);
            }
            if v.major >= 12 {
                found.package_key("allowScripts", &["allowScripts"])?;
                if found.rc(".npmrc", "dangerously-allow-all-scripts")? {
                    deps = Run;
                }
            }
        }
        Installer::Pnpm(v) => {
            if found.rc(".npmrc", "ignore-scripts")?
                | found.yaml(PNPM_WORKSPACE, "ignoreScripts", is_true)?
            {
                return Ok(blocked);
            }
            // pnpm 11 replaced these keys with allowBuilds.
            if v.major < 11 {
                let mut list = |key: &'static str, recorded: &'static str| {
                    Ok::<_, Error>(
                        found.yaml(PNPM_WORKSPACE, key, |_| true)?
                            | found.package_key(recorded, &["pnpm", key])?,
                    )
                };
                let only = list("onlyBuiltDependencies", "pnpm.onlyBuiltDependencies")?
                    | list(
                        "onlyBuiltDependenciesFile",
                        "pnpm.onlyBuiltDependenciesFile",
                    )?;
                // A deny list runs everything else; assume that when both exist.
                if list("neverBuiltDependencies", "pnpm.neverBuiltDependencies")? {
                    deps = Run;
                } else if only {
                    deps = Allowed;
                }
                if found.yaml(PNPM_WORKSPACE, "ignoreDepScripts", is_true)?
                    | found.rc(".npmrc", "ignore-dep-scripts")?
                {
                    deps = Blocked;
                }
            }
            if v.major >= 10 {
                found.yaml(PNPM_WORKSPACE, "allowBuilds", |_| true)?;
                if found.yaml(PNPM_WORKSPACE, "dangerouslyAllowAllBuilds", is_true)?
                    | found.rc(".npmrc", "dangerously-allow-all-builds")?
                {
                    deps = Run;
                }
            }
        }
        // Yarn classic's .npmrc fallback is not counted: assume scripts run.
        Installer::Yarn(v) if v.major < 2 => {
            if found.rc(".yarnrc", "ignore-scripts")? {
                return Ok(blocked);
            }
        }
        Installer::Yarn(_) => {
            let disabled = |value: &serde_yaml_ng::Value| value.as_bool() == Some(false);
            if found.yaml(".yarnrc.yml", "enableScripts", disabled)? {
                deps = Blocked;
            }
            let meta = found.package_value(&["dependenciesMeta"])?;
            if meta
                .as_ref()
                .and_then(Value::as_object)
                .is_some_and(|meta| meta.values().any(|entry| entry.get("built").is_some()))
            {
                found.record("package.json", "dependenciesMeta.*.built");
            }
        }
        Installer::Bun(_) => {
            if found.rc(".npmrc", "ignore-scripts")? | found.bunfig_ignore_scripts()? {
                return Ok(blocked);
            }
            match found.package_value(&["trustedDependencies"])? {
                Some(Value::Array(trusted)) => {
                    found.record("package.json", "trustedDependencies");
                    if trusted.is_empty() {
                        deps = Blocked;
                    }
                }
                Some(_) => return Err(Error::Settings("package.json")),
                None => {}
            }
        }
        Installer::Uv | Installer::Cargo | Installer::Go => {}
    }
    Ok((Some(Run), Some(deps)))
}

/// Bounded, cached reads of committed settings, recording each one that
/// shapes native policy.
struct Found<'a> {
    root: &'a Path,
    texts: BTreeMap<&'static str, Option<String>>,
    settings: Vec<Setting>,
}

impl Found<'_> {
    fn record(&mut self, file: &'static str, key: &'static str) {
        self.settings.push(Setting { file, key });
    }

    fn read(&mut self, file: &'static str) -> Result<Option<&str>, Error> {
        if !self.texts.contains_key(file) {
            let path = self.root.join(file);
            let text = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_SETTINGS_BYTES => {
                    let mut text = String::new();
                    fs::File::open(&path)
                        .and_then(|f| f.take(MAX_SETTINGS_BYTES + 1).read_to_string(&mut text))
                        .map_err(|_| Error::Settings(file))?;
                    if text.len() as u64 > MAX_SETTINGS_BYTES {
                        return Err(Error::Settings(file));
                    }
                    Some(text)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                _ => return Err(Error::Settings(file)),
            };
            self.texts.insert(file, text);
        }
        Ok(self.texts.get(file).and_then(Option::as_deref))
    }

    /// Whether an `.npmrc` (`key=true`) or `.yarnrc` (`key true`) line
    /// enables `key`.
    fn rc(&mut self, file: &'static str, key: &'static str) -> Result<bool, Error> {
        let separator = if file == ".npmrc" { '=' } else { ' ' };
        let enabled = self.read(file)?.is_some_and(|text| {
            text.lines().any(|line| {
                let line = line.trim().trim_start_matches("--");
                line.split_once(separator).is_some_and(|(name, value)| {
                    name.trim() == key && value.trim().trim_matches('"') == "true"
                })
            })
        });
        if enabled {
            self.record(file, key);
        }
        Ok(enabled)
    }

    /// The root package.json value at `path`.
    fn package_value(&mut self, path: &[&str]) -> Result<Option<Value>, Error> {
        let Some(text) = self.read("package.json")? else {
            return Ok(None);
        };
        let mut value: Value =
            serde_json::from_str(text).map_err(|_| Error::Settings("package.json"))?;
        for part in path {
            match value.get_mut(*part) {
                Some(next) => value = next.take(),
                None => return Ok(None),
            }
        }
        Ok(Some(value))
    }

    fn package_key(&mut self, key: &'static str, path: &[&str]) -> Result<bool, Error> {
        let present = self.package_value(path)?.is_some();
        if present {
            self.record("package.json", key);
        }
        Ok(present)
    }

    /// Whether top-level `key` is present and satisfies `applies`.
    fn yaml(
        &mut self,
        file: &'static str,
        key: &'static str,
        applies: impl Fn(&serde_yaml_ng::Value) -> bool,
    ) -> Result<bool, Error> {
        let Some(text) = self.read(file)? else {
            return Ok(false);
        };
        let value: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(text).map_err(|_| Error::Settings(file))?;
        if !(value.is_mapping() || value.is_null()) {
            return Err(Error::Settings(file));
        }
        let applies = value.get(key).is_some_and(applies);
        if applies {
            self.record(file, key);
        }
        Ok(applies)
    }

    fn bunfig_ignore_scripts(&mut self) -> Result<bool, Error> {
        let Some(text) = self.read("bunfig.toml")? else {
            return Ok(false);
        };
        let table: toml::Table =
            toml::from_str(text).map_err(|_| Error::Settings("bunfig.toml"))?;
        let enabled = table
            .get("install")
            .and_then(|install| install.get("ignoreScripts"))
            .and_then(toml::Value::as_bool)
            == Some(true);
        if enabled {
            self.record("bunfig.toml", "install.ignoreScripts");
        }
        Ok(enabled)
    }
}

#[cfg(test)]
mod tests;
