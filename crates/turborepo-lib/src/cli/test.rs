use std::{assert_matches, ffi::OsString};

use camino::Utf8PathBuf;
use clap::{CommandFactory, Parser};
use insta::assert_snapshot;
use itertools::Itertools;
use pretty_assertions::assert_eq;

use crate::cli::{ExecutionArgs, RunArgs};

fn get_subcommand(name: &str) -> clap::Command {
    Args::command()
        .find_subcommand(name)
        .unwrap_or_else(|| panic!("subcommand '{name}' not found"))
        .clone()
}

#[test_case::test_case("", None ; "root")]
#[test_case::test_case("apps/web", Some("apps/web") ; "workspace")]
#[test_case::test_case("crates", Some("crates") ; "plain directory")]
#[test_case::test_case("crates/super-crate/tests/test-package", Some("crates/super-crate/tests/test-package") ; "nested package")]
#[test_case::test_case("packages/ui-library/src", Some("packages/ui-library/src") ; "nested source directory")]
fn inferred_package_root_returns_repo_relative_invocation_path(
    invocation_suffix: &str,
    expected: Option<&str>,
) {
    let tmp = tempfile::tempdir().unwrap();
    let repo_root = turbopath::AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let invocation_path = if invocation_suffix.is_empty() {
        repo_root.clone()
    } else {
        repo_root.join_unix_path(turbopath::RelativeUnixPathBuf::new(invocation_suffix).unwrap())
    };
    invocation_path.ensure_dir().unwrap();
    let invocation_path = camino::Utf8Path::from_path(invocation_path.as_std_path()).unwrap();

    assert_eq!(
        super::inferred_package_root(invocation_path, &repo_root),
        expected.map(str::to_string)
    );
}

#[test_case::test_case(vec!["turbo", "run", "build"], None ; "missing")]
#[test_case::test_case(vec!["turbo", "run", "build", "--summarize"], Some(true) ; "bare flag")]
#[test_case::test_case(vec!["turbo", "run", "build", "--summarize=true"], Some(true) ; "enabled")]
#[test_case::test_case(vec!["turbo", "run", "build", "--summarize=false"], Some(false) ; "disabled")]
fn run_args_summarize_parses_optional_boolean(args: Vec<&str>, expected: Option<bool>) {
    let args = Args::try_parse_from(args).unwrap();
    let Command::Run { run_args, .. } = args.command.unwrap() else {
        panic!("expected run command");
    };

    assert_eq!(run_args.summarize(), expected);
}

#[test]
fn turbo_short_help() {
    let mut cmd = Args::command();
    let mut buf = Vec::new();
    cmd.write_help(&mut buf).unwrap();
    assert_snapshot!(String::from_utf8(buf).unwrap());
}

#[test]
fn turbo_long_help() {
    let mut cmd = Args::command();
    let mut buf = Vec::new();
    cmd.write_long_help(&mut buf).unwrap();
    assert_snapshot!(String::from_utf8(buf).unwrap());
}

#[test]
fn link_short_help() {
    let mut cmd = get_subcommand("link");
    let mut buf = Vec::new();
    cmd.write_help(&mut buf).unwrap();
    assert_snapshot!(String::from_utf8(buf).unwrap());
}

#[test]
fn unlink_short_help() {
    let mut cmd = get_subcommand("unlink");
    let mut buf = Vec::new();
    cmd.write_help(&mut buf).unwrap();
    assert_snapshot!(String::from_utf8(buf).unwrap());
}

#[test]
fn login_short_help() {
    let mut cmd = get_subcommand("login");
    let mut buf = Vec::new();
    cmd.write_help(&mut buf).unwrap();
    assert_snapshot!(String::from_utf8(buf).unwrap());
}

#[test]
fn logout_short_help() {
    let mut cmd = get_subcommand("logout");
    let mut buf = Vec::new();
    cmd.write_help(&mut buf).unwrap();
    assert_snapshot!(String::from_utf8(buf).unwrap());
}

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

fn get_default_execution_args() -> ExecutionArgs {
    ExecutionArgs {
        output_logs: None,
        framework_inference: true,
        ..ExecutionArgs::default()
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
            for command_args_permutation in command_args.into_iter().permutations(command_args_len)
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

use turborepo_types::{ContinueMode, DryRunMode, EnvMode, LogOrder, LogPrefix, OutputLogsMode};

use crate::cli::{Args, Command};

#[test_case::test_case(
    &["turbo", "run", "build"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "default case"
)]
#[test_case::test_case(
    &["turbo", "run", "build"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                framework_inference: true,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "framework_inference: default to true"
)]
#[test_case::test_case(
		&["turbo", "run", "build", "--framework-inference"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                 tasks: vec!["build".to_string()],
                 framework_inference: true,
                 ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "framework_inference: flag only"
)]
#[test_case::test_case(
		&["turbo", "run", "build", "--framework-inference", "true"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                framework_inference: true,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
		} ;
    "framework_inference: flag set to true"
)]
#[test_case::test_case(
		&["turbo", "run", "build", "--framework-inference",
"false"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                framework_inference: false,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
		} ;
    "framework_inference: flag set to false"
	)]
