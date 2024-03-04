use std::time::Duration;

use camino::Utf8PathBuf;
use futures::FutureExt;
use pidlock::PidlockError::AlreadyOwned;
use serde_json::json;
use time::{format_description, OffsetDateTime};
use tokio::signal::ctrl_c;
use tracing::{trace, warn};
use turbopath::AbsoluteSystemPath;
use turborepo_ui::{color, BOLD_GREEN, BOLD_RED, GREY};
use which::which;

use super::CommandBase;
use crate::{
    cli::DaemonCommand,
    daemon::{
        endpoint::SocketOpenError, CloseReason, DaemonConnector, DaemonConnectorError, DaemonError,
        Paths,
    },
    tracing::TurboSubscriber,
};

const DAEMON_NOT_RUNNING_MESSAGE: &str =
    "daemon is not running, run `turbo daemon start` to start it";

/// Runs the daemon command.
pub async fn daemon_client(command: &DaemonCommand, base: &CommandBase) -> Result<(), DaemonError> {
    let (can_start_server, can_kill_server) = match command {
        DaemonCommand::Status { .. } | DaemonCommand::Logs => (false, false),
        DaemonCommand::Stop => (false, true),
        DaemonCommand::Restart | DaemonCommand::Start => (true, true),
        DaemonCommand::Clean { .. } => (false, true),
    };

    let connector = DaemonConnector::new(can_start_server, can_kill_server, &base.repo_root);

    match command {
        DaemonCommand::Restart => {
            let result: Result<_, DaemonError> = try {
                let client = connector.clone().connect().await?;
                client.restart().await?
            };

            if let Err(e) = result {
                tracing::debug!("failed to restart the daemon: {:?}", e);
                tracing::debug!("falling back to clean");
                clean(&connector.paths.pid_file, &connector.paths.sock_file)?;
                tracing::debug!("connecting for second time");
                let _ = connector.connect().await?;
            }

            println!("{} restarted daemon", color!(base.ui, BOLD_GREEN, "✓"));
        }
        DaemonCommand::Start => {
            // We don't care about the client, but we do care that we can connect
            // which ensures that daemon is started if it wasn't already.
            let _ = connector.connect().await?;
            println!("{} daemon is running", color!(base.ui, BOLD_GREEN, "✓"));
        }
        DaemonCommand::Stop => {
            let client = match connector.connect().await {
                Ok(client) => client,
                Err(DaemonConnectorError::NotRunning) => {
                    println!("{} stopped daemon", color!(base.ui, BOLD_GREEN, "✓"));
                    return Ok(());
                }
                Err(e) => {
                    return Err(e.into());
                }
            };
            client.stop().await?;
            println!("{} stopped daemon", color!(base.ui, BOLD_GREEN, "✓"));
        }
        DaemonCommand::Status { json } => {
            let mut client = match connector.connect().await {
                Ok(status) => status,
                Err(DaemonConnectorError::NotRunning) if *json => {
                    println!("{}", json!({ "error": DAEMON_NOT_RUNNING_MESSAGE }));
                    return Ok(());
                }
                Err(DaemonConnectorError::NotRunning) => {
                    println!(
                        "{} {}",
                        color!(base.ui, BOLD_RED, "x"),
                        DAEMON_NOT_RUNNING_MESSAGE
                    );
                    return Ok(());
                }
                Err(e) => {
                    return Err(e.into());
                }
            };
            let status = client.status().await?;
            let log_file = log_filename(&status.log_file)?;
            let paths = client.paths();
            let status = DaemonStatus {
                uptime_ms: status.uptime_msec,
                log_file: log_file.into(),
                pid_file: paths.pid_file.to_owned(),
                sock_file: paths.sock_file.to_owned(),
            };

            if *json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("{} daemon is running", color!(base.ui, BOLD_GREEN, "✓"));
                println!("log file: {}", color!(base.ui, GREY, "{}", status.log_file));
                println!(
                    "uptime: {}",
                    color!(
                        base.ui,
                        GREY,
                        "{}s",
                        humantime::format_duration(Duration::from_millis(status.uptime_ms))
                    )
                );
                println!("pid file: {}", color!(base.ui, GREY, "{}", status.pid_file));
                println!(
                    "socket file: {}",
                    color!(base.ui, GREY, "{}", status.sock_file)
                );
            }
        }
        DaemonCommand::Logs => {
            let log_file = if let Ok(log_file) = get_log_file_from_daemon(connector).await {
                log_file
            } else {
                get_log_file_from_folder(base).await?
            };

            let tail = which("tail").map_err(|_| DaemonError::TailNotInstalled)?;

            std::process::Command::new(tail)
                .arg("-f")
                .arg(log_file)
                .status()
                .expect("failed to execute tail");
        }
        DaemonCommand::Clean {
            clean_logs: should_clean_logs,
        } => {
            // try to connect and shutdown the daemon
            let paths = connector.paths.clone();
            let client = connector.connect().await;
            match client {
                Ok(client) => match client.stop().await {
                    Ok(_) => {
                        tracing::trace!("successfully stopped the daemon");
                    }
                    Err(e) => {
                        tracing::trace!("unable to stop the daemon: {:?}", e);
                    }
                },
                Err(e) => {
                    tracing::trace!("unable to connect to the daemon: {:?}", e);
                }
            }
            clean(&paths.pid_file, &paths.sock_file)?;
            if *should_clean_logs {
                clean_logs(&paths.log_folder)?;
            }
            println!("Done");
        }
    };

    Ok(())
}

