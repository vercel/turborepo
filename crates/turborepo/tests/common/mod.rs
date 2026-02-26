pub mod setup;

use std::{path::Path, process::Output};

/// Insta filters that normalize non-deterministic parts of turbo's stdout:
/// - Path separators (backslash → forward slash for Windows)
/// - Timing lines (e.g. "Time:    1.234s" → "Time:    [TIME]")
#[allow(dead_code)]
pub fn turbo_output_filters() -> Vec<(&'static str, &'static str)> {
    vec![(r"\\", "/"), (r"Time:\s*[\.0-9]+m?s", "Time:    [TIME]")]
}

/// Run turbo with standard env var suppression. Returns the raw Output.
#[allow(dead_code)]
pub fn run_turbo(test_dir: &Path, args: &[&str]) -> Output {
    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut cmd = assert_cmd::Command::cargo_bin("turbo").expect("turbo binary not found");
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("DO_NOT_TRACK", "1")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(test_dir);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("failed to execute turbo")
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