#[test_case::test_case(
    &["turbo", "run", "build", "--env-mode"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                env_mode: Some(EnvMode::Strict),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "env_mode: not fully-specified"
)]
#[test_case::test_case(
		&["turbo", "run", "build", "--env-mode", "loose"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                env_mode: Some(EnvMode::Loose),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
		} ;
    "env_mode: specified loose"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--env-mode", "strict"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                env_mode: Some(EnvMode::Strict),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
		} ;
    "env_mode: specified strict"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "lint", "test"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "multiple tasks"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--cache-dir", "foobar"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                cache_dir: Some(Utf8PathBuf::from("foobar")),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "cache dir"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--cache-workers", "100"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec ! ["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                cache_workers: 100,
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "cache workers"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--concurrency", "20"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                concurrency: Some("20".to_string()),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "concurrency"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--continue"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                continue_execution: ContinueMode::Always,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "continue option with no value"
	)]
#[test_case::test_case(
		&["turbo", "run", "--continue", "build"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                continue_execution: ContinueMode::Always,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "continue option with no value before task"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--continue=dependencies-successful"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                continue_execution: ContinueMode::DependenciesSuccessful,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "continue option with explicit value"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--dry-run"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                dry_run: Some(DryRunMode::Text),
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "dry run"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--dry-run", "json"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                dry_run: Some(DryRunMode::Json),
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "dry run json"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--filter", "water", "--filter", "earth", "--filter", "fire", "--filter", "air"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                filter: vec![
                    "water".to_string(),
                    "earth".to_string(),
                    "fire".to_string(),
                    "air".to_string()
                ],
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "multiple filters"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "-F", "water", "-F", "earth", "-F", "fire", "-F", "air"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                filter: vec![
                    "water".to_string(),
                    "earth".to_string(),
                    "fire".to_string(),
                    "air".to_string()
                ],
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "multiple filters short"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--filter", "water", "-F", "earth", "--filter", "fire", "-F", "air"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                filter: vec![
                    "water".to_string(),
                    "earth".to_string(),
                    "fire".to_string(),
                    "air".to_string()
                ],
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "multiple filters short and long"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--force"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                force: Some(Some(true)),
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "force"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--global-deps", ".env"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                global_deps: vec![".env".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "global deps"
	)]
#[test_case::test_case(
		&[ "turbo", "run", "build", "--global-deps", ".env", "--global-deps", ".env.development"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                global_deps: vec![".env".to_string(), ".env.development".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "multiple global deps"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--graph"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                graph: Some("".to_string()),
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "graph"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--graph", "out.html"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                graph: Some("out.html".to_string()),
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "graph with output"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--no-cache"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                no_cache: true,
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "no cache"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--only"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                only: true,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "only"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--no-daemon"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                no_daemon: true,
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "no daemon"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--daemon"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                daemon: true,
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "daemon"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--output-logs", "full"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                output_logs: Some(OutputLogsMode::Full),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "output logs full"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--output-logs", "none"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                output_logs: Some(OutputLogsMode::None),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "output logs none"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--output-logs", "hash-only"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                output_logs: Some(OutputLogsMode::HashOnly),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "output logs hash only"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--log-order", "stream"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                log_order: Some(LogOrder::Stream),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "log order stream"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--log-order", "grouped"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                log_order: Some(LogOrder::Grouped),
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    };
    "log order grouped"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--log-prefix", "auto"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                log_prefix: LogPrefix::Auto,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "log prefix auto"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--log-prefix", "none"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                log_prefix: LogPrefix::None,
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "log prefix none"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--log-prefix", "task"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                 tasks: vec!["build".to_string()],
                 log_prefix: LogPrefix::Task,
                 ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "log prefix task"
	)]
#[test_case::test_case(
		&["turbo", "run", "build"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(get_default_run_args())
        }),
        ..Args::default()
    } ;
    "just build"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--parallel"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                parallel: true,
                ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "parallel"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--profile", "profile_out"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
              profile: Some("profile_out".to_string()),
              ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "profile"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--profile"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
              profile: Some(String::new()),
              ..get_default_run_args()
            })
        }),
        ..Args::default()
    } ;
    "profile_no_value"
	)]
