//! Standalone command surface: no Node.js, manager, local CLI, tasks, or HTTP.
#![cfg_attr(test, allow(clippy::expect_used))]

use std::{fs, io, net::TcpListener, path::Path, process::Command};

const PENDING: &str = "this setup mode is not implemented";
const ENABLED: &str = r#"{"futureFlags":{"experimentalSetup":true}}"#;
const DISABLED: &str = "requires root futureFlags.experimentalSetup";

fn write(root: &Path, name: &str, contents: &str) {
    fs::write(root.join(name), contents).expect("write fixture");
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Entry(
    std::path::PathBuf,
    u32,
    std::time::SystemTime,
    Vec<u8>,
    Option<std::path::PathBuf>,
);

fn snapshot(root: &Path) -> Vec<Entry> {
    fn visit(path: &Path, files: &mut Vec<Entry>) {
        let metadata = fs::symlink_metadata(path).expect("fixture metadata");
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::MetadataExt;
            metadata.mode()
        };
        #[cfg(not(unix))]
        let mode = u32::from(metadata.permissions().readonly());
        files.push(Entry(
            path.to_owned(),
            mode,
            metadata.modified().expect("fixture modification time"),
            if metadata.is_file() {
                fs::read(path).expect("fixture file bytes")
            } else {
                vec![]
            },
            if metadata.file_type().is_symlink() {
                Some(fs::read_link(path).expect("fixture link target"))
            } else {
                None
            },
        ));
        if metadata.is_dir() {
            for entry in fs::read_dir(path).expect("read fixture directory") {
                visit(&entry.expect("read fixture entry").path(), files);
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
    assert!(diagnostic_contains(&text, expected), "{text}");
    text
}

fn external_system_policy_blocked(code: i32, text: &str) -> bool {
    let blocked = diagnostic_contains(text, "official-only setup rejects System configuration")
        || diagnostic_contains(text, "cannot safely inspect System configuration");
    if blocked {
        assert_eq!(code, 1);
        assert!(!text.contains(": ready"), "{text}");
        eprintln!(
            "Native readiness qualification blocked by external System npm configuration; policy \
             was not bypassed."
        );
    }
    blocked
}

fn check_failure(root: &Path, words: &[&str], ci: bool, expected: &str) {
    let (code, text) = invoke(root, words, ci);
    assert_eq!(code, 1, "{text}");
    assert!(
        diagnostic_contains(&text, expected) || external_system_policy_blocked(code, &text),
        "{text}"
    );
}

fn diagnostic_contains(text: &str, expected: &str) -> bool {
    // Miette may wrap sentences and filenames across lines with `|` gutters.
    text.replace([' ', '|'], "")
        .contains(&expected.replace(' ', ""))
}

#[test]
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
fn frozen_and_local_unsupported_plans_are_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    write(root, ".nvmrc", "24.0.0");
    write(
        root,
        "turbo.lock",
        r#"{"schemaVersion":0,"tools":{"node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],"installation":{"kind":"verify-system","executables":["node"]}}}}"#,
    );
    failure(
        root,
        &["setup", "--frozen", "--tools-only"],
        false,
        "invalid locked Node artifact or mappings",
    );
    failure(
        root,
        &["setup", "--no-frozen", "--tools-only"],
        true,
        "invalid locked Node artifact or mappings",
    );
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
        assert!(!diagnostic_contains(&text, PENDING), "{text}");
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
        vec!["--tools-only", "--update-lock", "--no-frozen"],
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
        let supported = words.contains(&"--tools-only");
        let text = failure(
            root,
            &words,
            false,
            if supported {
                "non-JavaScript workspace setup"
            } else {
                PENDING
            },
        );
        if !supported {
            assert!(text.contains("no tasks were run"), "{text}");
        }
        assert!(!text.contains("test run successful"), "{text}");
    }
    failure(
        root,
        &["setup", "--check", "--tools-only"],
        false,
        "non-JavaScript workspace setup",
    );
    let text = failure(root, &["setup", "--update-lock"], true, "inferred in CI");
    assert!(text.contains("--no-frozen"), "{text}");
    for flags in [vec!["--update-lock", "--no-frozen"], vec!["--no-lock"]] {
        let words: Vec<_> = ["setup"].into_iter().chain(flags).collect();
        failure(root, &words, true, PENDING);
    }
}

fn canonical_path(path: &Path) -> turbopath::AbsoluteSystemPathBuf {
    turbopath::AbsoluteSystemPathBuf::try_from(path)
        .expect("absolute fixture path")
        .to_realpath()
        .expect("canonical fixture path")
}

fn assert_diagnostic_path(text: &str, path: &Path) {
    // Miette wraps long paths and adds `|` gutters on continuation lines.
    let rendered = text.replace([' ', '|'], "");
    let expected = canonical_path(path).as_str().replace(' ', "");
    assert!(rendered.contains(&expected), "{text}");
}

#[test]
fn diagnostic_assertions_preserve_wrapped_sentences_and_filenames() {
    assert!(diagnostic_contains(
        "outside the selected root | configuration /tmp/repo/custom.json",
        "outside the selected root configuration",
    ));
    assert!(diagnostic_contains(
        "/tmp/repo/pnpm- | workspace.yaml",
        "pnpm-workspace.yaml"
    ));
    assert!(!diagnostic_contains(
        "/tmp/repo/package.json",
        "pnpm-workspace.yaml"
    ));
}

fn inferred(root: &Path, words: &[&str], expected: &Path) {
    let words: Vec<_> = words.iter().copied().chain(["--verbosity=2"]).collect();
    let text = failure(root, &words, true, PENDING);
    let expected = canonical_path(expected);
    assert!(text.contains(&format!("setup root: {expected}")), "{text}");
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
        assert_diagnostic_path(&text, &root.join(format!("packages/inner/{marker}")));
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
fn standalone_check_is_readonly_in_ci_offline_and_unsupported_hosts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    let qualified = cfg!(all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        any(
            target_os = "macos",
            all(target_os = "linux", target_env = "gnu")
        )
    ));
    for ci in [false, true] {
        for extra in [
            None,
            Some("--offline"),
            Some("--frozen"),
            Some("--no-frozen"),
        ] {
            let mut words = vec!["setup", "--check", "--tools-only"];
            words.extend(extra);
            check_failure(
                root,
                &words,
                ci,
                if qualified {
                    "managed activation requires turbo.lock"
                } else {
                    "managed promotion requires macOS or GNU Linux"
                },
            );
        }
        for conflict in ["--force", "--update-lock", "--no-lock", "--plan"] {
            let (code, text) = invoke(root, &["setup", "--check", "--tools-only", conflict], ci);
            assert_eq!(code, 1, "{text}");
            assert!(!text.contains("ready"), "{text}");
        }
    }
}

