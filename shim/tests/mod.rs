use assert_cmd::Command;

#[test]
fn test_find_correct_turbo() {
    let mut cmd = Command::cargo_bin("turbo").unwrap();
    cmd.assert()
        .append_context("turbo", "no arguments")
        .append_context(
            "expect",
            "`turbo` with no arguments should exit with code 1",
        )
        .code(1);

    let mut cmd = Command::cargo_bin("turbo").unwrap();
    cmd.args(&["--cwd", "../examples/basic", "bin"])
        .assert()
        .append_context(
            "turbo",
            "bin command with cwd flag set to package with local turbo installed",
        )
        .append_context(
            "expect",
            "`turbo --cwd ../../examples/basic bin` should print out local turbo binary installed \
             in examples/basic",
        )
        .success()
        .stdout(predicates::str::ends_with(
            "examples/basic/node_modules/.bin/turbo\n",
        ));

    let mut cmd = Command::cargo_bin("turbo").unwrap();
    cmd.args(&["--cwd", "..", "bin"])
        .assert()
        .append_context(
            "turbo",
            "bin command with cwd flag set to package without local turbo installed",
        )
        .append_context(
            "expect",
            "`turbo --cwd .. bin` should print out current turbo binary",
        )
        .success()
        .stdout(predicates::str::ends_with("target/debug/turbo\n"));
}