// remote-only flag tests
#[test_case::test_case(
		&["turbo", "run", "build"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                remote_only: None,
                ..get_default_run_args()
            })
        }),
        ..Args::default()
		} ;
    "remote_only default to false"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--remote-only"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                remote_only: Some(Some(true)),
                ..get_default_run_args()
            })
        }),
        ..Args::default()
		} ;
    "remote_only with no value, means true"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--remote-only", "true"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                remote_only: Some(Some(true)),
                ..get_default_run_args()
            })
        }),
        ..Args::default()
		} ;
    "remote_only=true works"
	)]
#[test_case::test_case(
		&["turbo", "run", "build", "--remote-only", "false"],
    Args {
        command: Some(Command::Run {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            run_args: Box::new(RunArgs {
                remote_only: Some(Some(false)),
                ..get_default_run_args()
            })
        }),
        ..Args::default()
		} ;
    "remote_only=false works"
	)]
#[test_case::test_case(
		&["turbo", "build"],
    Args {
        execution_args: Some(ExecutionArgs {
            tasks: vec!["build".to_string()],
            ..get_default_execution_args()
        }),
        ..Args::default()
    } ;
    "build no run prefix"
)]
#[test_case::test_case(
	&["turbo", "build", "lint", "test"],
    Args {
        execution_args: Some(ExecutionArgs {
            tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
            ..get_default_execution_args()
        }),
        ..Args::default()
    } ;
    "multiple tasks no run prefix"
)]
fn test_parse_run(args: &[&str], expected: Args) {
    assert_eq!(Args::try_parse_from(args).unwrap(), expected);
}

#[test_case::test_case(
    &["turbo", "watch", "build"],
    Args {
        command: Some(Command::Watch {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            experimental_write_cache: false
        }),
        ..Args::default()
    };
    "default watch"
)]
#[test_case::test_case(
    &["turbo", "watch", "build", "--cache-dir", "foobar"],
    Args {
        command: Some(Command::Watch {
            execution_args: Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                cache_dir: Some(Utf8PathBuf::from("foobar")),
                ..get_default_execution_args()
            }),
            experimental_write_cache: false
        }),
        ..Args::default()
    };
    "with cache-dir"
)]
#[test_case::test_case(
    &["turbo", "watch", "build", "lint", "check"],
    Args {
        command: Some(Command::Watch {
            execution_args: Box::new(ExecutionArgs {
              tasks: vec!["build".to_string(), "lint".to_string(), "check".to_string()],
              ..get_default_execution_args()
            }),
            experimental_write_cache: false
        }),
        ..Args::default()
    };
    "with multiple tasks"
)]
#[test_case::test_case(
    &["turbo", "watch", "build", "--experimental-write-cache"],
    Args {
        command: Some(Command::Watch {
            execution_args: Box::new(ExecutionArgs {
              tasks: vec!["build".to_string()],
              ..get_default_execution_args()
            }),
            experimental_write_cache: true
        }),
        ..Args::default()
    };
    "with experimental-write-cache"
)]
fn test_parse_watch(args: &[&str], expected: Args) {
    assert_eq!(Args::try_parse_from(args).unwrap(), expected);
}

#[test_case::test_case(
    &["turbo", "run", "build", "--daemon", "--no-daemon"],
    "cannot be used with '--no-daemon'" ;
    "daemon and no-daemon at the same time"
)]
#[test_case::test_case(
    &["turbo", "run", "build", "--since", "foo"],
    "unexpected argument '--since' found" ;
    "since without filter or scope"
)]
#[test_case::test_case(
    &["turbo", "run", "build", "--include-dependencies"],
    "unexpected argument '--include-dependencies' found" ;
    "include-dependencies without filter or scope"
)]
#[test_case::test_case(
    &["turbo", "run", "build", "--no-deps"],
    "unexpected argument '--no-deps' found" ;
    "no-deps without filter or scope"
)]
#[test_case::test_case(
    &["turbo", "run", "build", "--log-prefix=blah"],
    "invalid value 'blah' for '--log-prefix" ;
    "invalid log prefix"
)]
#[test_case::test_case(
    &["turbo", "run", "build", "--log-prefix"],
    "a value is required for '--log-prefix" ;
    "missing log prefix value"
)]
#[test_case::test_case(
    &["turbo", "run", "build", "-v", "--verbosity=1"],
    "cannot be used with" ;
    "verbosity flags conflict"
)]
fn test_parse_run_failures(args: &[&str], expected: &str) {
    assert_matches!(
        Args::try_parse_from(args),
        Err(err) if err.to_string().contains(expected)
    );
}

#[test_case::test_case(&["turbo", "run", "build", "-v"], 1 ; "short once")]
#[test_case::test_case(&["turbo", "run", "build", "-vv"], 2 ; "short twice")]
#[test_case::test_case(&["turbo", "run", "build", "--verbosity=1"], 1 ; "long one")]
#[test_case::test_case(&["turbo", "run", "build", "--verbosity=2"], 2 ; "long two")]
fn test_parse_verbosity(args: &[&str], expected: u8) {
    let args = Args::try_parse_from(args).unwrap();

    assert_eq!(u8::from(args.verbosity), expected);
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
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
            ..Args::default()
        },
    }
    .test();
}

