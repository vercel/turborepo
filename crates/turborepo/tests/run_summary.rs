mod common;

use std::{fs, path::Path};

use common::{run_turbo, run_turbo_with_env, setup};

fn replace_turbo_json(dir: &Path, config_name: &str) {
    let workspace_root =
        dunce::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    let src = workspace_root
        .join("turborepo-tests/integration/fixtures/turbo-configs")
        .join(config_name);
    fs::copy(&src, dir.join("turbo.json")).unwrap();
    let normalized = fs::read_to_string(dir.join("turbo.json"))
        .unwrap()
        .replace("\r\n", "\n");
    fs::write(dir.join("turbo.json"), normalized).unwrap();
    std::process::Command::new("git")
        .args([
            "commit",
            "-am",
            "replace turbo.json",
            "--quiet",
            "--allow-empty",
        ])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
}

fn read_run_summaries(dir: &Path) -> Vec<serde_json::Value> {
    let runs_dir = dir.join(".turbo/runs");
    if !runs_dir.exists() {
        return vec![];
    }
    let mut files: Vec<_> = fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".json") {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files
        .iter()
        .map(|p| serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap())
        .collect()
}

fn get_task(summary: &serde_json::Value, task_id: &str) -> serde_json::Value {
    summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"].as_str() == Some(task_id))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

// --- discovery.t ---

#[test]
fn test_run_summary_discovery() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--summarize", "--filter=my-app"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Summary:"));
    assert!(stdout.contains(".turbo"));
}

// --- enable.t ---

#[test]
fn test_run_summary_enable_matrix() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    struct Case {
        env_val: Option<&'static str>,
        flag: Option<&'static str>,
        expect_summary: bool,
    }

    let cases = vec![
        // env=true
        Case {
            env_val: Some("true"),
            flag: None,
            expect_summary: true,
        },
        Case {
            env_val: Some("true"),
            flag: Some("--summarize=true"),
            expect_summary: true,
        },
        Case {
            env_val: Some("true"),
            flag: Some("--summarize=false"),
            expect_summary: false,
        },
        Case {
            env_val: Some("true"),
            flag: Some("--summarize"),
            expect_summary: true,
        },
        // env=false
        Case {
            env_val: Some("false"),
            flag: None,
            expect_summary: false,
        },
        Case {
            env_val: Some("false"),
            flag: Some("--summarize=true"),
            expect_summary: true,
        },
        Case {
            env_val: Some("false"),
            flag: Some("--summarize=false"),
            expect_summary: false,
        },
        Case {
            env_val: Some("false"),
            flag: Some("--summarize"),
            expect_summary: true,
        },
        // env missing
        Case {
            env_val: None,
            flag: None,
            expect_summary: false,
        },
        Case {
            env_val: None,
            flag: Some("--summarize=true"),
            expect_summary: true,
        },
        Case {
            env_val: None,
            flag: Some("--summarize=false"),
            expect_summary: false,
        },
        Case {
            env_val: None,
            flag: Some("--summarize"),
            expect_summary: true,
        },
    ];

    for (i, case) in cases.iter().enumerate() {
        let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

        let mut args = vec!["run", "build"];
        if let Some(flag) = case.flag {
            args.push(flag);
        }

        let env: Vec<(&str, &str)> = match case.env_val {
            Some(val) => vec![("TURBO_RUN_SUMMARY", val)],
            None => vec![],
        };

        run_turbo_with_env(tempdir.path(), &args, &env);

        let has_summary = tempdir.path().join(".turbo/runs").exists()
            && fs::read_dir(tempdir.path().join(".turbo/runs"))
                .map(|d| {
                    d.filter_map(|e| e.ok())
                        .any(|e| e.file_name().to_string_lossy().ends_with(".json"))
                })
                .unwrap_or(false);

        assert_eq!(
            has_summary, case.expect_summary,
            "case {i}: env={:?} flag={:?} expected summary={} got={}",
            case.env_val, case.flag, case.expect_summary, has_summary
        );
    }
}

// --- sorting-deps.t ---

