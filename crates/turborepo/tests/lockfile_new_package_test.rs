#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{setup_lockfile_test, turbo_command};

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

    // Add the exact importer pnpm v7 would write, reusing the existing
    // has-symbols resolution. This keeps the lockfile-filter regression
    // deterministic without invoking pnpm or relying on a populated package
    // store.
    let lockfile_path = tempdir.path().join("pnpm-lock.yaml");
    let lockfile = fs::read_to_string(&lockfile_path).unwrap();
    let mut lockfile_lines = lockfile.lines().map(str::to_owned).collect::<Vec<_>>();
    let packages_index = lockfile_lines
        .iter()
        .position(|line| line == "packages:")
        .expect("pnpm lockfile should include package resolutions");
    for (offset, line) in [
        "  apps/c:",
        "    specifiers:",
        "      has-symbols: ^1.0.3",
        "    dependencies:",
        "      has-symbols: 1.0.3",
        "",
    ]
    .into_iter()
    .enumerate()
    {
        lockfile_lines.insert(packages_index + offset, line.to_string());
    }
    fs::write(&lockfile_path, format!("{}\n", lockfile_lines.join("\n"))).unwrap();

    let lockfile = fs::read_to_string(&lockfile_path).unwrap();
    let importer_start = lockfile
        .lines()
        .position(|line| line.trim_end() == "  apps/c:")
        .expect("lockfile fixture must add the new package importer");
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