#[test]
fn test_parse_link() {
    assert_eq!(
        Args::try_parse_from(["turbo", "link"]).unwrap(),
        Args {
            command: Some(Command::Link {
                no_gitignore: false,
                scope: None,
                yes: false,
            }),
            ..Args::default()
        }
    );

    CommandTestCase {
        command: "link",
        command_args: vec![],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(Command::Link {
                no_gitignore: false,
                scope: None,
                yes: false,
            }),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
            ..Args::default()
        },
    }
    .test();

    CommandTestCase {
        command: "link",
        command_args: vec![vec!["--yes"]],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(Command::Link {
                yes: true,
                no_gitignore: false,
                scope: None,
            }),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
            ..Args::default()
        },
    }
    .test();

    CommandTestCase {
        command: "link",
        command_args: vec![vec!["--scope", "foo"]],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(Command::Link {
                yes: false,
                no_gitignore: false,
                scope: Some("foo".to_string()),
            }),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
            ..Args::default()
        },
    }
    .test();

    CommandTestCase {
        command: "link",
        command_args: vec![vec!["--no-gitignore"]],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(Command::Link {
                yes: false,
                no_gitignore: true,
                scope: None,
            }),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
            command: Some(Command::Login {
                sso_team: None,
                force: false,
                manual: false,
            }),
            ..Args::default()
        }
    );

    CommandTestCase {
        command: "login",
        command_args: vec![],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(Command::Login {
                sso_team: None,
                force: false,
                manual: false,
            }),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
                force: false,
                manual: false,
            }),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
            command: Some(Command::Logout { invalidate: true }),
            ..Args::default()
        }
    );

    CommandTestCase {
        command: "logout",
        command_args: vec![],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(Command::Logout { invalidate: true }),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
            ..Args::default()
        },
    }
    .test();

    assert_eq!(
        Args::try_parse_from(["turbo", "logout", "--invalidate=false"]).unwrap(),
        Args {
            command: Some(Command::Logout { invalidate: false }),
            ..Args::default()
        }
    );
}

#[test]
fn test_parse_unlink() {
    assert_eq!(
        Args::try_parse_from(["turbo", "unlink"]).unwrap(),
        Args {
            command: Some(Command::Unlink),
            ..Args::default()
        }
    );

    CommandTestCase {
        command: "unlink",
        command_args: vec![],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(Command::Unlink),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
            ..Args::default()
        },
    }
    .test();
}

