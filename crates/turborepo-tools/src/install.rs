//! The install driver: resolve each declaration, compare against the
//! manifest, and materialize whatever is missing.

use std::{collections::BTreeMap, fmt, path::Path, process::Stdio};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    Error, InstalledTool, Manifest, ToolsDir,
    archive::{self, ArchiveKind},
    declared::Declaration,
    http::{Checksum, Downloader},
    platform::Platform,
    shim,
    sources::Sources,
    tools,
};

/// Receives human-readable progress while installing.
pub trait Reporter: Send + Sync {
    /// A top-level step, e.g. "Downloading node 22.1.0".
    fn step(&self, message: &str);
    /// Supporting detail, e.g. the URL being fetched.
    fn detail(&self, message: &str);
}

/// A reporter that discards everything.
pub struct SilentReporter;

impl Reporter for SilentReporter {
    fn step(&self, _message: &str) {}
    fn detail(&self, _message: &str) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStatus {
    /// Freshly installed by this invocation.
    Installed,
    /// Already present at the resolved version.
    UpToDate,
    /// `--check` only: not installed.
    Missing,
    /// `--check` only: installed at a different version.
    Outdated { installed: String },
}

impl InstallStatus {
    pub fn is_satisfied(&self) -> bool {
        matches!(self, Self::Installed | Self::UpToDate)
    }
}

impl fmt::Display for InstallStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Installed => f.write_str("installed"),
            Self::UpToDate => f.write_str("up to date"),
            Self::Missing => f.write_str("missing"),
            Self::Outdated { installed } => write!(f, "outdated (installed: {installed})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub tool: String,
    pub requested: String,
    pub version: String,
    pub source: String,
    pub status: InstallStatus,
}

/// Everything a tool installer needs.
pub struct InstallContext<'a> {
    pub repo_root: &'a AbsoluteSystemPath,
    pub tools: ToolsDir,
    pub platform: Platform,
    pub http: Downloader,
    pub sources: Sources,
    pub reporter: &'a dyn Reporter,
}

