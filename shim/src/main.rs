mod commands;
mod ffi;
mod package_manager;

use std::{
    env,
    env::current_exe,
    ffi::CString,
    fs, io, mem,
    path::{Path, PathBuf},
    process,
    process::Stdio,
};

use anyhow::{anyhow, Context, Result};
use clap::CommandFactory;
use clap_complete::generate;
use serde::Serialize;

use crate::{
    commands::{Args, Command, RunArgs},
    ffi::{nativeRunWithTurboState, GoString},
    package_manager::PackageManager,
};

static TURBO_JSON: &str = "turbo.json";

#[derive(Debug, Clone, Serialize)]
struct RepoState {
    root: PathBuf,
    mode: RepoMode,
}

#[derive(Debug, Clone, Serialize)]
enum RepoMode {
    SinglePackage,
    MultiPackage,
}

/// The entire state of the execution, including args, repo state, etc.
#[derive(Debug, Serialize)]
struct TurboState {
    /// The repo_state is not required for the `link`, `unlink`, `login`,
    /// `logout` commands
    repo_state: Option<RepoState>,
    parsed_args: Args,
}

impl TryInto<GoString> for TurboState {
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

impl TurboState {
    /// Runs the Go code linked in current binary.
    ///
    /// # Arguments
    ///
    /// * `args`: Arguments for turbo
    ///
    /// returns: Result<i32, Error>
    fn run_current_turbo(self) -> Result<i32> {
        match self.parsed_args.command {
            Some(Command::Bin { .. }) => {
                commands::bin::run()?;
                Ok(0)
            }
            Some(Command::Completion { .. }) => {
                unreachable!("shell completion should be handled by clap_complete")
            }
            Some(Command::Link { .. })
            | Some(Command::Login { .. })
            | Some(Command::Logout { .. })
            | Some(Command::Unlink { .. })
            | Some(Command::Daemon { .. })
            | Some(Command::Run(_))
            | Some(Command::Prune { .. })
            | None => self.native_run(),
        }
    }

    fn native_run(self) -> Result<i32> {
        let serialized_state = self.try_into()?;
        let exit_code = unsafe { nativeRunWithTurboState(serialized_state) };
        Ok(exit_code.try_into()?)
    }

    /// Attempts to run correct turbo by finding nearest package.json,
    /// then finding local turbo installation. If the current binary is the
    /// local turbo installation, then we run current turbo. Otherwise we
    /// kick over to the local turbo installation.
    ///
    /// # Arguments
    ///
    /// * `turbo_state`: state for current execution
    ///
    /// returns: Result<i32, Error>
    fn run_correct_turbo(mut self, current_dir: &Path) -> Result<i32> {
        let repo_state = RepoState::infer(current_dir)?;
        let local_turbo_path = repo_state.root.join("node_modules").join(".bin").join({
            #[cfg(windows)]
            {
                "turbo.cmd"
            }
            #[cfg(not(windows))]
            {
                "turbo"
            }
        });

        self.repo_state = Some(repo_state);
        let current_turbo_is_local_turbo = local_turbo_path == current_exe()?;
        // If the local turbo path doesn't exist or if we are local turbo, then we go
        // ahead and run the Go code linked in the current binary.
        if current_turbo_is_local_turbo || !local_turbo_path.try_exists()? {
            self.run_current_turbo()
        } else {
            // Otherwise we spawn the local turbo process.
            self.spawn_local_turbo(&local_turbo_path)
        }
    }

