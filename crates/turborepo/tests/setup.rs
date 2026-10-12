//! Standalone command surface: no Node.js, manager, local CLI, tasks, or HTTP.
#![cfg_attr(test, allow(clippy::expect_used))]

mod common;

use std::{fs, io, net::TcpListener, path::Path, process::Command};

#[cfg(all(
    target_os = "linux",
    target_env = "gnu",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[path = "setup/no_lock.rs"]
mod no_lock;

const PENDING: &str = "this setup mode is not implemented";
const ENABLED: &str = r#"{"futureFlags":{"experimentalSetup":true}}"#;
const DISABLED: &str = "requires root futureFlags.experimentalSetup";
const UNQUALIFIED: &str = "managed promotion requires macOS, GNU Linux or Windows x64";
const QUALIFIED: bool = cfg!(any(
    all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        any(
            target_os = "macos",
            all(target_os = "linux", target_env = "gnu")
        )
    ),
    all(windows, target_arch = "x86_64")
));

#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#[path = "setup/recovery.rs"]
mod recovery;

#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#[path = "setup/concurrency.rs"]
mod concurrency;

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

fn invoke_raw(root: &Path, words: &[&str], ci: bool) -> (i32, String, String) {
    invoke_raw_with(root, words, ci, &[])
}

fn invoke_raw_with(
    root: &Path,
    words: &[&str],
    ci: bool,
    env: &[(&str, std::ffi::OsString)],
) -> (i32, String, String) {
    invoke_in(root, words, ci, env, None, true)
}

/// A run with an explicit PATH that may write when `read_only` is false.
fn invoke_on_path(
    root: &Path,
    words: &[&str],
    ci: bool,
    path: &std::ffi::OsStr,
    read_only: bool,
) -> (i32, String) {
    let (code, stdout, stderr) = invoke_in(root, words, ci, &[], Some(path), read_only);
    let text = format!("{stdout}{stderr}")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (code, text)
}

