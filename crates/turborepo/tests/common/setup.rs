#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(dead_code)]

use std::{
    ffi::{OsStr, OsString},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
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

/// Write the `packageManager` field into `package.json`.
pub fn setup_package_manager(
    target_dir: &Path,
    package_manager: &str,
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

    Ok(())
}

/// Configure Corepack for the package manager used by a fixture.
///
/// The install directory is placed outside `target_dir` so Corepack shims don't
/// appear as task inputs.
pub fn setup_corepack(
    target_dir: &Path,
    package_manager: &str,
    corepack_dir: &Path,
) -> Result<(), anyhow::Error> {
    let pm_name = package_manager.split('@').next().unwrap_or(package_manager);
    if !corepack_supports(pm_name) {
        return Ok(());
    }

    fs::create_dir_all(corepack_dir)?;
    let corepack_home = corepack_home();
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
    prepare_corepack_package_manager(target_dir, &corepack_home, package_manager)?;

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

    let corepack_dir = corepack_dir_for_test_dir(dir);
    setup_corepack(dir, &pm, &corepack_dir).expect("failed to configure corepack");
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
            run_cmd(target_dir, "npm", &["install", "--offline"], &path_env)?;
            normalize_lockfile_on_windows(target_dir, "package-lock.json");
        }
        "pnpm" => {
            run_cmd(target_dir, "pnpm", &["install"], &path_env)?;
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
            run_cmd(target_dir, "bun", &["install"], &path_env)?;
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
    copy_fixture(fixture, target_dir)?;
    setup_git(target_dir)?;
    setup_package_manager(target_dir, package_manager)?;
    if install {
        let corepack_dir = corepack_dir_for_test_dir(target_dir);
        setup_corepack(target_dir, package_manager, &corepack_dir)?;
        install_deps(target_dir, package_manager, &corepack_dir)?;
    }
    Ok(())
}

fn run_cmd(dir: &Path, program: &str, args: &[&str], path_env: &str) -> Result<(), anyhow::Error> {
    let output = cmd_with_path(program, path_env)
        .args(args)
        .current_dir(dir)
        .env("COREPACK_HOME", corepack_home())
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

pub fn corepack_home() -> PathBuf {
    std::env::temp_dir().join(format!(
        "turborepo-test-corepack-{:x}",
        stable_hash(workspace_root().as_os_str().as_encoded_bytes())
    ))
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

fn prepare_corepack_package_manager(
    dir: &Path,
    corepack_home: &Path,
    package_manager: &str,
) -> Result<(), anyhow::Error> {
    let marker = corepack_home
        .join("prepared")
        .join(safe_filename(package_manager));
    if marker.exists() {
        return Ok(());
    }

    with_corepack_prepare_lock(corepack_home, || {
        if marker.exists() {
            return Ok(());
        }

        run_corepack(
            dir,
            corepack_home,
            ["prepare", package_manager, "--activate"],
        )?;
        if let Some(parent) = marker.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(marker, b"")?;

        Ok(())
    })
}

fn with_corepack_prepare_lock<F>(corepack_home: &Path, f: F) -> Result<(), anyhow::Error>
where
    F: FnOnce() -> Result<(), anyhow::Error>,
{
    const LOCK_TIMEOUT: Duration = Duration::from_secs(120);
    const STALE_LOCK_AGE: Duration = Duration::from_secs(300);

    let lock_dir = corepack_home.join("prepare.lock");
    let start = Instant::now();
    loop {
        match fs::create_dir(&lock_dir) {
            Ok(()) => break,
            Err(e) => {
                let lock_exists = e.kind() == ErrorKind::AlreadyExists;
                // Windows can report access denied while another test process is
                // removing the directory lock. Treat it as contention and retry.
                let access_denied_during_lock_transition =
                    cfg!(windows) && e.kind() == ErrorKind::PermissionDenied;
                if !lock_exists && !access_denied_during_lock_transition {
                    return Err(e.into());
                }

                if lock_exists && is_stale_lock(&lock_dir, STALE_LOCK_AGE) {
                    let _ = fs::remove_dir_all(&lock_dir);
                    continue;
                }
                if start.elapsed() > LOCK_TIMEOUT {
                    anyhow::bail!(
                        "timed out waiting for Corepack prepare lock: {} ({e})",
                        lock_dir.display(),
                    );
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    struct LockGuard(PathBuf);
    impl Drop for LockGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let _guard = LockGuard(lock_dir);
    f()
}

fn is_stale_lock(lock_dir: &Path, stale_after: Duration) -> bool {
    fs::metadata(lock_dir)
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
        .map(|age| age > stale_after)
        .unwrap_or(false)
}

fn safe_filename(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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
