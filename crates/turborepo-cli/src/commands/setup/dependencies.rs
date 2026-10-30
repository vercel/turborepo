//! Locked dependency setup after tool provisioning. Each enabled ecosystem
//! runs its native frozen install with the managed tools that setup just
//! installed and recorded, never a Turborepo task. Committed
//! ecosystem lockfiles are required and never rewritten. Repeats rerun the
//! native command: an unchanged lockfile does not prove that installed
//! dependencies are ready. `--offline` uses each manager's native offline
//! mode, which fails on anything missing from its cache; a manager without one
//! is refused before setup changes anything.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

use semver::Version;
use turborepo_repository::toolchain::ToolchainId;
use turborepo_setup::{
    activation::ActivationPlan,
    cargo_prefetch,
    go_prefetch::{self, Prefetch},
    js_install::{self, Frozen, Offline},
    lock::{Ecosystems, Lock, Snapshot},
    uv_sync::{self, UvSync},
};
use turborepo_turbo_json::{DependencyPolicy, ToolPolicy};

use super::{
    SetupRequest, effective,
    output::{Console, DependencyReport, InstallStep, StepOutcome, StepResult},
    root::Discovery,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "JavaScript dependency setup needs a package manager declared in the root package.json \
         (packageManager or devEngines.packageManager); declare one, or set \
         setup.javascript.dependencies to \"skip\" in turbo.json"
    )]
    UndeclaredManager,
    #[error("turbo.lock has no valid {0} selection; run turbo setup locally and commit turbo.lock")]
    Unlocked(&'static str),
    #[error("managed {0} is missing from the managed tools; run turbo setup")]
    NotInstalled(&'static str),
    #[error("managed tools have no matching trusted setup record; rerun turbo setup")]
    Untrusted,
    #[error("cannot run dependency setup on this host: {0}")]
    Host(String),
    #[error(
        "offline setup cannot keep these dependency installers off the network; nothing was \
         installed:\n{0}\nRun turbo setup without --offline, add --tools-only to prepare only \
         tools offline, or set setup.<toolchain>.dependencies to \"skip\" in turbo.json"
    )]
    OfflineUnsupported(String),
    #[error(transparent)]
    Install(#[from] js_install::Error),
    #[error(transparent)]
    Uv(#[from] uv_sync::Error),
    #[error(transparent)]
    Cargo(#[from] cargo_prefetch::Error),
    #[error(transparent)]
    Go(#[from] go_prefetch::Error),
    #[error(transparent)]
    Activation(#[from] turborepo_setup::activation::Error),
    #[error(transparent)]
    Sources(#[from] turborepo_setup::lock::StorageError),
    #[error("dependency setup I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// What setup does with one eligible toolchain's dependencies.
enum Step {
    /// Skipped, with the reason it reports.
    Skip(String),
    Install(Adapter),
}

enum Adapter {
    Javascript {
        frozen: Frozen,
        lockfile: &'static str,
    },
    Uv,
    /// `cargo fetch --locked` into the clone's private Cargo home.
    Cargo,
    /// `go mod download` with read-only module files.
    Go,
}

/// One installer invocation for the locked versions.
struct Run {
    command: Vec<&'static str>,
    /// Native offline settings, set over the managed environment.
    env: &'static [(&'static str, &'static str)],
    offline: bool,
    /// Why the installer cannot be kept off the network, when offline.
    blocked: Option<String>,
}

impl Run {
    /// The command as reported, native environment settings first.
    fn display(&self) -> String {
        let env = self.env.iter().map(|(key, value)| format!("{key}={value}"));
        env.chain(self.command.iter().map(|word| word.to_string()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Adapter {
    fn lockfile(&self) -> &'static str {
        match self {
            Self::Javascript { lockfile, .. } => lockfile,
            Self::Uv => "uv.lock",
            Self::Cargo => "Cargo.lock",
            Self::Go => "go.sum",
        }
    }

    /// Reads native offline settings; runs nothing. Offline,
    /// each manager uses its native mode that fails on anything missing from
    /// its cache instead of contacting a registry.
    fn run(&self, root: &Path, lock: &Lock, offline: bool) -> Result<Run, Error> {
        let (mut env, mut blocked): (&'static [_], _) = (&[], None);
        let command = match self {
            Self::Javascript { frozen, .. } => {
                let id = frozen.program();
                let version = lock
                    .tools()
                    .get(id)
                    .and_then(|tool| Version::parse(&tool.version).ok())
                    .ok_or(Error::Unlocked(id))?;
                let mut command: Vec<_> = std::iter::once(id)
                    .chain(frozen.args(&version).iter().copied())
                    .collect();
                match offline.then(|| frozen.offline(&version, root)) {
                    None => {}
                    Some(Offline::Args(args)) => command.extend(args),
                    Some(Offline::Env(vars)) => env = vars,
                    Some(Offline::Unsupported(reason)) => blocked = Some(reason),
                }
                command
            }
            Self::Uv => {
                let none = BTreeMap::new();
                let sync = UvSync {
                    uv: Path::new("uv"),
                    root,
                    offline,
                    env: &none,
                }
                .args();
                std::iter::once("uv").chain(sync).collect()
            }
            Self::Cargo => {
                let mut command = vec!["cargo", "fetch", "--locked"];
                command.extend(offline.then_some("--offline"));
                command
            }
            Self::Go => {
                if offline {
                    env = go_prefetch::OFFLINE_ENV;
                }
                vec!["go", "mod", "download"]
            }
        };
        Ok(Run {
            command,
            env,
            offline,
            blocked,
        })
    }

    /// `None` when the installer succeeded, else its exit status.
    fn execute(
        &self,
        run: &Run,
        root: &Path,
        env: &Environment,
        step: &mut InstallStep,
    ) -> Result<Option<i32>, Error> {
        let mut vars = env.vars.clone();
        vars.extend(
            run.env
                .iter()
                .map(|(key, value)| (key.to_string(), value.into())),
        );
        let (vars, offline) = (&vars, run.offline);
        match self {
            Self::Javascript { frozen, .. } => {
                let program = env.program(frozen.program())?;
                let stdio = step.stdio()?;
                let status = frozen.run(&program, &run.command[1..], root, vars, |command| {
                    super::installer::status(stdio.apply(command))
                })?;
                Ok((!status.success()).then(|| crate::commands::exec::exit_code(status)))
            }
            Self::Uv => {
                let uv = env.program("uv")?;
                match (UvSync {
                    uv: &uv,
                    root,
                    offline,
                    env: vars,
                })
                .run()
                {
                    Ok(()) => Ok(None),
                    Err(uv_sync::Error::Failed(code)) => Ok(Some(code)),
                    Err(error) => Err(error.into()),
                }
            }
            Self::Cargo => {
                let cargo = env.program("cargo")?;
                // Activation exposes this same home to tasks as CARGO_HOME.
                let home = cargo_prefetch::cargo_home(root)?;
                match cargo_prefetch::prefetch(root, &cargo, &home, offline, vars) {
                    Ok(()) => Ok(None),
                    Err(cargo_prefetch::Error::Fetch(code)) => Ok(Some(code.parse().unwrap_or(1))),
                    Err(error) => Err(error.into()),
                }
            }
            Self::Go => {
                let go = env.program("go")?;
                let prefetch = Prefetch {
                    go: &go,
                    root,
                    offline,
                    env: vars,
                };
                match prefetch.run() {
                    Ok(()) => Ok(None),
                    // go's output is captured; show its diagnostic tail.
                    Err(
                        error @ (go_prefetch::Error::Failed { code, .. }
                        | go_prefetch::Error::Offline { code, .. }),
                    ) => {
                        eprintln!("{error}");
                        Ok(Some(code))
                    }
                    Err(error) => Err(error.into()),
                }
            }
        }
    }
}

/// The effective dependency setup for every eligible toolchain, JavaScript
/// first.
pub(super) struct Dependencies {
    steps: Vec<(ToolchainId, Step)>,
    /// `--offline` setup and plans use each manager's native offline mode.
    /// Checks run no installer, so they describe the usual commands.
    offline: bool,
}

impl Dependencies {
    /// Decided before any download, so a missing lockfile, an undeclared
    /// package manager, or an installer that cannot be kept offline fails
    /// before setup changes anything.
    pub fn select(
        discovery: &Discovery,
        plan: &effective::Plan,
        snapshot: &Snapshot,
        request: &SetupRequest,
    ) -> Result<Self, super::Error> {
        let root = discovery.snapshot_root()?.as_std_path();
        let mut steps = Vec::new();
        for toolchain in plan.toolchains() {
            let (dependencies, source) = toolchain.dependencies;
            let id = &toolchain.id;
            let step = match toolchain.tools.0 {
                _ if dependencies == DependencyPolicy::Skip => Step::Skip(source.label().into()),
                ToolPolicy::Managed if *id == ToolchainId::JAVASCRIPT => {
                    javascript(root, snapshot)?
                }
                ToolPolicy::Managed if *id == ToolchainId::PYTHON => python(root, snapshot)?,
                ToolPolicy::Managed if *id == ToolchainId::RUST => rust(root, snapshot)?,
                ToolPolicy::Managed if *id == ToolchainId::GO => go(snapshot),
                ToolPolicy::Managed => Step::Skip(format!(
                    "{} dependency setup is not supported yet",
                    id.as_str()
                )),
                ToolPolicy::Skip => Step::Skip("tools are skipped".into()),
            };
            steps.push((id.clone(), step));
        }
        let dependencies = Self {
            steps,
            offline: request.offline && request.mode != super::Mode::Check,
        };
        // Offline setup needs the committed lock anyway; without one, or with
        // a stale one, the offline tool path reports that instead.
        if dependencies.offline
            && request.mode == super::Mode::Provision
            && let Some(lock) = snapshot.previous_lock()
        {
            let blocked = dependencies.blocked(root, lock);
            if !blocked.is_empty() {
                return Err(Error::OfflineUnsupported(blocked.join("\n")).into());
            }
        }
        Ok(dependencies)
    }

    /// Every installer that cannot be kept off the network, with its reason.
    fn blocked(&self, root: &Path, lock: &Lock) -> Vec<String> {
        let runs = self.steps.iter().filter_map(|(id, step)| match step {
            Step::Install(adapter) => Some((id, adapter.run(root, lock, true).ok()?)),
            Step::Skip(_) => None,
        });
        runs.filter_map(|(id, run)| Some(format!("  {}: {}", id.as_str(), run.blocked?)))
            .collect()
    }

    /// Whether no eligible toolchain installs dependencies.
    pub fn skipped(&self) -> bool {
        self.steps
            .iter()
            .all(|(_, step)| matches!(step, Step::Skip(_)))
    }

    fn reasons(&self) -> String {
        skip_reasons(self.steps.iter().filter_map(|(_, step)| match step {
            Step::Skip(reason) => Some(reason.as_str()),
            Step::Install(_) => None,
        }))
    }

    /// `--tools-only` for a JavaScript repository, for direct plan tests.
    #[cfg(test)]
    pub fn tools_only() -> Self {
        Self {
            steps: vec![(ToolchainId::JAVASCRIPT, Step::Skip("--tools-only".into()))],
            offline: false,
        }
    }

    /// `--plan` lines for the locked or proposed tool versions.
    pub fn plan(&self, root: &Path, lock: &Lock) -> Result<Vec<String>, Error> {
        if self.skipped() {
            return Ok(vec![format!(
                "Dependencies skipped ({}); dependency readiness was not checked.",
                self.reasons()
            )]);
        }
        self.describe(root, lock, |run, lockfile| {
            format!(
                "{} from {lockfile} with managed tools ({lockfile} stays unchanged)",
                run.display()
            )
        })
    }

    /// `--check` lines. The initial adapters keep no readiness evidence, so
    /// dependencies are never reported ready.
    pub fn check(&self, root: &Path, lock: &Lock) -> Result<Vec<String>, Error> {
        if self.skipped() {
            return Ok(vec![format!(
                "Dependencies skipped ({}); dependency readiness was not checked.",
                self.reasons()
            )]);
        }
        self.describe(root, lock, |run, lockfile| {
            format!(
                "not checked; turbo setup reruns {} from {lockfile}, because dependency readiness \
                 is not tracked",
                run.display()
            )
        })
    }

    fn describe(
        &self,
        root: &Path,
        lock: &Lock,
        action: impl Fn(&Run, &str) -> String,
    ) -> Result<Vec<String>, Error> {
        let mut lines = Vec::new();
        for (id, step) in &self.steps {
            let id = id.as_str();
            match step {
                Step::Skip(reason) => lines.push(format!("{id} dependencies: skipped ({reason})")),
                Step::Install(adapter) => {
                    let run = adapter.run(root, lock, self.offline)?;
                    if let Some(reason) = &run.blocked {
                        lines.push(format!(
                            "{id} dependencies: cannot install offline: {reason}"
                        ));
                        continue;
                    }
                    let action = action(&run, adapter.lockfile());
                    lines.push(format!("{id} dependencies: {action}"));
                }
            }
        }
        Ok(lines)
    }

    /// After provisioning: every enabled installer in order, each with the
    /// managed environment that setup just recorded. Online, the first
    /// failing installer ends the run. Offline, every installer still runs
    /// from its cache, so the report lists each one that could not.
    /// `trust_base` overrides the user state directory for fixtures.
    pub fn install(
        &self,
        root: &Path,
        ecosystems: Ecosystems,
        trust_base: Option<PathBuf>,
        console: &Console,
    ) -> Result<DependencyReport, Error> {
        let skipped = |id: &ToolchainId, reason: &str| StepOutcome {
            toolchain: id.clone(),
            result: StepResult::Skipped {
                reason: reason.into(),
            },
        };
        let mut report = DependencyReport {
            steps: Vec::new(),
            offline: self.offline,
        };
        if self.skipped() {
            report.steps = self
                .steps
                .iter()
                .filter_map(|(id, step)| match step {
                    Step::Skip(reason) => Some(skipped(id, reason)),
                    Step::Install(_) => None,
                })
                .collect();
            return Ok(report);
        }
        let trust_base = trust_base
            .or_else(dirs_next::data_local_dir)
            .ok_or_else(|| std::io::Error::other("cannot locate setup user state directory"))?;
        // The lock and installation that provisioning just published.
        let sources = Snapshot::capture_with(root, ecosystems)?;
        let lock = sources
            .previous_lock()
            .ok_or(turborepo_setup::activation::Error::MissingLock)?;
        // Rechecked against the lock that provisioning used.
        let blocked = self.offline.then(|| self.blocked(root, lock));
        if let Some(blocked) = blocked.filter(|blocked| !blocked.is_empty()) {
            return Err(Error::OfflineUnsupported(blocked.join("\n")));
        }
        for (id, step) in &self.steps {
            let adapter = match step {
                Step::Skip(reason) => {
                    report.steps.push(skipped(id, reason));
                    continue;
                }
                Step::Install(adapter) => adapter,
            };
            let run = adapter.run(root, lock, self.offline)?;
            let env = Environment::recorded(root, &sources, &trust_base)?;
            let mut step = console.step(id.clone(), run.command[0], run.display());
            let exit = adapter.execute(&run, root, &env, &mut step)?;
            report.steps.push(step.finish(exit)?);
            if exit.is_some() && !self.offline {
                break;
            }
        }
        Ok(report)
    }
}

/// Distinct skip reasons, as the dependency lines report them.
pub(super) fn skip_reasons<'a>(reasons: impl Iterator<Item = &'a str>) -> String {
    let reasons: BTreeSet<_> = reasons.collect();
    if reasons.is_empty() {
        return "no toolchain has dependencies".into();
    }
    reasons.into_iter().collect::<Vec<_>>().join("; ")
}

fn javascript(root: &Path, snapshot: &Snapshot) -> Result<Step, Error> {
    let Some(declaration) = snapshot.package_manager()? else {
        if !root.join("package.json").exists() {
            return Ok(Step::Skip("no root package.json".into()));
        }
        return Err(Error::UndeclaredManager);
    };
    let Some(frozen) = Frozen::new(declaration.manager) else {
        return Ok(Step::Skip(format!(
            "{} dependency installation is not supported yet",
            declaration.manager.name()
        )));
    };
    Ok(Step::Install(Adapter::Javascript {
        lockfile: frozen.lockfile(root)?,
        frozen,
    }))
}

fn python(root: &Path, snapshot: &Snapshot) -> Result<Step, Error> {
    // uv syncs with the managed uv only, so it needs a uv declaration.
    if !snapshot.declarations().contains_key("uv") {
        let reason = "no managed uv; declare tool.uv.required-version or uv.toml";
        return Ok(Step::Skip(reason.into()));
    }
    if !root.join("uv.lock").is_file() {
        return Err(uv_sync::Error::MissingLock.into());
    }
    Ok(Step::Install(Adapter::Uv))
}

fn rust(root: &Path, snapshot: &Snapshot) -> Result<Step, Error> {
    // Cargo fetches with the managed toolchain only, so it needs a toolchain file.
    if !snapshot.declarations().contains_key("rust") {
        let reason = "no managed Rust toolchain; declare one in rust-toolchain.toml";
        return Ok(Step::Skip(reason.into()));
    }
    if !root.join("Cargo.lock").is_file() {
        return Err(cargo_prefetch::Error::MissingLock.into());
    }
    Ok(Step::Install(Adapter::Cargo))
}

/// Modules without dependencies have no go.sum, so go itself reports missing
/// checksums rather than setup requiring the file.
fn go(snapshot: &Snapshot) -> Step {
    if !snapshot.declarations().contains_key("go") {
        return Step::Skip("no managed Go toolchain; declare go in go.mod or go.work".into());
    }
    Step::Install(Adapter::Go)
}

/// Managed tools first on PATH plus their runtime variables, over the
/// caller's environment so native credentials and settings still apply.
struct Environment {
    bin: PathBuf,
    vars: BTreeMap<String, OsString>,
}

impl Environment {
    /// Authorized as strictly as `turbo run`: the activation plan must match
    /// the trusted setup record that provisioning just published.
    fn recorded(root: &Path, sources: &Snapshot, trust_base: &Path) -> Result<Self, Error> {
        let context = crate::commands::managed::context().map_err(Error::Host)?;
        let plan = ActivationPlan::captured(
            sources,
            context,
            turborepo_tool_install::Verification::Shallow,
        )?;
        let trust = super::trust::TrustStore::new(trust_base, root)?;
        if !trust.authorizes(&plan)? {
            return Err(Error::Untrusted);
        }
        let mut vars = plan.runtime_env().clone();
        let path = plan.path(std::env::var_os("PATH").as_deref())?;
        vars.insert("PATH".into(), path);
        let bin = plan.path_prepend().to_owned();
        Ok(Self { bin, vars })
    }

    /// The managed executable only, never a lookup on the caller's PATH.
    /// Windows exports are exactly `name.exe` hard links or `name.cmd`
    /// forwarders, found without the caller's PATHEXT.
    fn program(&self, name: &'static str) -> Result<PathBuf, Error> {
        #[cfg(windows)]
        return ["exe", "cmd"]
            .iter()
            .map(|extension| self.bin.join(format!("{name}.{extension}")))
            .find(|path| path.is_file())
            .ok_or(Error::NotInstalled(name));
        #[cfg(not(windows))]
        which::which_in(name, Some(&self.bin), &self.bin).map_err(|_| Error::NotInstalled(name))
    }
}
