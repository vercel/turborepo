use assert_cmd::Command;

static TURBO_HELP: &str = "The build system that makes ship happen

Usage: turbo [OPTIONS] [TASKS]... [COMMAND]

Commands:
  bin         Get the path to the Turbo binary
  completion  Generate the autocompletion script for the specified shell
  daemon      Runs the Turborepo background daemon
  help        Help about any command
  link        Link your local directory to a Vercel organization and enable remote caching
  login       Login to your Vercel account
  logout      Logout to your Vercel account
  prune       Prepare a subset of your monorepo
  run         Run tasks across projects in your monorepo
  unlink      Unlink the current directory from your Vercel organization and disable Remote Caching

Arguments:
  [TASKS]...  

Options:
  -h, --help                     
      --version                  
      --api <API>                Override the endpoint for API calls
      --color                    Force color usage in the terminal
      --cpuprofile <CPUPROFILE>  Specify a file to save a cpu profile
      --cwd <CWD>                The directory in which to run turbo
      --heap <HEAP>              Specify a file to save a pprof heap profile
      --login <LOGIN>            Override the login endpoint
      --no-color                 Suppress color usage in the terminal
      --preflight                When enabled, turbo will precede HTTP requests with an OPTIONS \
                           request for authorization
      --team <TEAM>              Set the team slug for API calls
      --token <TOKEN>            Set the auth token for API calls
      --trace <TRACE>            Specify a file to save a pprof trace
  -v, --verbosity <VERBOSITY>    verbosity
";

fn get_version() -> &'static str {
    include_str!("../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
}

#[test]
fn test_help() {
    let mut cmd = Command::cargo_bin("turbo").unwrap();
    cmd.arg("--help");
    cmd.assert().success().stdout(TURBO_HELP);

    let mut cmd = Command::cargo_bin("turbo").unwrap();
    cmd.arg("-h");
    cmd.assert().success().stdout(TURBO_HELP);

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
            "`turbo` with no arguments should exit with code 1",
        )
        .code(1);
}

#[test]
fn test_find_turbo_in_example() {
    let mut cmd = Command::cargo_bin("turbo").unwrap();
    cmd.args(["--cwd", "../examples/basic", "bin"])
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
        .stdout(predicates::str::ends_with({
            #[cfg(target_os = "linux")]
            {
                "examples/basic/node_modules/turbo/bin/turbo\n"
            }
            #[cfg(target_os = "macos")]
            {
                "examples/basic/node_modules/.bin/turbo\n"
            }
            #[cfg(target_os = "windows")]
            {
                #[cfg(target_arch = "x86_64")]
                {
                    "examples\\basic\\node_modules\\turbo-windows-64\\bin\\turbo.exe\n"
                }
                #[cfg(target_arch = "aarch64")]
                {
                    "examples\\basic\\node_modules\\turbo-windows-arm64\\bin\\turbo.exe\n"
                }
            }
        }));
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
    } else {
        if cfg!(windows) {
            assertion.stdout(predicates::str::ends_with("target\\release\\turbo.exe\n"));
        } else {
            assertion.stdout(predicates::str::ends_with("target/release/turbo\n"));
        }
    }
}