#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#[test]
fn standalone_check_reports_only_node_pnpm_readiness_and_rejects_damage() {
    use std::os::unix::fs::PermissionsExt;

    use turborepo_setup::{
        activation::ActivationPlan,
        execution_identity::{ExecutionContext, Libc},
        lock::{Lock, Platform, Snapshot},
        node_provision::NodePlan,
        pnpm_provision::PnpmPlan,
        test_support::OwnedSetupFixture,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    write(root, ".nvmrc", "24.0.0");
    write(root, ".gitignore", "/.turbo/\n");
    write(
        root,
        "package.json",
        r#"{"name":"fixture","packageManager":"pnpm@10.0.0","workspaces":["apps/*"]}"#,
    );
    write(root, "pnpm-workspace.yaml", "packages: ['apps/*']\n");
    write(
        root,
        "pnpm-lock.yaml",
        "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n",
    );
    fs::create_dir_all(root.join("apps/web/src")).unwrap();
    write(root, "apps/web/package.json", r#"{"name":"web"}"#);
    let (platform, selector, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => (Platform::MacosArm64, "macos-arm64", "darwin-arm64"),
        ("macos", _) => (Platform::MacosX64, "macos-x64", "darwin-x64"),
        ("linux", "aarch64") => (Platform::LinuxArm64Gnu, "linux-arm64-gnu", "linux-arm64"),
        _ => (Platform::LinuxX64Gnu, "linux-x64-gnu", "linux-x64"),
    };
    let prefix = format!("node-v24.0.0-{spelling}");
    let lock = serde_json::json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{selector:{"distribution":{
                "url":format!("https://nodejs.org/dist/v24.0.0/{prefix}.tar.gz"),"sha256":"0".repeat(64),
                "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}}}}}},
        "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":"1".repeat(64),
                "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}}}
    }}).to_string();
    write(root, "turbo.lock", &lock);
    let lock = Lock::parse(lock.as_bytes()).unwrap();
    let node = NodePlan::from_lock(&lock, platform).unwrap();
    let declaration = turborepo_setup::lock::Snapshot::capture(root)
        .unwrap()
        .package_manager()
        .unwrap()
        .unwrap();
    let pnpm = PnpmPlan::from_declaration(&lock, platform, &node, &declaration).unwrap();
    let tools = [node.inventory_tool().clone(), pnpm.inventory_tool().clone()];
    // Production has no loopback override. Seed through the real Store/adapter
    // contract; the CLI library fixtures separately exercise actual downloads.
    let stage = |tool: &turborepo_tool_install::Tool, destination: &Path| {
        for relative in tool.executables.values() {
            let file = destination.join(relative);
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(&file, "#!/bin/sh\ntouch probe-ran\nexit 99\n")?;
            fs::set_permissions(file, fs::Permissions::from_mode(0o755))?;
        }
        fs::write(destination.join("resource"), "adjacent resource")?;
        Ok::<_, turborepo_tool_install::Error>(())
    };
    let mut store = turborepo_tool_install::Store::open(root).unwrap();
    store.reconcile(&tools, stage).unwrap();
    let current = store.current().unwrap().unwrap();
    drop(store);
    for ci in [false, true] {
        let (code, text) = invoke(
            root,
            &[
                "setup",
                "--check",
                "--tools-only",
                "--offline",
                "--cwd=apps/web/src",
            ],
            ci,
        );
        if external_system_policy_blocked(code, &text) {
            continue;
        }
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains("node 24.0.0: ready") && text.contains("pnpm 10.0.0: ready"),
            "{text}"
        );
        assert!(
            text.contains("Dependencies skipped")
                && text.contains("dependency readiness was not checked"),
            "{text}"
        );
        assert!(!text.contains("tasks are ready") && !root.join("probe-ran").exists());
    }
    // Actual native prune, both output shapes, not a manually copied fixture.
    let pruned = OwnedSetupFixture::new().unwrap();
    let context = ExecutionContext::new(
        turborepo_platform::Platform::current(),
        if cfg!(target_os = "linux") {
            Libc::Gnu { abi: "gnu".into() }
        } else {
            Libc::None
        },
        "host-readiness".into(),
        ["unprobed".into()].into(),
    )
    .unwrap();
    assert_eq!(context.artifact_platform(), platform);
    let monitor = TcpListener::bind("127.0.0.1:0").unwrap();
    monitor.set_nonblocking(true).unwrap();
    let proxy = format!("http://{}", monitor.local_addr().unwrap());
    for docker in [false, true] {
        let output_root = pruned.root().join(if docker { "docker" } else { "plain" });
        let before = snapshot(root);
        let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
        command
            .env_clear()
            .current_dir(root)
            .args(["prune", "web", "--out-dir"])
            .arg(&output_root)
            .env("PATH", "")
            .env("HOME", root.join("home"))
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("HTTP_PROXY", &proxy)
            .env("HTTPS_PROXY", &proxy)
            .env("ALL_PROXY", &proxy)
            .env("NO_PROXY", "");
        if docker {
            command.arg("--docker");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(snapshot(root), before);
        let roots = if docker {
            vec![output_root.join("full"), output_root.join("json")]
        } else {
            vec![output_root]
        };
        for output_root in roots {
            for file in ["turbo.lock", ".nvmrc"] {
                assert_eq!(
                    fs::read(output_root.join(file)).unwrap(),
                    fs::read(root.join(file)).unwrap()
                );
            }
            assert!(!output_root.join(".turbo").exists());
            check_failure(
                &output_root,
                &["setup", "--check", "--tools-only"],
                true,
                "managed installation is missing; run turbo setup",
            );
            // A separate repo-local generation, not copied ignored state.
            let mut store = turborepo_tool_install::Store::open(&output_root).unwrap();
            store.reconcile(&tools, stage).unwrap();
            drop(store);
            let (code, text) = invoke(
                &output_root,
                &["setup", "--check", "--tools-only", "--offline"],
                true,
            );
            if external_system_policy_blocked(code, &text) {
                eprintln!(
                    "TURBO-6277: real pruned fixture check success remains blocked by external \
                     System npm configuration."
                );
            } else {
                assert_eq!(code, 0, "{text}");
                assert!(text.contains("Dependencies skipped"), "{text}");
            }
            // Owned policy is a test-only typed seam, never a production opt-out.
            // Inspect the actual prune output and all fixture state without locks.
            let before = snapshot(pruned.root().parent().unwrap());
            let sources = Snapshot::capture(&output_root).unwrap();
            pruned.policy_at(&output_root).unwrap();
            let plan = ActivationPlan::inspect(&output_root, context.clone()).unwrap();
            assert_eq!(plan.tools(), tools);
            pruned.policy_at(&output_root).unwrap();
            sources.ensure_current().unwrap();
            assert_eq!(snapshot(pruned.root().parent().unwrap()), before);
            eprintln!(
                "Real prune output {}: owned typed Node/pnpm readiness passed",
                output_root.display()
            );
            fs::write(plan.path_prepend().join("../tools/pnpm/resource"), "damage").unwrap();
            let damaged = snapshot(pruned.root().parent().unwrap());
            assert!(matches!(
                ActivationPlan::inspect(&output_root, context.clone()),
                Err(turborepo_setup::activation::Error::DamagedInventory(_))
            ));
            assert_eq!(snapshot(pruned.root().parent().unwrap()), damaged);
        }
    }
    assert!(matches!(monitor.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock));
    fs::write(current.bin.join("../tools/pnpm/resource"), "damage").unwrap();
    check_failure(
        root,
        &["setup", "--check", "--tools-only"],
        true,
        "managed installation is damaged or unsafe; run turbo setup",
    );
}

#[test]
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
fn native_local_tools_only_rejects_unsupported_inputs_before_writers() {
    for (node, package, expected) in [
        (None, "{}", "a native Node declaration is required"),
        (
            Some("24.x"),
            r#"{"packageManager":"npm@11.6.1"}"#,
            "npm override provisioning",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "turbo.json", ENABLED);
        write(temp.path(), "package.json", package);
        if let Some(node) = node {
            write(temp.path(), ".nvmrc", node);
        }
        // Real binary, empty PATH, observed proxy, exact recursive file snapshot.
        for ci in [false, true] {
            failure(
                temp.path(),
                &["setup", "--tools-only", "--no-frozen"],
                ci,
                expected,
            );
        }
    }
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "turbo.json", ENABLED);
    write(temp.path(), ".nvmrc", "24.x");
    failure(
        temp.path(),
        &["setup", "--tools-only"],
        true,
        "frozen mode requires turbo.lock",
    );
}

#[test]
fn native_refresh_rejects_ci_and_unimplemented_combinations_before_snapshot_or_traffic() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    write(
        root,
        "turbo.lock",
        "must not be parsed for a denied request",
    );
    for words in [
        vec!["setup", "--update-lock", "--tools-only"],
        vec!["setup", "--update-lock", "--tools-only", "--plan"],
    ] {
        failure(root, &words, true, "inferred in CI");
    }
    for ci in [false, true] {
        for control in [Some("--force"), Some("--offline"), Some("--plan"), None] {
            let mut words = vec!["setup", "--no-frozen", "--update-lock"];
            if let Some(control) = control {
                words.extend(["--tools-only", control]);
            }
            failure(root, &words, ci, PENDING);
        }
        for conflict in ["--frozen", "--no-lock", "--check"] {
            let (code, text) = invoke(
                root,
                &["setup", "--update-lock", "--tools-only", conflict],
                ci,
            );
            assert_eq!(code, 1, "{text}");
            assert!(
                !text.contains("prepared") && !text.contains("must not be parsed"),
                "{text}"
            );
        }
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
