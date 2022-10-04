use assert_cmd::Command;

#[test]
fn test_find_correct_turbo() {
    // `shim` with no arguments should exit with code 1
    let mut cmd = Command::cargo_bin("shim").unwrap();
    cmd.assert().append_context("shim", "no arguments").code(1);

    // `shim --cwd ../../examples/basic bin` should print out local turbo binary
    let mut cmd = Command::cargo_bin("shim").unwrap();
    cmd.args(&["--cwd", "../examples/basic", "bin"])
        .assert()
        .append_context(
            "shim",
            "bin command with cwd flag set to package with local turbo installed",
        )
        .success()
        .stdout(predicates::str::ends_with(
            "../examples/basic/node_modules/.bin/turbo\n",
        ));

    // `shim --cwd .. bin` should print out shim binary
    let mut cmd = Command::cargo_bin("shim").unwrap();
    cmd.args(&["--cwd", "..", "bin"])
        .assert()
        .append_context(
            "shim",
            "bin command with cwd flag set to package without local turbo installed",
        )
        .success()
        .stdout(predicates::str::ends_with("shim/target/debug/shim\n"));
}
