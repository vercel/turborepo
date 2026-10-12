//! Locked dependency setup after tool provisioning. Each enabled ecosystem
//! runs its native frozen install with the managed tools that setup just
//! installed and recorded, never a Turborepo task. The effective
//! lifecycle-script policy is shown before each installer starts. Committed
//! ecosystem lockfiles are required and never rewritten. Repeats rerun the
//! native command: an unchanged lockfile does not prove that installed
//! dependencies are ready.

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
    js_install::{self, Frozen},
    lifecycle::{self, Installer, Policy},
    lock::{Ecosystems, Lock, Snapshot},
    uv_sync::{self, Builds, UvSync},
};
use turborepo_turbo_json::{DependencyPolicy, ToolPolicy};

use super::{SetupRequest, effective, root::Discovery};

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
    #[error(transparent)]
    Install(#[from] js_install::Error),
    #[error(transparent)]
    Lifecycle(#[from] lifecycle::Error),
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
        lifecycle: lifecycle::Request,
    },
    /// `uv sync --locked`; skipping lifecycle scripts refuses source builds.
    Uv { lifecycle: lifecycle::Request },
    /// `cargo fetch --locked` into the clone's private Cargo home.
    Cargo { lifecycle: lifecycle::Request },
    /// `go mod download` with read-only module files.
    Go { lifecycle: lifecycle::Request },
}

/// One installer invocation for the locked versions.
struct Run {
    /// The managed program, its native frozen arguments, then exactly the
    /// lifecycle policy's arguments.
    command: Vec<&'static str>,
    installer: Installer,
    policy: Policy,
}

fn uv_sync<'a>(
    uv: &'a Path,
    root: &'a Path,
    builds: Builds,
    env: &'a BTreeMap<String, OsString>,
) -> UvSync<'a> {
    UvSync {
        uv,
        root,
        offline: false,
        builds,
        env,
    }
}

