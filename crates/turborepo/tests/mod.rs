use assert_cmd::Command;

fn get_version() -> &'static str {
    include_str!("../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
}

#[test]
fn test_version() {
    let mut cmd = Command::cargo_bin("turbo").unwrap();
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(format!("{}\n", get_version()));
}

#[test]
fn test_no_arguments() {
    let mut cmd = Command::cargo_bin("turbo").unwrap();
    cmd.assert()
        .append_context("turbo", "no arguments")
        .append_context(
            "expect",
            "`turbo` with no arguments should exit with code 2",
        )
        .code(2);
}

#[test]
fn test_find_correct_turbo() {
    let mut cmd = Command::cargo_bin("turbo").unwrap();
    let assertion = cmd
        .args(["--cwd", "..", "bin"])
        .assert()
        .append_context(
            "turbo",
            "bin command with cwd flag set to package without local turbo installed",
        )
        .append_context(
            "expect",
            "`turbo --cwd .. bin` should print out current turbo binary",
        )
        .success();

    if cfg!(debug_assertions) {
        if cfg!(windows) {
            assertion.stdout(predicates::str::ends_with("target\\debug\\turbo.exe\n"));
        } else {
            assertion.stdout(predicates::str::ends_with("target/debug/turbo\n"));
        }
    } else if cfg!(windows) {
        assertion.stdout(predicates::str::ends_with("target\\release\\turbo.exe\n"));
    } else {
        assertion.stdout(predicates::str::ends_with("target/release/turbo\n"));
    }
}