fn invoke_in(
    root: &Path,
    words: &[&str],
    ci: bool,
    env: &[(&str, std::ffi::OsString)],
    path: Option<&std::ffi::OsStr>,
    read_only: bool,
) -> (i32, String, String) {
    let proxy = TcpListener::bind("127.0.0.1:0").expect("bind HTTP monitor");
    proxy.set_nonblocking(true).expect("nonblocking monitor");
    let url = format!("http://{}", proxy.local_addr().expect("monitor address"));
    // Windows has no unprivileged symlinks, so the merged check-and-plan test
    // (#14591 widening) exposes Git's own directory there.
    #[cfg(windows)]
    let git_path = which::which("git")
        .expect("host git")
        .parent()
        .expect("git directory")
        .to_path_buf();
    #[cfg(not(windows))]
    let git_path = root.join("host-bin");
    #[cfg(unix)]
    if path.is_none() && !git_path.exists() {
        fs::create_dir(&git_path).expect("create git-only fixture PATH");
        std::os::unix::fs::symlink(which::which("git").expect("host git"), git_path.join("git"))
            .expect("bind fixture git");
    }
    let before = snapshot(root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
    command
        .env_clear()
        .current_dir(root)
        .args(words)
        .env("PATH", path.unwrap_or(git_path.as_os_str()))
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("APPDATA", root.join("appdata"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("TURBO_CONFIG_DIR_PATH", root.join("config"))
        .env("VERCEL_CONFIG_DIR_PATH", root.join("config"))
        .env("TURBO_INVOCATION_DIR", root.join("unrelated-invocation"))
        .env("TURBO_FORCE", "true")
        // These assertions cover setup's own traffic and writes. Standard
        // telemetry is covered separately with its own config and proxy.
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .env("AI_AGENT", "setup-test")
        .env("HTTP_PROXY", &url)
        .env("HTTPS_PROXY", &url)
        .env("ALL_PROXY", &url)
        .env("NO_PROXY", "")
        .envs(env.iter().cloned());
    if ci {
        // Even this normal shim override must not trigger JS inference or handoff.
        command
            .env("CI", "1")
            .env("TURBO_BINARY_PATH", root.join("unbuilt-local-turbo"));
    }
    let output = command.output().expect("run standalone turbo");
    if read_only {
        assert_eq!(
            snapshot(root),
            before,
            "setup must not write files or directories"
        );
    }
    assert!(
        matches!(proxy.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock),
        "unexpected HTTP connection"
    );
    (
        output.status.code().expect("standalone turbo exit code"),
        String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        String::from_utf8(output.stderr).expect("UTF-8 stderr"),
    )
}

fn invoke(root: &Path, words: &[&str], ci: bool) -> (i32, String) {
    invoke_with(root, words, ci, &[])
}

fn invoke_with(
    root: &Path,
    words: &[&str],
    ci: bool,
    env: &[(&str, std::ffi::OsString)],
) -> (i32, String) {
    let (code, stdout, stderr) = invoke_raw_with(root, words, ci, env);
    let text = format!("{stdout}{stderr}")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (code, text)
}

fn failure(root: &Path, words: &[&str], ci: bool, expected: &str) -> String {
    let (code, text) = invoke(root, words, ci);
    assert_eq!(code, 1, "{words:?}: {text}");
    assert!(diagnostic_contains(&text, expected), "{text}");
    text
}

/// The official source policy's fixed System npmrc locations, plus the bundled
/// npm config under each prefix. Production has no override for these.
fn host_system_npm_config() -> Option<std::path::PathBuf> {
    [
        "/etc/npmrc",
        "/usr/etc/npmrc",
        "/usr/local/etc/npmrc",
        "/opt/homebrew/etc/npmrc",
    ]
    .into_iter()
    .map(Path::new)
    .flat_map(|config| {
        let bundled = config
            .parent()
            .and_then(Path::parent)
            .map(|prefix| prefix.join("lib/node_modules/npm/npmrc"));
        std::iter::once(config.to_owned()).chain(bundled)
    })
    .find(|path| fs::symlink_metadata(path).is_ok())
}

fn external_system_policy_blocked(code: i32, text: &str) -> bool {
    let blocked = diagnostic_contains(text, "official-only setup rejects System configuration")
        || diagnostic_contains(text, "cannot safely inspect System configuration");
    if blocked {
        assert_eq!(code, 1);
        assert!(!text.contains(": ready"), "{text}");
        // Contributor hosts may carry npm policy; CI runner images must qualify.
        assert!(
            std::env::var_os("CI").is_none(),
            "host npm configuration blocked setup in CI: {text}"
        );
        // A clean host must prove readiness; only real host state may block it.
        let Some(config) = host_system_npm_config() else {
            panic!("System policy blocked readiness without host System npm configuration: {text}");
        };
        eprintln!(
            "Native readiness qualification blocked by external System npm configuration at {}; \
             policy was not bypassed.",
            config.display()
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

/// Setup qualification follows the host C library, not this test's target.
fn qualified_host() -> bool {
    cfg!(any(target_arch = "x86_64", target_arch = "aarch64"))
        && (cfg!(target_os = "macos")
            || matches!(
                turborepo_platform::host_libc(),
                Ok(Some(turborepo_platform::Libc::Glibc { .. }))
            ))
}

fn diagnostic_contains(text: &str, expected: &str) -> bool {
    // Miette may wrap sentences and filenames across lines with `|` gutters.
    text.replace([' ', '|'], "")
        .contains(&expected.replace(' ', ""))
}

#[test]
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", target_os = "linux")
))]
fn frozen_and_local_unsupported_plans_are_read_only() {
    if !qualified_host() {
        return;
    }
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
    // #14518 admits --offline, which reads (and rejects) the invalid lock.
    failure(
        root,
        &["setup", "--offline"],
        false,
        "invalid turbo.lock JSON",
    );
    for words in [
        vec!["--root-turbo-json", "custom.json", "setup", "--plan"],
        vec!["--cwd=selected-root", "setup", "--plan"],
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
    // Inference parses enabled native manifests in-process, but a missing local
    // pin/manager and Cargo executable must not trigger provisioning or handoff.
    let package = r#"{"devDependencies":{"turbo":"2.0.3"},"scripts":{"setup":"exit 99"}}"#;
    write(root, "package.json", package);
    write(root, "Cargo.toml", "[workspace]\nmembers = []\n");
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
        // Plain setup provisions like --tools-only, so it reaches the same gate.
        let supported = !words.iter().any(|word| {
            matches!(
                *word,
                "--plan" | "--check" | "--offline" | "--no-lock" | "--__test-run"
            )
        });
        // #14518 admits --offline, which needs a lock first, and #14486 admits
        // --no-lock, whose native baseline covers only Node and pnpm.
        let offline = words.contains(&"--offline");
        let no_lock = words.contains(&"--no-lock");
        // A Cargo workspace without a root rust-toolchain file declares no tool.
        // Frozen (#14492) and forced (#14540) setup need turbo.lock before
        // they look for a Node declaration.
        let text = failure(
            root,
            &words,
            false,
            match (supported, QUALIFIED) {
                (_, true) if offline => "offline setup requires turbo.lock",
                (_, true) if no_lock => "invalid or unsupported native resolution record",
                (false, _) => PENDING,
                (true, true) if words.contains(&"--frozen") => "frozen mode requires turbo.lock",
                (true, true) if words.contains(&"--force") => {
                    "locked provisioning requires turbo.lock"
                }
                (true, true) => "a native Node declaration is required",
                (true, false) => UNQUALIFIED,
            },
        );
        if !supported && !offline && !no_lock {
            assert!(text.contains("no tasks were run"), "{text}");
        }
        assert!(!text.contains("test run successful"), "{text}");
    }
    check_failure(
        root,
        &["setup", "--check", "--tools-only"],
        false,
        if QUALIFIED {
            "managed activation requires turbo.lock"
        } else {
            UNQUALIFIED
        },
    );
    let text = failure(root, &["setup", "--update-lock"], true, "inferred in CI");
    assert!(text.contains("--no-frozen"), "{text}");
    failure(
        root,
        &["setup", "--update-lock", "--no-frozen"],
        true,
        if QUALIFIED {
            "a native Node declaration is required"
        } else {
            UNQUALIFIED
        },
    );
    // #14486 admits CI --no-lock; its native baseline rejects a Rust-only repo.
    failure(
        root,
        &["setup", "--no-lock"],
        true,
        if QUALIFIED {
            "invalid or unsupported native resolution record"
        } else {
            UNQUALIFIED
        },
    );
}

#[test]
fn malformed_cargo_manifest_is_rejected_only_when_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Cargo.toml", "must not run cargo metadata");
    for enabled in [false, true] {
        write(
            root,
            "turbo.json",
            &format!(
                r#"{{"futureFlags":{{"experimentalSetup":true,"experimentalCargoWorkspaces":{enabled}}}}}"#
            ),
        );
        for words in [&["setup"][..], &["setup", "--plan"], &["setup", "--check"]] {
            if enabled {
                let text = failure(root, words, false, "failed to parse Cargo.toml");
                assert_diagnostic_path(&text, &root.join("Cargo.toml"));
                assert!(!text.contains(PENDING), "{text}");
            } else if words.len() == 1 {
                // Plain setup reaches JS provisioning without parsing Cargo.
                // (#14653's Node-required message names experimentalCargoWorkspaces.)
                let (code, text) = invoke(root, words, false);
                assert_eq!(code, 1, "{text}");
                assert!(!text.contains("Cargo.toml"), "{text}");
            } else {
                failure(root, words, false, PENDING);
            }
        }
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
    // A pending mode stops right after root discovery is logged.
    let words: Vec<_> = words
        .iter()
        .copied()
        .chain(["--plan", "--verbosity=2"])
        .collect();
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
    // Windows arm64 hosts are refused even when this build is x64.
    let arm64_windows = common::windows_arm64_host();
    let qualified =
        (qualified_host() || cfg!(all(windows, target_arch = "x86_64"))) && !arm64_windows;
    let musl = matches!(
        turborepo_platform::host_libc(),
        Ok(Some(turborepo_platform::Libc::Musl))
    );
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
                } else if musl {
                    "not supported on musl Linux hosts"
                } else if arm64_windows {
                    "managed setup is not qualified for Windows arm64"
                } else {
                    "managed promotion requires macOS, GNU Linux or Windows x64"
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

#[cfg(unix)]
#[test]
fn verify_system_locks_and_verifies_installed_tools_without_installing() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let (root, bin) = (temp.path().join("repo"), temp.path().join("bin"));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let tool = |name: &str, version: &str| {
        let path = bin.join(name);
        fs::write(&path, format!("#!/bin/sh\necho {version}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    };
    tool("node", "v24.0.0");
    tool("pnpm", "10.0.0");
    let git = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status();
    assert!(git.unwrap().success());
    write(&root, ".gitignore", "/.turbo/\n");
    write(&root, ".nvmrc", "24.x\n");
    write(&root, "package.json", r#"{"packageManager":"pnpm@10.0.0"}"#);
    write(
        &root,
        "turbo.json",
        r#"{"futureFlags":{"experimentalSetup":true},
            "setup":{"dependencies":"skip","javascript":{"tools":"verify-system"}}}"#,
    );
    // Fake tools come first; the host PATH only supplies git.
    let host = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&host)))
            .unwrap();

    // Local setup locks the installed versions; nothing is downloaded or installed.
    let (code, text) = invoke_on_path(&root, &["setup"], false, &path, false);
    assert_eq!(code, 0, "{text}");
    for expected in [
        "javascript: tools verify-system (turbo.json setup policy)",
        "turbo.lock created",
        "node: locked 24.0.0",
        "node 24.0.0: verified at",
        "pnpm 10.0.0: verified at",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    let lock = fs::read_to_string(root.join("turbo.lock")).unwrap();
    assert!(
        lock.contains(r#""installation":{"kind":"verify-system","executables":["node"]}"#),
        "{lock}"
    );
    assert!(!root.join(".turbo/tools").exists());

    // Frozen repeats and checks are read-only and verify the locked versions.
    for words in [["setup", "--frozen"], ["setup", "--check"]] {
        let (code, text) = invoke_on_path(&root, &words, false, &path, true);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("node 24.0.0:"), "{text}");
    }
    tool("node", "v24.1.0");
    for words in [
        ["setup", "--frozen"],
        ["setup", "--check"],
        ["setup", "--force"],
    ] {
        let (code, text) = invoke_on_path(&root, &words, false, &path, true);
        assert_eq!(code, 1, "{text}");
        assert!(diagnostic_contains(&text, "requires 24.0.0"), "{text}");
    }
    // A managed lock is never silently trusted as an external tool.
    write(&root, "turbo.json", ENABLED);
    let (code, text) = invoke_on_path(
        &root,
        &["setup", "--frozen", "--tools-only"],
        false,
        &path,
        true,
    );
    assert_eq!(code, 1, "{text}");
}

#[test]
fn standalone_root_policy_selects_tools_only_and_skips_ecosystems() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Cargo.toml", "[workspace]\nmembers = []\n");
    let config = |dependencies: &str| {
        format!(
            r#"{{"futureFlags":{{"experimentalSetup":true,"experimentalCargoWorkspaces":true}},
                "setup":{{"dependencies":"{dependencies}","rust":{{"tools":"skip"}}}}}}"#
        )
    };
    write(root, "turbo.json", &config("skip"));
    // The source preflight accepts the policy, which makes this a tools-only
    // check without --tools-only and lifts the unsupported Rust ecosystem.
    let (code, text) = invoke(root, &["setup", "--check"], false);
    assert_eq!(code, 1, "{text}");
    if cfg!(all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        any(
            target_os = "macos",
            all(target_os = "linux", target_env = "gnu")
        )
    )) && !external_system_policy_blocked(code, &text)
    {
        assert!(
            diagnostic_contains(&text, "managed activation requires turbo.lock"),
            "{text}"
        );
        for line in [
            "javascript: tools managed (inferred); dependencies skipped (turbo.json setup policy)",
            "rust: skipped (turbo.json setup policy)",
        ] {
            assert!(text.contains(line), "{text}");
        }
    }
    // Locked dependencies still need dependency setup, unless the CLI skips it.
    write(root, "turbo.json", &config("locked"));
    failure(root, &["setup", "--check"], false, PENDING);
}

