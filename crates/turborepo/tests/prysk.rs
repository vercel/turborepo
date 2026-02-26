use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use fs4::fs_std::FileExt;

static PRYSK_VENV: OnceLock<PathBuf> = OnceLock::new();

/// Location of the prysk venv, shared across all test invocations within a
/// single nextest process. Cross-process synchronization (for parallel nextest
/// workers) is handled by a file lock in `ensure_prysk_venv`.
fn prysk_venv_dir() -> &'static Path {
    PRYSK_VENV
        .get_or_init(|| {
            let dir = integration_tests_dir().join(".cram_env");
            ensure_prysk_venv(&dir).expect("failed to set up prysk venv");
            dir
        })
        .as_path()
}

fn workspace_root() -> PathBuf {
    // Use dunce to avoid \\?\ extended-length path prefix on Windows,
    // which breaks Python's venv module.
    dunce::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("failed to resolve workspace root")
}

fn integration_tests_dir() -> PathBuf {
    workspace_root().join("turborepo-tests").join("integration")
}

/// Create (or reuse) a Python venv with prysk installed.
///
/// Uses an advisory file lock so that multiple nextest worker processes don't
/// race to create the venv simultaneously.
fn ensure_prysk_venv(venv_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let lock_path = venv_dir.with_extension("lock");

    // Ensure parent directory exists for the lock file.
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;

    lock_file.lock_exclusive()?;

    // Check if venv already has prysk installed (the bin must exist).
    let prysk_bin = venv_bin(venv_dir, "prysk");
    if prysk_bin.exists() {
        lock_file.unlock()?;
        return Ok(());
    }

    let python = find_python()?;

    // Create venv
    let status = Command::new(&python)
        .args(["-m", "venv"])
        .arg(venv_dir)
        .status()?;
    if !status.success() {
        return Err(format!("failed to create python venv at {}", venv_dir.display()).into());
    }

    // Install prysk
    let pip = venv_bin(venv_dir, "pip");
    let status = Command::new(&pip)
        .args(["install", "--quiet", "prysk==0.15.2"])
        .status()?;
    if !status.success() {
        return Err("failed to install prysk into venv".into());
    }

    lock_file.unlock()?;
    Ok(())
}

fn find_python() -> Result<PathBuf, Box<dyn std::error::Error>> {
    for candidate in ["python3", "python"] {
        if let Ok(path) = which::which(candidate) {
            return Ok(path);
        }
    }
    Err("could not find python3 or python on PATH".into())
}

fn venv_bin(venv_dir: &Path, tool: &str) -> PathBuf {
    let bin_dir = if cfg!(windows) { "Scripts" } else { "bin" };
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    venv_dir.join(bin_dir).join(format!("{tool}{suffix}"))
}

/// Tests that are known to fail on Windows due to fixture/symlink issues
/// that predate this harness. Skip them here; they'll be fixed when migrated
/// to pure Rust integration tests.
const WINDOWS_SKIP_PREFIXES: &[&str] = &["find-turbo/"];

fn run_prysk_test(path: &Path) -> datatest_stable::Result<()> {
    if cfg!(windows) {
        let rel = path.to_string_lossy().replace('\\', "/");
        if WINDOWS_SKIP_PREFIXES
            .iter()
            .any(|prefix| rel.contains(prefix))
        {
            return Ok(());
        }
    }

    let prysk_bin = venv_bin(prysk_venv_dir(), "prysk");

    let mut cmd = Command::new(&prysk_bin);
    cmd.arg("--shell=bash");

    if cfg!(windows) {
        cmd.arg("--dos2unix");
    }

    cmd.arg(path);

    // Strip CI env vars so turbo inside .t files uses its default log format
    // instead of GitHub Actions `::group::` markers. The old CI setup ran prysk
    // through `turbo run --env-mode=strict` which stripped these automatically.
    cmd.env_remove("CI");
    cmd.env_remove("GITHUB_ACTIONS");

    // macOS tmp dirs set by prysk can fail — use /tmp directly.
    if cfg!(target_os = "macos") {
        cmd.env("TMPDIR", "/tmp");
    }

    let output = cmd.output()?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "prysk failed (exit {:?}) for {}\n\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
            output.status.code(),
            path.display(),
        )
        .into());
    }

    Ok(())
}

datatest_stable::harness! {
    {
        test = run_prysk_test,
        root = "../../turborepo-tests/integration/tests",
        pattern = r"\.t$",
    },
}
