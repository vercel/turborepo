mod commands;
mod ffi;

use std::{
    env,
    ffi::CString,
    os::raw::{c_char, c_int},
    process,
};

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;

use crate::ffi::{nativeRunWithArgs, nativeRunWithTurboState, GoString};

#[derive(Parser, Clone, Default, Debug, PartialEq, Serialize)]
#[clap(author, about = "The build system that makes ship happen", long_about = None)]
#[clap(
    ignore_errors = true,
    disable_help_flag = true,
    disable_help_subcommand = true,
    disable_colored_help = true
)]
#[clap(disable_version_flag = true)]
struct Args {
    #[clap(long, short)]
    help: bool,
    #[clap(long, global = true)]
    version: bool,
    /// Override the endpoint for API calls
    #[clap(long, global = true, value_parser)]
    api: Option<String>,
    /// Force color usage in the terminal
    #[clap(long, global = true)]
    color: bool,
    /// Specify a file to save a cpu profile
    #[clap(long, global = true, value_parser)]
    cpu_profile: Option<String>,
    /// The directory in which to run turbo
    #[clap(long, global = true, value_parser)]
    cwd: Option<String>,
    /// Specify a file to save a pprof heap profile
    #[clap(long, global = true, value_parser)]
    heap: Option<String>,
    /// Override the login endpoint
    #[clap(long, global = true, value_parser)]
    login: Option<String>,
    /// Suppress color usage in the terminal
    #[clap(long, global = true)]
    no_color: bool,
    /// When enabled, turbo will precede HTTP requests with an OPTIONS request
    /// for authorization
    #[clap(long, global = true)]
    preflight: bool,
    /// Set the team slug for API calls
    #[clap(long, global = true, value_parser)]
    team: Option<String>,
    /// Set the auth token for API calls
    #[clap(long, global = true, value_parser)]
    token: Option<String>,
    /// Specify a file to save a pprof trace
    #[clap(long, global = true, value_parser)]
    trace: Option<String>,
    /// verbosity
    #[clap(short, long, global = true, value_parser)]
    verbosity: Option<u8>,
    #[clap(long = "__test-run", global = true, hide = true)]
    test_run: bool,
    #[clap(subcommand)]
    command: Option<Command>,
    tasks: Vec<String>,
}