impl Adapter {
    fn lockfile(&self) -> &'static str {
        match self {
            Self::Javascript { lockfile, .. } => lockfile,
            Self::Uv { .. } => "uv.lock",
            Self::Cargo { .. } => "Cargo.lock",
            Self::Go { .. } => "go.sum",
        }
    }

    /// Reads committed lifecycle settings; runs nothing.
    fn run(&self, root: &Path, lock: &Lock) -> Result<Run, Error> {
        let (installer, lifecycle, mut command): (_, _, Vec<_>) = match self {
            Self::Javascript {
                frozen, lifecycle, ..
            } => {
                let id = frozen.program();
                let version = lock
                    .tools()
                    .get(id)
                    .and_then(|tool| Version::parse(&tool.version).ok())
                    .ok_or(Error::Unlocked(id))?;
                let installer = Installer::javascript(frozen.manager(), version.clone())
                    .ok_or(Error::Unlocked(id))?;
                let args = frozen.args(&version).iter().copied();
                (
                    installer,
                    *lifecycle,
                    std::iter::once(id).chain(args).collect(),
                )
            }
            Self::Uv { lifecycle } => {
                let sync = uv_sync(Path::new("uv"), root, Builds::Native, &BTreeMap::new()).args();
                (
                    Installer::Uv,
                    *lifecycle,
                    std::iter::once("uv").chain(sync).collect(),
                )
            }
            Self::Cargo { lifecycle } => (
                Installer::Cargo,
                *lifecycle,
                vec!["cargo", "fetch", "--locked"],
            ),
            Self::Go { lifecycle } => (Installer::Go, *lifecycle, vec!["go", "mod", "download"]),
        };
        let policy = Policy::evaluate(root, installer.clone(), lifecycle)?;
        command.extend(policy.args().iter().copied());
        Ok(Run {
            command,
            installer,
            policy,
        })
    }

    /// `None` when the installer succeeded, else its exit status.
    fn execute(&self, run: &Run, root: &Path, env: &Environment) -> Result<Option<i32>, Error> {
        match self {
            Self::Javascript { frozen, .. } => {
                let program = env.program(frozen.program())?;
                let status = frozen.run(&program, &run.command[1..], root, &env.vars)?;
                Ok((!status.success()).then(|| crate::commands::exec::exit_code(status)))
            }
            Self::Uv { .. } => {
                let uv = env.program("uv")?;
                let builds = match run.policy.args() {
                    [] => Builds::Native,
                    _ => Builds::Skip,
                };
                match uv_sync(&uv, root, builds, &env.vars).run() {
                    Ok(()) => Ok(None),
                    Err(uv_sync::Error::Failed(code)) => Ok(Some(code)),
                    Err(error) => Err(error.into()),
                }
            }
            Self::Cargo { .. } => {
                let cargo = env.program("cargo")?;
                // Activation exposes this same home to tasks as CARGO_HOME.
                let home = cargo_prefetch::cargo_home(root)?;
                match cargo_prefetch::prefetch(root, &cargo, &home, false, &env.vars) {
                    Ok(()) => Ok(None),
                    Err(cargo_prefetch::Error::Fetch(code)) => Ok(Some(code.parse().unwrap_or(1))),
                    Err(error) => Err(error.into()),
                }
            }
            Self::Go { .. } => {
                let go = env.program("go")?;
                let prefetch = Prefetch {
                    go: &go,
                    root,
                    offline: false,
                    env: &env.vars,
                };
                match prefetch.run() {
                    Ok(()) => Ok(None),
                    // go's output is captured; show its diagnostic tail.
                    Err(error @ go_prefetch::Error::Failed { code, .. }) => {
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
pub(super) struct Dependencies(Vec<(ToolchainId, Step)>);

impl Dependencies {
    /// Decided before any download, so a missing lockfile or undeclared
    /// package manager fails before setup changes anything.
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
            let lifecycle = request.lifecycle(discovery.policy().toolchain(id).lifecycle_scripts);
            let step = match toolchain.tools.0 {
                _ if dependencies == DependencyPolicy::Skip => Step::Skip(source.label().into()),
                ToolPolicy::Managed if *id == ToolchainId::JAVASCRIPT => {
                    javascript(root, snapshot, lifecycle)?
                }
                ToolPolicy::Managed if *id == ToolchainId::PYTHON => {
                    python(root, snapshot, lifecycle)?
                }
                ToolPolicy::Managed if *id == ToolchainId::RUST => rust(root, snapshot, lifecycle)?,
                ToolPolicy::Managed if *id == ToolchainId::GO => go(snapshot, lifecycle),
                ToolPolicy::Managed => Step::Skip(format!(
                    "{} dependency setup is not supported yet",
                    id.as_str()
                )),
                _ => Step::Skip("not supported yet with verify-system tools".into()),
            };
            steps.push((id.clone(), step));
        }
        Ok(Self(steps))
    }

    /// Whether no eligible toolchain installs dependencies.
    pub fn skipped(&self) -> bool {
        self.0.iter().all(|(_, step)| matches!(step, Step::Skip(_)))
    }

    fn reasons(&self) -> String {
        let reasons: BTreeSet<_> = self
            .0
            .iter()
            .filter_map(|(_, step)| match step {
                Step::Skip(reason) => Some(reason.as_str()),
                Step::Install(_) => None,
            })
            .collect();
        if reasons.is_empty() {
            return "no toolchain has dependencies".into();
        }
        reasons.into_iter().collect::<Vec<_>>().join("; ")
    }

    /// `--tools-only` for a JavaScript repository, for direct plan tests.
    #[cfg(test)]
    pub fn tools_only() -> Self {
        Self(vec![(
            ToolchainId::JAVASCRIPT,
            Step::Skip("--tools-only".into()),
        )])
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
                "{} from {lockfile} with managed {} ({lockfile} stays unchanged)",
                run.command.join(" "),
                run.installer
            )
        })
    }

    /// `--check` lines. The initial adapters keep no readiness evidence, so
    /// dependencies are never reported ready.
    pub fn check(&self, root: &Path, lock: &Lock) -> Result<Vec<String>, Error> {
        if self.skipped() {
            return Ok(vec![format!(
                "Dependencies skipped ({}); dependency readiness was not checked. No tasks run.",
                self.reasons()
            )]);
        }
        let mut lines = self.describe(root, lock, |run, lockfile| {
            format!(
                "not checked; turbo setup reruns {} from {lockfile}, because dependency readiness \
                 is not tracked",
                run.command.join(" ")
            )
        })?;
        lines.push("No tasks run.".into());
        Ok(lines)
    }

    fn describe(
        &self,
        root: &Path,
        lock: &Lock,
        action: impl Fn(&Run, &str) -> String,
    ) -> Result<Vec<String>, Error> {
        let mut lines = Vec::new();
        for (id, step) in &self.0 {
            let id = id.as_str();
            match step {
                Step::Skip(reason) => lines.push(format!("{id} dependencies: skipped ({reason})")),
                Step::Install(adapter) => {
                    let run = adapter.run(root, lock)?;
                    let action = action(&run, adapter.lockfile());
                    lines.push(format!("{id} dependencies: {action}"));
                    let policy = run.policy.report().into_iter();
                    lines.extend(policy.map(|line| format!("  {line}")));
                }
            }
        }
        Ok(lines)
    }

    /// After provisioning: every enabled installer in order, each with the
    /// managed environment that setup just recorded. The first failing
    /// installer's exit status becomes setup's, after it is reported.
    /// `trust_base` overrides the user state directory for fixtures.
    pub fn install(
        &self,
        root: &Path,
        ecosystems: Ecosystems,
        trust_base: Option<PathBuf>,
    ) -> Result<i32, Error> {
        if self.skipped() {
            println!("Dependencies skipped ({}). No tasks run.", self.reasons());
            return Ok(0);
        }
        let trust_base = trust_base
            .or_else(dirs_next::data_local_dir)
            .ok_or_else(|| std::io::Error::other("cannot locate setup user state directory"))?;
        // The lock and installation that provisioning just published.
        let sources = Snapshot::capture_with(root, ecosystems)?;
        let lock = sources
            .previous_lock()
            .ok_or(turborepo_setup::activation::Error::MissingLock)?;
        let mut summary = Vec::new();
        for (id, step) in &self.0 {
            let id = id.as_str();
            let adapter = match step {
                Step::Skip(reason) => {
                    summary.push(format!("{id} skipped ({reason})"));
                    continue;
                }
                Step::Install(adapter) => adapter,
            };
            let (run, lockfile) = (adapter.run(root, lock)?, adapter.lockfile());
            let command = run.command.join(" ");
            // Reported before the installer can run any lifecycle script.
            println!(
                "Installing {id} dependencies from {lockfile} with managed {}: {command} \
                 (dependency installation, not a Turborepo task)",
                run.installer
            );
            for line in run.policy.report() {
                println!("{line}");
            }
            let env = Environment::recorded(root, &sources, &trust_base)?;
            if let Some(code) = adapter.execute(&run, root, &env)? {
                eprintln!(
                    "{id} dependency setup failed: `{command}` exited with status {code}. \
                     {lockfile} is unchanged; fix the reported problem and rerun turbo setup. No \
                     Turborepo tasks run."
                );
                return Ok(code);
            }
            println!("{id} dependencies: installed; {lockfile} unchanged");
            summary.push(format!("{id} installed"));
        }
        println!(
            "Dependencies: {}. No Turborepo tasks run.",
            summary.join("; ")
        );
        Ok(0)
    }
}