#[test]
fn test_run_summary_sorting_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "with-pkg-deps", "npm@10.5.0", true).unwrap();
    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

    // Need a fresh commit for the filter to work
    std::process::Command::new("git")
        .args(["commit", "--quiet", "-am", "new sha", "--allow-empty"])
        .current_dir(tempdir.path())
        .status()
        .unwrap();

    run_turbo(tempdir.path(), &["run", "build", "--summarize"]);

    let summaries = read_run_summaries(tempdir.path());
    assert!(!summaries.is_empty());

    let task = get_task(&summaries[0], "my-app#build");
    let deps: Vec<String> = task["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(deps, vec!["another#build", "util#build"]);
}

// --- single-package.t ---

#[test]
fn test_run_summary_single_package() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    run_turbo(tempdir.path(), &["run", "build", "--summarize"]);

    let summaries = read_run_summaries(tempdir.path());
    assert_eq!(summaries.len(), 1);

    let summary = &summaries[0];
    assert_eq!(summary["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(summary["version"], "1");
    assert_eq!(summary["scm"]["type"], "git");
    assert_eq!(summary["execution"]["exitCode"], 0);
    assert_eq!(summary["execution"]["attempted"], serde_json::json!(1));
    assert_eq!(summary["execution"]["cached"], 0);
    assert_eq!(summary["execution"]["failed"], 0);
    assert_eq!(summary["execution"]["success"], serde_json::json!(1));

    // Check top-level keys
    let mut keys: Vec<String> = summary.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "envMode",
            "execution",
            "frameworkInference",
            "globalCacheInputs",
            "id",
            "monorepo",
            "scm",
            "tasks",
            "turboVersion",
            "user",
            "version"
        ]
    );

    // Task summary keys
    let task = get_task(summary, "build");
    let mut task_keys: Vec<String> = task.as_object().unwrap().keys().cloned().collect();
    task_keys.sort();
    assert_eq!(
        task_keys,
        vec![
            "cache",
            "cliArguments",
            "command",
            "dependencies",
            "dependents",
            "envMode",
            "environmentVariables",
            "excludedOutputs",
            "execution",
            "expandedOutputs",
            "framework",
            "hash",
            "hashOfExternalDependencies",
            "inputs",
            "logFile",
            "outputs",
            "resolvedTaskDefinition",
            "task",
            "taskId",
            "with"
        ]
    );

    assert_eq!(task["cache"]["status"], "MISS");
    assert_eq!(task["cliArguments"], serde_json::json!([]));
}

// --- monorepo.t ---

