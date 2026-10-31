#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path, process::Command};

use common::{git, run_turbo, run_turbo_with_env};
use serde_json::{Value, json};

fn lockfile(version: &str, later: bool) -> String {
    let specifier = if later { "^2.0.0" } else { "^1.0.0" };
    let b = if later {
        "    dependencies:\n      bar:\n        specifier: 1.0.0\n        version: 1.0.0\n"
    } else {
        "    {}\n"
    };
    let extra = if later {
        "  bar@1.0.0:\n    resolution: {integrity: sha512-bar}\n  baz@1.0.0:\n    resolution: \
         {integrity: sha512-baz}\n"
    } else {
        ""
    };
    let snapshot = if later {
        "  bar@1.0.0:\n    dependencies:\n      baz: 1.0.0\n  baz@1.0.0: {}\n"
    } else {
        ""
    };
    format!(
        "lockfileVersion: '9.0'\nimporters:\n  .: {{}}\n  packages/a:\n    dependencies:\n      \
         foo:\n        specifier: '{specifier}'\n        version: {version}\n  packages/b:\n{b}  \
         packages/app:\n    dependencies:\n      a:\n        specifier: workspace:*\n        \
         version: link:../a\npackages:\n  foo@{version}:\n    resolution: {{integrity: \
         sha512-foo}}\n{extra}snapshots:\n  foo@{version}: {{}}\n{snapshot}"
    )
}

fn revision(root: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn query_names(root: &Path, base: &str, head: Option<&str>) -> Vec<String> {
    let mut args = vec!["query", "affected", "--packages", "--base", base];
    if let Some(head) = head {
        args.extend(["--head", head]);
    }
    let output = run_turbo(root, &args);
    assert!(
        output.status.success(),
        "{}",
        common::combined_output(&output)
    );
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    data["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn explicit_head_lockfile_comparison_is_independent_of_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["a", "b", "app"] {
        fs::create_dir_all(root.join("packages").join(name)).unwrap();
    }
    fs::write(
        root.join("package.json"),
        json!({"name":"root","private":true,"packageManager":"pnpm@11.22.0"}).to_string(),
    )
    .unwrap();
    fs::write(
        root.join("pnpm-workspace.yaml"),
        "packages:\n  - packages/*\n",
    )
    .unwrap();
    fs::write(
        root.join("turbo.json"),
        json!({"agentGuidance":false,"tasks":{"build":{"dependsOn":["^build"]}}}).to_string(),
    )
    .unwrap();
    for (name, deps) in [
        ("a", json!({"foo":"^1.0.0"})),
        ("b", json!({})),
        ("app", json!({"a":"workspace:*"})),
    ] {
        fs::write(root.join("packages").join(name).join("package.json"), json!({"name":name,"version":"1.0.0","dependencies":deps,"scripts":{"build":"echo build"}}).to_string()).unwrap();
    }
    fs::write(root.join("pnpm-lock.yaml"), lockfile("1.0.0", false)).unwrap();
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    git(root, &["config", "user.email", "turbo-test@example.com"]);
    git(root, &["config", "user.name", "Turbo Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "base"]);
    let base = revision(root);
    fs::write(root.join("pnpm-lock.yaml"), lockfile("1.1.0", false)).unwrap();
    git(root, &["add", "."]);
    git(
        root,
        &[
            "commit",
            "--quiet",
            "-m",
            "A: update a transitive resolution",
        ],
    );
    let head = revision(root);
    for (name, deps) in [
        ("a", json!({"foo":"^2.0.0"})),
        ("b", json!({"bar":"1.0.0"})),
    ] {
        fs::write(root.join("packages").join(name).join("package.json"), json!({"name":name,"version":"1.0.0","dependencies":deps,"scripts":{"build":"echo build"}}).to_string()).unwrap();
    }
    fs::write(root.join("pnpm-lock.yaml"), lockfile("2.0.0", true)).unwrap();
    git(root, &["add", "."]);
    git(
        root,
        &[
            "commit",
            "--quiet",
            "-m",
            "B: change later package declarations",
        ],
    );
    let later = revision(root);
    assert!(query_names(root, &base, None).contains(&"b".to_string()));
    for checkout in [&later, &head] {
        git(root, &["checkout", "--quiet", "--detach", checkout]);
        assert_eq!(query_names(root, &base, Some(&head)), ["//", "a", "app"]);
        let output = run_turbo_with_env(
            root,
            &["run", "build", "--affected", "--dry=json"],
            &[("TURBO_SCM_BASE", &base), ("TURBO_SCM_HEAD", &head)],
        );
        assert!(
            output.status.success(),
            "{}",
            common::combined_output(&output)
        );
        let data: Value = serde_json::from_slice(&output.stdout).unwrap();
        let tasks: Vec<_> = data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["taskId"].as_str().unwrap())
            .collect();
        assert_eq!(tasks, ["a#build", "app#build"]);
    }
    // A newer local lockfile must not introduce B-only dependencies at A.
    fs::write(root.join("pnpm-lock.yaml"), lockfile("2.0.0", true)).unwrap();
    assert_eq!(query_names(root, &base, Some(&head)), ["//", "a", "app"]);
    // Machine-local settings unrelated to resolution must not force all packages.
    fs::write(root.join(".npmrc"), "script-shell=bash\n").unwrap();
    assert_eq!(query_names(root, &base, Some(&head)), ["//", "a", "app"]);
    fs::remove_file(root.join(".npmrc")).unwrap();

    fs::write(root.join("pnpm-lock.yaml"), lockfile("1.1.0", false)).unwrap();
    git(root, &["checkout", "--quiet", "--detach", &later]);
    let dirty = lockfile("2.0.0", true)
        .replace("baz@1.0.0", "baz@1.0.1")
        .replace("baz: 1.0.0", "baz: 1.0.1");
    for staged in [false, true] {
        fs::write(root.join("pnpm-lock.yaml"), &dirty).unwrap();
        if staged {
            git(root, &["add", "pnpm-lock.yaml"]);
        }
        assert_eq!(query_names(root, &later, Some(&later)), ["//", "b"]);
        // An explicit ref-to-ref filter ignores local edits, including staged edits.
        let filter = format!("--filter=[{later}...{later}]");
        let committed = run_turbo(root, &["run", "build", &filter, "--dry=json"]);
        assert!(
            committed.status.success(),
            "{}",
            common::combined_output(&committed)
        );
        let data: Value = serde_json::from_slice(&committed.stdout).unwrap();
        assert!(data["tasks"].as_array().unwrap().is_empty());
        for (start, end, expected) in [
            (&later, &later, vec!["b#build"]),
            (&base, &head, vec!["a#build", "app#build", "b#build"]),
        ] {
            let output = run_turbo_with_env(
                root,
                &["run", "build", "--affected", "--dry=json"],
                &[("TURBO_SCM_BASE", start), ("TURBO_SCM_HEAD", end)],
            );
            assert!(
                output.status.success(),
                "staged={staged}: {}",
                common::combined_output(&output)
            );
            let data: Value = serde_json::from_slice(&output.stdout).unwrap();
            let tasks: Vec<_> = data["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|task| task["taskId"].as_str().unwrap())
                .collect();
            assert_eq!(tasks, expected, "staged={staged}, base={start}, head={end}");
        }
        git(
            root,
            &["restore", "--staged", "--worktree", "pnpm-lock.yaml"],
        );
    }
}