#[test]
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
fn npm_and_pnpm_wrapper_environments_reach_setup_but_custom_registries_do_not() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    let at = |path: &str| root.join(path).into_os_string();
    // Default `npx turbo setup` (npm 11) and `pnpm turbo setup` (pnpm 11+).
    let npm = vec![
        ("npm_config_cache", at("home/.npm")),
        ("npm_config_global_prefix", at("prefix")),
        ("npm_config_globalconfig", at("prefix/etc/npmrc")),
        ("npm_config_init_module", at("home/.npm-init.js")),
        ("npm_config_local_prefix", root.as_os_str().to_owned()),
        ("npm_config_node_gyp", at("prefix/node-gyp.js")),
        ("npm_config_noproxy", "".into()),
        ("npm_config_npm_version", "11.13.0".into()),
        ("npm_config_prefix", at("prefix")),
        ("npm_config_user_agent", "npm/11.13.0 node/v24.0.0".into()),
        ("npm_config_userconfig", at("home/.npmrc")),
        ("npm_config_yes", "true".into()),
        ("npm_lifecycle_event", "npx".into()),
    ];
    let pnpm = vec![
        ("npm_config_node_gyp", at("pnpm/node-gyp.js")),
        ("npm_config_user_agent", "pnpm/11.0.0 node/v24.0.0".into()),
    ];
    // `pnpm run` in pnpm 10 and earlier also exports default registries.
    let pnpm10 = vec![
        ("npm_config__jsr_registry", "https://npm.jsr.io/".into()),
        ("npm_config_frozen_lockfile", "".into()),
        ("npm_config_globalconfig", at("home/.config/pnpm/rc")),
        ("npm_config_node_gyp", at("pnpm/node-gyp.js")),
        ("npm_config_registry", "https://registry.npmjs.org/".into()),
        (
            "npm_config_user_agent",
            "pnpm/10.34.7 npm/? node/v24.0.0".into(),
        ),
        ("npm_config_verify_deps_before_run", "false".into()),
    ];
    let words = ["setup", "--check", "--tools-only"];
    for env in [&npm, &pnpm, &pnpm10] {
        let (code, text) = invoke_with(root, &words, false, env);
        assert_eq!(code, 1, "{text}");
        assert!(
            diagnostic_contains(&text, "managed activation requires turbo.lock")
                || external_system_policy_blocked(code, &text),
            "{text}"
        );
        let mut custom = env.clone();
        custom.push(("npm_config_registry", "https://private.invalid/".into()));
        let (code, text) = invoke_with(root, &words, false, &custom);
        assert_eq!(code, 1, "{text}");
        assert!(
            diagnostic_contains(&text, "rejects npm configuration environment"),
            "{text}"
        );
        assert!(!text.contains("private.invalid"), "{text}");
    }
}

