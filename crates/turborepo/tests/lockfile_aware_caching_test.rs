mod common;

use common::{git, run_turbo, setup_lockfile_test};

fn apply_patch(dir: &std::path::Path, target: &str, patch_file: &str) {
    let status = std::process::Command::new("patch")
        .args([target, patch_file])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "patch {target} {patch_file} failed");
}

fn run_lockfile_test(pm_name: &str, lockfile: &str, dep_patch: &str, root_patch: &str) {
    let tempdir = tempfile::tempdir().unwrap();
    setup_lockfile_test(tempdir.path(), pm_name);

    let output_a = run_turbo(tempdir.path(), &["build", "--filter=a"]);
    assert!(output_a.status.success());
    let stdout_a = String::from_utf8_lossy(&output_a.stdout);
    assert!(stdout_a.contains("cache miss"));

    let output_b = run_turbo(tempdir.path(), &["build", "--filter=b"]);
    assert!(output_b.status.success());
    let stdout_b = String::from_utf8_lossy(&output_b.stdout);
    assert!(stdout_b.contains("cache miss"));

    apply_patch(tempdir.path(), lockfile, dep_patch);

    let output_a2 = run_turbo(tempdir.path(), &["build", "--filter=a"]);
    let stdout_a2 = String::from_utf8_lossy(&output_a2.stdout);
    assert!(
        stdout_a2.contains("FULL TURBO"),
        "{pm_name}: a should be cache hit after b's dep bump: {stdout_a2}"
    );

    let output_b2 = run_turbo(tempdir.path(), &["build", "--filter=b"]);
    let stdout_b2 = String::from_utf8_lossy(&output_b2.stdout);
    assert!(
        stdout_b2.contains("cache miss"),
        "{pm_name}: b should be cache miss after dep bump: {stdout_b2}"
    );

    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "bump lockfile", "--quiet"],
    );

    let output_filter = run_turbo(
        tempdir.path(),
        &["build", "--filter=[HEAD^1]", "--dry=json"],
    );
    let json: serde_json::Value = serde_json::from_slice(&output_filter.stdout).unwrap();
    let mut packages: Vec<String> = json["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    packages.sort();
    assert!(
        packages.contains(&"b".to_string()),
        "{pm_name}: b should be in filter after dep bump: {packages:?}"
    );
    assert!(
        !packages.contains(&"a".to_string()),
        "{pm_name}: a should NOT be in filter after b's dep bump: {packages:?}"
    );

    apply_patch(tempdir.path(), lockfile, root_patch);

    let output_a3 = run_turbo(tempdir.path(), &["build", "--filter=a"]);
    let stdout_a3 = String::from_utf8_lossy(&output_a3.stdout);
    assert!(
        stdout_a3.contains("cache miss"),
        "{pm_name}: a should miss after root bump: {stdout_a3}"
    );

    let output_b3 = run_turbo(tempdir.path(), &["build", "--filter=b"]);
    let stdout_b3 = String::from_utf8_lossy(&output_b3.stdout);
    assert!(
        stdout_b3.contains("cache miss"),
        "{pm_name}: b should miss after root bump: {stdout_b3}"
    );

    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "global lockfile change", "--quiet"],
    );

    let output_filter2 = run_turbo(
        tempdir.path(),
        &["build", "--filter=[HEAD^1]", "--dry=json"],
    );
    let json2: serde_json::Value = serde_json::from_slice(&output_filter2.stdout).unwrap();
    let mut packages2: Vec<String> = json2["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    packages2.sort();
    assert!(
        packages2.contains(&"a".to_string()) && packages2.contains(&"b".to_string()),
        "{pm_name}: all packages should be rebuilt after root bump: {packages2:?}"
    );
}

#[test]
fn test_lockfile_aware_caching_npm() {
    run_lockfile_test(
        "npm",
        "package-lock.json",
        "package-lock.patch",
        "turbo-bump.patch",
    );
}

#[test]
fn test_lockfile_aware_caching_yarn() {
    run_lockfile_test("yarn", "yarn.lock", "yarn-lock.patch", "turbo-bump.patch");
}

#[test]
fn test_lockfile_aware_caching_pnpm() {
    run_lockfile_test(
        "pnpm",
        "pnpm-lock.yaml",
        "pnpm-lock.patch",
        "turbo-bump.patch",
    );
}

#[test]
fn test_lockfile_aware_caching_berry() {
    run_lockfile_test("berry", "yarn.lock", "yarn-lock.patch", "turbo-bump.patch");
}

#[test]
fn test_lockfile_aware_caching_bun() {
    run_lockfile_test("bun", "bun.lock", "bun-lock.patch", "turbo-bump.patch");
}
