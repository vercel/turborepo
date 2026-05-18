#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(dead_code)]

use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn manifest_dir() -> PathBuf {
    // Prefer the runtime env var, which nextest sets when using --workspace-remap
    // (e.g. running archived tests on a different machine). Fall back to the
    // compile-time value for normal `cargo test` runs.
    match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    }
}

fn workspace_root() -> PathBuf {
    manifest_dir()
        .join("../..")
        .canonicalize()
        .expect("failed to resolve workspace root")
}

fn fixtures_dir() -> PathBuf {
    workspace_root()
        .join("turborepo-tests")
        .join("integration")
        .join("fixtures")
}

/// Copy a fixture directory into `target_dir`.
pub fn copy_fixture(fixture: &str, target_dir: &Path) -> Result<(), anyhow::Error> {
    let src = fixtures_dir().join(fixture);
    if !src.exists() {
        anyhow::bail!("fixture not found: {}", src.display());
    }
    copy_dir_all(&src, target_dir)?;
    Ok(())
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), anyhow::Error> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        // file_type() on DirEntry doesn't follow symlinks
        let file_type = entry.file_type()?;

        if file_type.is_symlink() {
            copy_symlink(&src_path, &dst_path)?;
        } else if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn copy_symlink(src: &Path, dst: &Path) -> Result<(), anyhow::Error> {
    let target = fs::read_link(src)?;

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, dst)?;

    #[cfg(windows)]
    {
        if src.is_dir() {
            std::os::windows::fs::symlink_dir(&target, dst)?;
        } else {
            std::os::windows::fs::symlink_file(&target, dst)?;
        }
    }

    Ok(())
}

/// Initialize a git repository in `target_dir` with a single commit.
/// Configures user, writes .npmrc, adds all files, and commits.
pub fn setup_git(target_dir: &Path) -> Result<(), anyhow::Error> {
    let git = |args: &[&str]| -> Result<(), anyhow::Error> {
        let status = cmd("git")
            .args(args)
            .current_dir(target_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| anyhow::anyhow!("failed to run git: {e}"))?;
        if !status.success() {
            anyhow::bail!("git {:?} failed with {}", args, status);
        }
        Ok(())
    };

    git(&["init", "--quiet", "--initial-branch=main"])?;
    git(&["config", "user.email", "turbo-test@example.com"])?;
    git(&["config", "user.name", "Turbo Test"])?;

    // npm script-shell=bash for cross-platform consistency
    // update-notifier=false suppresses "npm notice" upgrade messages that cause
    // test flakes
    fs::write(
        target_dir.join(".npmrc"),
        "script-shell=bash\nupdate-notifier=false\n",
    )?;

    git(&["add", "."])?;
    git(&["commit", "-m", "Initial", "--quiet"])?;

    Ok(())
}

/// Write the `packageManager` field into `package.json` and configure corepack.
/// The corepack install directory is placed outside `target_dir` so corepack
/// shims don't appear as task inputs.
pub fn setup_package_manager(
    target_dir: &Path,
    package_manager: &str,
    corepack_dir: &Path,
) -> Result<(), anyhow::Error> {
    // Read, modify, and write package.json
    let pkg_json_path = target_dir.join("package.json");
    let contents = fs::read_to_string(&pkg_json_path)?;
    let mut pkg: serde_json::Value = serde_json::from_str(&contents)?;
    pkg["packageManager"] = serde_json::Value::String(package_manager.to_string());
    let new_contents = serde_json::to_string_pretty(&pkg)? + "\n";

    // Write with Unix line endings
    let normalized = new_contents.replace("\r\n", "\n");
    fs::write(&pkg_json_path, normalized)?;

    // Commit the change
    git_commit_if_changed(
        target_dir,
        &format!("Updated package manager to {package_manager}"),
    )?;

    // Exercise package manager resolution through Corepack for every PM it
    // supports, including npm. Keep Corepack state per test so parallel tests
    // do not contend over the user's global Corepack cache/last-known-good data.
    let pm_name = package_manager.split('@').next().unwrap_or(package_manager);
    if !corepack_supports(pm_name) {
        return Ok(());
    }

    fs::create_dir_all(corepack_dir)?;
    let corepack_home = corepack_home(corepack_dir);
    fs::create_dir_all(&corepack_home)?;

    run_corepack(
        target_dir,
        &corepack_home,
        [
            OsString::from("enable"),
            OsString::from(pm_name),
            OsString::from(format!("--install-directory={}", corepack_dir.display())),
        ],
    )?;

    // Pre-download the exact PM version into corepack's cache so that
    // subsequent invocations (yarn install, turbo run build → yarn run build)
    // resolve locally without any network access. Without this, every
    // corepack-intercepted PM call can trigger a slow download that causes
    // tests to timeout in CI.
    run_corepack(
        target_dir,
        &corepack_home,
        ["prepare", package_manager, "--activate"],
    )?;

    Ok(())
}