/// A managed Node/pnpm repository whose `turbo.lock` records `.nvmrc` and
/// `package.json#/packageManager`. Returns the tools a healthy generation
/// holds.
#[cfg(any(
    all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        any(target_os = "macos", target_os = "linux")
    ),
    all(windows, target_arch = "x86_64")
))]
fn managed_fixture(root: &Path) -> Vec<turborepo_tool_install::Tool> {
    use turborepo_setup::{
        lock::{Lock, Platform},
        node_provision::NodePlan,
        pnpm_provision::PnpmPlan,
    };
    // Ubuntu's default 0002 umask leaves fresh clones group-writable.
    #[cfg(unix)]
    fs::set_permissions(
        root,
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o775),
    )
    .expect("make the fixture root group-writable");
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
    fs::create_dir_all(root.join("apps/web/src")).expect("fixture package directory");
    write(root, "apps/web/package.json", r#"{"name":"web"}"#);
    let (platform, selector, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => (Platform::MacosArm64, "macos-arm64", "darwin-arm64"),
        ("macos", _) => (Platform::MacosX64, "macos-x64", "darwin-x64"),
        ("linux", "aarch64") => (Platform::LinuxArm64Gnu, "linux-arm64-gnu", "linux-arm64"),
        ("windows", _) => (Platform::WindowsX64, "windows-x64", "win-x64"),
        _ => (Platform::LinuxX64Gnu, "linux-x64-gnu", "linux-x64"),
    };
    let prefix = format!("node-v24.0.0-{spelling}");
    let (extension, format, executable) = if cfg!(windows) {
        ("zip", "zip", "node.exe")
    } else {
        ("tar.gz", "tar-gz", "bin/node")
    };
    let lock = serde_json::json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{selector:{"distribution":{
                "url":format!("https://nodejs.org/dist/v24.0.0/{prefix}.{extension}"),"sha256":"0".repeat(64),
                "format":format,"rootPrefix":prefix,"executables":{"node":executable}}}}}},
        "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":"1".repeat(64),
                "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}}}
    }}).to_string();
    write(root, "turbo.lock", &lock);
    let lock = Lock::parse(lock.as_bytes()).expect("fixture lock");
    let node = NodePlan::from_lock(&lock, platform).expect("fixture Node plan");
    let declaration = turborepo_setup::lock::Snapshot::capture(root)
        .expect("fixture sources")
        .package_manager()
        .expect("fixture package manager")
        .expect("declared package manager");
    let pnpm = PnpmPlan::from_declaration(&lock, platform, &node, &declaration)
        .expect("fixture pnpm plan");
    vec![node.inventory_tool().clone(), pnpm.inventory_tool().clone()]
}

/// Seed a repo-local generation through the real Store/adapter contract.
/// Production has no loopback override; the CLI library fixtures separately
/// exercise actual downloads. Executables fail loudly if anything probes them.
#[cfg(any(
    all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        any(target_os = "macos", target_os = "linux")
    ),
    all(windows, target_arch = "x86_64")
))]
fn seed_generation(
    root: &Path,
    tools: &[turborepo_tool_install::Tool],
) -> turborepo_tool_install::Current {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let stage = |tool: &turborepo_tool_install::Tool, destination: &Path| {
        for relative in tool.executables.values() {
            let file = destination.join(relative);
            fs::create_dir_all(file.parent().expect("executable parent"))?;
            fs::write(&file, "#!/bin/sh\ntouch probe-ran\nexit 99\n")?;
            #[cfg(unix)]
            fs::set_permissions(file, fs::Permissions::from_mode(0o755))?;
        }
        fs::write(destination.join("resource"), "adjacent resource")?;
        Ok::<_, turborepo_tool_install::Error>(())
    };
    let mut store = turborepo_tool_install::Store::open(root).expect("open tool store");
    store.reconcile(tools, stage).expect("seed generation");
    store
        .current()
        .expect("read generation")
        .expect("current generation")
}

