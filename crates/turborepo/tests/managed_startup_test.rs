//! Real native startup, not synthetic capability peers or direct RunBuilder
//! calls.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use std::{fs, io, net::TcpListener, path::Path, process::Output, time::Duration};

use turborepo_shim::{
    TurboState,
    capabilities::{Capabilities, QUERY_FLAG},
};

fn binary() -> std::path::PathBuf {
    assert_cmd::cargo::cargo_bin("turbo")
}

fn local(root: &Path, copy: bool) -> std::path::PathBuf {
    let package = root
        .join("node_modules")
        .join(TurboState::scoped_platform_package_scope())
        .join(TurboState::scoped_platform_package_dir());
    let executable = package.join("bin").join(TurboState::binary_name());
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(
        package.join("package.json"),
        serde_json::json!({
            "version": include_str!("../../../version.txt").lines().next().unwrap()
        })
        .to_string(),
    )
    .unwrap();
    if copy {
        fs::copy(binary(), &executable).unwrap();
    } else {
        std::os::unix::fs::symlink(binary(), &executable).unwrap();
    }
    executable
}

fn tree(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn visit(root: &Path, entries: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_symlink() {
                entries.push((
                    path.clone(),
                    fs::read_link(path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                ));
            } else if path.is_dir() {
                entries.push((path.clone(), vec![]));
                visit(&path, entries);
            } else {
                // Avoid reading large CLI copies, but retain file-change detection.
                let bytes = if path.file_name().unwrap() == TurboState::binary_name() {
                    let metadata = fs::metadata(&path).unwrap();
                    format!(
                        "{}:{:?}:{:?}",
                        metadata.len(),
                        metadata.modified().unwrap(),
                        metadata.permissions()
                    )
                    .into_bytes()
                } else {
                    fs::read(&path).unwrap()
                };
                entries.push((path, bytes));
            }
        }
    }
    let mut entries = vec![];
    visit(root, &mut entries);
    entries.sort();
    entries
}

fn invoke(executable: &Path, root: &Path, cwd: &Path, args: &[&str], read_only: bool) -> Output {
    let monitor = TcpListener::bind("127.0.0.1:0").unwrap();
    monitor.set_nonblocking(true).unwrap();
    let proxy = format!("http://{}", monitor.local_addr().unwrap());
    let before = tree(root);
    let output = assert_cmd::Command::new(executable)
        .timeout(Duration::from_secs(15))
        .env_clear()
        .current_dir(cwd)
        .args(args)
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("TURBO_CONFIG_DIR_PATH", root.join("config"))
        .env("DO_NOT_TRACK", "1")
        .env("TURBO_NO_UPDATE_NOTIFIER", "1")
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "1")
        .env("HTTP_PROXY", &proxy)
        .env("HTTPS_PROXY", &proxy)
        .env("ALL_PROXY", &proxy)
        .env("NO_PROXY", "")
        .output()
        .unwrap();
    if read_only {
        assert_eq!(
            tree(root),
            before,
            "info exit changed repository/user state"
        );
    }
    assert!(
        matches!(monitor.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock),
        "startup contacted HTTP: {output:?}"
    );
    output
}

fn repository(root: &Path, enabled: bool) {
    fs::write(root.join("package.json"), r#"{"name":"fixture","private":true,"packageManager":"pnpm@10.0.0","scripts":{"build":"exit 97"}}"#).unwrap();
    fs::write(
        root.join("turbo.json"),
        serde_json::json!({
            "futureFlags": {"experimentalSetup": enabled},
            "noUpdateNotifier": true,
            "agentGuidance": false,
            "tasks": {"build": {}}
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        root.join("turbo.lock"),
        "invalid; must not authorize for info exits",
    )
    .unwrap();
}

#[test]
fn help_and_version_preserve_normal_info_exits_without_setup_with_flag_on_or_off() {
    for enabled in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        repository(temp.path(), enabled);
        for args in [
            vec!["--help"],
            vec!["run", "--help"],
            vec!["run", "build", "-h"],
            vec!["--version"],
            vec!["run", "build", "--version"],
        ] {
            let output = invoke(&binary(), temp.path(), temp.path(), &args, true);
            assert!(output.status.success(), "{enabled} {args:?}: {output:?}");
            assert!(!output.stdout.is_empty());
            assert!(!String::from_utf8_lossy(&output.stderr).contains("managed"));
        }
    }
}

#[test]
fn installed_identical_copy_and_self_local_info_exits_do_not_require_trusted_evidence() {
    for copy in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        repository(temp.path(), true);
        let installed = local(temp.path(), copy);
        for executable in [&binary(), &installed] {
            for args in [vec!["run", "--help"], vec!["run", "build", "--version"]] {
                let output = invoke(executable, temp.path(), temp.path(), &args, true);
                assert!(output.status.success(), "copy={copy} {args:?}: {output:?}");
                assert!(!String::from_utf8_lossy(&output.stderr).contains("managed"));
            }
        }
    }
}

#[test]
fn actual_current_native_report_matches_installed_copy_and_self_local_reports() {
    let temp = tempfile::tempdir().unwrap();
    repository(temp.path(), true);
    let version = include_str!("../../../version.txt").lines().next().unwrap();
    let expected = Capabilities::current(version).encode().unwrap();
    for copy in [false, true] {
        let case = temp.path().join(if copy { "copy" } else { "self" });
        fs::create_dir(&case).unwrap();
        let installed = local(&case, copy);
        for executable in [&binary(), &installed] {
            let output = invoke(executable, temp.path(), &case, &[QUERY_FLAG], true);
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            assert_eq!(output.stdout, expected);
        }
    }
}

#[test]
fn local_handoff_forwards_resolved_root_jsonc_policy_and_preserves_task_arg_boundary() {
    for equals in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        repository(root, true); // Wrong selection would try authorizing invalid turbo.lock.
        fs::create_dir_all(root.join("apps/web")).unwrap();
        fs::write(
            root.join("apps/web/package.json"),
            r#"{"name":"web","scripts":{"build":"exit 97"}}"#,
        )
        .unwrap();
        fs::write(root.join("pnpm-workspace.yaml"), "packages:\n  - apps/*\n").unwrap();
        fs::write(
            root.join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n",
        )
        .unwrap();
        fs::write(
            root.join("turbo.jsonc"),
            r#"{
            // This is the explicitly selected, unconfigured policy.
            "futureFlags": {"experimentalSetup": false},
            "noUpdateNotifier": true, "agentGuidance": false,
            "tasks": {"build": {"outputs": ["selected-jsonc/**"]}}
        }"#,
        )
        .unwrap();
        local(root, true);
        let nested = root.join("apps/web");
        let mut args = if equals {
            vec!["--root-turbo-json=../../turbo.jsonc"]
        } else {
            vec!["--root-turbo-json", "../../turbo.jsonc"]
        };
        args.extend([
            "run",
            "build",
            "--dry=json",
            "--",
            "--root-turbo-json=task-value",
        ]);
        let output = invoke(&binary(), root, &nested, &args, false);
        assert!(output.status.success(), "{args:?}: {output:?}");
        let dry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            dry["tasks"][0]["resolvedTaskDefinition"]["outputs"],
            serde_json::json!(["selected-jsonc/**"]),
            "{dry}"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("--root-turbo-json=task-value"));
        assert!(!root.join("task-ran").exists());
    }
}