#[test]
fn test_run_summary_monorepo() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

    // First run (cache miss)
    run_turbo(
        tempdir.path(),
        &["run", "build", "--summarize", "--", "someargs"],
    );

    // Sleep for ksuid ordering
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Second run (cache hit)
    run_turbo(
        tempdir.path(),
        &["run", "build", "--summarize", "--", "someargs"],
    );

    let summaries = read_run_summaries(tempdir.path());
    assert_eq!(summaries.len(), 2);

    let first = &summaries[0];
    let second = &summaries[1];

    // Top-level keys
    let mut keys: Vec<String> = first.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert!(keys.contains(&"execution".to_string()));
    assert!(keys.contains(&"tasks".to_string()));

    assert_eq!(first["scm"]["type"], "git");
    assert_eq!(first["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(first["version"], "1");
    assert_eq!(first["execution"]["exitCode"], 0);
    assert_eq!(first["execution"]["attempted"], 2);
    assert_eq!(first["execution"]["cached"], 0);
    assert_eq!(first["execution"]["success"], 2);

    // Task summaries
    let first_app = get_task(first, "my-app#build");
    let second_app = get_task(second, "my-app#build");

    assert_eq!(first_app["execution"]["exitCode"], 0);
    assert_eq!(first_app["cliArguments"], serde_json::json!(["someargs"]));
    assert_eq!(first_app["hashOfExternalDependencies"], "459c029558afe716");

    // First run: MISS, second run: HIT
    assert_eq!(first_app["cache"]["status"], "MISS");
    assert_eq!(second_app["cache"]["status"], "HIT");
    assert_eq!(second_app["cache"]["local"], true);

    // util#build present
    let first_util = get_task(first, "util#build");
    assert_eq!(first_util["execution"]["exitCode"], 0);

    // another#build not present (no build script)
    let another = get_task(first, "another#build");
    assert!(another.is_null());
}

// --- error.t ---

#[test]
fn test_run_summary_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

    // Run failing task with summarize
    run_turbo(
        tempdir.path(),
        &["run", "maybefails", "--filter=my-app", "--summarize"],
    );

    let summaries = read_run_summaries(tempdir.path());
    assert_eq!(summaries.len(), 1);

    let summary = &summaries[0];
    assert_eq!(summary["execution"]["failed"], serde_json::json!(1));
    assert!(
        [1, 4].contains(&summary["execution"]["exitCode"].as_i64().unwrap()),
        "exitCode should be 1 or 4"
    );
    assert_eq!(summary["execution"]["attempted"], serde_json::json!(1));

    // Task summary for failed task
    let task = get_task(summary, "my-app#maybefails");
    assert!(!task.is_null());
    assert_eq!(task["hash"], "9f05a7188fdf4e93");
    assert_eq!(task["cache"]["status"], "MISS");
    assert!([1, 4].contains(&task["execution"]["exitCode"].as_i64().unwrap()));
    let error = task["execution"]["error"].as_str().unwrap();
    assert!(
        error.contains("maybefails exited"),
        "expected error message, got: {error}"
    );

    // With --continue --force
    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));
    run_turbo(
        tempdir.path(),
        &["run", "maybefails", "--summarize", "--force", "--continue"],
    );

    let summaries = read_run_summaries(tempdir.path());
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary["execution"]["success"], serde_json::json!(1));
    assert_eq!(summary["execution"]["failed"], serde_json::json!(1));
    assert_eq!(summary["execution"]["attempted"], serde_json::json!(2));
    assert_eq!(summary["tasks"].as_array().unwrap().len(), 2);

    let failed_task = get_task(summary, "my-app#maybefails");
    assert!([1, 4].contains(&failed_task["execution"]["exitCode"].as_i64().unwrap()));
}

// --- strict-env.t ---

