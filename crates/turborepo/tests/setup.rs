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
        // Even this normal shim override must not trigger JS inference or handoff.
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

fn assert_diagnostic_path(text: &str, path: &Path) {
    // Miette wraps long paths and adds `|` gutters on continuation lines.
    let rendered = text.replace([' ', '|'], "");
    let expected = path
        .canonicalize()
        .unwrap()
        .display()
        .to_string()
        .replace(' ', "");
    assert!(rendered.contains(&expected), "{text}");
}

fn inferred(root: &Path, words: &[&str], expected: &Path) {
    let words: Vec<_> = words.iter().copied().chain(["--verbosity=2"]).collect();
    let text = failure(root, &words, true, PENDING);
    let expected = expected.canonicalize().unwrap();
    assert!(
        text.contains(&format!("setup root: {}", expected.display())),
        "{text}"
    );
}

#[test]
fn nested_js_workspaces_need_no_manager_declaration_lock_or_executable() {
    for (package, pnpm) in [
        (r#"{"workspaces":["packages/*"]}"#, false),
        (r#"{"workspaces":{"packages":["packages/*"]}}"#, false),
        ("{}", true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "turbo.json", ENABLED);
        write(root, "package.json", package);
        if pnpm {
            write(root, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
        }
        fs::create_dir_all(root.join("packages/app/src")).unwrap();
        write(root, "packages/app/package.json", r#"{"name":"app"}"#);
        write(root, "packages/app/turbo.json", r#"{"extends":["//"]}"#);
        inferred(root, &["setup", "--cwd=packages/app/src"], root);
        inferred(&root.join("packages/app/src"), &["setup"], root);
        inferred(
            root,
            &[
                "setup",
                "--cwd=packages/app/src",
                "--root-turbo-json=turbo.json",
            ],
            root,
        );
    }
}

#[test]
fn nested_non_js_roots_need_no_package_json_or_language_tools() {
    for (flag, manifest, contents, member, member_contents) in [
        (
            "experimentalCargoWorkspaces",
            "Cargo.toml",
            "[workspace]\nmembers = ['packages/app']\n",
            "Cargo.toml",
            "[package]\nname = 'app'\nversion = '0.1.0'\n",
        ),
        (
            "experimentalPythonWorkspaces",
            "pyproject.toml",
            "[tool.uv.workspace]\nmembers = ['packages/*']\n",
            "pyproject.toml",
            "[project]\nname = 'app'\nversion = '0.1.0'\n",
        ),
        (
            "experimentalGoWorkspaces",
            "go.work",
            "go 1.22\nuse ./packages/app\n",
            "go.mod",
            "module example.com/app\ngo 1.22\n",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            root,
            "turbo.jsonc",
            &format!(r#"{{"futureFlags":{{"experimentalSetup":true,"{flag}":true}}}}"#),
        );
        write(root, manifest, contents);
        fs::create_dir_all(root.join("packages/app/src")).unwrap();
        write(root, &format!("packages/app/{member}"), member_contents);
        inferred(root, &["--cwd", "packages/app/src", "setup"], root);
        inferred(&root.join("packages/app/src"), &["setup"], root);
    }
}

#[test]
fn ambiguous_configs_require_an_exact_explicit_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    fs::create_dir_all(root.join("inner/src")).unwrap();
    write(root, "inner/turbo.jsonc", ENABLED);
    write(
        root,
        "inner/.turbo-tools-placeholder",
        "must not borrow these",
    );
    let text = failure(
        root,
        &["setup", "--cwd=inner/src"],
        false,
        "ambiguous setup roots",
    );
    assert!(
        text.contains("--cwd=<root>") && text.contains(".turbo/tools"),
        "{text}"
    );
    assert!(text.contains("inner"), "{text}");
    let text = failure(
        root,
        &["setup", "--cwd=inner/src", "--root-turbo-json=turbo.json"],
        false,
        "nested setup root marker",
    );
    assert_diagnostic_path(&text, &root.join("inner/turbo.jsonc"));
    inferred(root, &["setup", "--cwd=inner"], &root.join("inner"));
    inferred(root, &["setup", "--cwd=."], root);
}

#[test]
fn nested_native_workspace_markers_follow_root_flags() {
    for (flag, marker, contents) in [
        (
            "experimentalCargoWorkspaces",
            "Cargo.toml",
            "[workspace]\nmembers = []\n",
        ),
        (
            "experimentalPythonWorkspaces",
            "pyproject.toml",
            "[tool.uv.workspace]\nmembers = []\n",
        ),
        ("experimentalGoWorkspaces", "go.work", "go 1.22\nuse .\n"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("inner/src")).unwrap();
        write(root, &format!("inner/{marker}"), contents);
        for enabled in [false, true] {
            write(
                root,
                "turbo.json",
                &format!(r#"{{"futureFlags":{{"experimentalSetup":true,"{flag}":{enabled}}}}}"#),
            );
            if enabled {
                let text = failure(
                    root,
                    &["setup", "--cwd=inner/src"],
                    false,
                    "nested setup root marker",
                );
                assert!(
                    text.contains(marker) && text.contains("--cwd=<root>"),
                    "{text}"
                );
            } else {
                inferred(root, &["setup", "--cwd=inner/src"], root);
                // Disabled ecosystems aren't even parsed, including tool declarations.
                write(root, &format!("inner/{marker}"), "not a valid declaration");
                write(root, "inner/rust-toolchain.toml", "also invalid");
                inferred(root, &["setup", "--cwd=inner/src"], root);
                write(root, &format!("inner/{marker}"), contents);
            }
        }
    }
}

#[test]
fn excluded_js_packages_and_secondary_workspaces_are_not_neighbors() {
    for marker in ["package.json", "pnpm-workspace.yaml"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "turbo.json", ENABLED);
        write(
            root,
            "package.json",
            r#"{"workspaces":["packages/*","!packages/inner"]}"#,
        );
        fs::create_dir_all(root.join("packages/inner/src")).unwrap();
        write(
            root,
            &format!("packages/inner/{marker}"),
            if marker == "package.json" {
                "{}"
            } else {
                "packages: ['*']"
            },
        );
        let text = failure(
            root,
            &["setup", "--cwd=packages/inner/src"],
            false,
            "nested setup root marker",
        );
        assert!(text.contains(marker), "{text}");
        inferred(root, &["setup", "--cwd=."], root);
    }
}

#[test]
fn git_boundaries_and_explicit_configs_do_not_borrow_outer_tools() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    fs::create_dir_all(root.join("inner/src")).unwrap();
    // Worktree .git files are boundaries just like clone .git directories.
    write(root, "inner/.git", "gitdir: unrelated");
    failure(root, &["setup", "--cwd=inner/src"], false, DISABLED);
    failure(
        root,
        &["setup", "--cwd=inner/src", "--root-turbo-json=turbo.json"],
        false,
        "nested setup root marker",
    );
    write(root, "inner/custom.json", ENABLED);
    inferred(
        root,
        &[
            "setup",
            "--cwd=inner/src",
            "--root-turbo-json=inner/custom.json",
        ],
        &root.join("inner"),
    );
    failure(
        root,
        &["setup", "--root-turbo-json=inner/custom.json"],
        false,
        "outside the selected root configuration",
    );
}

#[test]
fn malformed_or_duplicate_root_configs_are_not_silently_skipped() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    write(root, "turbo.json", "not json");
    let (code, text) = invoke(root, &["setup", "--cwd=src"], false);
    assert_eq!(code, 1);
    assert!(
        !text.contains(DISABLED) && !text.contains(PENDING),
        "{text}"
    );
    assert_diagnostic_path(&text, &root.join("turbo.json"));
    write(root, "turbo.json", ENABLED);
    write(root, "turbo.jsonc", ENABLED);
    let (code, text) = invoke(root, &["setup", "--cwd=src"], false);
    assert_eq!(code, 1);
    assert!(
        text.contains("turbo.json") && text.contains("turbo.jsonc"),
        "{text}"
    );
    assert!(!text.contains(PENDING), "{text}");
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