async fn get_log_file_from_daemon(connector: DaemonConnector) -> Result<String, DaemonError> {
    let mut client = connector.connect().await?;
    let status = client.status().await?;
    Ok(log_filename(&status.log_file)?)
}

async fn get_log_file_from_folder(base: &CommandBase) -> Result<String, DaemonError> {
    warn!("couldn't connect to daemon, looking for old log files");
    let log_folder = base.repo_root.join_components(&[".turbo", "daemon"]);
    let Ok(dir) = std::fs::read_dir(log_folder) else {
        return Err(DaemonError::LogFileNotFound);
    };

    let (latest_file, _) = dir
        .flatten()
        .filter_map(|entry| {
            let modified_time = entry.metadata().ok()?.modified().ok()?;
            Some((entry, modified_time))
        })
        .max_by(|(_, mt1), (_, mt2)| mt1.cmp(mt2))
        .ok_or(DaemonError::LogFileNotFound)?;

    Ok(latest_file
        .path()
        .to_str()
        .expect("log file should be utf-8")
        .to_string())
}

fn clean(pid_file: &AbsoluteSystemPath, sock_file: &AbsoluteSystemPath) -> Result<(), DaemonError> {
    // remove pid and sock files
    let mut success = true;
    trace!("cleaning up daemon files");
    // if the pid_file and sock_file still exist, remove them:
    if pid_file.exists() {
        let result = pid_file.remove_file();
        // ignore this error
        if let Err(e) = result {
            println!("Failed to remove pid file: {}", e);
            println!("Please remove manually: {}", pid_file);
            success = false;
        }
    }
    if sock_file.exists() {
        let result = sock_file.remove_file();
        // ignore this error
        if let Err(e) = result {
            println!("Failed to remove socket file: {}", e);
            println!("Please remove manually: {}", sock_file);
            success = false;
        }
    }

    if success {
        Ok(())
    } else {
        // return error
        Err(DaemonError::CleanFailed)
    }
}

fn clean_logs(log_folder: &AbsoluteSystemPath) -> Result<(), DaemonError> {
    trace!("cleaning up log files");
    // clear all files in the log folder. we want to keep the
    // folder just remove the contents
    // `remove_dir_all_recursive` is lifted from `std`
    log_folder.remove_dir_all().map_err(|e| {
        println!("Failed to remove log files: {}", e);
        println!("Please remove manually: {}", log_folder);
        DaemonError::CleanFailed
    })
}

// log_filename matches the algorithm used by tracing_appender::Rotation::DAILY
// to generate the log filename. This is kind of a hack, but there didn't appear
// to be a simple way to grab the generated filename.
fn log_filename(base_filename: &str) -> Result<String, time::Error> {
    let now = OffsetDateTime::now_utc();
    let format = format_description::parse("[year]-[month]-[day]")?;
    let date = now.format(&format)?;
    Ok(format!("{}.{}", base_filename, date))
}

#[tracing::instrument(skip(base, logging), fields(repo_root = %base.repo_root))]
pub async fn daemon_server(
    base: &CommandBase,
    idle_time: &String,
    logging: &TurboSubscriber,
) -> Result<(), DaemonError> {
    let paths = Paths::from_repo_root(&base.repo_root);

    tracing::trace!("logging to file: {:?}", paths.log_file);
    if let Err(e) = logging.set_daemon_logger(tracing_appender::rolling::daily(
        &paths.log_folder,
        &paths.log_file,
    )) {
        // error here is not fatal, just log it
        tracing::error!("failed to set file logger: {}", e);
    }

    let timeout = go_parse_duration::parse_duration(idle_time)
        .map_err(|_| DaemonError::InvalidTimeout(idle_time.to_owned()))
        .map(|d| Duration::from_nanos(d as u64))?;

    let exit_signal = ctrl_c().map(|result| {
        if let Err(e) = result {
            tracing::error!("Error with signal handling: {}", e);
        }
        CloseReason::Interrupt
    });
    let server =
        crate::daemon::TurboGrpcService::new(base.repo_root.clone(), paths, timeout, exit_signal);

    let reason = server.serve().await?;

    match reason {
        CloseReason::SocketOpenError(SocketOpenError::LockError(AlreadyOwned)) => {
            warn!("daemon already running");
        }
        CloseReason::SocketOpenError(e) => return Err(e.into()),
        CloseReason::Interrupt
        | CloseReason::ServerClosed
        | CloseReason::WatcherClosed
        | CloseReason::Timeout
        | CloseReason::Shutdown => {
            // these are all ok, just exit
            trace!("shutting down daemon: {:?}", reason);
        }
    };

    Ok(())
}

#[derive(serde::Serialize)]
pub struct DaemonStatus {
    pub uptime_ms: u64,
    // this comes from the daemon server, so we trust that
    // it is correct
    pub log_file: Utf8PathBuf,
    pub pid_file: turbopath::AbsoluteSystemPathBuf,
    pub sock_file: turbopath::AbsoluteSystemPathBuf,
}
