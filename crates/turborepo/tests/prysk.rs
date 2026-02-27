use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use fs4::fs_std::FileExt;

static PRYSK_VENV: OnceLock<PathBuf> = OnceLock::new();
#[cfg(windows)]
static FIXTURE_SETUP: OnceLock<()> = OnceLock::new();

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

/// On Windows, the find-turbo test fixtures need a real `turbo.exe` in the
/// platform-specific bin directories. The fixture source only has `.keep`
/// placeholders (the fixtures were originally designed for Linux/macOS where
/// shell scripts suffice). We build a tiny `echo_args` binary (a `[[bin]]`
/// target that just prints its arguments) and copy it into the fixture dirs
/// so prysk's setup scripts include it when copying fixtures to the temp dir.
#[cfg(windows)]
fn setup_windows_find_turbo_fixtures() {
    FIXTURE_SETUP.get_or_init(|| {
        let echo_args_exe = workspace_root()
            .join("target")
            .join("debug")
            .join("echo_args.exe");

        // Build echo_args if it doesn't already exist. nextest only builds
        // test targets, not [[bin]] targets.
        if !echo_args_exe.exists() {
            let status = Command::new("cargo")
                .args(["build", "--bin", "echo_args", "-p", "turbo"])
                .current_dir(workspace_root())
                .status();
            if !status.map_or(false, |s| s.success()) {
                return;
            }
        }

        let fixtures_dir = workspace_root()
            .join("turborepo-tests")
            .join("integration")
            .join("fixtures")
            .join("find_turbo");

        // Walk the fixture tree and place turbo.exe next to every .keep in
        // turbo-windows-*/bin/ directories.
        place_echo_args_in_fixtures(&fixtures_dir, &echo_args_exe);
    });
}

#[cfg(windows)]
fn place_echo_args_in_fixtures(dir: &Path, echo_args_exe: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            place_echo_args_in_fixtures(&path, echo_args_exe);
        } else if path.file_name().map_or(false, |n| n == ".keep") {
            // Check if this .keep is inside a turbo-windows-*/bin/ directory
            if let (Some(bin_dir), Some(platform_dir)) =
                (path.parent(), path.parent().and_then(|p| p.parent()))
            {
                let platform_name = platform_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let dir_name = bin_dir.file_name().unwrap_or_default().to_string_lossy();
                if platform_name.starts_with("turbo-windows-") && dir_name == "bin" {
                    let turbo_exe = bin_dir.join("turbo.exe");
                    if !turbo_exe.exists() {
                        let _ = fs::copy(echo_args_exe, &turbo_exe);
                    }
                }
            }
        }
    }
}

fn run_prysk_test(path: &Path) -> datatest_stable::Result<()> {
    #[cfg(windows)]
    setup_windows_find_turbo_fixtures();

    let prysk_bin = venv_bin(prysk_venv_dir(), "prysk");

    let mut cmd = Command::new(&prysk_bin);
    cmd.arg("--shell=bash");

    if cfg!(windows) {
        cmd.arg("--dos2unix");
    }

    cmd.arg(path);

    // Strip CI env vars to match the old CI setup which ran prysk through
    // `turbo run --env-mode=strict` with
    // `passThroughEnv: ["CI", "!GITHUB_*", "!RUNNER_*"]`.
    // This stripped GITHUB_ACTIONS (preventing ::group:: log format),
    // plus all GITHUB_* and RUNNER_* vars that can affect turbo's behavior.
    for (key, _) in env::vars() {
        if key.starts_with("GITHUB_") || key.starts_with("RUNNER_") {
            cmd.env_remove(&key);
        }
    }
    cmd.env_remove("CI");

    // Suppress package manager update notifications. The old CI's Node.js
    // prysk wrapper set this, and CI=true also suppressed them — but we
    // strip CI above.
    cmd.env("NO_UPDATE_NOTIFIER", "1");
    cmd.env("NPM_CONFIG_UPDATE_NOTIFIER", "false");

    // Prevent corepack from prompting to download package managers. Without
    // this, corepack blocks on stdin in non-TTY environments (causing hangs
    // on Windows CI).
    cmd.env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0");

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