    fn spawn_local_turbo(&self, local_turbo_path: &Path) -> Result<i32> {
        let mut raw_args: Vec<_> = env::args().skip(1).collect();
        let has_single_package_flag = self
            .parsed_args
            .run_args
            .as_ref()
            .map_or(false, |run_args| run_args.single_package)
            || matches!(
                self.parsed_args.command,
                Some(Command::Run(RunArgs {
                    single_package: true,
                    ..
                }))
            );

        if matches!(
            self.repo_state,
            Some(RepoState {
                mode: RepoMode::SinglePackage,
                ..
            })
        ) && self.parsed_args.is_run_command()
            && !has_single_package_flag
        {
            raw_args.push("--single-package".to_string());
        }

        // Otherwise, we spawn a process that executes the local turbo
        // that we've found in node_modules/.bin/turbo.
        let mut command = process::Command::new(local_turbo_path)
            .args(&raw_args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Failed to execute turbo.");

        Ok(command.wait()?.code().unwrap_or(2))
    }
}

impl RepoState {
    /// Infers `RepoState` from current directory.
    ///
    /// # Arguments
    ///
    /// * `current_dir`: Current working directory
    ///
    /// returns: Result<RepoState, Error>
    pub fn infer(current_dir: &Path) -> Result<Self> {
        // First we look for a `turbo.json`. This iterator returns the first ancestor
        // that contains a `turbo.json` file.
        let root_path = current_dir
            .ancestors()
            .find(|p| fs::metadata(p.join(TURBO_JSON)).is_ok());

        // If that directory exists, then we figure out if there are workspaces defined
        // in it NOTE: This may change with multiple `turbo.json` files
        if let Some(root_path) = root_path {
            let pnpm = PackageManager::Pnpm;
            let npm = PackageManager::Npm;
            let is_workspace = pnpm.get_workspace_globs(root_path).is_ok()
                || npm.get_workspace_globs(root_path).is_ok();

            let mode = if is_workspace {
                RepoMode::MultiPackage
            } else {
                RepoMode::SinglePackage
            };

            return Ok(Self {
                root: root_path.to_path_buf(),
                mode,
            });
        }

        // What we look for next is a directory that contains a `package.json`.
        let potential_roots = current_dir
            .ancestors()
            .filter(|path| fs::metadata(path.join("package.json")).is_ok());

        let mut first_package_json_dir = None;
        // We loop through these directories and see if there are workspaces defined in
        // them, either in the `package.json` or `pnm-workspaces.yml`
        for dir in potential_roots {
            if first_package_json_dir.is_none() {
                first_package_json_dir = Some(dir)
            }

            let pnpm = PackageManager::Pnpm;
            let npm = PackageManager::Npm;
            let is_workspace =
                pnpm.get_workspace_globs(dir).is_ok() || npm.get_workspace_globs(dir).is_ok();

            if is_workspace {
                return Ok(Self {
                    root: dir.to_path_buf(),
                    mode: RepoMode::MultiPackage,
                });
            }
        }

        // Finally, if we don't detect any workspaces, go to the first `package.json`
        // and use that in single package mode.
        let root = first_package_json_dir
            .ok_or_else(|| {
                anyhow!(
                    "Unable to find `{}` or `package.json` in current path",
                    TURBO_JSON
                )
            })?
            .to_path_buf();

        Ok(Self {
            root,
            mode: RepoMode::SinglePackage,
        })
    }
}

impl Args {
    /// Checks if either we have an explicit run command, i.e. `turbo run build`
    /// or an implicit run, i.e. `turbo build`, where the command after `turbo`
    /// is not one of the reserved commands like `link`, `login`, `bin`,
    /// etc.
    ///
    /// # Arguments
    ///
    /// * `clap_args`:
    ///
    /// returns: bool
    fn is_run_command(&self) -> bool {
        let is_explicit_run = matches!(self.command, Some(Command::Run { .. }));
        let is_implicit_run = self.command.is_none()
            && self
                .run_args
                .as_ref()
                .map_or(false, |args| !args.tasks.is_empty());

        is_explicit_run || is_implicit_run
    }
}

fn get_version() -> &'static str {
    include_str!("../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
}

/// Checks for `TURBO_BINARY_PATH` variable. If it is set,
/// we do not do any inference, we simply run the command as
/// the current binary. This is due to legacy behavior of `TURBO_BINARY_PATH`
/// that lets users dynamically set the path of the turbo binary. Because
/// inference involves finding a local turbo installation and executing that
/// binary, these two features are fundamentally incompatible.
fn is_turbo_binary_path_set() -> bool {
    env::var("TURBO_BINARY_PATH").is_ok()
}

fn main() -> Result<()> {
    let mut clap_args = Args::new()?;

    let current_dir = if let Some(cwd) = &clap_args.cwd {
        fs::canonicalize::<PathBuf>(cwd.into())?
    } else {
        env::current_dir()?
    };

    clap_args.cwd = Some(
        current_dir
            .to_str()
            .context("--cwd is not valid Unicode")?
            .to_string(),
    );
    if clap_args.test_run {
        println!("{:#?}", clap_args);
    }
    // If there is no command, we set the command to `Command::Run` with
    // `self.parsed_args.run_args` as arguments.
    if clap_args.command.is_none() {
        if let Some(run_args) = mem::take(&mut clap_args.run_args) {
            clap_args.command = Some(Command::Run(run_args));
        } else {
            return Err(anyhow!("No command specified"));
        }
    }

    let mut turbo_state = TurboState {
        repo_state: None,
        parsed_args: clap_args,
    };

    // We run this *before* doing any inference because login/logout/link/unlink
    // should work regardless of whether or not we're in a monorepo.
    let exit_code = match turbo_state.parsed_args.command {
        Some(Command::Login { .. })
        | Some(Command::Link { .. })
        | Some(Command::Logout { .. })
        | Some(Command::Unlink { .. }) => turbo_state.run_current_turbo()?,
        Some(Command::Completion { shell }) => {
            generate(shell, &mut Args::command(), "turbo", &mut io::stdout());

            0
        }
        _ => {
            // When the `TURBO_BINARY_PATH` variable is set, the user is effectively saying
            // that the `turbo` package should run a specific binary. Because
            // this code is running, and the `TURBO_BINARY_PATH` variable is
            // set, we can deduce that this code is in the binary that the user
            // wishes to run. Therefore, we will not find local turbo
            // and execute it, because that would go against the user's wishes.
            if is_turbo_binary_path_set() {
                let repo_state = RepoState::infer(&current_dir)?;
                turbo_state.repo_state = Some(repo_state);
                turbo_state.native_run()?
            } else {
                match turbo_state.run_correct_turbo(&current_dir) {
                    Ok(exit_code) => exit_code,
                    Err(e) => {
                        eprintln!("failed: {:?}", e);
                        2
                    }
                }
            }
        }
    };

    process::exit(exit_code)
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
                    dry_run: Some(DryRunMode::Stdout),
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
                    graph: Some("stdout".to_string()),
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