#[test]
fn test_parse_prune() {
    let default_prune = Command::Prune {
        scope: None,
        scope_arg: Some(vec!["foo".into()]),
        docker: false,
        output_dir: "out".to_string(),
        use_gitignore: None,
    };

    assert_eq!(
        Args::try_parse_from(["turbo", "prune", "foo"]).unwrap(),
        Args {
            command: Some(default_prune.clone()),
            ..Args::default()
        }
    );

    CommandTestCase {
        command: "prune",
        command_args: vec![vec!["foo"]],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(default_prune),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
            ..Args::default()
        },
    }
    .test();

    assert_eq!(
        Args::try_parse_from(["turbo", "prune", "--scope", "bar"]).unwrap(),
        Args {
            command: Some(Command::Prune {
                scope: Some(vec!["bar".to_string()]),
                scope_arg: None,
                docker: false,
                output_dir: "out".to_string(),
                use_gitignore: None,
            }),
            ..Args::default()
        }
    );

    assert_eq!(
        Args::try_parse_from(["turbo", "prune", "foo", "bar"]).unwrap(),
        Args {
            command: Some(Command::Prune {
                scope: None,
                scope_arg: Some(vec!["foo".to_string(), "bar".to_string()]),
                docker: false,
                output_dir: "out".to_string(),
                use_gitignore: None,
            }),
            ..Args::default()
        }
    );

    assert_eq!(
        Args::try_parse_from(["turbo", "prune", "--docker", "foo"]).unwrap(),
        Args {
            command: Some(Command::Prune {
                scope: None,
                scope_arg: Some(vec!["foo".into()]),
                docker: true,
                output_dir: "out".to_string(),
                use_gitignore: None,
            }),
            ..Args::default()
        }
    );

    assert_eq!(
        Args::try_parse_from(["turbo", "prune", "--out-dir", "dist", "foo"]).unwrap(),
        Args {
            command: Some(Command::Prune {
                scope: None,
                scope_arg: Some(vec!["foo".into()]),
                docker: false,
                output_dir: "dist".to_string(),
                use_gitignore: None,
            }),
            ..Args::default()
        }
    );

    CommandTestCase {
        command: "prune",
        command_args: vec![vec!["foo"], vec!["--out-dir", "dist"], vec!["--docker"]],
        global_args: vec![],
        expected_output: Args {
            command: Some(Command::Prune {
                scope: None,
                scope_arg: Some(vec!["foo".into()]),
                docker: true,
                output_dir: "dist".to_string(),
                use_gitignore: None,
            }),
            ..Args::default()
        },
    }
    .test();

    CommandTestCase {
        command: "prune",
        command_args: vec![vec!["foo"], vec!["--out-dir", "dist"], vec!["--docker"]],
        global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
        expected_output: Args {
            command: Some(Command::Prune {
                scope: None,
                scope_arg: Some(vec!["foo".into()]),
                docker: true,
                output_dir: "dist".to_string(),
                use_gitignore: None,
            }),
            cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
                scope: Some(vec!["foo".to_string()]),
                scope_arg: None,
                docker: true,
                output_dir: "dist".to_string(),
                use_gitignore: None,
            }),
            ..Args::default()
        },
    }
    .test();

    CommandTestCase {
        command: "prune",
        command_args: vec![vec!["foo"], vec!["--use-gitignore"]],
        global_args: vec![],
        expected_output: Args {
            command: Some(Command::Prune {
                scope: None,
                scope_arg: Some(vec!["foo".to_string()]),
                docker: false,
                output_dir: "out".to_string(),
                use_gitignore: Some(true),
            }),
            ..Args::default()
        },
    }
    .test();

    CommandTestCase {
        command: "prune",
        command_args: vec![vec!["foo"], vec!["--use-gitignore=true"]],
        global_args: vec![],
        expected_output: Args {
            command: Some(Command::Prune {
                scope: None,
                scope_arg: Some(vec!["foo".to_string()]),
                docker: false,
                output_dir: "out".to_string(),
                use_gitignore: Some(true),
            }),
            ..Args::default()
        },
    }
    .test();

    CommandTestCase {
        command: "prune",
        command_args: vec![vec!["foo"], vec!["--use-gitignore=false"]],
        global_args: vec![],
        expected_output: Args {
            command: Some(Command::Prune {
                scope: None,
                scope_arg: Some(vec!["foo".to_string()]),
                docker: false,
                output_dir: "out".to_string(),
                use_gitignore: Some(false),
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
            command: Some(Command::Run {
                run_args: Box::new(RunArgs {
                    ..get_default_run_args()
                }),
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    pass_through_args: vec!["--script-arg=42".to_string()],
                    ..get_default_execution_args()
                }),
            }),
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
            command: Some(Command::Run {
                run_args: Box::new(RunArgs {
                    ..get_default_run_args()
                }),
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    pass_through_args: vec![
                        "--script-arg=42".to_string(),
                        "--foo".to_string(),
                        "--bar".to_string(),
                        "bat".to_string()
                    ],
                    ..get_default_execution_args()
                }),
            }),
            ..Args::default()
        }
    );
}

#[test]
fn test_parse_prune_no_mixed_arg_and_flag() {
    assert!(Args::try_parse_from(["turbo", "prune", "foo", "--scope", "bar"]).is_err(),);
}

#[test]
fn test_parse_gen() {
    let default_gen = Command::Generate {
        tag: None,
        generator_name: None,
        config: None,
        root: None,
        args: vec![],
        command: None,
    };

    assert_eq!(
        Args::try_parse_from(["turbo", "gen"]).unwrap(),
        Args {
            command: Some(default_gen.clone()),
            ..Args::default()
        }
    );

    assert_eq!(
        Args::try_parse_from([
            "turbo",
            "gen",
            "--args",
            "my long arg string",
            "my-second-arg"
        ])
        .unwrap(),
        Args {
            command: Some(Command::Generate {
                tag: None,
                generator_name: None,
                config: None,
                root: None,
                args: vec![
                    "my long arg string".to_string(),
                    "my-second-arg".to_string()
                ],
                command: None,
            }),
            ..Args::default()
        }
    );

    assert_eq!(
        Args::try_parse_from([
            "turbo",
            "gen",
            "--tag",
            "canary",
            "--config",
            "~/custom-gen-config/gen",
            "my-generator"
        ])
        .unwrap(),
        Args {
            command: Some(Command::Generate {
                tag: Some("canary".to_string()),
                generator_name: Some("my-generator".to_string()),
                config: Some("~/custom-gen-config/gen".to_string()),
                root: None,
                args: vec![],
                command: None,
            }),
            ..Args::default()
        }
    );
}

#[test]
fn test_gen_default_tag_is_not_latest() {
    let args = Args::try_parse_from(["turbo", "gen"]).unwrap();
    let tag = match args.command {
        Some(Command::Generate { tag, .. }) => tag,
        _ => panic!("expected Generate command"),
    };
    assert_eq!(tag, None, "default tag should be None, not \"latest\"");
}