/// Defines the subcommands for CLI. NOTE: If we change the commands in Go,
/// we must change these as well to avoid accidentally passing the
/// --single-package flag into non-build commands.
#[derive(Subcommand, Clone, Debug, Serialize, PartialEq)]
enum Command {
    /// Get the path to the Turbo binary
    Bin {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
    /// Generate the autocompletion script for the specified shell
    Completion {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
    /// Runs the Turborepo background daemon
    Daemon {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
    /// Help about any command
    Help {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
    /// Link your local directory to a Vercel organization and enable remote
    /// caching.
    Link {
        /// help for link
        #[clap(long, short)]
        help: bool,
        /// Do not create or modify .gitignore (default false)
        #[clap(long)]
        no_gitignore: bool,
    },
    /// Login to your Vercel account
    Login {
        /// Help flag
        #[clap(long, short)]
        help: bool,
        #[clap(long = "sso-team")]
        sso_team: Option<String>,
    },
    /// Logout to your Vercel account
    Logout {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
    /// Prepare a subset of your monorepo.
    Prune {
        /// Help flag
        #[clap(long, short)]
        help: bool,
        #[clap(long)]
        scope: Option<String>,
        #[clap(long)]
        docker: bool,
        #[clap(long = "out-dir", default_value = "out")]
        output_dir: String,
    },
    /// Run tasks across projects in your monorepo
    Run {
        /// Help flag
        #[clap(long, short)]
        help: bool,
        tasks: Vec<String>,
    },
    /// Unlink the current directory from your Vercel organization and disable
    /// Remote Caching
    Unlink {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
}

impl TryInto<GoString> for Args {
    type Error = anyhow::Error;

    fn try_into(self) -> std::result::Result<GoString, Self::Error> {
        let json = serde_json::to_string(&self)?;
        let cstring = CString::new(json)?;
        let n = cstring.as_bytes().len() as isize;

        Ok(GoString {
            p: cstring.into_raw(),
            n,
        })
    }
}

/// If a command has a help flag passed, print help and return true.
/// Otherwise, return false.
///
/// # Arguments
///
/// * `command`: The parsed command.
///
/// returns: Result<bool, Error>
// fn try_run_help(command: &Command) -> Result<bool> {
//     let (help, command_name) = match command {
//         Command::Bin { help, .. } => (help, "bin"),
//         Command::Completion { help, .. } => (help, "completion"),
//         Command::Daemon { help, .. } => (help, "daemon"),
//         Command::Help { help, .. } => (help, "help"),
//         Command::Link { help, .. } => (help, "link"),
//         Command::Login { help, .. } => (help, "login"),
//         Command::Logout { help, .. } => (help, "logout"),
//         Command::Prune { help, .. } => (help, "prune"),
//         Command::Run { help, .. } => (help, "run"),
//         Command::Unlink { help, .. } => (help, "unlink"),
//     };
//
//     if *help {
//         Args::command()
//             .find_subcommand_mut(command_name)
//             .unwrap_or_else(|| panic!("Could not find subcommand:
// {command_name}"))             .print_long_help()?;
//         Ok(true)
//     } else {
//         Ok(false)
//     }
// }

impl Args {
    /// Runs the Go code linked in current binary.
    ///
    /// # Arguments
    ///
    /// * `args`: Arguments for turbo
    ///
    /// returns: Result<i32, Error>
    fn run(self) -> Result<i32> {
        match self.command {
            Some(Command::Bin { .. }) => {
                commands::bin::run()?;
                Ok(0)
            }
            Some(Command::Link { .. })
            | Some(Command::Login { .. })
            | Some(Command::Logout { .. })
            | Some(Command::Unlink { .. }) => {
                let serialized_state = self.try_into()?;
                let exit_code = unsafe { nativeRunWithTurboState(serialized_state) };
                Ok(exit_code.try_into()?)
            }
            _ => {
                let mut args = env::args()
                    .map(|s| {
                        let c_string = CString::new(s.as_str())?;
                        Ok(c_string.into_raw())
                    })
                    .collect::<Result<Vec<*mut c_char>>>()?;
                // With vectors there is a possibility of over-allocating, whether
                // from the allocator itself or the Vec implementation.
                // Therefore we shrink the vector to just the length we need.
                args.shrink_to_fit();
                let argc: c_int = args.len() as c_int;
                let argv = args.as_mut_ptr();
                let exit_code = unsafe { nativeRunWithArgs(argc, argv) };
                Ok(exit_code.try_into()?)
            }
        }
    }
}

pub fn get_version() -> &'static str {
    include_str!("../../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
}

pub fn run() -> Result<i32> {
    let clap_args = Args::parse();
    // --help doesn't work with ignore_errors in clap.
    if clap_args.help {
        let mut command = Args::command();
        command.print_help()?;
        return Ok(0);
    }
    // --version flag doesn't work with ignore_errors in clap, so we have to handle
    // it manually
    if clap_args.version {
        println!("{}", get_version());
        return Ok(0);
    }

    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() {
        process::exit(1);
    }

    clap_args.run()
}

#[cfg(test)]
mod test {
    use clap::Parser;
    use itertools::Itertools;
    use semver::Version;

    struct CommandTestCase {
        command: &'static str,
        command_args: Vec<Vec<&'static str>>,
        global_args: Vec<Vec<&'static str>>,
        expected_output: Args,
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

    use crate::{get_version, Args, Command};

    #[test]
    fn test_parse_run() {
        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build"]).unwrap(),
            Args {
                command: Some(Command::Run {
                    help: false,
                    tasks: vec!["build".to_string()]
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "lint", "test"]).unwrap(),
            Args {
                command: Some(Command::Run {
                    help: false,
                    tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()]
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "build"]).unwrap(),
            Args {
                tasks: vec!["build".to_string()],
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "build", "lint", "test"]).unwrap(),
            Args {
                tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                ..Args::default()
            }
        );
    }

    #[test]
    fn test_parse_bin() {
        assert_eq!(
            Args::try_parse_from(["turbo", "bin"]).unwrap(),
            Args {
                command: Some(Command::Bin { help: false }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "bin",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/basic"]],
            expected_output: Args {
                command: Some(Command::Bin { help: false }),
                cwd: Some("../examples/basic".to_string()),
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
                    help: false,
                    sso_team: None
                }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "login",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/basic"]],
            expected_output: Args {
                command: Some(Command::Login {
                    help: false,
                    sso_team: None,
                }),
                cwd: Some("../examples/basic".to_string()),
                ..Args::default()
            },
        }
        .test();

        CommandTestCase {
            command: "login",
            command_args: vec![vec!["--sso-team", "my-team"]],
            global_args: vec![vec!["--cwd", "../examples/basic"]],
            expected_output: Args {
                command: Some(Command::Login {
                    help: false,
                    sso_team: Some("my-team".to_string()),
                }),
                cwd: Some("../examples/basic".to_string()),
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
                command: Some(Command::Logout { help: false }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "logout",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/basic"]],
            expected_output: Args {
                command: Some(Command::Logout { help: false }),
                cwd: Some("../examples/basic".to_string()),
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
                command: Some(Command::Unlink { help: false }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "unlink",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/basic"]],
            expected_output: Args {
                command: Some(Command::Unlink { help: false }),
                cwd: Some("../examples/basic".to_string()),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_prune() {
        let default_prune = Command::Prune {
            help: false,
            scope: None,
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
            global_args: vec![vec!["--cwd", "../examples/basic"]],
            expected_output: Args {
                command: Some(default_prune),
                cwd: Some("../examples/basic".to_string()),
                ..Args::default()
            },
        }
        .test();

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--scope", "bar"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    help: false,
                    scope: Some("bar".to_string()),
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
                    help: false,
                    scope: None,
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
                    help: false,
                    scope: None,
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
                    help: false,
                    scope: None,
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
            global_args: vec![vec!["--cwd", "../examples/basic"]],
            expected_output: Args {
                command: Some(Command::Prune {
                    help: false,
                    scope: None,
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                cwd: Some("../examples/basic".to_string()),
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
                    help: false,
                    scope: Some("foo".to_string()),
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            },
        }
        .test();
    }

    /// Ensures that current version is parsable by semver crate.
    #[test]
    fn test_parse_current_version() {
        Version::parse(get_version()).unwrap();
    }
}