#[cfg(any(
    all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        any(target_os = "macos", target_os = "linux")
    ),
    all(windows, target_arch = "x86_64")
))]
#[test]
fn standalone_check_and_locked_plan_report_only_node_pnpm_and_reject_damage() {
    if common::windows_arm64_host() {
        eprintln!("Skipped: managed setup refuses Windows arm64 hosts, even for x64 turbo.");
        return;
    }
    if !(qualified_host() || cfg!(all(windows, target_arch = "x86_64"))) {
        return;
    }
    let owned = turborepo_setup::test_support::OwnedSetupFixture::new().unwrap();
    owned.init_git().unwrap();
    let root = owned.root();
    let tools = managed_fixture(root);
    let spelling = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", _) => "darwin-x64",
        ("linux", "aarch64") => "linux-arm64",
        // #14591's Windows x64 fixture.
        ("windows", _) => "win-x64",
        _ => "linux-x64",
    };
    let prefix = format!("node-v24.0.0-{spelling}");
    let positives = std::cell::Cell::new(0);
    let blocked = std::cell::Cell::new(0);
    let assert_plans = |installed: bool| {
        for ci in [false, true] {
            for extra in [
                None,
                Some("--frozen"),
                Some("--offline"),
                Some("--no-frozen"),
            ] {
                for force in [false, true] {
                    let mut words = vec!["setup", "--plan", "--tools-only", "--cwd=apps/web/src"];
                    words.extend(extra);
                    if force {
                        words.push("--force");
                    }
                    let (code, stdout, stderr) = invoke_raw(root, &words, ci);
                    if external_system_policy_blocked(code, &stderr) {
                        assert_eq!(stdout, "");
                        blocked.set(blocked.get() + 1);
                        continue;
                    }
                    let action = if force {
                        "reinstall (--force)"
                    } else if installed {
                        "reuse (healthy installation)"
                    } else {
                        "install (missing installation)"
                    };
                    assert_eq!(code, 0, "{stderr}");
                    assert_eq!(stderr, "");
                    // #14664 reports the effective policy before every plan;
                    // #14591 fixtures use Node's Windows zip.
                    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
                    assert_eq!(stdout, format!(
                        "javascript: tools managed (inferred); dependencies skipped (--tools-only)\n\
                         Locked tools-only plan (turbo.lock unchanged):\n\
                         node 24.0.0: {action}\n  Declaration: .nvmrc (24.0.0)\n  Source: https://nodejs.org/dist/v24.0.0/{prefix}.{extension}\n  SHA-256: {}\n\
                         pnpm 10.0.0: {action}\n  Declaration: package.json#/packageManager (pnpm@10.0.0)\n  Source: https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz\n  SHA-256: {}\n\
                         Dependencies skipped (--tools-only); dependency readiness was not checked.\n\
                         Tracked changes: none. No downloads, probes, or tasks run.\n",
                        "0".repeat(64), "1".repeat(64),
                    ));
                    assert_eq!(root.join(".turbo").exists(), installed);
                    positives.set(positives.get() + 1);
                }
            }
        }
    };
    assert_plans(false);
    let current = seed_generation(root, &tools);
    assert_plans(true);
    eprintln!(
        "Native locked-plan qualification: {}/32 positive, {} blocked",
        positives.get(),
        blocked.get()
    );
    assert_eq!(positives.get() + blocked.get(), 32);
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
        // Disk use is reported without any write (checked by `invoke`).
        assert!(text.contains("in this clone's .turbo/tools."), "{text}");
        assert!(
            text.contains("Dependencies skipped")
                && text.contains("dependency readiness was not checked"),
            "{text}"
        );
        assert!(!text.contains("tasks are ready") && !root.join("probe-ran").exists());
    }
    fs::write(current.bin.join("../tools/pnpm/resource"), "damage").unwrap();
    check_failure(
        root,
        &["setup", "--check", "--tools-only"],
        true,
        "managed installation is damaged or unsafe; run turbo setup",
    );
    for corrupt_manifest in [false, true] {
        if corrupt_manifest {
            write(root, ".turbo/tools/manifest.json", "invalid");
        }
        for force in [false, true] {
            let mut words = vec!["setup", "--plan", "--tools-only", "--offline"];
            if force {
                words.push("--force");
            }
            let (code, stdout, stderr) = invoke_raw(root, &words, true);
            if external_system_policy_blocked(code, &stderr) || corrupt_manifest {
                assert_eq!(code, 1, "{stderr}");
                // #14664 reports the effective policy before planning, so a
                // failed plan still prints only that line.
                assert_eq!(
                    stdout,
                    "javascript: tools managed (inferred); dependencies skipped (--tools-only)\n"
                );
                assert!(
                    diagnostic_contains(&stderr, "managed installation is damaged or unsafe")
                        || external_system_policy_blocked(code, &stderr),
                    "{stderr}"
                );
            } else {
                assert_eq!(code, 0, "{stderr}");
                assert_eq!(stderr, "");
                assert!(stdout.contains(if force {
                    "reinstall (--force)"
                } else {
                    "repair (damaged installation)"
                }));
            }
        }
    }
}