#[test]
fn test_gen_default_tag_resolves_to_current_version() {
    let args = Args::try_parse_from(["turbo", "gen"]).unwrap();
    let tag = match args.command {
        Some(Command::Generate { tag, .. }) => tag,
        _ => panic!("expected Generate command"),
    };
    let resolved = tag.unwrap_or_else(|| crate::get_version().to_string());
    assert_eq!(
        resolved,
        crate::get_version(),
        "gen tag should resolve to the turbo binary version, not \"latest\""
    );
}

#[test]
fn test_gen_explicit_tag_is_preserved() {
    let args = Args::try_parse_from(["turbo", "gen", "--tag", "1.2.3"]).unwrap();
    let tag = match args.command {
        Some(Command::Generate { tag, .. }) => tag,
        _ => panic!("expected Generate command"),
    };
    assert_eq!(tag, Some("1.2.3".to_string()));
    let resolved = tag.unwrap_or_else(|| crate::get_version().to_string());
    assert_eq!(
        resolved, "1.2.3",
        "explicit --tag should override the default version"
    );
}

#[test]
fn test_profile_usage() {
    // Without a filename, profile should still be accepted
    assert!(Args::try_parse_from(["turbo", "build", "--profile"]).is_ok());
    assert!(Args::try_parse_from(["turbo", "build", "--anon-profile"]).is_ok());
    // With a filename, profile should be accepted
    assert!(Args::try_parse_from(["turbo", "build", "--profile", "foo.json"]).is_ok());
    assert!(Args::try_parse_from(["turbo", "build", "--anon-profile", "foo.json"]).is_ok());
    // Both flags simultaneously should be rejected
    assert!(Args::try_parse_from([
        "turbo",
        "build",
        "--profile",
        "foo.json",
        "--anon-profile",
        "bar.json"
    ])
    .is_err());
}

#[test]
fn test_profile_default_filename() {
    let run_args = RunArgs {
        profile: Some(String::new()),
        ..get_default_run_args()
    };
    let (file, include_args) = run_args.profile_file_and_include_args().unwrap();
    assert!(
        file.starts_with("profile."),
        "expected default profile filename, got: {file}"
    );
    assert!(include_args);

    let run_args = RunArgs {
        anon_profile: Some(String::new()),
        ..get_default_run_args()
    };
    let (file, include_args) = run_args.profile_file_and_include_args().unwrap();
    assert!(
        file.starts_with("profile."),
        "expected default profile filename, got: {file}"
    );
    assert!(!include_args);

    let run_args = RunArgs {
        profile: Some("custom.json".to_string()),
        ..get_default_run_args()
    };
    let (file, include_args) = run_args.profile_file_and_include_args().unwrap();
    assert_eq!(file, "custom.json");
    assert!(include_args);
}

#[test]
fn test_empty_cache_dir() {
    assert!(Args::try_parse_from(["turbo", "build", "--cache-dir"]).is_err());
    assert!(Args::try_parse_from(["turbo", "build", "--cache-dir="]).is_err());
    assert!(Args::try_parse_from(["turbo", "build", "--cache-dir", ""]).is_err());
}

#[test]
fn test_preflight() {
    assert!(!Args::try_parse_from(["turbo", "build",]).unwrap().preflight);
    assert!(
        Args::try_parse_from(["turbo", "build", "--preflight"])
            .unwrap()
            .preflight
    );
    assert!(Args::try_parse_from(["turbo", "build", "--preflight=true"]).is_err());
}

#[test]
fn test_log_stream_tui_compatibility() {
    assert!(LogOrder::Auto.compatible_with_tui());
    assert!(!LogOrder::Stream.compatible_with_tui());
    assert!(!LogOrder::Grouped.compatible_with_tui());
}

#[test]
fn test_dangerously_allow_no_package_manager() {
    assert!(
        !Args::try_parse_from(["turbo", "build",])
            .unwrap()
            .dangerously_disable_package_manager_check
    );
    assert!(
        Args::try_parse_from([
            "turbo",
            "build",
            "--dangerously-disable-package-manager-check"
        ])
        .unwrap()
        .dangerously_disable_package_manager_check
    );
}

#[test]
fn test_affected_and_filter_can_be_combined() {
    assert!(
        Args::try_parse_from(["turbo", "run", "build", "--affected", "--filter", "foo"]).is_ok(),
    );
    assert!(Args::try_parse_from(["turbo", "build", "--affected", "--filter", "foo"]).is_ok(),);
    assert!(Args::try_parse_from(["turbo", "build", "--filter", "foo", "--affected"]).is_ok(),);
    assert!(Args::try_parse_from(["turbo", "ls", "--filter", "foo", "--affected"]).is_ok(),);
}

