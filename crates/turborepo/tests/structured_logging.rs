mod common;

use common::{run_turbo, setup};

/// Verify `--log-file` produces a valid JSON array file
/// containing task output entries.
#[test]
fn log_file_produces_valid_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    let log_path = tempdir.path().join("structured.json");
    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--force",
            &format!("--log-file={}", log_path.display()),
        ],
    );

    assert!(output.status.success(), "turbo run should succeed");
    assert!(log_path.exists(), "structured log file should be created");

    let content = std::fs::read_to_string(&log_path).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&content).expect("structured log file should be valid JSON");
    let entries = parsed.as_array().expect("top-level should be an array");

    assert!(!entries.is_empty(), "should contain at least one entry");

    // Every entry must have the spec-required fields.
    for entry in entries {
        assert!(entry["source"].is_string(), "entry missing 'source'");
        assert!(entry["level"].is_string(), "entry missing 'level'");
        assert!(entry["timestamp"].is_u64(), "entry missing 'timestamp'");
        assert!(entry["text"].is_string(), "entry missing 'text'");
    }

    // At least one entry should be task output from app-a's build script.
    let has_task_output = entries.iter().any(|e| {
        e["source"].as_str().is_some_and(|s| s.contains("build"))
            && e["text"]
                .as_str()
                .is_some_and(|t| t.contains("build-app-a"))
    });
    assert!(has_task_output, "should capture task stdout");
}

/// Verify `--json` produces NDJSON on stdout.
#[test]
fn json_flag_produces_ndjson() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--force", "--json"]);

    assert!(output.status.success(), "turbo run should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert!(!lines.is_empty(), "should produce output on stdout");

    // Every line must be valid JSON with the spec-required fields.
    for line in &lines {
        let parsed: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|_| panic!("invalid JSON line: {line}"));
        assert!(parsed["source"].is_string());
        assert!(parsed["level"].is_string());
        assert!(parsed["timestamp"].is_u64());
        assert!(parsed["text"].is_string());
    }
}

/// Verify structured log file has no ANSI escape codes in text fields.
#[test]
fn structured_log_strips_ansi() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    let log_path = tempdir.path().join("ansi_test.json");
    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--force",
            &format!("--log-file={}", log_path.display()),
        ],
    );

    assert!(output.status.success());

    let content = std::fs::read_to_string(&log_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    for entry in parsed.as_array().unwrap() {
        let text = entry["text"].as_str().unwrap();
        assert!(
            !text.contains('\x1b'),
            "structured log text should not contain ANSI escape codes: {text:?}"
        );
    }
}

/// Verify path traversal is rejected (path escaping repo root falls back to
/// default).
#[test]
fn log_file_rejects_path_traversal() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    // Attempt to write outside the repo root.
    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--force",
            "--log-file=../../../tmp/evil.json",
        ],
    );

    assert!(output.status.success(), "run should still succeed");

    // The traversal path should NOT have been created.
    assert!(
        !std::path::Path::new("/tmp/evil.json").exists(),
        "path traversal should be rejected"
    );

    // Instead, a log should exist under .turbo/logs/ (the default fallback).
    let turbo_logs = tempdir.path().join(".turbo").join("logs");
    if turbo_logs.exists() {
        let json_files: Vec<_> = std::fs::read_dir(&turbo_logs)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .collect();
        assert!(
            !json_files.is_empty(),
            "should fall back to default .turbo/logs/ location"
        );
    }
}
