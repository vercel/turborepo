use assert_cmd::Command;

#[test]
fn test_find_correct_turbo() {
    let mut cmd = Command::cargo_bin("shim").unwrap();
    cmd.assert()
        .append_context("shim", "no arguments")
        .append_context("expect", "`shim` with no arguments should exit with code 1")
        .code(1);

    let mut cmd = Command::cargo_bin("shim").unwrap();
    cmd.args(&["--cwd", "../examples/basic", "bin"])
        .assert()
        .append_context(
            "shim",
            "bin command with cwd flag set to package with local turbo installed",
        )
        .append_context(
            "expect",
            "`shim --cwd ../../examples/basic bin` should print out current turbo binary",
        )
        .success()
        .stdout(predicates::str::ends_with(
            "examples/basic/node_modules/.bin/turbo\n",
        ));

    let mut cmd = Command::cargo_bin("shim").unwrap();
    cmd.args(&["--cwd", "..", "bin"])
        .assert()
        .append_context(
            "shim",
            "bin command with cwd flag set to package without local turbo installed",
        )
        .append_context("expect", "`shim --cwd .. bin` should error")
        .code(2)
        .stderr("failed No local turbo installation found in package.json.\n");
}
