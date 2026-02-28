pub mod setup;

use std::{fs, path::Path, process::Output};

/// Insta filters that normalize non-deterministic parts of turbo's stdout:
/// - Path separators (backslash → forward slash for Windows)
/// - Timing lines (e.g. "Time:    1.234s" → "Time:    [TIME]")
/// - Binary name (turbo.exe → turbo on Windows)
#[allow(dead_code)]
pub fn turbo_output_filters() -> Vec<(&'static str, &'static str)> {
    vec![
        (r"\\", "/"),
        (r"Time:\s*[\.0-9]+m?s", "Time:    [TIME]"),
        (r"turbo\.exe", "turbo"),
    ]
}

/// Run turbo with standard env var suppression. Returns the raw Output.
#[allow(dead_code)]
pub fn run_turbo(test_dir: &Path, args: &[&str]) -> Output {
    run_turbo_with_env(test_dir, args, &[])
}

/// Run turbo with standard env var suppression plus additional env overrides.
#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn turbo_configs_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/integration/fixtures/turbo-configs")
}

/// Copy a turbo-config JSON into the test directory as `turbo.json` and commit.
/// Equivalent to `replace_turbo_json.sh`.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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

#[allow(dead_code)]
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
