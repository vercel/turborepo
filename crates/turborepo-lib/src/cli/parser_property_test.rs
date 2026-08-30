use std::{ffi::OsString, num::NonZeroU32};

use proptest::{
    prelude::*,
    test_runner::{Config, RngAlgorithm, TestRng, TestRunner},
};

use super::Args;

const MAX_ARGS: usize = 32;
const MAX_ARG_BYTES: usize = 128;
const DEFAULT_CASES: u32 = 10_000;

fn argv_strategy() -> impl Strategy<Value = Vec<OsString>> {
    prop::collection::vec(
        prop::collection::vec(any::<u8>(), 0..=MAX_ARG_BYTES),
        0..=MAX_ARGS,
    )
    .prop_map(|raw_args| {
        let mut args = Vec::with_capacity(raw_args.len() + 1);
        args.push(OsString::from("turbo"));
        args.extend(raw_args.into_iter().map(os_string_from_bytes));
        args
    })
}

#[cfg(unix)]
fn os_string_from_bytes(bytes: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes)
}

#[cfg(not(unix))]
fn os_string_from_bytes(bytes: Vec<u8>) -> OsString {
    OsString::from(String::from_utf8_lossy(&bytes).into_owned())
}

fn cases() -> u32 {
    std::env::var("TURBO_CLI_PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

#[test]
fn arbitrary_argv_is_deterministic_and_never_panics() {
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: cases(),
            max_shrink_iters: 10_000,
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &[0x54; 32]),
    );

    runner
        .run(&argv_strategy(), |args| {
            let first = Args::parse_args(args.clone());
            let second = Args::parse_args(args);
            prop_assert_eq!(first, second);
            Ok(())
        })
        .unwrap();
}

#[test]
fn parser_semantic_invariants() {
    let unknown = Args::parse_args(vec![
        "turbo".into(),
        "run".into(),
        "build".into(),
        "--definitely-unknown".into(),
    ]);
    assert!(unknown.is_err(), "unknown flags must be rejected");

    let duplicate_scalar = Args::parse_args(vec![
        "turbo".into(),
        "run".into(),
        "build".into(),
        "--concurrency".into(),
        "1".into(),
        "--concurrency".into(),
        "2".into(),
    ]);
    assert!(
        duplicate_scalar.is_err(),
        "duplicate scalar flags must be rejected"
    );

    let boundary = Args::parse_args(vec![
        "turbo".into(),
        "run".into(),
        "build".into(),
        "--".into(),
        "--definitely-unknown".into(),
        "🦀".into(),
    ])
    .unwrap();
    assert_eq!(
        boundary.execution_args().unwrap().pass_through_args,
        ["--definitely-unknown", "🦀"]
    );

    for args in [
        vec!["turbo", "run", "build", "--dry-run"],
        vec!["turbo", "run", "build", "--dry-run=json"],
        vec!["turbo", "build", "--filter", "应用"],
        vec!["turbo", "g"],
        vec!["turbo", "run", "build", "--color"],
        vec!["turbo", "run", "build", "--color", "--no-update-notifier"],
    ] {
        assert!(Args::parse_args(args.into_iter().map(OsString::from).collect()).is_ok());
    }

    assert!(NonZeroU32::new(cases()).is_some());
}
