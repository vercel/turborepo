#![allow(dead_code)]

pub mod setup;

use std::{fs, path::Path, process::Output};

/// Insta filters that normalize non-deterministic parts of turbo's stdout:
/// - Path separators (backslash → forward slash for Windows)
/// - Timing lines (e.g. "Time:    1.234s" → "Time:    [TIME]")
/// - Binary name (turbo.exe → turbo on Windows)
pub fn turbo_output_filters() -> Vec<(&'static str, &'static str)> {
    vec![
        (r"\\", "/"),
        (r"Time:\s*[\.0-9]+m?s", "Time:    [TIME]"),
        (r"turbo\.exe", "turbo"),
    ]
}

/// Return a pre-configured `Command` for the turbo binary with all standard
/// env var suppression applied. Callers can chain `.arg()`, `.env()`, etc.
/// before calling `.output()`.
pub fn turbo_command(test_dir: &Path) -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("turbo").expect("turbo binary not found");
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(test_dir);
    cmd
}

/// Run turbo with standard env var suppression. Returns the raw Output.
pub fn run_turbo(test_dir: &Path, args: &[&str]) -> Output {
    run_turbo_with_env(test_dir, args, &[])
}

/// Run turbo with standard env var suppression plus additional env overrides.
pub fn run_turbo_with_env(test_dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut cmd = turbo_command(test_dir);
    cmd.env("TURBO_CONFIG_DIR_PATH", config_dir.path());
    for (k, v) in env {
        cmd.env(k, v);
    }
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("failed to execute turbo")
}

/// Run a git command silently in the given directory.
pub fn git(dir: &Path, args: &[&str]) {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("git command failed");
}

/// Combine stdout and stderr into a single string.
pub fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

pub fn turbo_configs_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/integration/fixtures/turbo-configs")
}

/// Copy a turbo-config JSON into the test directory as `turbo.json` and commit.
pub fn replace_turbo_json(dir: &Path, config_name: &str) {
    replace_turbo_json_from(dir, &turbo_configs_dir(), config_name);
}

/// Copy a config JSON from `configs_dir` into `dir` as `turbo.json` and commit.
pub fn replace_turbo_json_from(dir: &Path, configs_dir: &Path, config_name: &str) {
    let src = configs_dir.join(config_name);
    fs::copy(&src, dir.join("turbo.json"))
        .unwrap_or_else(|e| panic!("copy {} failed: {e}", src.display()));
    let normalized = fs::read_to_string(dir.join("turbo.json"))
        .unwrap()
        .replace("\r\n", "\n");
    fs::write(dir.join("turbo.json"), normalized).unwrap();
    git(
        dir,
        &[
            "commit",
            "-am",
            "replace turbo.json",
            "--quiet",
            "--allow-empty",
        ],
    );
}

/// Create a mock turbo config directory with a fake auth token.
pub fn mock_turbo_config(config_dir: &Path) {
    let turbo_dir = config_dir.join("turborepo");
    fs::create_dir_all(&turbo_dir).unwrap();
    fs::write(
        turbo_dir.join("config.json"),
        r#"{"token":"normal-user-token"}"#,
    )
    .unwrap();
}

/// Create a mock telemetry config directory with telemetry enabled.
pub fn mock_telemetry_config(config_dir: &Path) {
    let turbo_dir = config_dir.join("turborepo");
    fs::create_dir_all(&turbo_dir).unwrap();
    fs::write(
        turbo_dir.join("telemetry_config.json"),
        r#"{"telemetry_enabled":true}"#,
    )
    .unwrap();
}

/// Set up a lockfile-aware-caching test. Copies the shared base fixture then
/// overlays the package-manager-specific files (lockfile, patches,
/// package.json).
pub fn setup_lockfile_test(dir: &Path, pm_name: &str) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
    let base_fixture =
        repo_root.join("turborepo-tests/integration/fixtures/lockfile_aware_caching");
    let pm_overlay = repo_root.join(format!(
        "turborepo-tests/integration/tests/lockfile-aware-caching/{pm_name}"
    ));

    setup::copy_dir_all(&base_fixture, dir).unwrap();
    setup::copy_dir_all(&pm_overlay, dir).unwrap();

    // Lockfile tests need a minimal git init without .npmrc or extra .gitignore
    // entries that setup::setup_git() creates, since those would appear in git
    // diffs and affect the filter results.
    git(dir, &["init", "--quiet"]);
    git(dir, &["config", "user.email", "turbo-test@example.com"]);
    git(dir, &["config", "user.name", "Turbo Test"]);

    let gitignore = dir.join(".gitignore");
    let mut gi = fs::read_to_string(&gitignore).unwrap_or_default();
    gi.push_str("\n.turbo\nnode_modules\n");
    fs::write(&gitignore, gi).unwrap();

    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "Initial", "--quiet"]);
}

