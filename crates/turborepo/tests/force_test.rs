#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, run_turbo_with_env, setup};

fn setup_and_prime_cache(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", false).unwrap();
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

#[test]
fn test_force_env_and_flag_precedence() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_and_prime_cache(tempdir.path());

    let cases = [
        // env var=true
        (Some("true"), None, true),
        (Some("true"), Some("--force=true"), true),
        (Some("true"), Some("--force=false"), false),
        (Some("true"), Some("--force"), true),
        // env var=false
        (Some("false"), None, false),
        (Some("false"), Some("--force=true"), true),
        (Some("false"), Some("--force=false"), false),
        (Some("false"), Some("--force"), true),
        // missing env var
        (None, None, false),
        (None, Some("--force=true"), true),
        (None, Some("--force=false"), false),
        (None, Some("--force"), true),
    ];

    for (env_force, flag, expect_bypass) in cases {
        run_force_test(tempdir.path(), env_force, flag, expect_bypass);
    }
}
