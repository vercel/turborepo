//! Fresh processes exercise real cached CI detection, parser, discovery,
//! loopback resolution/provisioning and publication. Never mutate host env.
use std::{path::Path, process::Command, time::SystemTime};

use super::*;

const CASE: &str = "TURBO_SETUP_CI_FIXTURE_CASE";

fn process(case: &str, ci_variable: Option<&str>) {
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env(CASE, case)
        .args([
            "--exact",
            "commands::setup::provisioning_tests::first_lock::ci::fixture_process",
            "--nocapture",
        ]);
    if let Some(variable) = ci_variable {
        child.env(variable, "1");
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "{case}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "child filter did not run"
    );
}

#[test]
fn ci_missing_lock_is_actionable_and_read_only() {
    process("missing", Some("CI"));
}
#[test]
fn ci_stale_lock_names_changed_and_removed_declarations() {
    process("stale", Some("BUILD_NUMBER"));
}
#[test]
fn ci_valid_lock_provisions_without_rewriting_locks() {
    process("valid", Some("CI"));
}
#[test]
fn local_setup_still_creates_lock() {
    process("local", None);
}
#[test]
fn ci_no_frozen_creates_and_updates_lock() {
    process("no-frozen", Some("CI"));
}
#[test]
fn ci_refresh_requires_explicit_no_frozen() {
    process("refresh", Some("CI"));
}

#[derive(Debug, PartialEq, Eq)]
struct Entry(PathBuf, Vec<u8>, SystemTime, u32);
fn state(root: &Path) -> Vec<Entry> {
    super::super::publication_guards::state(root)
        .into_iter()
        .map(|(path, bytes)| {
            let metadata = fs::symlink_metadata(&path).unwrap();
            use std::os::unix::fs::MetadataExt;
            Entry(path, bytes, metadata.modified().unwrap(), metadata.mode())
        })
        .collect()
}
fn text(error: Error) -> String {
    format!("{:?}", miette::Report::new(error)).replace([' ', '|', '\n', '\r'], "")
}
fn guidance(text: &str) {
    assert!(text.contains("turbosetup--tools-only"), "{text}");
    assert!(text.contains("locally"), "{text}");
    assert!(
        text.contains("committurbo.lock") || text.contains("committheupdatedturbo.lock"),
        "{text}"
    );
}

#[test]
fn fixture_process() {
    let Ok(case) = std::env::var(CASE) else {
        return;
    };
    assert_eq!(turborepo_ci::is_ci(), case != "local");
    match case.as_str() {
        "missing" => {
            let f = fresh();
            let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
            let before = state(f.owned.root());
            let message = text(f.run(&u, &["--tools-only"]).unwrap_err());
            assert!(
                message.contains("frozenmoderequiresturbo.lock"),
                "{message}"
            );
            guidance(&message);
            assert_eq!(u.hits(), 0);
            assert_eq!(state(f.owned.root()), before);
            untouched(&f);
        }
        "stale" => {
            // Added, changed and removed provenance; declaration values remain
            // syntactically valid so the frozen lock, not discovery, rejects it.
            for change in ["node", "manager", "removed", "added"] {
                let f = Fixture::new();
                let (file, field) = match change {
                    "node" => {
                        fs::write(f.owned.root().join(".nvmrc"), "24.x\n").unwrap();
                        (".nvmrc", None)
                    }
                    "manager" => {
                        fs::write(
                            f.owned.root().join("package.json"),
                            r#"{"packageManager":"pnpm@10.1.0"}"#,
                        )
                        .unwrap();
                        ("package.json", Some("/packageManager"))
                    }
                    "removed" => {
                        fs::remove_file(f.owned.root().join(".nvmrc")).unwrap();
                        (".nvmrc", None)
                    }
                    _ => {
                        fs::write(f.owned.root().join(".node-version"), "24.0.0\n").unwrap();
                        (".node-version", None)
                    }
                };
                let u = Upstream::new(&f, false, None);
                let before = state(f.owned.root());
                let message = text(f.run(&u, &["--tools-only"]).unwrap_err());
                assert!(
                    message.contains("turbo.lockdeclarationschanged"),
                    "{message}"
                );
                assert!(message.contains(file), "{message}");
                if let Some(field) = field {
                    assert!(message.contains(field), "{message}");
                }
                guidance(&message);
                assert_eq!(u.hits(), 0);
                assert_eq!(state(f.owned.root()), before);
                f.no_execution();
            }
        }
        "valid" => {
            let f = Fixture::new();
            let u = Upstream::new(&f, false, None);
            let lock = f.owned.root().join("turbo.lock");
            let bytes = fs::read(&lock).unwrap();
            let modified = fs::metadata(&lock).unwrap().modified().unwrap();
            assert_eq!(f.run(&u, &["--tools-only"]).unwrap(), 0);
            assert_eq!(u.hits(), 3); // Node artifact + exact pnpm metadata/artifact; no resolution.
            assert!(f.manifest().is_some());
            assert_eq!(fs::read(&lock).unwrap(), bytes);
            assert_eq!(fs::metadata(&lock).unwrap().modified().unwrap(), modified);
            let before = state(f.owned.root());
            assert_eq!(f.run(&u, &["--tools-only"]).unwrap(), 0);
            assert_eq!(u.hits(), 3);
            assert_eq!(state(f.owned.root()), before);
            f.no_execution();
        }
        "local" | "no-frozen" => {
            let f = fresh();
            let flags = if case == "local" {
                &["--tools-only"][..]
            } else {
                LOCAL
            };
            let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
            assert_eq!(f.run(&u, flags).unwrap(), 0);
            assert_eq!(selected(&f).tools()["node"].version, "24.0.0");
            assert!(f.manifest().is_some());
            untouched(&f);
            if case == "no-frozen" {
                fs::write(
                    f.owned.root().join("package.json"),
                    r#"{"packageManager":"pnpm@10.1.0"}"#,
                )
                .unwrap();
                let u = LoopbackServer::new(routes(&f, "24.0.0", "10.1.0"), |_| {}).unwrap();
                assert_eq!(f.run(&u, flags).unwrap(), 0);
                assert_eq!(selected(&f).tools()["pnpm"].version, "10.1.0");
                untouched(&f);
            }
        }
        "refresh" => {
            let f = fresh();
            let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
            let before = state(f.owned.root());
            let message = text(f.run(&u, &["--tools-only", "--update-lock"]).unwrap_err());
            assert!(message.contains("--no-frozen--update-lock"), "{message}");
            assert_eq!(state(f.owned.root()), before);
            assert_eq!(u.hits(), 0);
            assert_eq!(
                f.run(&u, &["--tools-only", "--no-frozen", "--update-lock"])
                    .unwrap(),
                0
            );
            assert!(f.owned.root().join("turbo.lock").exists());
            untouched(&f);
        }
        _ => panic!("unknown fixture case {case}"),
    }
}
