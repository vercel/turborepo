use std::{ffi::OsString, time::Duration};

use tokio::signal::ctrl_c;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_daemon::{
    CloseReason, DaemonLifecycleCommand, DaemonLifecycleOutput, Paths,
    RediscoveringPackageChangesWatcher, clean_daemon, follow_daemon_logs, run_lifecycle_command,
    serve,
};
use turborepo_repository::inference::RepoState;
use usage::{Cli, Subcommands};

const DAEMON_NOT_RUNNING_MESSAGE: &str =
    "daemon is not running, run `turbo daemon start` to start it";

fn main() {
    if has_daemon_command(std::env::args_os()) {
        run_daemon_command();
    }

    turborepo_lsp::run_lsp_server();
}

fn has_daemon_command(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter().skip(1).any(|arg| arg == "daemon")
}

fn run_daemon_command() -> ! {
    let args = DaemonCli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build();
    let exit_code = match runtime {
        Ok(runtime) => runtime
            .block_on(execute_daemon_command(args))
            .map(|_| 0)
            .unwrap_or_else(|error| {
                eprintln!("{error:?}");
                1
            }),
        Err(error) => {
            eprintln!("failed to build tokio runtime: {error}");
            1
        }
    };

    std::process::exit(exit_code)
}

#[allow(clippy::result_large_err)]
async fn execute_daemon_command(args: DaemonCli) -> Result<(), turborepo_daemon::DaemonError> {
    let verbosity = args.verbosity.unwrap_or(args.v);
    let cwd = AbsoluteSystemPathBuf::cwd()?;
    let invocation_dir = args
        .cwd
        .as_deref()
        .map(AbsoluteSystemPathBuf::from_cwd)
        .transpose()?
        .unwrap_or(cwd);
    let repo_root = RepoState::infer(&invocation_dir)
        .map(|state| state.root)
        .unwrap_or_else(|_| invocation_dir.clone());
    let DaemonTopLevelCommand::Daemon {
        idle_time,
        turbo_json_path,
        command,
    } = args.command;
    let custom_turbo_json_path = turbo_json_path
        .or(args.root_turbo_json)
        .map(|path| AbsoluteSystemPathBuf::from_unknown(&invocation_dir, path));

    match command {
        Some(DaemonSubcommand::Clean { clean_logs }) => {
            clean_daemon(&repo_root, custom_turbo_json_path, clean_logs).await?;
            println!("Done");
        }
        Some(DaemonSubcommand::Logs) => {
            follow_daemon_logs(&repo_root, custom_turbo_json_path).await?;
        }
        Some(command) => {
            let (command, json) = match command {
                DaemonSubcommand::Restart => (DaemonLifecycleCommand::Restart, false),
                DaemonSubcommand::Start => (DaemonLifecycleCommand::Start, false),
                DaemonSubcommand::Status { json } => (DaemonLifecycleCommand::Status, json),
                DaemonSubcommand::Stop => (DaemonLifecycleCommand::Stop, false),
                DaemonSubcommand::Clean { .. } | DaemonSubcommand::Logs => unreachable!(),
            };
            let output = run_lifecycle_command(command, &repo_root, custom_turbo_json_path).await?;
            print_lifecycle_output(output, json)?;
        }
        None => {
            let timeout = go_parse_duration::parse_duration(&idle_time)
                .map_err(|_| turborepo_daemon::DaemonError::InvalidTimeout(idle_time.clone()))?;
            let timeout = Duration::from_nanos(timeout as u64);
            let paths = Paths::from_repo_root(&repo_root)?;
            let appender = tracing_appender::rolling::daily(&paths.log_folder, &paths.log_file);
            let level = match verbosity {
                0 | 1 => tracing::level_filters::LevelFilter::INFO,
                2 => tracing::level_filters::LevelFilter::DEBUG,
                _ => tracing::level_filters::LevelFilter::TRACE,
            };
            let _ = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(appender)
                .with_max_level(level)
                .try_init();
            tracing::info!("daemon started");
            let exit_signal = async {
                if let Err(error) = ctrl_c().await {
                    eprintln!("error waiting for interrupt signal: {error}");
                }
                CloseReason::Interrupt
            };
            serve(
                repo_root,
                timeout,
                exit_signal,
                custom_turbo_json_path,
                args.dangerously_disable_package_manager_check,
                RediscoveringPackageChangesWatcher::new,
            )
            .await?;
        }
    }

    Ok(())
}

