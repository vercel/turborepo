use std::{path::Path, process::Command};

use camino::Utf8Path;

fn setup_fixture(
    fixture: &str,
    package_manager: Option<&str>,
    test_dir: &Path,
) -> Result<(), anyhow::Error> {
    let script_path = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/helpers/setup_integration_test.sh");

    Command::new("bash")
        .arg("-c")
        .arg(format!(
            "{} {} {}",
            script_path,
            fixture,
            package_manager.unwrap_or("npm@10.5.0")
        ))
        .current_dir(test_dir)
        .spawn()?
        .wait()?;

    Ok(())
}

fn check_query(fixture: &str, query: &str) -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup_fixture(fixture, None, tempdir.path())?;
    let output = assert_cmd::Command::cargo_bin("turbo")?
        .arg("query")
        .arg(query)
        .current_dir(tempdir.path())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let query_output: serde_json::Value = serde_json::from_str(&stdout)?;
    insta::assert_json_snapshot!(query_output);

    Ok(())
}

#[cfg(not(windows))]
#[test]
fn test_double_symlink() -> Result<(), anyhow::Error> {
    check_query(
        "oxc_repro",
        "query {
                 file(path: \"./index.js\") {
                    path
                    dependencies {
                      files { items { path } }
                      errors { items { message import } }
                    }
                 }
              }",
    )?;
    Ok(())
}
