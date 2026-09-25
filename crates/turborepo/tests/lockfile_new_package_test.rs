#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{setup, setup_lockfile_test, turbo_command};

#[test]
fn test_new_package_in_lockfile_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_lockfile_test(tempdir.path(), "pnpm");

    fs::create_dir_all(tempdir.path().join("apps/c")).unwrap();
    fs::write(
        tempdir.path().join("apps/c/package.json"),
        r#"{"name":"c", "dependencies": {"has-symbols": "^1.0.3"}}"#,
    )
    .unwrap();

    // Update only the lockfile, using the fixture's pinned pnpm version and
    // existing resolutions. The new importer must be present for this test to
    // exercise lockfile-aware package selection; don't hide a failed update.
    let pnpm_output = std::process::Command::new("pnpm")
        .args([
            "install",
            "--lockfile-only",
            "--offline",
            "--ignore-scripts",
            "--no-frozen-lockfile",
        ])
        .current_dir(tempdir.path())
        .env(
            "PATH",
            setup::prepend_to_path(&setup::corepack_dir_for_test_dir(tempdir.path())),
        )
        .env("COREPACK_HOME", setup::corepack_home())
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .output()
        .expect("failed to execute pnpm");
    assert!(
        pnpm_output.status.success(),
        "pnpm lockfile-only update failed with {}\nstdout:\n{}\nstderr:\n{}",
        pnpm_output.status,
        String::from_utf8_lossy(&pnpm_output.stdout),
        String::from_utf8_lossy(&pnpm_output.stderr),
    );

    let lockfile = fs::read_to_string(tempdir.path().join("pnpm-lock.yaml")).unwrap();
    let importer_start = lockfile
        .lines()
        .position(|line| line.trim_end() == "  apps/c:")
        .expect("pnpm lockfile update must add the new package importer");
    let importer = lockfile
        .lines()
        .skip(importer_start + 1)
        .take_while(|line| line.starts_with("    "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        importer
            .lines()
            .any(|line| line.trim() == "has-symbols: ^1.0.3")
            && importer
                .lines()
                .any(|line| line.trim() == "has-symbols: 1.0.3"),
        "pnpm lockfile importer must record the package specifier and resolution:\n{importer}"
    );

    // --skip-infer ensures the smoke exercises the repository-built CLI rather
    // than delegating to the fixture's declared local turbo dependency.
    let config_dir = tempfile::tempdir().unwrap();
    let output = turbo_command(tempdir.path())
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("MSYS_NO_PATHCONV", "1")
        .args([
            "--skip-infer",
            "build",
            "-F",
            "[HEAD]",
            "-F",
            "!//",
            "--dry=json",
        ])
        .output()
        .expect("failed to execute turbo");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "expected valid JSON, got error {e}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });

    let packages: Vec<&str> = json["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    assert_eq!(packages, vec!["c"]);
}
