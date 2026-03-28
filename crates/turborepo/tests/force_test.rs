mod common;

use common::{run_turbo, run_turbo_with_env, setup};

fn setup_and_prime_cache(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", true).unwrap();
    // Baseline run to populate the cache
    let output = run_turbo(
        dir,
        &["run", "build", "--output-logs=hash-only", "--filter=my-app"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "baseline should be a cache miss"
    );
}

fn run_force_test(
    dir: &std::path::Path,
    env_force: Option<&str>,
    flag: Option<&str>,
    expect_bypass: bool,
) {
    let mut args = vec!["run", "build", "--output-logs=hash-only", "--filter=my-app"];
    if let Some(f) = flag {
        args.push(f);
    }

    let env: Vec<(&str, &str)> = match env_force {
        Some(val) => vec![("TURBO_FORCE", val)],
        None => vec![],
    };

    let output = run_turbo_with_env(dir, &args, &env);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if expect_bypass {
        assert!(
            stdout.contains("cache bypass"),
            "expected cache bypass with env={env_force:?} flag={flag:?}, got:\n{stdout}"
        );
    } else {
        assert!(
            stdout.contains("cache hit"),
            "expected cache hit with env={env_force:?} flag={flag:?}, got:\n{stdout}"
        );
    }
}

// env var=true, missing flag: bypass
#[test]
fn test_force_env_true_no_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), Some("true"), None, true);
}

// env var=true, --force=true: bypass
#[test]
fn test_force_env_true_flag_true() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), Some("true"), Some("--force=true"), true);
}

// env var=true, --force=false: cache hit (flag wins)
#[test]
fn test_force_env_true_flag_false() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), Some("true"), Some("--force=false"), false);
}

// env var=true, --force (no value): bypass
#[test]
fn test_force_env_true_flag_no_value() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), Some("true"), Some("--force"), true);
}

// env var=false, missing flag: cache hit
#[test]
fn test_force_env_false_no_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), Some("false"), None, false);
}

// env var=false, --force=true: bypass
#[test]
fn test_force_env_false_flag_true() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), Some("false"), Some("--force=true"), true);
}

// env var=false, --force=false: cache hit
#[test]
fn test_force_env_false_flag_false() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), Some("false"), Some("--force=false"), false);
}

// env var=false, --force (no value): bypass
#[test]
fn test_force_env_false_flag_no_value() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), Some("false"), Some("--force"), true);
}

// missing env var, missing flag: cache hit
#[test]
fn test_force_no_env_no_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), None, None, false);
}

// missing env var, --force=true: bypass
#[test]
fn test_force_no_env_flag_true() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), None, Some("--force=true"), true);
}

// missing env var, --force=false: cache hit
#[test]
fn test_force_no_env_flag_false() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), None, Some("--force=false"), false);
}

// missing env var, --force (no value): bypass
#[test]
fn test_force_no_env_flag_no_value() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());
    run_force_test(tempdir.path(), None, Some("--force"), true);
}
