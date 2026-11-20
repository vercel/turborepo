//! The test harness the command tests run: `examples/harness.rs`, which `cargo
//! test` builds with the tests (add `--example harness` when selecting tests
//! with `--test`).

use std::path::PathBuf;

/// The harness binary next to this test binary
/// (`target/<profile>/examples/harness`).
pub fn harness() -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    let profile = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<profile>/deps");
    let p = profile
        .join("examples")
        .join(format!("harness{}", std::env::consts::EXE_SUFFIX));
    assert!(
        p.exists(),
        "{} is missing: run the tests with `cargo test -p turborepo-trace` (or add `--example \
         harness`)",
        p.display()
    );
    p
}

/// A directory for test repositories outside the temporary directory and
/// outside any package: cargo's test directory, unless (as in the turbo
/// checkout) a `package.json` above it would make it part of a package; then
/// `~/.cache/turborepo-trace-tests` (not `/var/tmp`: reads below a temporary
/// directory are a case of their own).
#[allow(dead_code)]
pub fn outside_tmp() -> PathBuf {
    let in_a_package =
        |d: &std::path::Path| d.ancestors().any(|a| a.join("package.json").is_file());
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    if !in_a_package(&target) {
        return target;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
    let dir = home.join(".cache/turborepo-trace-tests");
    assert!(
        !in_a_package(&dir),
        "{} is below a package.json",
        dir.display()
    );
    std::fs::create_dir_all(&dir).expect("create the test directory");
    dir
}
