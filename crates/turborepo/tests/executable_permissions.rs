#![cfg(unix)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use common::{git, run_turbo};
use serde_json::json;

fn fixture(root: &Path, enabled: bool, global: bool, explicit: bool, use_git: bool) {
    fs::write(
        root.join("package.json"),
        json!({
            "name": "permissions-test",
            "private": true,
            "packageManager": "npm@10.9.4",
            "workspaces": ["packages/*"]
        })
        .to_string(),
    )
    .unwrap();
    fs::write(root.join(".gitignore"), ".turbo\nnode_modules\ndist\n").unwrap();
    let mut config = json!({
        "agentGuidance": false,
        "futureFlags": { "hashFilePermissions": enabled },
        "tasks": { "build": { "outputs": ["dist/**"] } }
    });
    if global {
        config["globalDependencies"] = json!(["tool.sh"]);
    }
    if explicit {
        config["tasks"]["build"]["inputs"] = json!(["src/**"]);
    }
    fs::write(root.join("turbo.json"), config.to_string()).unwrap();

    for name in if global { &['a', 'b'][..] } else { &['a'][..] } {
        let package = root.join(format!("packages/{name}"));
        fs::create_dir_all(package.join("src")).unwrap();
        let input = if global {
            "../../tool.sh"
        } else {
            "src/tool.sh"
        };
        let command = format!(
            "mkdir -p dist && (test -x {input} && echo executable || echo not-executable) > \
             dist/mode.txt"
        );
        fs::write(
            package.join("package.json"),
            json!({ "name": name.to_string(), "scripts": { "build": command } }).to_string(),
        )
        .unwrap();
    }
    let input = root.join(if global {
        "tool.sh"
    } else {
        "packages/a/src/tool.sh"
    });
    fs::write(&input, "#!/bin/sh\necho hello\n").unwrap();
    fs::set_permissions(&input, fs::Permissions::from_mode(0o644)).unwrap();
    if use_git {
        git(root, &["init", "--quiet"]);
        git(root, &["config", "user.email", "turbo-test@example.com"]);
        git(root, &["config", "user.name", "Turborepo Test"]);
        git(root, &["add", "."]);
        git(
            root,
            &["-c", "commit.gpgsign=false", "commit", "-qm", "initial"],
        );
    }
}

fn assert_build(root: &Path, global: bool, expected: &str, cache_hit: bool) {
    let packages = if global { &['a', 'b'][..] } else { &['a'][..] };
    for name in packages {
        let output = root.join(format!("packages/{name}/dist"));
        if output.exists() {
            fs::remove_dir_all(output).unwrap();
        }
    }
    let result = run_turbo(root, &["run", "build", "--output-logs=full"]);
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        result.status.success(),
        "build failed\n{stdout}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for name in packages {
        let cache = if cache_hit { "cache hit" } else { "cache miss" };
        assert!(
            stdout.contains(&format!("{name}:build: {cache}")),
            "{stdout}"
        );
        assert_eq!(
            fs::read_to_string(root.join(format!("packages/{name}/dist/mode.txt")))
                .unwrap()
                .trim(),
            expected,
            "package {name}, output:\n{stdout}"
        );
    }
}

#[test]
fn executable_permissions_control_cache_replay() {
    for enabled in [false, true] {
        for (global, explicit, use_git) in [
            (false, false, true),
            (false, true, true),
            (false, true, false),
            (true, true, true),
            (true, true, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            fixture(root, enabled, global, explicit, use_git);
            let input = root.join(if global {
                "tool.sh"
            } else {
                "packages/a/src/tool.sh"
            });
            assert_build(root, global, "not-executable", false);
            assert_build(root, global, "not-executable", true);

            fs::set_permissions(&input, fs::Permissions::from_mode(0o755)).unwrap();
            let expected = if enabled {
                "executable"
            } else {
                "not-executable"
            };
            assert_build(root, global, expected, !enabled);
            if use_git {
                git(root, &["add", "."]);
                git(
                    root,
                    &["-c", "commit.gpgsign=false", "commit", "-qm", "executable"],
                );
            }
            assert_build(root, global, expected, true);

            fs::set_permissions(&input, fs::Permissions::from_mode(0o644)).unwrap();
            assert_build(root, global, "not-executable", true);
        }
    }
}