fn print_lifecycle_output(
    output: DaemonLifecycleOutput,
    json: bool,
) -> Result<(), serde_json::Error> {
    match output {
        DaemonLifecycleOutput::Restarted => println!("✓ restarted daemon"),
        DaemonLifecycleOutput::Running => println!("✓ daemon is running"),
        DaemonLifecycleOutput::Stopped => println!("✓ stopped daemon"),
        DaemonLifecycleOutput::Status(None) if json => {
            println!(
                "{}",
                serde_json::json!({ "error": DAEMON_NOT_RUNNING_MESSAGE })
            );
        }
        DaemonLifecycleOutput::Status(None) => println!("x {DAEMON_NOT_RUNNING_MESSAGE}"),
        DaemonLifecycleOutput::Status(Some(status)) if json => {
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        DaemonLifecycleOutput::Status(Some(status)) => {
            println!("✓ daemon is running");
            println!("log file: {}", status.log_file);
            println!(
                "uptime: {}s",
                humantime::format_duration(Duration::from_millis(status.uptime_ms))
            );
            println!("pid file: {}", status.pid_file);
            println!("socket file: {}", status.sock_file);
        }
    }

    Ok(())
}

#[derive(Cli, Debug)]
#[usage(name = "turborepo-lsp")]
struct DaemonCli {
    #[usage(long = "skip-infer", global = true, hide = true)]
    _skip_infer: bool,
    #[usage(long, global = true)]
    cwd: Option<String>,
    #[usage(long, global = true)]
    root_turbo_json: Option<String>,
    #[usage(long, global = true)]
    dangerously_disable_package_manager_check: bool,
    #[usage(long = "color", global = true)]
    _color: bool,
    #[usage(long = "no-color", global = true)]
    _no_color: bool,
    #[usage(long, global = true, value_name = "COUNT")]
    verbosity: Option<u8>,
    #[usage(short = 'v', global = true, count, hide = true)]
    v: u8,
    #[usage(subcommand)]
    command: DaemonTopLevelCommand,
}

#[derive(Subcommands, Debug)]
enum DaemonTopLevelCommand {
    Daemon {
        #[usage(long, default_value_t = String::from("4h0m0s"), default = "4h0m0s")]
        idle_time: String,
        #[usage(long)]
        turbo_json_path: Option<String>,
        #[usage(subcommand)]
        command: Option<DaemonSubcommand>,
    },
}

#[derive(Subcommands, Debug)]
enum DaemonSubcommand {
    Clean {
        #[usage(long, default = "true")]
        clean_logs: bool,
    },
    Logs,
    Restart,
    Start,
    Status {
        #[usage(long)]
        json: bool,
    },
    Stop,
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};

    use super::{DaemonCli, DaemonSubcommand, DaemonTopLevelCommand, has_daemon_command};

    fn parse<'a, const N: usize>(
        args: [&'a str; N],
    ) -> Result<DaemonCli, usage::Error<'static, 'a>> {
        let words: Vec<&OsStr> = args.iter().map(|arg| OsStr::new(*arg)).collect();
        DaemonCli::try_parse_from(&words)
    }

    #[test]
    fn detects_daemon_command() {
        assert!(has_daemon_command([
            OsString::from("turborepo-lsp"),
            OsString::from("--skip-infer"),
            OsString::from("daemon"),
        ]));
    }

    #[test]
    fn parses_spawned_daemon_server_command() {
        parse([
            "turborepo-lsp",
            "--skip-infer",
            "daemon",
            "--turbo-json-path",
            "turbo.json",
        ])
        .unwrap();
    }

    #[test]
    fn parses_daemon_status_command() {
        parse(["turborepo-lsp", "daemon", "status", "--json"]).unwrap();
    }

    #[test]
    fn parses_behavioral_global_options() {
        parse([
            "turborepo-lsp",
            "daemon",
            "status",
            "--cwd",
            "repo",
            "--dangerously-disable-package-manager-check",
        ])
        .unwrap();
    }

    #[test]
    fn parses_daemon_defaults_and_subcommands() {
        let args = parse(["turborepo-lsp", "daemon"]).unwrap();
        let DaemonTopLevelCommand::Daemon {
            idle_time,
            turbo_json_path,
            command,
        } = args.command;
        assert_eq!(idle_time, "4h0m0s");
        assert!(turbo_json_path.is_none());
        assert!(command.is_none());
        assert_eq!(args.v, 0);
        assert!(args.verbosity.is_none());

        for (name, expected) in [
            ("logs", "Logs"),
            ("restart", "Restart"),
            ("start", "Start"),
            ("stop", "Stop"),
        ] {
            let args = parse(["turborepo-lsp", "daemon", name]).unwrap();
            let DaemonTopLevelCommand::Daemon { command, .. } = args.command;
            assert_eq!(format!("{:?}", command.unwrap()), expected);
        }
        let args = parse(["turborepo-lsp", "daemon", "clean"]).unwrap();
        assert!(matches!(
            args.command,
            DaemonTopLevelCommand::Daemon {
                command: Some(DaemonSubcommand::Clean { clean_logs: true }),
                ..
            }
        ));
        let args = parse([
            "turborepo-lsp",
            "daemon",
            "--idle-time",
            "1m",
            "clean",
            "--clean-logs",
        ])
        .unwrap();
        assert!(matches!(
            args.command,
            DaemonTopLevelCommand::Daemon {
                idle_time,
                command: Some(DaemonSubcommand::Clean { clean_logs: true }),
                ..
            } if idle_time == "1m"
        ));
    }

    #[test]
    fn parses_global_flags_at_each_position() {
        for args in [
            ["turborepo-lsp", "--cwd", "repo", "daemon", "status"],
            ["turborepo-lsp", "daemon", "--cwd", "repo", "status"],
            ["turborepo-lsp", "daemon", "status", "--cwd", "repo"],
        ] {
            let parsed = parse(args).unwrap();
            assert_eq!(parsed.cwd.as_deref(), Some("repo"));
        }
        let args = parse([
            "turborepo-lsp",
            "--root-turbo-json",
            "root.json",
            "daemon",
            "status",
            "--color",
            "--no-color",
            "--dangerously-disable-package-manager-check",
            "--verbosity",
            "3",
            "-vv",
        ])
        .unwrap();
        assert_eq!(args.root_turbo_json.as_deref(), Some("root.json"));
        assert!(args._color && args._no_color && args.dangerously_disable_package_manager_check);
        assert_eq!(args.verbosity, Some(3));
        assert_eq!(args.v, 2);
    }

    #[test]
    fn rejects_invalid_verbosity_and_shows_help() {
        assert!(parse(["turborepo-lsp", "daemon", "--verbosity", "256"]).is_err());
        assert!(parse(["turborepo-lsp", "daemon", "--help"]).is_err());
        assert!(parse(["turborepo-lsp", "daemon", "status", "--help"]).is_err());
    }

    #[test]
    fn ignores_lsp_mode() {
        assert!(!has_daemon_command([OsString::from("turborepo-lsp")]));
    }
}
