use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
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
///
/// Equivalent to: `cp -a fixtures/$fixture/. $target_dir/`
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
///
/// Equivalent to setup_git.sh:
///   git init, configure user, write .npmrc, git add ., git commit
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
///
/// Returns the path to the corepack install directory (outside `target_dir` so
/// corepack shims don't appear as task inputs).
///
/// Equivalent to setup_package_manager.sh.
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

    // Enable corepack for this package manager
    let pm_name = package_manager.split('@').next().unwrap_or(package_manager);
    fs::create_dir_all(corepack_dir)?;

    let status = cmd("corepack")
        .arg("enable")
        .arg(pm_name)
        .arg(format!("--install-directory={}", corepack_dir.display()))
        .current_dir(target_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| anyhow::anyhow!("failed to run corepack: {e}"))?;

    if !status.success() {
        anyhow::bail!("corepack enable {} failed with {}", pm_name, status);
    }

    Ok(())
}

/// Install dependencies using the specified package manager.
///
/// Equivalent to install_deps.sh.
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
                &["install", &format!("--cache-folder={}", cache.display())],
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

/// The full integration test setup, equivalent to `setup_integration_test.sh`.
///
/// The corepack install directory is placed outside `target_dir` (in a sibling
/// temp directory) so that corepack shims don't appear as turbo task inputs,
/// matching the prysk shell setup behavior.
pub fn setup_integration_test(
    target_dir: &Path,
    fixture: &str,
    package_manager: &str,
    install: bool,
) -> Result<(), anyhow::Error> {
    let corepack_dir = target_dir.with_file_name(format!(
        "{}-corepack",
        target_dir
            .file_name()
            .expect("target_dir should have a file name")
            .to_string_lossy()
    ));
    fs::create_dir_all(&corepack_dir)?;

    copy_fixture(fixture, target_dir)?;
    setup_git(target_dir)?;
    setup_package_manager(target_dir, package_manager, &corepack_dir)?;
    if install {
        install_deps(target_dir, package_manager, &corepack_dir)?;
    }
    Ok(())
}

fn run_cmd(dir: &Path, program: &str, args: &[&str], path_env: &str) -> Result<(), anyhow::Error> {
    let status = cmd_with_path(program, path_env)
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| anyhow::anyhow!("failed to run `{program}`: {e}"))?;

    if !status.success() {
        anyhow::bail!("{} {:?} failed with {}", program, args, status);
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

fn prepend_to_path(dir: &Path) -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    format!("{}{sep}{current}", dir.display())
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