struct SinglePackageTestCase {
    args: &'static [&'static str],
    expected_is_single: bool,
    expected: &'static [&'static str],
}

impl SinglePackageTestCase {
    pub const fn new<const N: usize>(args: &'static [&'static str; N]) -> Self {
        let args = args.as_slice();
        Self {
            args,
            expected_is_single: false,
            expected: args,
        }
    }

    pub const fn expected<const N: usize>(mut self, expected: &'static [&'static str; N]) -> Self {
        self.expected_is_single = true;
        self.expected = expected.as_slice();
        self
    }

    pub fn os_args(&self) -> Vec<OsString> {
        self.args.iter().map(|s| OsString::from(*s)).collect()
    }

    pub fn assert_actual(&self, actual: (bool, impl Iterator<Item = OsString>)) {
        let (is_single, args) = actual;
        assert_eq!(is_single, self.expected_is_single);
        assert_eq!(
            args.map(|s| s.to_str().unwrap().to_string())
                .collect::<Vec<_>>()
                .as_slice(),
            self.expected
        );
    }
}

const NO_SINGLE_PKG: SinglePackageTestCase = SinglePackageTestCase::new(&["turbo", "--version"]);
const SINGLE_PKG_AFTER_PASS: SinglePackageTestCase =
    SinglePackageTestCase::new(&["turbo", "--", "--single-package"]);
const SINGLE_PKG: SinglePackageTestCase =
    SinglePackageTestCase::new(&["turbo", "--single-package", "run", "build"])
        .expected(&["turbo", "run", "build"]);
const SINGLE_PKG_BEFORE_AFTER: SinglePackageTestCase = SinglePackageTestCase::new(&[
    "turbo",
    "--single-package",
    "run",
    "build",
    "--",
    "--single-package",
])
.expected(&["turbo", "run", "build", "--", "--single-package"]);

#[test_case::test_case(NO_SINGLE_PKG)]
#[test_case::test_case(SINGLE_PKG_AFTER_PASS)]
#[test_case::test_case(SINGLE_PKG)]
#[test_case::test_case(SINGLE_PKG_BEFORE_AFTER)]
fn test_single_package_removal(test: SinglePackageTestCase) {
    let os_args = test.os_args();
    let actual = Args::remove_single_package(os_args);
    test.assert_actual(actual);
}

#[test]
fn test_set_single_package() {
    let inferred_run = Args::parse(
        ["turbo", "--single-package", "build"]
            .iter()
            .map(|s| OsString::from(*s))
            .collect(),
    )
    .unwrap();
    let explicit_run = Args::parse(
        ["turbo", "run", "--single-package", "build"]
            .iter()
            .map(|s| OsString::from(*s))
            .collect(),
    )
    .unwrap();
    assert!(inferred_run
        .execution_args
        .as_ref()
        .is_some_and(|e| e.single_package));
    assert!(explicit_run
        .command
        .as_ref()
        .and_then(|cmd| if let Command::Run { execution_args, .. } = cmd {
            Some(execution_args.single_package)
        } else {
            None
        })
        .unwrap_or(false));

    let watch = Args::parse(
        ["turbo", "watch", "--single-package", "build"]
            .iter()
            .map(|s| OsString::from(*s))
            .collect(),
    )
    .unwrap();
    assert!(watch
        .command
        .as_ref()
        .and_then(|cmd| if let Command::Watch { execution_args, .. } = cmd {
            Some(execution_args.single_package)
        } else {
            None
        })
        .unwrap_or(false));
}

#[test_case::test_case(&["turbo", "watch", "build", "--no-daemon"]; "after watch")]
#[test_case::test_case(&["turbo", "--no-daemon", "watch", "build"]; "before watch")]
fn test_no_run_args_outside_of_run(args: &[&str]) {
    let os_args = args.iter().map(|s| OsString::from(*s)).collect();
    let err = Args::parse(os_args).unwrap_err();
    assert_snapshot!(args.join("-").as_str(), err);
}

#[test_case::test_case(&["turbo", "--filter=foo", "run", "build"], false; "execution args")]
#[test_case::test_case(&["turbo", "--no-daemon", "run", "build"], false; "run args")]
#[test_case::test_case(&["turbo", "build", "run"], true; "task")]
#[test_case::test_case(&["turbo", "--filter=web", "watch", "build"], false; "execution before watch")]
fn test_no_run_args_before_run(args: &[&str], is_okay: bool) {
    let os_args = args.iter().map(|s| OsString::from(*s)).collect();
    let cli = Args::parse(os_args);
    if is_okay {
        cli.unwrap();
    } else {
        let err = cli.unwrap_err();
        assert_snapshot!(args.join("-").as_str(), err);
    }
}