/// Read the `packageManager` field from `package.json` in `dir` and run
/// `corepack prepare <value> --activate` to pre-warm the corepack cache.
/// This is used by test setups that copy fixtures with a pre-existing
/// `packageManager` field (e.g. lockfile-aware-caching tests) and don't go
/// through `setup_package_manager`.
pub fn prepare_corepack_from_package_json(dir: &Path) {
    let pkg_json_path = dir.join("package.json");
    let contents = fs::read_to_string(&pkg_json_path).expect("failed to read package.json");
    let pkg: serde_json::Value =
        serde_json::from_str(&contents).expect("failed to parse package.json");

    let pm = match pkg.get("packageManager").and_then(|v| v.as_str()) {
        Some(pm) => pm.to_string(),
        None => return,
    };

    let pm_name = pm.split('@').next().unwrap_or(&pm);
    if !corepack_supports(pm_name) {
        return;
    }

    let corepack_dir = corepack_dir_for_test_dir(dir);
    fs::create_dir_all(&corepack_dir).expect("failed to create corepack dir");
    let corepack_home = corepack_home(&corepack_dir);
    fs::create_dir_all(&corepack_home).expect("failed to create corepack home");

    run_corepack(
        dir,
        &corepack_home,
        [
            OsString::from("enable"),
            OsString::from(pm_name),
            OsString::from(format!("--install-directory={}", corepack_dir.display())),
        ],
    )
    .expect("failed to enable corepack");

    run_corepack(dir, &corepack_home, ["prepare", &pm, "--activate"])
        .expect("failed to prepare corepack");
}

/// Install dependencies using the specified package manager.
pub fn install_deps(
    target_dir: &Path,
    package_manager: &str,
    corepack_dir: &Path,
) -> Result<(), anyhow::Error> {
    let pm_name = package_manager.split('@').next().unwrap_or(package_manager);

    // Build the PATH with the corepack directory so the correct PM version is used
    let path_env = prepend_to_path(corepack_dir);

    match pm_name {
        "npm" => {
            run_cmd(
                target_dir,
                "npm",
                &["install", "--offline"],
                &path_env,
                corepack_dir,
            )?;
            normalize_lockfile_on_windows(target_dir, "package-lock.json");
        }
        "pnpm" => {
            run_cmd(target_dir, "pnpm", &["install"], &path_env, corepack_dir)?;
            normalize_lockfile_on_windows(target_dir, "pnpm-lock.yaml");
        }
        "yarn" => {
            let cache = target_dir.join(".yarn-cache");
            run_cmd(
                target_dir,
                "yarn",
                &[
                    "install",
                    "--prefer-offline",
                    "--frozen-lockfile",
                    &format!("--cache-folder={}", cache.display()),
                ],
                &path_env,
                corepack_dir,
            )?;

            // Ignore the cache from git
            let mut gitignore =
                fs::read_to_string(target_dir.join(".gitignore")).unwrap_or_default();
            if !gitignore.contains(".yarn-cache") {
                gitignore.push_str(".yarn-cache\n");
                fs::write(target_dir.join(".gitignore"), gitignore)?;
            }

            normalize_lockfile_on_windows(target_dir, "yarn.lock");
        }
        "bun" => {
            run_cmd(target_dir, "bun", &["install"], &path_env, corepack_dir)?;
        }
        other => anyhow::bail!("unsupported package manager: {other}"),
    }

    // Stage and commit installed deps
    let _ = cmd("git")
        .args(["add", "."])
        .current_dir(target_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    git_commit_if_changed(target_dir, "Install dependencies")?;

    Ok(())
}

/// The full integration test setup.
///
/// The corepack install directory is placed outside `target_dir` (in a sibling
/// temp directory) so that corepack shims don't appear as turbo task inputs.
pub fn setup_integration_test(
    target_dir: &Path,
    fixture: &str,
    package_manager: &str,
    install: bool,
) -> Result<(), anyhow::Error> {
    let corepack_dir = corepack_dir_for_test_dir(target_dir);
    fs::create_dir_all(&corepack_dir)?;

    copy_fixture(fixture, target_dir)?;
    setup_git(target_dir)?;
    setup_package_manager(target_dir, package_manager, &corepack_dir)?;
    if install {
        install_deps(target_dir, package_manager, &corepack_dir)?;
    }
    Ok(())
}

fn run_cmd(
    dir: &Path,
    program: &str,
    args: &[&str],
    path_env: &str,
    corepack_dir: &Path,
) -> Result<(), anyhow::Error> {
    let output = cmd_with_path(program, path_env)
        .args(args)
        .current_dir(dir)
        .env("COREPACK_HOME", corepack_home(corepack_dir))
        // Safety net: auto-approve any corepack download prompt in case the
        // cache is somehow cold. The setup pre-warms the cache via
        // `corepack prepare` so this should rarely be needed.
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run `{program}`: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "{} {:?} failed with {}:\n{}",
            program,
            args,
            output.status,
            stderr
        );
    }
    Ok(())
}

