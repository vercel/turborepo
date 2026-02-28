mod common;

use std::fs;

use common::{replace_turbo_json, run_turbo, setup};

#[test]
fn test_gitignored_file_in_explicit_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), "gitignored-inputs.json");

    // Create internal.txt for util and add it to gitignore
    fs::write(
        tempdir.path().join("packages/util/internal.txt"),
        "hello world\n",
    )
    .unwrap();

    // Append to .gitignore
    let mut gitignore = fs::read_to_string(tempdir.path().join(".gitignore")).unwrap_or_default();
    gitignore.push_str("\npackages/util/internal.txt\n");
    fs::write(tempdir.path().join(".gitignore"), gitignore).unwrap();

    // Commit the change
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(tempdir.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "add internal.txt", "--quiet"])
        .current_dir(tempdir.path())
        .output()
        .unwrap();

    // First run with --summarize
    let output1 = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=util",
            "--output-logs=hash-only",
            "--summarize",
        ],
    );
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(stdout1.contains("cache miss"));

    // Read the run summary and verify internal.txt has a hash
    let runs_dir = tempdir.path().join(".turbo/runs");
    let summary_file = fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .expect("expected a run summary JSON file");

    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(summary_file.path()).unwrap()).unwrap();

    let task = summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"].as_str() == Some("util#build"))
        .expect("expected util#build task in summary");

    let internal_hash = task["inputs"]["internal.txt"].as_str();
    assert!(
        internal_hash.is_some(),
        "internal.txt should appear in inputs despite being gitignored"
    );
    let first_hash = internal_hash.unwrap().to_string();

    // Clean up runs dir
    fs::remove_dir_all(&runs_dir).unwrap();

    // Change internal.txt content
    fs::write(
        tempdir.path().join("packages/util/internal.txt"),
        "hello world\nchanged!\n",
    )
    .unwrap();

    // Second run
    let output2 = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=util",
            "--output-logs=hash-only",
            "--summarize",
        ],
    );
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("cache miss"),
        "expected cache miss after changing gitignored input, got: {stdout2}"
    );

    // Read second summary and verify hash changed
    let summary_file2 = fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .expect("expected second run summary");

    let summary2: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(summary_file2.path()).unwrap()).unwrap();

    let task2 = summary2["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"].as_str() == Some("util#build"))
        .unwrap();

    let second_hash = task2["inputs"]["internal.txt"]
        .as_str()
        .unwrap()
        .to_string();

    assert_ne!(
        first_hash, second_hash,
        "internal.txt hash should change after content change"
    );
}
