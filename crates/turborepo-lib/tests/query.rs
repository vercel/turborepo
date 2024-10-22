use std::{path::Path, process::Command};

use camino::Utf8Path;

fn setup_fixture(fixture: Option<&str>, test_dir: &Path) -> Result<(), anyhow::Error> {
    let script_path = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/helpers/setup_integration_test.sh");

    let mut command = Command::new("bash");

    command
        .arg("-c")
        .arg(format!(
            "{} {} npm@10.5.0",
            script_path,
            fixture.unwrap_or_default()
        ))
        .current_dir(test_dir);

    println!("{:?}", command);

    command.spawn()?.wait()?;
    Ok(())
}

#[test]
fn test_double_symlink() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup_fixture(Some("oxc_repro"), tempdir.path())?;

    Ok(())
}