impl InstallContext<'_> {
    /// Scratch space for in-flight downloads, inside the tools directory so
    /// renames into place stay on one filesystem.
    fn scratch_dir(&self) -> Result<AbsoluteSystemPathBuf, Error> {
        let dir = self.tools.root().join_component("tmp");
        dir.create_dir_all()
            .map_err(|source| Error::io(dir.as_str(), source))?;
        Ok(dir)
    }

    /// Downloads `url` (verifying `checksum`) and extracts it into `dest`,
    /// replacing whatever was there. The extraction happens in a sibling
    /// directory that is renamed into place only on success.
    pub(crate) async fn download_and_extract(
        &self,
        url: &str,
        checksum: Option<&Checksum>,
        kind: ArchiveKind,
        dest: &AbsoluteSystemPath,
        strip_components: usize,
    ) -> Result<(), Error> {
        let scratch = self.scratch_dir()?;
        let file_name = url.rsplit('/').next().unwrap_or("download");
        let archive_path = scratch.join_component(&format!("{}.{}", file_name, kind.extension()));
        self.reporter.detail(&format!("Downloading {url}"));
        self.http
            .download(url, archive_path.as_std_path(), checksum)
            .await?;

        let partial =
            scratch.join_component(&format!("{}.partial", dest.file_name().unwrap_or("tool")));
        let _ = partial.remove_dir_all();
        let archive_for_task = archive_path.clone();
        let partial_for_task = partial.clone();
        tokio::task::spawn_blocking(move || {
            archive::extract(
                kind,
                archive_for_task.as_std_path(),
                partial_for_task.as_std_path(),
                strip_components,
            )
        })
        .await
        .map_err(|err| Error::Archive {
            archive: archive_path.to_string(),
            reason: err.to_string(),
        })??;
        let _ = archive_path.remove_file();

        if let Some(parent) = dest.parent() {
            parent
                .create_dir_all()
                .map_err(|source| Error::io(parent.as_str(), source))?;
        }
        if dest.exists() {
            dest.remove_dir_all()
                .map_err(|source| Error::io(dest.as_str(), source))?;
        }
        partial
            .rename(dest)
            .map_err(|source| Error::io(dest.as_str(), source))?;
        Ok(())
    }

    /// Downloads a single file (not an archive) to `dest`.
    pub(crate) async fn download_file(
        &self,
        url: &str,
        checksum: Option<&Checksum>,
        dest: &AbsoluteSystemPath,
    ) -> Result<(), Error> {
        if let Some(parent) = dest.parent() {
            parent
                .create_dir_all()
                .map_err(|source| Error::io(parent.as_str(), source))?;
        }
        self.reporter.detail(&format!("Downloading {url}"));
        self.http.download(url, dest.as_std_path(), checksum).await
    }

    /// Fetches a `<hex>` or `<hex>  <name>` sha256 sidecar file.
    pub(crate) async fn sha256_sidecar(
        &self,
        url: &str,
        file_name: &str,
    ) -> Result<Checksum, Error> {
        let text = self.http.get_text(url).await?;
        let first = text.split_whitespace().next().unwrap_or("");
        if let Some(found) = Checksum::from_shasums(&text, file_name) {
            return found;
        }
        Checksum::sha256_hex(first).map_err(|_| Error::ChecksumMissing {
            url: url.to_string(),
            file: file_name.to_string(),
        })
    }

    /// Runs a tool as a subprocess, streaming its stderr to the user and
    /// returning stdout. Fails on a non-zero exit.
    pub(crate) async fn run(
        &self,
        program: &Path,
        args: &[&str],
        env: &[(&str, &str)],
        cwd: &AbsoluteSystemPath,
    ) -> Result<String, Error> {
        let display = format!(
            "{} {}",
            program
                .file_name()
                .unwrap_or(program.as_os_str())
                .to_string_lossy(),
            args.join(" ")
        );
        self.reporter.detail(&format!("Running `{display}`"));
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .current_dir(cwd.as_std_path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command
            .output()
            .await
            .map_err(|source| Error::CommandSpawn {
                program: display.clone(),
                source,
            })?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                program: display,
                status: output.status.to_string(),
                stderr: String::new(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Records the shims a tool owns: every `(name, target)` becomes a
    /// symlink / `.cmd` wrapper in `bin/`.
    pub(crate) fn link_bins(
        &self,
        bins: &[(String, AbsoluteSystemPathBuf)],
    ) -> Result<Vec<String>, Error> {
        let bin_dir = self.tools.bin_dir();
        let mut names = Vec::new();
        for (name, target) in bins {
            shim::link_binary(&bin_dir, name, target)?;
            names.push(name.clone());
        }
        Ok(names)
    }

    /// Like [`Self::link_bins`], but each shim exports `env` before running
    /// its target so the tool works outside of turbo as well.
    pub(crate) fn link_bins_with_env(
        &self,
        bins: &[(String, AbsoluteSystemPathBuf)],
        env: &[(&str, &str)],
    ) -> Result<Vec<String>, Error> {
        let bin_dir = self.tools.bin_dir();
        let mut names = Vec::new();
        for (name, target) in bins {
            shim::env_wrapper(&bin_dir, name, target, env)?;
            names.push(name.clone());
        }
        Ok(names)
    }

    pub(crate) fn installed_tool(
        &self,
        version: &str,
        source: &str,
        path: &AbsoluteSystemPath,
        bins: Vec<String>,
        env: BTreeMap<String, String>,
    ) -> Result<InstalledTool, Error> {
        Ok(InstalledTool {
            version: version.to_string(),
            source: source.to_string(),
            path: self.tools.relative_of(path)?,
            bins,
            env,
        })
    }
}

/// Drives installation for a repository.
pub struct Installer<'a> {
    ctx: InstallContext<'a>,
    manifest: Manifest,
}

impl<'a> Installer<'a> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        reporter: &'a dyn Reporter,
        sources: Sources,
        http: Downloader,
    ) -> Result<Self, Error> {
        let tools = ToolsDir::new(repo_root);
        let manifest = tools.read_manifest()?;
        Ok(Self {
            ctx: InstallContext {
                repo_root,
                tools,
                platform: Platform::current()?,
                http,
                sources,
                reporter,
            },
            manifest,
        })
    }

    pub fn tools_dir(&self) -> &ToolsDir {
        &self.ctx.tools
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Resolves every declaration and reports what would happen, without
    /// downloading anything beyond release indexes.
    pub async fn check(&self, declarations: &[Declaration]) -> Result<Vec<InstallOutcome>, Error> {
        let mut outcomes = Vec::with_capacity(declarations.len());
        for declaration in declarations {
            let version = tools::resolve(&self.ctx, declaration).await?;
            let status = match self.current(declaration, &version) {
                Current::UpToDate => InstallStatus::UpToDate,
                Current::Outdated(installed) => InstallStatus::Outdated { installed },
                Current::Missing => InstallStatus::Missing,
            };
            outcomes.push(outcome(declaration, version, status));
        }
        Ok(outcomes)
    }

    /// Installs every declaration that is missing or outdated (or everything
    /// when `force` is set), writing the manifest after each tool so an
    /// interrupted run leaves a consistent record.
    pub async fn install(
        &mut self,
        declarations: &[Declaration],
        force: bool,
    ) -> Result<Vec<InstallOutcome>, Error> {
        let mut outcomes = Vec::with_capacity(declarations.len());
        for declaration in declarations {
            let version = tools::resolve(&self.ctx, declaration).await?;
            if !force && matches!(self.current(declaration, &version), Current::UpToDate) {
                self.ctx.reporter.step(&format!(
                    "{} {} is up to date",
                    declaration.tool(),
                    version
                ));
                outcomes.push(outcome(declaration, version, InstallStatus::UpToDate));
                continue;
            }
            self.ctx
                .reporter
                .step(&format!("Installing {} {}", declaration.tool(), version));
            let installed =
                tools::install(&self.ctx, declaration, &version, &self.manifest).await?;
            self.manifest
                .tools
                .insert(declaration.tool().to_string(), installed);
            self.ctx.tools.write_manifest(&self.manifest)?;
            outcomes.push(outcome(declaration, version, InstallStatus::Installed));
        }
        let _ = self.ctx.tools.root().join_component("tmp").remove_dir_all();
        Ok(outcomes)
    }

    fn current(&self, declaration: &Declaration, version: &str) -> Current {
        let Some(installed) = self.manifest.get(declaration.tool()) else {
            return Current::Missing;
        };
        if installed.version != version {
            return Current::Outdated(installed.version.clone());
        }
        if tools::is_present(&self.ctx, declaration, installed) {
            Current::UpToDate
        } else {
            Current::Missing
        }
    }
}

enum Current {
    UpToDate,
    Outdated(String),
    Missing,
}

fn outcome(declaration: &Declaration, version: String, status: InstallStatus) -> InstallOutcome {
    InstallOutcome {
        tool: declaration.tool().to_string(),
        requested: declaration.requested(),
        version,
        source: declaration.source().to_string(),
        status,
    }
}
