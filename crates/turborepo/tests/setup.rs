//! Standalone command surface: no Node.js, manager, local CLI, tasks, or HTTP.
#![cfg_attr(test, allow(clippy::expect_used))]

use std::{fs, io, net::TcpListener, path::Path, process::Command};

const PENDING: &str = "provisioning is not implemented yet";
const ENABLED: &str = r#"{"futureFlags":{"experimentalSetup":true}}"#;
const DISABLED: &str = "requires root futureFlags.experimentalSetup";

fn write(root: &Path, name: &str, contents: &str) {
    fs::write(root.join(name), contents).expect("write fixture");
}

fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn visit(dir: &Path, files: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).expect("read fixture directory") {
            let path = entry.expect("read fixture entry").path();
            if path.is_dir() {
                files.push((path.clone(), vec![]));
                visit(&path, files);
            } else {
                let contents = fs::read(&path).expect("read fixture file");
                files.push((path, contents));
            }
        }
    }
    let mut files = vec![];
    visit(root, &mut files);
    files.sort();
    files
}

fn invoke(root: &Path, words: &[&str], ci: bool) -> (i32, String) {
    let proxy = TcpListener::bind("127.0.0.1:0").expect("bind HTTP monitor");
    proxy.set_nonblocking(true).expect("nonblocking monitor");
    let url = format!("http://{}", proxy.local_addr().expect("monitor address"));
    let before = snapshot(root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
    command
        .env_clear()
        .current_dir(root)
        .args(words)
        .env("PATH", "")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("TURBO_CONFIG_DIR_PATH", root.join("config"))
        .env("VERCEL_CONFIG_DIR_PATH", root.join("config"))
        .env("TURBO_INVOCATION_DIR", root.join("unrelated-invocation"))
        .env("TURBO_FORCE", "true")
        .env("AI_AGENT", "setup-test")
        .env("HTTP_PROXY", &url)
        .env("HTTPS_PROXY", &url)
        .env("ALL_PROXY", &url)
        .env("NO_PROXY", "");
    if ci {
        // Even this normal shim override must not trigger repository inference.
        command
            .env("CI", "1")
            .env("TURBO_BINARY_PATH", root.join("unbuilt-local-turbo"));
    }
    let output = command.output().expect("run standalone turbo");
    assert_eq!(
        snapshot(root),
        before,
        "setup must not write files or directories"
    );
    assert!(
        matches!(proxy.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock),
        "unexpected HTTP connection"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = format!("{stdout}{stderr}")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let code = output.status.code().expect("standalone turbo exit code");
    (code, text)
}

fn failure(root: &Path, words: &[&str], ci: bool, expected: &str) -> String {
    let (code, text) = invoke(root, words, ci);
    assert_eq!(code, 1, "{words:?}: {text}");
    assert!(text.contains(expected), "{text}");
    text
}

#[test]
fn standalone_help_needs_no_js_or_gate_and_does_not_parse_config() {
    let temp = tempfile::tempdir().unwrap();
    for malformed_config in [false, true] {
        if malformed_config {
            write(temp.path(), "turbo.json", "not json");
        }
        let (code, text) = invoke(temp.path(), &["setup", "--help"], malformed_config);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("Usage: turbo setup"), "{text}");
        for flag in [
            "--plan",
            "--check",
            "--force",
            "--frozen",
            "--no-frozen",
            "--offline",
            "--tools-only",
            "--no-lock",
            "--update-lock",
            "--cwd",
        ] {
            assert!(text.contains(flag), "missing {flag}: {text}");
        }
        assert!(text.contains("not yet implemented"));
    }
}

#[test]
fn standalone_gate_reads_only_the_selected_root_config() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.lock", "must not activate anything");
    for config in [
        None,
        Some("{}"),
        Some(r#"{"futureFlags":{"experimentalSetup":false}}"#),
    ] {
        if let Some(config) = config {
            write(root, "turbo.json", config);
        }
        let text = failure(root, &["setup"], false, DISABLED);
        assert!(text.contains("--cwd"), "{text}");
        assert!(!text.contains("provisioning is not implemented"));
    }
    // jsonc and both existing global root selectors work without JS.
    fs::remove_file(root.join("turbo.json")).unwrap();
    let jsonc = "{ // gate\n\"futureFlags\": {\"experimentalSetup\": true},\n}";
    write(root, "turbo.jsonc", jsonc);
    write(root, "custom.json", ENABLED);
    fs::create_dir(root.join("selected-root")).unwrap();
    write(root, "selected-root/turbo.json", ENABLED);
    for words in [
        vec!["setup", "--offline"],
        vec!["--root-turbo-json", "custom.json", "setup"],
        vec!["--cwd=selected-root", "setup"],
    ] {
        failure(root, &words, false, PENDING);
    }
    let invalid_gate = r#"{"futureFlags":{"experimentalSetup":"true"}}"#;
    write(root, "custom.json", invalid_gate);
    let (code, text) = invoke(root, &["setup", "--root-turbo-json=custom.json"], false);
    assert_eq!(code, 1);
    assert!(!text.contains(PENDING), "{text}");
}

#[test]
fn accepted_setup_modes_are_typed_failures_not_prepared_success() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // A missing local pin/manager and unusable Cargo marker must not bootstrap
    // anything.
    let package = r#"{"devDependencies":{"turbo":"2.0.3"},"scripts":{"setup":"exit 99"}}"#;
    write(root, "package.json", package);
    write(root, "Cargo.toml", "must not run cargo metadata");
    write(
        root,
        "turbo.json",
        r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#,
    );
    for flags in [
        vec![],
        vec!["--plan"],
        vec!["--check"],
        vec!["--force"],
        vec!["--frozen"],
        vec!["--no-frozen"],
        vec!["--offline"],
        vec!["--tools-only"],
        vec!["--no-lock"],
        vec!["--update-lock"],
        vec!["--plan", "--force", "--update-lock"],
        vec![
            "--offline",
            "--check-for-update",
            "--experimental-otel-enabled=true",
            "--api=http://127.0.0.1:9",
        ],
        vec!["--__test-run"],
    ] {
        let words: Vec<_> = ["setup"].into_iter().chain(flags).collect();
        let text = failure(root, &words, false, PENDING);
        assert!(text.contains("no tasks were run"), "{text}");
        assert!(!text.contains("test run successful"), "{text}");
    }
    let text = failure(root, &["setup", "--update-lock"], true, "inferred in CI");
    assert!(text.contains("--no-frozen"), "{text}");
    for flags in [vec!["--update-lock", "--no-frozen"], vec!["--no-lock"]] {
        let words: Vec<_> = ["setup"].into_iter().chain(flags).collect();
        failure(root, &words, true, PENDING);
    }
}

