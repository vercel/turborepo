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

/// Run turbo with standard env var suppression. Returns the raw Output.
pub fn run_turbo(test_dir: &Path, args: &[&str]) -> Output {
    run_turbo_with_env(test_dir, args, &[])
}

/// Run turbo with standard env var suppression plus additional env overrides.
pub fn run_turbo_with_env(test_dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut cmd = assert_cmd::Command::cargo_bin("turbo").expect("turbo binary not found");
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("DO_NOT_TRACK", "1")
        .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(test_dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("failed to execute turbo")
}

pub fn turbo_configs_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/integration/fixtures/turbo-configs")
}

/// Copy a turbo-config JSON into the test directory as `turbo.json` and commit.
pub fn replace_turbo_json(dir: &Path, config_name: &str) {
    let src = turbo_configs_dir().join(config_name);
    fs::copy(&src, dir.join("turbo.json"))
        .unwrap_or_else(|e| panic!("copy {} failed: {e}", src.display()));
    let normalized = fs::read_to_string(dir.join("turbo.json"))
        .unwrap()
        .replace("\r\n", "\n");
    fs::write(dir.join("turbo.json"), normalized).unwrap();
    std::process::Command::new("git")
        .args([
            "commit",
            "-am",
            "replace turbo.json",
            "--quiet",
            "--allow-empty",
        ])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok();
}

/// Create a mock turbo config directory with a fake auth token.
/// Returns the config dir path (pass as TURBO_CONFIG_DIR_PATH).
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
/// Returns the config dir path (pass as TURBO_CONFIG_DIR_PATH).
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

    // Copy base fixture
    setup::copy_dir_all(&base_fixture, dir).unwrap();
    // Overlay package-manager-specific files
    setup::copy_dir_all(&pm_overlay, dir).unwrap();

    // Init git
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.email", "turbo-test@example.com"]);
    git(&["config", "user.name", "Turbo Test"]);

    let gitignore = dir.join(".gitignore");
    let mut gi = fs::read_to_string(&gitignore).unwrap_or_default();
    gi.push_str("\n.turbo\nnode_modules\n");
    fs::write(&gitignore, gi).unwrap();

    git(&["add", "."]);
    git(&["commit", "-m", "Initial", "--quiet"]);
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
        for entry in walkdir(dir, "turbo") {
            if entry.ends_with("bin/turbo") {
                fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).ok();
            }
        }
    }

    // On Windows, place the echo_args binary as turbo.exe next to every .keep
    // in turbo-windows-*/bin/ directories.
    #[cfg(windows)]
    {
        let echo_args_exe = assert_cmd::cargo::cargo_bin("echo_args");
        if !echo_args_exe.exists() {
            // Build echo_args if nextest didn't build it
            std::process::Command::new("cargo")
                .args(["build", "--bin", "echo_args", "-p", "turbo"])
                .current_dir(&repo_root)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok();
        }
        place_echo_args(dir, &echo_args_exe);

        // For the linked fixture, recreate symlinks as NTFS junctions
        if fixture_name == "linked" {
            setup_linked_junctions(dir);
        }
    }
}

fn walkdir(dir: &Path, name: &str) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(walkdir(&path, name));
            } else if path.file_name().map(|f| f == name).unwrap_or(false) {
                results.push(path);
            }
        }
    }
    results
}

/// Place echo_args.exe as turbo.exe next to .keep files in turbo-windows-*/bin/
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

/// Recreate symlinks as NTFS junctions for the linked fixture on Windows.
/// Git on Windows may check out symlinks as plain text files containing the
/// target path, so we remove whatever was copied and create real junctions.
#[cfg(windows)]
fn setup_linked_junctions(dir: &Path) {
    let nm = dir.join("node_modules");
    let pnpm_store = nm.join(".pnpm");
    let pnpm_turbo_nm = pnpm_store.join("turbo@1.0.0").join("node_modules");

    let mkjunction = |link: &Path, target: &Path| {
        // Remove whatever copy_dir_all created (file or dir)
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

    // Level 1: node_modules/turbo -> .pnpm/turbo@1.0.0/node_modules/turbo
    mkjunction(&nm.join("turbo"), &pnpm_turbo_nm.join("turbo"));

    // Level 2: platform package symlinks inside the pnpm virtual store
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
/// Set all turbo package versions in a find-turbo fixture.
pub fn set_find_turbo_version(dir: &Path, version: &str) {
    set_find_turbo_version_inner(dir, version);
}

fn set_find_turbo_version_inner(dir: &Path, version: &str) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                set_find_turbo_version_inner(&path, version);
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

/// Replace all fake turbo binaries with symlinks to the real turbo binary.
/// Replace all fake turbo binaries with symlinks to the given binary.
pub fn set_find_turbo_link(dir: &Path, turbo_path: &Path) {
    set_find_turbo_link_inner(dir, turbo_path);
}

fn set_find_turbo_link_inner(dir: &Path, turbo_path: &Path) {
    let target_name = if cfg!(windows) { "turbo.exe" } else { "turbo" };
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                set_find_turbo_link_inner(&path, turbo_path);
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
                let mut command = assert_cmd::Command::cargo_bin("turbo")?;

                command
                    .arg($command)
                    // Ensure telemetry can initialize by providing a writable config directory.
                    // This prevents debug builds from printing errors to stdout when telemetry
                    // init fails due to missing config directories.
                    .env("TURBO_CONFIG_DIR_PATH", tempdir.path())
                    // Disable telemetry and various warnings to ensure clean JSON output
                    .env("DO_NOT_TRACK", "1")
                    .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
                    .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
                    .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
                    // Prevent CI-specific output formatting (::group:: markers)
                    .env_remove("CI")
                    .env_remove("GITHUB_ACTIONS");

                $(
                    command.arg($query);
                )*

                let output = command.current_dir(tempdir.path()).output()?;

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
