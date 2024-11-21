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
        .arg(format!(
            "{} {} {}",
            unix_script_path, fixture, package_manager
        ))
        .current_dir(test_dir)
        .spawn()?
        .wait()?;

    Ok(())
}

/// Executes a command with different arguments in a specific fixture and
/// package manager and snapshots the output as JSON.
/// Creates a snapshot file for each set of arguments.
/// Note that the command must return valid JSON
#[macro_export]
macro_rules! check_json {
    ($fixture:expr, $package_manager:expr, $command:expr, $($name:expr => $query:expr,)*) => {
        {
            let tempdir = tempfile::tempdir()?;
            crate::common::setup_fixture($fixture, $package_manager, tempdir.path())?;
            $(
                println!("Running command: `turbo {} {}` in {}", $command, $query, $fixture);
                let output = assert_cmd::Command::cargo_bin("turbo")?
                    .arg($command)
                    .arg($query)
                    .current_dir(tempdir.path())
                    .output()?;

                let stdout = String::from_utf8(output.stdout)?;
                let stderr = String::from_utf8_lossy(&output.stderr);

                println!("stderr: {}", stderr);

                let query_output: serde_json::Value = serde_json::from_str(&stdout)?;
                let test_name = format!(
                    "{}_{}_({})",
                    $fixture,
                    $name.replace(' ', "_"),
                    $package_manager
                );
                insta::assert_json_snapshot!(test_name, query_output);
            )*
        }
    }
}
