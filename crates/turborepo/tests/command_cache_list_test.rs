#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{combined_output, run_turbo, setup};

#[test]
fn test_cache_list_shows_locally_cached_task() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["cache", "list"]);
    assert!(output.status.success(), "{}", combined_output(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("No artifacts in the local cache at "),
        "{stdout}"
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    assert!(output.status.success(), "{}", combined_output(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hash = stdout
        .lines()
        .find_map(|line| {
            let prefix = "cache miss, executing ";
            let rest = &line[line.find(prefix)? + prefix.len()..];
            rest.split_whitespace().next()
        })
        .expect("could not find hash in output")
        .to_string();

    let list = run_turbo(tempdir.path(), &["cache", "list"]);
    assert!(list.status.success(), "{}", combined_output(&list));
    let stdout = String::from_utf8_lossy(&list.stdout);
    let mut lines = stdout.lines();
    let header = lines.next().unwrap();
    assert!(
        header.starts_with("HASH") && header.ends_with("CACHED AT"),
        "{stdout}"
    );
    let entry = lines.next().unwrap();
    assert_eq!(
        entry.split_whitespace().next(),
        Some(hash.as_str()),
        "{stdout}"
    );
    assert_eq!(lines.next(), None, "{stdout}");

    let ls = run_turbo(tempdir.path(), &["cache", "ls"]);
    assert!(ls.status.success(), "{}", combined_output(&ls));
    assert_eq!(ls.stdout, list.stdout);
}