/// The real `turbo prune` binary, without network or writes to the source.
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", target_os = "linux")
))]
fn prune(root: &Path, output: &Path, docker: bool) {
    let monitor = TcpListener::bind("127.0.0.1:0").expect("bind HTTP monitor");
    monitor.set_nonblocking(true).expect("nonblocking monitor");
    let proxy = format!("http://{}", monitor.local_addr().expect("monitor address"));
    let before = snapshot(root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
    command
        .env_clear()
        .current_dir(root)
        .args(["prune", "web", "--out-dir"])
        .arg(output)
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
    let result = command.output().expect("run turbo prune");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(snapshot(root), before);
    assert!(matches!(monitor.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock));
}

/// TURBO-6277 acceptance: each actual prune output (plain, Docker full and
/// Docker json) passes the real read-only check against its own copied
/// `turbo.lock`, from the output root and from a pruned package.
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", target_os = "linux")
))]
#[test]
fn pruned_outputs_pass_real_setup_check_against_their_own_lock() {
    if !qualified_host() {
        return;
    }
    use turborepo_setup::{
        activation::ActivationPlan,
        execution_identity::{ExecutionContext, Libc},
        test_support::OwnedSetupFixture,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let tools = managed_fixture(root);
    // Typed fallback only for hosts whose System npm config blocks the CLI.
    let fallback = OwnedSetupFixture::new().unwrap();
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
    let mut proven = 0;
    for docker in [false, true] {
        let output = fallback
            .root()
            .join(if docker { "docker" } else { "plain" });
        prune(root, &output, docker);
        let layouts = if docker {
            vec![output.join("full"), output.join("json")]
        } else {
            vec![output]
        };
        for pruned in layouts {
            // package.json is rewritten for the pruned workspaces; the check
            // below proves its packageManager provenance still matches the lock.
            for file in ["turbo.lock", ".nvmrc"] {
                assert_eq!(
                    fs::read(pruned.join(file)).unwrap(),
                    fs::read(root.join(file)).unwrap(),
                    "{file}"
                );
            }
            // Ignored installation state is never copied: each output gets its
            // own setup, so the check first reports the missing generation.
            assert!(!pruned.join(".turbo").exists());
            check_failure(
                &pruned,
                &["setup", "--check", "--tools-only"],
                true,
                "managed installation is missing; run turbo setup",
            );
            let current = seed_generation(&pruned, &tools);
            for ci in [false, true] {
                for words in [
                    &["setup", "--check", "--tools-only"][..],
                    &["setup", "--check", "--tools-only", "--offline"],
                    &["setup", "--check", "--tools-only", "--cwd=apps/web"],
                ] {
                    let (code, text) = invoke(&pruned, words, ci);
                    if external_system_policy_blocked(code, &text) {
                        // Not a pass: the same inspection through owned policy.
                        fallback.policy_at(&pruned).unwrap();
                        let plan = ActivationPlan::inspect(&pruned, context.clone()).unwrap();
                        assert_eq!(plan.tools(), tools);
                        continue;
                    }
                    assert_eq!(code, 0, "{}: {words:?}: {text}", pruned.display());
                    assert!(
                        text.contains("node 24.0.0: ready")
                            && text.contains("pnpm 10.0.0: ready")
                            && text.contains("Dependencies skipped"),
                        "{text}"
                    );
                    assert!(!pruned.join("probe-ran").exists());
                    proven += 1;
                }
            }
            fs::write(current.bin.join("../tools/pnpm/resource"), "damage").unwrap();
            check_failure(
                &pruned,
                &["setup", "--check", "--tools-only", "--offline"],
                false,
                "managed installation is damaged or unsafe; run turbo setup",
            );
        }
    }
    eprintln!("Real pruned setup checks passed: {proven} of 18");
    assert!(proven == 18 || host_system_npm_config().is_some());
}

#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#[test]
fn plain_setup_provisions_tools_and_reports_dependencies_skipped() {
    // #14494 reuses only generations with a trusted user record, so a
    // generation seeded straight into `.turbo/tools` is never reused. Seed the
    // shared download cache instead, like #14569's recovery fixture: plain
    // frozen setup installs from verified cached bytes and records trust, and
    // the repeat reuses them. `Repo::run` fails on any network connection.
    let Some(repo) = recovery::Repo::new() else {
        return;
    };
    repo.seed();
    for outcome in ["installed from the shared download cache", "reused"] {
        let recovery::Run { status, text } = repo.run(None, &["setup", "--frozen"]);
        let code = status.code().unwrap();
        if external_system_policy_blocked(code, &text) {
            return;
        }
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains(&format!("node 24.0.0: {outcome}"))
                && text.contains(&format!("pnpm 10.0.0: {outcome}")),
            "{text}"
        );
        assert!(
            text.contains("Dependencies skipped (dependency installation is not supported yet)"),
            "{text}"
        );
    }
    assert!(!repo.root.join("probe-ran").exists() && !repo.root.join("node_modules").exists());
}

