mod cli;
mod commands;
mod package_manager;
mod shim;

use anyhow::Result;
use tiny_gradient::{GradientStr, RGB};
use turbo_updater::check_for_updates;

pub use crate::cli::Args;
use crate::package_manager::PackageManager;

/// The payload from running main, if the program can complete without using Go
/// the Rust variant will be returned. If Go is needed then the turbostate that
/// should be passed to Go will be returned.
pub enum Payload {
    Rust(Result<i32>),
    Go(Box<Args>),
}

fn get_version() -> &'static str {
    include_str!("../../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
}

pub fn main() -> Result<Payload> {
    // custom footer for update message
    let footer = format!(
        "Follow {username} for updates: {url}",
        username = "@turborepo".gradient([RGB::new(0, 153, 247), RGB::new(241, 23, 18)]),
        url = "https://twitter.com/turborepo"
    );

    // check for updates
    let _ = check_for_updates(
        "turbo",
        "https://github.com/vercel/turbo",
        Some(&footer),
        get_version(),
        // use defaults for timeout and refresh interval (800ms and 1 day respectively)
        None,
        None,
    );

    shim::run()
}

#[cfg(test)]
mod test {
    use clap::Parser;
    use itertools::Itertools;
    use pretty_assertions::assert_eq;

    struct CommandTestCase {
        command: &'static str,
        command_args: Vec<Vec<&'static str>>,
        global_args: Vec<Vec<&'static str>>,
        expected_output: Args,
    }

    fn get_default_run_args() -> RunArgs {
        RunArgs {
            cache_workers: 10,
            ..RunArgs::default()
        }
    }

    impl CommandTestCase {
        fn test(&self) {
            let permutations = self.create_all_arg_permutations();
            for command in permutations {
                assert_eq!(Args::try_parse_from(command).unwrap(), self.expected_output)
            }
        }

        fn create_all_arg_permutations(&self) -> Vec<Vec<&'static str>> {
            let mut permutations = Vec::new();
            let mut global_args = vec![vec![self.command]];
            global_args.extend(self.global_args.clone());
            let global_args_len = global_args.len();
            let command_args_len = self.command_args.len();

            // Iterate through all the different permutations of args
            for global_args_permutation in global_args.into_iter().permutations(global_args_len) {
                let command_args = self.command_args.clone();
                for command_args_permutation in
                    command_args.into_iter().permutations(command_args_len)
                {
                    let mut command = vec![vec!["turbo"]];
                    command.extend(global_args_permutation.clone());
                    command.extend(command_args_permutation);
                    permutations.push(command.into_iter().flatten().collect())
                }
            }