fn javascript(
    root: &Path,
    snapshot: &Snapshot,
    lifecycle: lifecycle::Request,
) -> Result<Step, Error> {
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
        lifecycle,
    }))
}

fn python(root: &Path, snapshot: &Snapshot, lifecycle: lifecycle::Request) -> Result<Step, Error> {
    // uv syncs with the managed uv only, so it needs a uv declaration.
    if !snapshot.declarations().contains_key("uv") {
        let reason = "no managed uv; declare tool.uv.required-version or uv.toml";
        return Ok(Step::Skip(reason.into()));
    }
    if !root.join("uv.lock").is_file() {
        return Err(uv_sync::Error::MissingLock.into());
    }
    Ok(Step::Install(Adapter::Uv { lifecycle }))
}

fn rust(root: &Path, snapshot: &Snapshot, lifecycle: lifecycle::Request) -> Result<Step, Error> {
    // Cargo fetches with the managed toolchain only, so it needs a toolchain file.
    if !snapshot.declarations().contains_key("rust") {
        let reason = "no managed Rust toolchain; declare one in rust-toolchain.toml";
        return Ok(Step::Skip(reason.into()));
    }
    if !root.join("Cargo.lock").is_file() {
        return Err(cargo_prefetch::Error::MissingLock.into());
    }
    Ok(Step::Install(Adapter::Cargo { lifecycle }))
}

/// Modules without dependencies have no go.sum, so go itself reports missing
/// checksums rather than setup requiring the file.
fn go(snapshot: &Snapshot, lifecycle: lifecycle::Request) -> Step {
    if !snapshot.declarations().contains_key("go") {
        return Step::Skip("no managed Go toolchain; declare go in go.mod or go.work".into());
    }
    Step::Install(Adapter::Go { lifecycle })
}

/// Managed tools first on PATH plus their runtime variables, over the
/// caller's environment so native credentials and settings still apply.
struct Environment {
    /// None when only verify-system tools are locked: nothing is managed.
    bin: Option<PathBuf>,
    vars: BTreeMap<String, OsString>,
}

impl Environment {
    /// Authorized as strictly as `turbo run`: the activation plan must match
    /// the trusted setup record that provisioning just published.
    fn recorded(root: &Path, sources: &Snapshot, trust_base: &Path) -> Result<Self, Error> {
        let context = crate::commands::managed::context().map_err(Error::Host)?;
        let plan = ActivationPlan::inspect_captured(sources, context)?;
        let trust = super::trust::TrustStore::new(trust_base, root)?;
        if !trust.authorizes(&plan)? {
            return Err(Error::Untrusted);
        }
        let mut vars = plan.runtime_env().clone();
        let path = plan.path(std::env::var_os("PATH").as_deref())?;
        vars.insert("PATH".into(), path);
        Ok(Self {
            bin: plan.path_prepend().map(Path::to_owned),
            vars,
        })
    }

    /// The managed executable only, never a lookup on the caller's PATH.
    fn program(&self, name: &'static str) -> Result<PathBuf, Error> {
        let bin = self.bin.as_deref().ok_or(Error::NotInstalled(name))?;
        which::which_in(name, Some(bin), bin).map_err(|_| Error::NotInstalled(name))
    }
}