#[test]
fn profiling_and_malformed_setup_tails_do_not_write() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "profile.json", "existing profile");
    for enabled in [false, true] {
        write(root, "turbo.json", if enabled { ENABLED } else { "{}" });
        for words in [
            vec!["setup", "--heap=profile.json"],
            vec!["setup", "--plan", "--heap=profile.json"],
            vec!["setup", "--check", "--heap=profile.json"],
            vec!["setup", "--help", "--heap=profile.json"],
            vec!["setup", "--profile=profile.json"],
            vec!["setup", "--anon-profile=profile.json"],
            vec!["setup", "--plan", "--profile=profile.json"],
            vec!["setup", "--check", "--anon-profile=profile.json"],
            vec!["setup", "--help", "--profile=profile.json"],
            vec!["setup", "--help", "--anon-profile=profile.json"],
            vec!["setup", "--unknown"],
            vec!["setup", "--force", "config"],
            vec!["setup", "--", "echo"],
        ] {
            let (code, text) = invoke(root, &words, false);
            let expected = if words.contains(&"--help") { 0 } else { 1 };
            assert_eq!(code, expected, "{words:?}: {text}");
            if !words.contains(&"--help") && !words.iter().any(|word| word.starts_with("--heap=")) {
                assert!(
                    !text.contains(PENDING) && !text.contains(DISABLED),
                    "{text}"
                );
            }
        }
    }
}