fn git_commit_if_changed(dir: &Path, message: &str) -> Result<(), anyhow::Error> {
    let output = cmd("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()?;

    if !output.stdout.is_empty() {
        let _ = cmd("git")
            .args(["commit", "-am", message, "--quiet"])
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok(())
}

fn corepack_supports(pm_name: &str) -> bool {
    matches!(pm_name, "npm" | "yarn" | "pnpm" | "berry")
}

pub fn corepack_dir_for_test_dir(target_dir: &Path) -> PathBuf {
    target_dir.with_file_name(format!(
        "{}-corepack",
        target_dir
            .file_name()
            .expect("target_dir should have a file name")
            .to_string_lossy()
    ))
}

pub fn corepack_home(corepack_dir: &Path) -> PathBuf {
    corepack_dir.join("home")
}

pub fn prepend_to_path(dir: &Path) -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    format!("{}{sep}{current}", dir.display())
}

fn run_corepack<I, S>(dir: &Path, corepack_home: &Path, args: I) -> Result<(), anyhow::Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect::<Vec<_>>();
    let output = cmd("corepack")
        .args(&args)
        .current_dir(dir)
        .env("COREPACK_HOME", corepack_home)
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run corepack {args:?}: {e}"))?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "corepack {:?} failed with {}:\n{}{}",
            args,
            output.status,
            stdout,
            stderr
        );
    }

    Ok(())
}

/// Create a Command for a program, resolving it via PATH (including PATHEXT on
/// Windows). This is necessary because Rust's Command::new on Windows doesn't
/// check PATHEXT, so `npm.cmd` / `corepack.cmd` won't be found by name alone.
fn cmd(program: &str) -> Command {
    match which::which(program) {
        Ok(path) => Command::new(path),
        Err(_) => Command::new(program),
    }
}

/// Like `cmd()` but searches a custom PATH.
fn cmd_with_path(program: &str, path_env: &str) -> Command {
    match which::which_in(program, Some(path_env), ".") {
        Ok(path) => {
            let mut c = Command::new(path);
            c.env("PATH", path_env);
            c
        }
        Err(_) => {
            let mut c = Command::new(program);
            c.env("PATH", path_env);
            c
        }
    }
}

fn normalize_lockfile_on_windows(_dir: &Path, _filename: &str) {
    #[cfg(windows)]
    {
        let path = _dir.join(_filename);
        if let Ok(contents) = fs::read_to_string(&path) {
            let normalized = contents.replace("\r\n", "\n");
            let _ = fs::write(&path, normalized);
        }
    }
}