#[test]
fn test_run_summary_strict_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "strict_env_vars", "npm@10.5.0", false).unwrap();

    let env_vars: &[(&str, &str)] = &[
        ("GLOBAL_VAR_PT", "higlobalpt"),
        ("GLOBAL_VAR_DEP", "higlobaldep"),
        ("LOCAL_VAR_PT", "hilocalpt"),
        ("LOCAL_VAR_DEP", "hilocaldep"),
        ("OTHER_VAR", "hiother"),
        ("SYSTEMROOT", "hisysroot"),
    ];

    struct Case {
        label: &'static str,
        config: Option<&'static str>,
        env_mode: Option<&'static str>,
        expect_env_mode: &'static str,
        expect_task_env_mode: &'static str,
        expect_passthrough_null: bool,
        expect_passthrough_empty: bool,
        expect_passthrough_has_value: bool,
    }

    let cases = vec![
        // Default (infer → strict)
        Case {
            label: "infer",
            config: None,
            env_mode: None,
            expect_env_mode: "strict",
            expect_task_env_mode: "strict",
            expect_passthrough_null: true,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: false,
        },
        // Strict
        Case {
            label: "strict",
            config: None,
            env_mode: Some("strict"),
            expect_env_mode: "strict",
            expect_task_env_mode: "strict",
            expect_passthrough_null: true,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: false,
        },
        // Loose
        Case {
            label: "loose",
            config: None,
            env_mode: Some("loose"),
            expect_env_mode: "loose",
            expect_task_env_mode: "loose",
            expect_passthrough_null: true,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: false,
        },
        // All specified + infer
        Case {
            label: "all+infer",
            config: Some("strict_env_vars/all.json"),
            env_mode: None,
            expect_env_mode: "strict",
            expect_task_env_mode: "strict",
            expect_passthrough_null: false,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: true,
        },
        // All specified + loose
        Case {
            label: "all+loose",
            config: Some("strict_env_vars/all.json"),
            env_mode: Some("loose"),
            expect_env_mode: "loose",
            expect_task_env_mode: "loose",
            expect_passthrough_null: false,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: true,
        },
        // Global pt empty + infer
        Case {
            label: "gpt-empty+infer",
            config: Some("strict_env_vars/global_pt-empty.json"),
            env_mode: None,
            expect_env_mode: "strict",
            expect_task_env_mode: "strict",
            expect_passthrough_null: true,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: false,
        },
        // Global pt value + infer
        Case {
            label: "gpt+infer",
            config: Some("strict_env_vars/global_pt.json"),
            env_mode: None,
            expect_env_mode: "strict",
            expect_task_env_mode: "strict",
            expect_passthrough_null: true,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: false,
        },
        // Global pt empty + loose
        Case {
            label: "gpt-empty+loose",
            config: Some("strict_env_vars/global_pt-empty.json"),
            env_mode: Some("loose"),
            expect_env_mode: "loose",
            expect_task_env_mode: "loose",
            expect_passthrough_null: true,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: false,
        },
        // Global pt value + loose
        Case {
            label: "gpt+loose",
            config: Some("strict_env_vars/global_pt.json"),
            env_mode: Some("loose"),
            expect_env_mode: "loose",
            expect_task_env_mode: "loose",
            expect_passthrough_null: true,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: false,
        },
        // Task pt empty + infer
        Case {
            label: "tpt-empty+infer",
            config: Some("strict_env_vars/task_pt-empty.json"),
            env_mode: None,
            expect_env_mode: "strict",
            expect_task_env_mode: "strict",
            expect_passthrough_null: false,
            expect_passthrough_empty: true,
            expect_passthrough_has_value: false,
        },
        // Task pt value + infer
        Case {
            label: "tpt+infer",
            config: Some("strict_env_vars/task_pt.json"),
            env_mode: None,
            expect_env_mode: "strict",
            expect_task_env_mode: "strict",
            expect_passthrough_null: false,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: true,
        },
        // Task pt empty + loose
        Case {
            label: "tpt-empty+loose",
            config: Some("strict_env_vars/task_pt-empty.json"),
            env_mode: Some("loose"),
            expect_env_mode: "loose",
            expect_task_env_mode: "loose",
            expect_passthrough_null: false,
            expect_passthrough_empty: true,
            expect_passthrough_has_value: false,
        },
        // Task pt value + loose
        Case {
            label: "tpt+loose",
            config: Some("strict_env_vars/task_pt.json"),
            env_mode: Some("loose"),
            expect_env_mode: "loose",
            expect_task_env_mode: "loose",
            expect_passthrough_null: false,
            expect_passthrough_empty: false,
            expect_passthrough_has_value: true,
        },
    ];

    for case in &cases {
        let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

        if let Some(config) = case.config {
            replace_turbo_json(tempdir.path(), config);
        }

        let mut args = vec!["run", "build", "--summarize"];
        let mode_flag;
        if let Some(mode) = case.env_mode {
            mode_flag = format!("--env-mode={mode}");
            args.push(&mode_flag);
        }

        run_turbo_with_env(tempdir.path(), &args, env_vars);

        let summaries = read_run_summaries(tempdir.path());
        assert!(
            !summaries.is_empty(),
            "{}: no summary generated",
            case.label
        );

        let summary = &summaries[0];
        assert_eq!(
            summary["envMode"].as_str().unwrap(),
            case.expect_env_mode,
            "{}: envMode mismatch",
            case.label
        );

        let task = &summary["tasks"][0];
        assert_eq!(
            task["envMode"].as_str().unwrap(),
            case.expect_task_env_mode,
            "{}: task envMode mismatch",
            case.label
        );

        let pt = &task["environmentVariables"]["passthrough"];
        if case.expect_passthrough_null {
            assert!(
                pt.is_null(),
                "{}: passthrough should be null, got {pt}",
                case.label
            );
        } else if case.expect_passthrough_empty {
            assert_eq!(
                pt.as_array().unwrap().len(),
                0,
                "{}: passthrough should be empty array",
                case.label
            );
        } else if case.expect_passthrough_has_value {
            let arr = pt.as_array().unwrap();
            assert!(
                !arr.is_empty(),
                "{}: passthrough should have values",
                case.label
            );
        }
    }
}
