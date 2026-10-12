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
    process::ExitStatus,
};

use semver::Version;
use turborepo_repository::toolchain::ToolchainId;
use turborepo_setup::{
    activation::ActivationPlan,
    js_install::{self, Frozen},
    lifecycle::{self, Installer, Policy},
    lock::{Ecosystems, Lock, Snapshot},
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
}

/// One installer invocation for the locked versions.
struct Run {
    /// The managed program, its native frozen arguments, then exactly the
    /// lifecycle policy's arguments.
    command: Vec<&'static str>,
    installer: Installer,
    policy: Policy,
}

impl Adapter {
    fn lockfile(&self) -> &'static str {
        match self {
            Self::Javascript { lockfile, .. } => lockfile,
        }
    }

    /// Reads committed lifecycle settings; runs nothing.
    fn run(&self, root: &Path, lock: &Lock) -> Result<Run, Error> {
        let Self::Javascript {
            frozen, lifecycle, ..
        } = self;
        let id = frozen.program();
        let version = lock
            .tools()
            .get(id)
            .and_then(|tool| Version::parse(&tool.version).ok())
            .ok_or(Error::Unlocked(id))?;
        let installer =
            Installer::javascript(frozen.manager(), version.clone()).ok_or(Error::Unlocked(id))?;
        let policy = Policy::evaluate(root, installer.clone(), *lifecycle)?;
        let command = std::iter::once(id)
            .chain(frozen.args(&version).iter().copied())
            .chain(policy.args().iter().copied())
            .collect();
        Ok(Run {
            command,
            installer,
            policy,
        })
    }

    fn execute(&self, run: &Run, root: &Path, env: &Environment) -> Result<ExitStatus, Error> {
        let Self::Javascript { frozen, .. } = self;
        let program = env.program(frozen.program())?;
        Ok(frozen.run(&program, &run.command[1..], root, &env.vars)?)
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
            let step = match toolchain.tools.0 {
                _ if dependencies == DependencyPolicy::Skip => Step::Skip(source.label().into()),
                ToolPolicy::Managed if *id == ToolchainId::JAVASCRIPT => {
                    let scripts = discovery.policy().toolchain(id).lifecycle_scripts;
                    javascript(root, snapshot, request.lifecycle(scripts))?
                }
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
            let status = adapter.execute(&run, root, &env)?;
            if !status.success() {
                let code = crate::commands::exec::exit_code(status);
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
        let plan = ActivationPlan::inspect_captured(sources, context)?;
        let trust = super::trust::TrustStore::new(trust_base, root)?;
        if !trust.authorizes(&plan)? {
            return Err(Error::Untrusted);
        }
        let mut vars = plan.runtime_env().clone();
        let path = plan.path(std::env::var_os("PATH").as_deref())?;
        vars.insert("PATH".into(), path);
        Ok(Self {
            bin: plan.path_prepend().to_owned(),
            vars,
        })
    }

    /// The managed executable only, never a lookup on the caller's PATH.
    fn program(&self, name: &'static str) -> Result<PathBuf, Error> {
        which::which_in(name, Some(&self.bin), &self.bin).map_err(|_| Error::NotInstalled(name))
    }
}