/// Set up a find-turbo test fixture. Copies the fixture directory, makes
/// scripts executable on Unix, and places echo_args as turbo.exe on Windows.
pub fn setup_find_turbo(dir: &Path, fixture_name: &str) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
    let fixture_src = repo_root.join(format!(
        "turborepo-tests/integration/fixtures/find_turbo/{fixture_name}"
    ));
    setup::copy_dir_all(&fixture_src, dir).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in find_files_by_name(dir, "turbo") {
            if entry.ends_with("bin/turbo") {
                fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).ok();
            }
        }
    }

    #[cfg(windows)]
    {
        let echo_args_exe = assert_cmd::cargo::cargo_bin("echo_args");
        if !echo_args_exe.exists() {
            std::process::Command::new("cargo")
                .args(["build", "--bin", "echo_args", "-p", "turbo"])
                .current_dir(&repo_root)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok();
        }
        place_echo_args(dir, &echo_args_exe);

        if fixture_name == "linked" {
            setup_linked_junctions(dir);
        }
    }
}

fn find_files_by_name(dir: &Path, name: &str) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(find_files_by_name(&path, name));
            } else if path.file_name().map(|f| f == name).unwrap_or(false) {
                results.push(path);
            }
        }
    }
    results
}

#[cfg(windows)]
fn place_echo_args(dir: &Path, echo_args_exe: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                place_echo_args(&path, echo_args_exe);
            } else if path.file_name().map_or(false, |n| n == ".keep") {
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
                            fs::copy(echo_args_exe, &turbo_exe).ok();
                        }
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn setup_linked_junctions(dir: &Path) {
    let nm = dir.join("node_modules");
    let pnpm_store = nm.join(".pnpm");
    let pnpm_turbo_nm = pnpm_store.join("turbo@1.0.0").join("node_modules");

    let mkjunction = |link: &Path, target: &Path| {
        if link.is_dir() {
            fs::remove_dir_all(link).ok();
        } else {
            fs::remove_file(link).ok();
        }
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        assert!(
            status.map_or(false, |s| s.success()),
            "mklink /J failed for {} -> {}",
            link.display(),
            target.display()
        );
    };

    mkjunction(&nm.join("turbo"), &pnpm_turbo_nm.join("turbo"));

    for platform in &[
        "darwin-64",
        "darwin-arm64",
        "linux-64",
        "linux-arm64",
        "windows-64",
        "windows-arm64",
    ] {
        mkjunction(
            &pnpm_turbo_nm.join(format!("turbo-{platform}")),
            &pnpm_store
                .join(format!("turbo-{platform}@1.0.0"))
                .join("node_modules")
                .join(format!("turbo-{platform}")),
        );
    }
}

/// Set all turbo package.json versions in a find-turbo fixture.
pub fn set_find_turbo_version(dir: &Path, version: &str) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                set_find_turbo_version(&path, version);
            } else if path
                .file_name()
                .map(|f| f == "package.json")
                .unwrap_or(false)
            {
                fs::write(&path, format!("{{ \"version\": \"{version}\" }}\n")).unwrap();
            }
        }
    }
}

/// Replace all fake turbo binaries with symlinks to the given binary.
pub fn set_find_turbo_link(dir: &Path, turbo_path: &Path) {
    let target_name = if cfg!(windows) { "turbo.exe" } else { "turbo" };
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                set_find_turbo_link(&path, turbo_path);
            } else if path.file_name().map(|f| f == target_name).unwrap_or(false) && path.is_file()
            {
                fs::remove_file(&path).unwrap();
                #[cfg(unix)]
                std::os::unix::fs::symlink(turbo_path, &path).unwrap();
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(turbo_path, &path).unwrap();
            }
        }
    }
}

pub fn setup_fixture(
    fixture: &str,
    package_manager: &str,
    test_dir: &Path,
) -> Result<(), anyhow::Error> {
    setup::setup_integration_test(test_dir, fixture, package_manager, true)
}

/// Executes a command and snapshots the output as JSON.
///
/// Takes fixture, package manager, and command, and sets of arguments.
/// Creates a snapshot file for each set of arguments.
/// Note that the command must return valid JSON
#[macro_export]
macro_rules! check_json_output {
    ($fixture:expr, $package_manager:expr, $command:expr, $($name:expr => [$($query:expr),*$(,)?],)*) => {
        {
            let tempdir = tempfile::tempdir()?;
            $crate::common::setup_fixture($fixture, $package_manager, tempdir.path())?;
            $(
                let mut command = $crate::common::turbo_command(tempdir.path());
                command
                    .arg($command)
                    .env("TURBO_CONFIG_DIR_PATH", tempdir.path());

                $(
                    command.arg($query);
                )*

                let output = command.output()?;

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                println!("stderr: {}", stderr);

                let query_output: serde_json::Value = serde_json::from_str(&stdout)?;
                let test_name = format!(
                    "{}_{}_({})",
                    $fixture,
                    $name.replace(' ', "_"),
                    $package_manager
                );

                insta::with_settings!({ filters => vec![(r"\\\\", "/")]}, {
                    insta::assert_json_snapshot!(
                        format!("{}", test_name),
                        query_output
                    )
                });
            )*
        }
    }
}
