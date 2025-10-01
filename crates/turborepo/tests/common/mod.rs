use std::{path::Path, process::Command};

use turbopath::AbsoluteSystemPath;
use which::which;

pub fn setup_fixture(
    fixture: &str,
    package_manager: &str,
    test_dir: &Path,
) -> Result<(), anyhow::Error> {
    let script_path = AbsoluteSystemPath::new(env!("CARGO_MANIFEST_DIR"))?.join_components(&[
        "..",
        "..",
        "turborepo-tests",
        "helpers",
        "setup_integration_test.sh",
    ]);

    let unix_script_path = if cfg!(windows) {
        script_path.as_str().replace("\\", "/")
    } else {
        script_path.to_string()
    };

    let bash = which("bash")?;

    Command::new(bash)
        .arg("-c")
        .arg(format!("{unix_script_path} {fixture} {package_manager}"))
        .current_dir(test_dir)
        .spawn()?
        .wait()?;

    Ok(())
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
                    .arg($command);

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