            permutations
        }
    }

    use crate::{
        commands::{DryRunMode, OutputLogsMode, RunArgs},
        Args, Command,
    };

    #[test]
    fn test_parse_run() {
        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "lint", "test"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--cache-dir", "foobar"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    cache_dir: Some("foobar".to_string()),
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--cache-workers", "100"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    cache_workers: 100,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--concurrency", "20"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    concurrency: Some("20".to_string()),
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--continue"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    continue_execution: true,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--dry-run"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    dry_run: Some(DryRunMode::Text),
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--dry-run", "json"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    dry_run: Some(DryRunMode::Json),
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo", "run", "build", "--filter", "water", "--filter", "earth", "--filter",
                "fire", "--filter", "air"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    filter: vec![
                        "water".to_string(),
                        "earth".to_string(),
                        "fire".to_string(),
                        "air".to_string()
                    ],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--force"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    force: true,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--global-deps", ".env"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    global_deps: vec![".env".to_string()],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo",
                "run",
                "build",
                "--global-deps",
                ".env",
                "--global-deps",
                ".env.development"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    global_deps: vec![".env".to_string(), ".env.development".to_string()],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--graph"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    graph: Some("".to_string()),
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--graph", "out.html"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    graph: Some("out.html".to_string()),
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--ignore", "foo.js"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    ignore: vec!["foo.js".to_string()],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo", "run", "build", "--ignore", "foo.js", "--ignore", "bar.js"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    ignore: vec!["foo.js".to_string(), "bar.js".to_string()],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--include-dependencies"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    include_dependencies: true,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--no-cache"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    no_cache: true,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--no-daemon"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    no_daemon: true,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--no-deps"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    no_deps: true,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--output-logs", "full"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: OutputLogsMode::Full,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--output-logs", "none"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: OutputLogsMode::None,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--output-logs", "hash-only"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: OutputLogsMode::HashOnly,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--parallel"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    parallel: true,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--profile", "profile_out"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    profile: Some("profile_out".to_string()),
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--remote-only"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    remote_only: true,
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--scope", "foo", "--scope", "bar"])
                .unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    scope: vec!["foo".to_string(), "bar".to_string()],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--since", "foo"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    since: Some("foo".to_string()),
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "build"]).unwrap(),
            Args {
                run_args: Some(RunArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_run_args()
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "build", "lint", "test"]).unwrap(),
            Args {
                run_args: Some(RunArgs {
                    tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                    ..get_default_run_args()
                }),
                ..Args::default()
            }
        );
    }

    #[test]
    fn test_parse_bin() {
        assert_eq!(
            Args::try_parse_from(["turbo", "bin"]).unwrap(),
            Args {
                command: Some(Command::Bin {}),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "bin",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Bin {}),
                cwd: Some("../examples/with-yarn".to_string()),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_login() {
        assert_eq!(
            Args::try_parse_from(["turbo", "login"]).unwrap(),
            Args {
                command: Some(Command::Login { sso_team: None }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "login",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Login { sso_team: None }),
                cwd: Some("../examples/with-yarn".to_string()),
                ..Args::default()
            },
        }
        .test();

        CommandTestCase {
            command: "login",
            command_args: vec![vec!["--sso-team", "my-team"]],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Login {
                    sso_team: Some("my-team".to_string()),
                }),
                cwd: Some("../examples/with-yarn".to_string()),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_logout() {
        assert_eq!(
            Args::try_parse_from(["turbo", "logout"]).unwrap(),
            Args {
                command: Some(Command::Logout {}),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "logout",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Logout {}),
                cwd: Some("../examples/with-yarn".to_string()),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_unlink() {
        assert_eq!(
            Args::try_parse_from(["turbo", "unlink"]).unwrap(),
            Args {
                command: Some(Command::Unlink {}),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "unlink",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Unlink {}),
                cwd: Some("../examples/with-yarn".to_string()),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_prune() {
        let default_prune = Command::Prune {
            scope: Vec::new(),
            docker: false,
            output_dir: "out".to_string(),
        };

        assert_eq!(
            Args::try_parse_from(["turbo", "prune"]).unwrap(),
            Args {
                command: Some(default_prune.clone()),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "prune",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(default_prune),
                cwd: Some("../examples/with-yarn".to_string()),
                ..Args::default()
            },
        }
        .test();

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--scope", "bar"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: vec!["bar".to_string()],
                    docker: false,
                    output_dir: "out".to_string(),
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--docker"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: Vec::new(),
                    docker: true,
                    output_dir: "out".to_string(),
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--out-dir", "dist"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: Vec::new(),
                    docker: false,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "prune",
            command_args: vec![vec!["--out-dir", "dist"], vec!["--docker"]],
            global_args: vec![],
            expected_output: Args {
                command: Some(Command::Prune {
                    scope: Vec::new(),
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            },
        }
        .test();

        CommandTestCase {
            command: "prune",
            command_args: vec![vec!["--out-dir", "dist"], vec!["--docker"]],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Prune {
                    scope: Vec::new(),
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                cwd: Some("../examples/with-yarn".to_string()),
                ..Args::default()
            },
        }
        .test();

        CommandTestCase {
            command: "prune",
            command_args: vec![
                vec!["--out-dir", "dist"],
                vec!["--docker"],
                vec!["--scope", "foo"],
            ],
            global_args: vec![],
            expected_output: Args {
                command: Some(Command::Prune {
                    scope: vec!["foo".to_string()],
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_pass_through_args() {
        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--", "--script-arg=42"]).unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    pass_through_args: vec!["--script-arg=42".to_string()],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo",
                "run",
                "build",
                "--",
                "--script-arg=42",
                "--foo",
                "--bar",
                "bat"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(RunArgs {
                    tasks: vec!["build".to_string()],
                    pass_through_args: vec![
                        "--script-arg=42".to_string(),
                        "--foo".to_string(),
                        "--bar".to_string(),
                        "bat".to_string()
                    ],
                    ..get_default_run_args()
                })),
                ..Args::default()
            }
        );
    }
}