#[test]
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", target_os = "linux")
))]
fn native_local_tools_only_rejects_unsupported_inputs_before_writers() {
    if !qualified_host() {
        return;
    }
    for (node, package, expected) in [
        (None, "{}", "a native Node declaration is required"),
        (
            Some("24.x"),
            r#"{"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":{"name":"npm","version":"11.6.1","onFail":"error"}}}"#,
            "invalid native setup declarations",
        ),
        (
            Some("24.x"),
            r#"{"packageManager":"yarn@https://repo.yarnpkg.com/4.5.0/packages/yarnpkg-cli/bin/yarn.js"}"#,
            "package.json#/packageManager: URLs, paths and aliases are unsupported",
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

/// Real setup with standard telemetry left on: a user config directory outside
/// the repository, debug output of every event sent, and a proxy that accepts
/// connections but never answers. Returns (code, stdout, stderr, connected).
fn invoke_with_telemetry(
    root: &Path,
    config: &Path,
    words: &[&str],
    opt_out: Option<&str>,
) -> (i32, String, String, bool) {
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
        .env("XDG_CONFIG_HOME", config)
        .env("TURBO_CONFIG_DIR_PATH", config)
        .env("VERCEL_CONFIG_DIR_PATH", config)
        .env("TURBO_TELEMETRY_DEBUG", "1")
        .env("HTTP_PROXY", &url)
        .env("HTTPS_PROXY", &url)
        .env("ALL_PROXY", &url)
        .env("NO_PROXY", "");
    if let Some(variable) = opt_out {
        command.env(variable, "1");
    }
    let started = std::time::Instant::now();
    let output = command.output().expect("run standalone turbo");
    // Never waits for the unanswered request (10s request timeout).
    assert!(started.elapsed() < std::time::Duration::from_secs(8));
    assert_eq!(snapshot(root), before, "telemetry must not write the repo");
    (
        output.status.code().expect("standalone turbo exit code"),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        proxy.accept().is_ok(),
    )
}

fn telemetry_events(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .split("[telemetry event]")
        .skip(1)
        .map(|chunk| {
            serde_json::Deserializer::from_str(chunk.trim_start())
                .into_iter::<serde_json::Value>()
                .next()
                .expect("telemetry event")
                .expect("telemetry event JSON")
        })
        .collect()
}

#[test]
fn setup_telemetry_is_standard_opt_out_redacted_and_never_changes_the_result() {
    const NOTICE: &str = "anonymous telemetry";
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private-repo-name");
    fs::create_dir(&root).unwrap();
    write(&root, "turbo.json", ENABLED);
    write(&root, ".nvmrc", "24.0.0");
    write(&root, "package.json", "{}");
    let cwd = format!("--cwd={}", root.display());
    let words = ["setup", "--frozen", "--tools-only", cwd.as_str()];
    let (diagnostic, category) = if QUALIFIED {
        ("frozen mode requires turbo.lock", "lock_missing")
    } else {
        (UNQUALIFIED, "platform")
    };
    let unchanged = |code: i32, stderr: &str| {
        assert_eq!(code, 1, "{stderr}");
        assert!(diagnostic_contains(stderr, diagnostic), "{stderr}");
    };

    // On by default: the one-time notice, then aggregate events only.
    let config = temp.path().join("config");
    let (code, stdout, stderr, _) = invoke_with_telemetry(&root, &config, &words, None);
    unchanged(code, &stderr);
    assert!(stderr.contains(NOTICE), "{stderr}");
    let events = telemetry_events(&stdout);
    let mut setup: Vec<String> = events
        .iter()
        .filter_map(|event| event.get("command"))
        .filter(|event| event["command"] == "setup" && event["parent_id"].is_string())
        .map(|event| {
            format!(
                "{}={}",
                event["key"].as_str().unwrap(),
                event["value"].as_str().unwrap()
            )
        })
        .collect();
    setup.sort();
    let duration = setup
        .iter()
        .position(|e| e.starts_with("duration_ms="))
        .unwrap();
    assert!(
        setup.remove(duration)["duration_ms=".len()..]
            .parse::<u64>()
            .is_ok()
    );
    let failure = format!("failure={category}");
    assert_eq!(
        setup,
        [
            "command=called",
            "dependencies=skip",
            &failure,
            "lock=frozen",
            "mode=provision",
            "outcome=failed"
        ]
    );
    // No paths, repository names, arguments, declarations, or URLs.
    let sent = serde_json::to_string(&events).unwrap();
    for private in [
        root.to_str().unwrap(),
        "private-repo-name",
        "--frozen",
        "--cwd",
        "24.0.0",
        ".nvmrc",
        "http",
    ] {
        assert!(!sent.contains(private), "{private}: {sent}");
    }
    let (code, stdout, stderr, _) = invoke_with_telemetry(&root, &config, &words, None);
    unchanged(code, &stderr);
    assert!(!stderr.contains(NOTICE) && !telemetry_events(&stdout).is_empty());

    // Each standard opt-out, and setup without the future flag: the same local
    // diagnostic with no notice, events, config, or connection.
    let disable = Command::new(env!("CARGO_BIN_EXE_turbo"))
        .env_clear()
        .current_dir(temp.path())
        .args(["telemetry", "disable"])
        .env("PATH", "")
        .env("HOME", temp.path().join("home"))
        .env("TURBO_CONFIG_DIR_PATH", &config)
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .output()
        .unwrap();
    assert!(
        disable.status.success(),
        "{}",
        String::from_utf8_lossy(&disable.stderr)
    );
    for (config, opt_out) in [
        (config, None),
        (
            temp.path().join("disabled"),
            Some("TURBO_TELEMETRY_DISABLED"),
        ),
        (temp.path().join("dnt"), Some("DO_NOT_TRACK")),
    ] {
        let (code, stdout, stderr, connected) =
            invoke_with_telemetry(&root, &config, &words, opt_out);
        unchanged(code, &stderr);
        assert!(!stderr.contains(NOTICE) && !stdout.contains("[telemetry") && !connected);
        assert_eq!(config.exists(), opt_out.is_none());
    }
    write(&root, "turbo.json", "{}");
    let config = temp.path().join("flagless");
    let (code, stdout, stderr, connected) = invoke_with_telemetry(&root, &config, &words, None);
    assert_eq!(code, 1);
    assert!(diagnostic_contains(&stderr, DISABLED), "{stderr}");
    assert!(!stderr.contains(NOTICE) && !stdout.contains("[telemetry") && !connected);
    assert!(!config.exists());
}

#[test]
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
fn version_manager_declarations_are_reported_as_ignored_never_silently() {
    const IGNORED: &str =
        "Ignored version manager declarations; turbo setup reads only native declarations:";
    // Each file alone: setup still requires a native Node declaration and says
    // which declarations it ignored, in local and CI-inferred modes.
    for (file, contents, expected) in [
        (
            ".tool-versions",
            "nodejs 20.11.0 # asdf\npnpm 8.15.0\n",
            ".tool-versions (asdf/mise): nodejs 20.11.0, pnpm 8.15.0",
        ),
        (
            "mise.toml",
            "[tools]\nnode = \"22\"\n",
            "mise.toml (mise): node 22",
        ),
        (
            "package.json",
            r#"{"volta":{"node":"20.11.0"}}"#,
            "package.json#volta (Volta): node 20.11.0",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "turbo.json", ENABLED);
        write(temp.path(), "package.json", "{}");
        write(temp.path(), file, contents);
        for ci in [false, true] {
            let text = failure(
                temp.path(),
                &["setup", "--tools-only", "--no-frozen"],
                ci,
                "a native Node declaration is required",
            );
            assert!(text.contains(IGNORED) && text.contains(expected), "{text}");
            assert!(!text.contains("takes precedence"), "{text}");
        }
    }

    // Alongside conflicting native declarations: native files decide, and both
    // provisioning and the read-only check say so before their outcome.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    write(root, ".nvmrc", "24.0.0");
    write(
        root,
        "package.json",
        r#"{"packageManager":"pnpm@10.0.0","volta":{"node":"18.20.0","pnpm":"9.0.0"}}"#,
    );
    write(root, ".tool-versions", "nodejs 20.11.0\nruby 3.3.0\n");
    write(root, "mise.toml", "[tools]\nnode = \"22\"\n");
    for (words, ci, expected) in [
        (
            &["setup", "--tools-only"][..],
            true,
            "frozen mode requires turbo.lock",
        ),
        (
            &["setup", "--check", "--tools-only"],
            false,
            "managed activation requires turbo.lock",
        ),
    ] {
        let (code, text) = invoke(root, words, ci);
        assert_eq!(code, 1, "{text}");
        assert!(
            diagnostic_contains(&text, expected) || external_system_policy_blocked(code, &text),
            "{text}"
        );
        for ignored in [
            IGNORED,
            ".tool-versions (asdf/mise): nodejs 20.11.0 (native node declaration takes \
             precedence), ruby 3.3.0",
            "mise.toml (mise): node 22 (native node declaration takes precedence)",
            "package.json#volta (Volta): node 18.20.0 (native node declaration takes precedence), \
             pnpm 9.0.0 (native pnpm declaration takes precedence)",
        ] {
            assert!(text.contains(ignored), "{text}");
        }
    }

    // An ignored file that cannot be interpreted is reported, never fatal.
    write(root, "mise.toml", "[tools\n");
    let text = failure(
        root,
        &["setup", "--tools-only"],
        true,
        "frozen mode requires turbo.lock",
    );
    assert!(
        text.contains("mise.toml (mise): present but not read"),
        "{text}"
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
        // `--plan --update-lock` proposes a refresh; with --force it is denied.
        for controls in [&["--force"][..], &["--offline"], &["--plan", "--force"]] {
            let mut words = vec!["setup", "--no-frozen", "--update-lock", "--tools-only"];
            words.extend(controls);
            let expected = if controls == ["--offline"] {
                "--offline cannot resolve --update-lock"
            } else {
                PENDING
            };
            failure(root, &words, ci, expected);
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
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
fn native_locked_plan_missing_lock_refresh_and_policy_remain_explicit_errors() {
    let owned = turborepo_setup::test_support::OwnedSetupFixture::new().unwrap();
    owned.init_git().unwrap();
    let root = owned.root();
    write(root, "turbo.json", ENABLED);
    write(root, ".nvmrc", "24.x");
    // Proposals that read upstream metadata are covered by loopback fixtures;
    // this real binary only exercises their network-free failures.
    let offline = "offline resolution metadata/artifacts are not cached";
    for ci in [false, true] {
        for (extra, expected) in [
            (&["--frozen"][..], "frozen setup requires turbo.lock"),
            (
                &["--force"],
                "--plan --force reports reinstalls of locked selections",
            ),
            (&["--no-frozen", "--offline"], offline),
            (&["--no-frozen", "--update-lock", "--offline"], offline),
        ] {
            let mut words = vec!["setup", "--plan", "--tools-only"];
            words.extend(extra);
            check_failure(root, &words, ci, expected);
        }
        for extra in [
            &["--no-lock"][..],
            &["--no-frozen", "--update-lock", "--force"],
        ] {
            let mut words = vec!["setup", "--plan", "--tools-only"];
            words.extend(extra);
            failure(root, &words, ci, PENDING);
        }
    }
    check_failure(
        root,
        &["setup", "--plan", "--tools-only"],
        true,
        "frozen setup requires turbo.lock",
    );
    write(root, "turbo.lock", "invalid");
    failure(
        root,
        &["setup", "--plan", "--tools-only"],
        true,
        "invalid turbo.lock JSON",
    );
    write(root, "turbo.json", "{}");
    failure(root, &["setup", "--plan", "--tools-only"], true, DISABLED);
    write(root, "turbo.json", ENABLED);
    write(root, "turbo.lock", r#"{"schemaVersion":0,"tools":{}}"#);
    write(root, ".npmrc", "registry=https://private.invalid");
    check_failure(
        root,
        &["setup", "--plan", "--tools-only"],
        true,
        "official-only setup rejects Repository configuration",
    );
    assert!(!root.join(".turbo").exists());
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

#[test]
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
fn python_frozen_setup_requires_and_checks_the_uv_lock_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "turbo.json",
        r#"{"futureFlags":{"experimentalSetup":true,"experimentalPythonWorkspaces":true}}"#,
    );
    write(
        root,
        "pyproject.toml",
        "[tool.uv]\nrequired-version = \">=0.13\"\n[tool.uv.workspace]\nmembers = []\n",
    );
    let words = ["setup", "--frozen", "--tools-only"];
    failure(root, &words, false, "frozen mode requires turbo.lock");
    let artifact = r#"{"url":"https://releases.astral.sh/github/uv/releases/download/0.13.0/uv-x86_64-unknown-linux-gnu.tar.gz","sha256":"1468ebd5a5541121837c5a2817b9972ba6090fa6caa3d142620850a47fb75154","format":"tar-gz","rootPrefix":"uv-x86_64-unknown-linux-gnu","executables":{"uv":"uv","uvx":"uvx"}}"#;
    write(
        root,
        "turbo.lock",
        &format!(
            r#"{{"schemaVersion":0,"tools":{{"uv":{{"adapter":"uv","version":"0.13.0","declarations":[{{"file":"pyproject.toml","field":"tool.uv.required-version","request":"==0.13.*"}}],"installation":{{"kind":"managed","artifacts":{{"linux-x64-gnu":{{"distribution":{artifact}}}}}}}}}}}}}"#
        ),
    );
    // The changed declaration is named before any storage or network access.
    for ci in [false, true] {
        let words: &[&str] = if ci {
            &["setup", "--tools-only"]
        } else {
            &words
        };
        failure(
            root,
            words,
            ci,
            "turbo.lock declarations changed: uv: pyproject.toml",
        );
    }
    // --check accepts uv selections: a matching lock without an installation
    // is reported missing on every qualified host, without writes.
    let artifacts: serde_json::Map<_, _> = [
        ("macos-x64", "x86_64-apple-darwin"),
        ("macos-arm64", "aarch64-apple-darwin"),
        ("linux-x64-gnu", "x86_64-unknown-linux-gnu"),
        ("linux-arm64-gnu", "aarch64-unknown-linux-gnu"),
    ]
    .into_iter()
    .map(|(platform, target)| {
        let url = format!(
            "https://releases.astral.sh/github/uv/releases/download/0.13.0/uv-{target}.tar.gz"
        );
        let artifact = serde_json::json!({"url":url,"sha256":"1".repeat(64),"format":"tar-gz",
            "rootPrefix":format!("uv-{target}"),"executables":{"uv":"uv","uvx":"uvx"}});
        (
            platform.to_owned(),
            serde_json::json!({"distribution": artifact}),
        )
    })
    .collect();
    let mut lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("turbo.lock")).unwrap()).unwrap();
    lock["tools"]["uv"]["declarations"][0]["request"] = ">=0.13".into();
    lock["tools"]["uv"]["installation"]["artifacts"] = artifacts.into();
    write(root, "turbo.lock", &lock.to_string());
    for ci in [false, true] {
        check_failure(
            root,
            &["setup", "--check", "--tools-only"],
            ci,
            "managed installation is missing; run turbo setup",
        );
    }
    assert!(!root.join(".turbo").exists());
}