#[test_case::test_case(&["turbo", "--filter=foo", "boundaries"], false; "execution args")]
#[test_case::test_case(&["turbo", "--no-daemon", "boundaries"], false; "run args")]
fn test_no_run_args_before_boundaries(args: &[&str], is_okay: bool) {
    let os_args = args.iter().map(|s| OsString::from(*s)).collect();
    let cli = Args::parse(os_args);
    if is_okay {
        cli.unwrap();
    } else {
        let err = cli.unwrap_err();
        assert_snapshot!(args.join("-").as_str(), err);
    }
}

#[test_case::test_case(&["turbo", "boundaries"], true; "empty")]
#[test_case::test_case(&["turbo", "boundaries", "--ignore"], true; "with ignore")]
#[test_case::test_case(&["turbo", "boundaries", "--ignore=all"], true; "with ignore all")]
#[test_case::test_case(&["turbo", "boundaries", "--ignore=prompt"], true; "with ignore prompt")]
#[test_case::test_case(&["turbo", "boundaries", "--filter", "ui"], true; "with filter")]
fn test_boundaries(args: &[&str], is_okay: bool) {
    let os_args = args.iter().map(|s| OsString::from(*s)).collect();
    let cli = Args::parse(os_args);
    if is_okay {
        cli.unwrap();
    } else {
        let err = cli.unwrap_err();
        assert_snapshot!(args.join("-").as_str(), err);
    }
}

#[test]
fn test_query_affected_no_args() {
    let args = Args::try_parse_from(["turbo", "query", "affected"]).unwrap();
    assert_eq!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(super::AffectedArgs {
                packages: None,
                tasks: None,
                base: None,
                head: None,
                exit_code: false,
            })),
            query: None,
            variables: None,
            schema: false,
        })
    );
}

#[test]
fn test_query_affected_bare_packages_flag() {
    let args = Args::try_parse_from(["turbo", "query", "affected", "--packages"]).unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.packages == Some(vec![])
    );
}

#[test]
fn test_query_affected_with_packages() {
    let args = Args::try_parse_from(["turbo", "query", "affected", "--packages", "web"]).unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.packages == Some(vec!["web".to_string()])
    );
}

#[test]
fn test_query_affected_with_multiple_packages() {
    let args =
        Args::try_parse_from(["turbo", "query", "affected", "--packages", "web", "docs"]).unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.packages == Some(vec!["web".to_string(), "docs".to_string()])
    );
}

#[test]
fn test_query_affected_bare_tasks_flag() {
    let args = Args::try_parse_from(["turbo", "query", "affected", "--tasks"]).unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.tasks == Some(vec![])
    );
}

#[test]
fn test_query_affected_with_tasks() {
    let args = Args::try_parse_from(["turbo", "query", "affected", "--tasks", "build"]).unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.tasks == Some(vec!["build".to_string()])
    );
}

#[test]
fn test_query_affected_with_base_head() {
    let args = Args::try_parse_from([
        "turbo", "query", "affected", "--base", "main", "--head", "HEAD",
    ])
    .unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.base == Some("main".to_string()) && a.head == Some("HEAD".to_string())
    );
}

#[test]
fn test_query_affected_combined_packages_and_tasks() {
    let args = Args::try_parse_from([
        "turbo",
        "query",
        "affected",
        "--packages",
        "web",
        "--tasks",
        "build",
    ])
    .unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.packages == Some(vec!["web".to_string()])
            && a.tasks == Some(vec!["build".to_string()])
    );
}

#[test]
fn test_query_affected_combined_bare_packages_and_bare_tasks() {
    let args =
        Args::try_parse_from(["turbo", "query", "affected", "--packages", "--tasks"]).unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.packages == Some(vec![]) && a.tasks == Some(vec![])
    );
}

#[test]
fn test_query_affected_exit_code_flag() {
    let args = Args::try_parse_from(["turbo", "query", "affected", "--exit-code"]).unwrap();
    assert_matches!(
        args.command,
        Some(Command::Query {
            subcommand: Some(super::QuerySubcommand::Affected(ref a)),
            ..
        }) if a.exit_code
    );
}

#[test]
fn test_query_raw_graphql_still_works() {
    let args = Args::try_parse_from(["turbo", "query", "{ packages { items { name } } }"]).unwrap();
    assert_eq!(
        args.command,
        Some(Command::Query {
            subcommand: None,
            query: Some("{ packages { items { name } } }".to_string()),
            variables: None,
            schema: false,
        })
    );
}

#[test]
fn test_query_schema_still_works() {
    let args = Args::try_parse_from(["turbo", "query", "--schema"]).unwrap();
    assert_eq!(
        args.command,
        Some(Command::Query {
            subcommand: None,
            query: None,
            variables: None,
            schema: true,
        })
    );
}
