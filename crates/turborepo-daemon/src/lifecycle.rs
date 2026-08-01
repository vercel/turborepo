use std::{future::Future, path::PathBuf, time::Duration};

use pidlock::PidlockError::AlreadyOwned;
use serde::Serialize;
use time::{format_description, OffsetDateTime};
use tracing::{trace, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use which::which;

use crate::{
    endpoint::SocketOpenError, CloseReason, DaemonConnector, DaemonConnectorError, DaemonError,
    PackageChangesWatcher, PackageChangesWatcherArgs, Paths, TurboGrpcService,
};

#[derive(Clone, Copy, Debug)]
pub enum DaemonLifecycleCommand {
    Restart,
    Start,
    Status,
    Stop,
}

#[derive(Debug)]
pub enum DaemonLifecycleOutput {
    Restarted,
    Running,
    Status(Option<DaemonStatus>),
    Stopped,
}

#[derive(Debug, Serialize)]
pub struct DaemonStatus {
    pub uptime_ms: u64,
    pub log_file: String,
    pub pid_file: AbsoluteSystemPathBuf,
    pub sock_file: AbsoluteSystemPathBuf,
}

pub async fn run_lifecycle_command(
    command: DaemonLifecycleCommand,
    repo_root: &AbsoluteSystemPath,
    custom_turbo_json_path: Option<AbsoluteSystemPathBuf>,
) -> Result<DaemonLifecycleOutput, DaemonError> {
    let (can_start_server, can_kill_server) = match command {
        DaemonLifecycleCommand::Status => (false, false),
        DaemonLifecycleCommand::Stop => (false, true),
        DaemonLifecycleCommand::Restart | DaemonLifecycleCommand::Start => (true, true),
    };
    let connector = DaemonConnector::new(
        can_start_server,
        can_kill_server,
        repo_root,
        custom_turbo_json_path,
    )?;

    match command {
        DaemonLifecycleCommand::Restart => {
            let result: Result<_, DaemonError> = async {
                let client = connector
                    .clone()
                    .connect()
                    .await
                    .map_err(DaemonError::DaemonConnect)?;
                client.restart().await
            }
            .await;

            if let Err(error) = result {
                tracing::debug!(
                    ?error,
                    "failed to restart the daemon; falling back to clean"
                );
                clean_daemon_files(&connector.paths.pid_file, &connector.paths.sock_file)?;
                let _ = connector.connect().await?;
            }

            Ok(DaemonLifecycleOutput::Restarted)
        }
        DaemonLifecycleCommand::Start => {
            let _ = connector.connect().await?;
            Ok(DaemonLifecycleOutput::Running)
        }
        DaemonLifecycleCommand::Status => {
            let mut client = match connector.connect().await {
                Ok(client) => client,
                Err(DaemonConnectorError::NotRunning) => {
                    return Ok(DaemonLifecycleOutput::Status(None));
                }
                Err(error) => return Err(error.into()),
            };
            let status = client.status().await?;
            let paths = client.paths();
            Ok(DaemonLifecycleOutput::Status(Some(DaemonStatus {
                uptime_ms: status.uptime_msec,
                log_file: daemon_log_filename(&status.log_file)?,
                pid_file: paths.pid_file.clone(),
                sock_file: paths.sock_file.clone(),
            })))
        }
        DaemonLifecycleCommand::Stop => {
            let client = match connector.connect().await {
                Ok(client) => client,
                Err(DaemonConnectorError::NotRunning) => {
                    return Ok(DaemonLifecycleOutput::Stopped);
                }
                Err(error) => return Err(error.into()),
            };
            client.stop().await?;
            Ok(DaemonLifecycleOutput::Stopped)
        }
    }
}

#[allow(clippy::result_large_err)]
pub fn clean_daemon_files(
    pid_file: &AbsoluteSystemPath,
    sock_file: &AbsoluteSystemPath,
) -> Result<(), DaemonError> {
    let mut success = true;
    trace!("cleaning up daemon files");
    if pid_file.exists() {
        if let Err(error) = pid_file.remove_file() {
            println!("Failed to remove pid file: {error}");
            println!("Please remove manually: {pid_file}");
            success = false;
        }
    }
    if sock_file.exists() {
        if let Err(error) = sock_file.remove_file() {
            println!("Failed to remove socket file: {error}");
            println!("Please remove manually: {sock_file}");
            success = false;
        }
    }

    if success {
        Ok(())
    } else {
        Err(DaemonError::CleanFailed)
    }
}

pub async fn clean_daemon(
    repo_root: &AbsoluteSystemPath,
    custom_turbo_json_path: Option<AbsoluteSystemPathBuf>,
    clean_logs: bool,
) -> Result<(), DaemonError> {
    let connector = DaemonConnector::new(false, true, repo_root, custom_turbo_json_path)?;
    let paths = connector.paths.clone();
    match connector.connect().await {
        Ok(client) => {
            if let Err(error) = client.stop().await {
                trace!(?error, "unable to stop daemon before cleaning");
            }
        }
        Err(error) => trace!(?error, "unable to connect to daemon before cleaning"),
    }

    clean_daemon_files(&paths.pid_file, &paths.sock_file)?;
    if clean_logs {
        clean_daemon_logs(&paths.log_folder)?;
    }
    Ok(())
}

pub async fn follow_daemon_logs(
    repo_root: &AbsoluteSystemPath,
    custom_turbo_json_path: Option<AbsoluteSystemPathBuf>,
) -> Result<(), DaemonError> {
    let connector = DaemonConnector::new(false, false, repo_root, custom_turbo_json_path)?;
    let log_file = if let Ok(log_file) = get_log_file_from_daemon(connector).await {
        log_file
    } else {
        warn!("couldn't connect to daemon, looking for old log files");
        latest_log_file_from_dir(&repo_root.join_components(&[".turbo", "daemon"]))?
    };
    let tail = which("tail").map_err(|_| DaemonError::TailNotInstalled)?;

    if let Err(error) = std::process::Command::new(tail)
        .arg("-f")
        .arg(log_file)
        .status()
    {
        tracing::error!(%error, "failed to execute tail");
    }
    Ok(())
}

pub async fn serve<W, F, S>(
    repo_root: AbsoluteSystemPathBuf,
    timeout: Duration,
    external_shutdown: S,
    custom_turbo_json_path: Option<AbsoluteSystemPathBuf>,
    allow_no_package_manager: bool,
    package_changes_watcher_factory: F,
) -> Result<(), DaemonError>
where
    W: PackageChangesWatcher + 'static,
    F: Fn(PackageChangesWatcherArgs) -> W + Send + Sync + 'static,
    S: Future<Output = CloseReason>,
{
    let paths = Paths::from_repo_root(&repo_root)?;
    let server = TurboGrpcService::new(
        repo_root,
        paths,
        timeout,
        external_shutdown,
        custom_turbo_json_path,
        allow_no_package_manager,
        package_changes_watcher_factory,
    );

    let reason = server.serve().await?;
    match reason {
        CloseReason::SocketOpenError(SocketOpenError::LockError(AlreadyOwned)) => {
            warn!("daemon already running");
        }
        CloseReason::SocketOpenError(error) => return Err(error.into()),
        CloseReason::WatcherSetupError(error) => return Err(error.into()),
        CloseReason::Interrupt
        | CloseReason::ServerClosed
        | CloseReason::WatcherClosed
        | CloseReason::Timeout
        | CloseReason::Shutdown => {
            trace!(?reason, "shutting down daemon");
        }
    }

    Ok(())
}

pub fn daemon_log_filename(base_filename: &str) -> Result<String, time::Error> {
    let now = OffsetDateTime::now_utc();
    let format = format_description::parse("[year]-[month]-[day]")?;
    let date = now.format(&format)?;
    Ok(format!("{base_filename}.{date}"))
}

async fn get_log_file_from_daemon(connector: DaemonConnector) -> Result<PathBuf, DaemonError> {
    let mut client = connector.connect().await?;
    let status = client.status().await?;
    Ok(PathBuf::from(daemon_log_filename(&status.log_file)?))
}

#[allow(clippy::result_large_err)]
fn latest_log_file_from_dir(log_folder: &AbsoluteSystemPath) -> Result<PathBuf, DaemonError> {
    let Ok(dir) = std::fs::read_dir(log_folder) else {
        return Err(DaemonError::LogFileNotFound);
    };

    let (latest_file, _) = dir
        .flatten()
        .filter_map(|entry| {
            let modified_time = entry.metadata().ok()?.modified().ok()?;
            Some((entry, modified_time))
        })
        .max_by(|(_, first), (_, second)| first.cmp(second))
        .ok_or(DaemonError::LogFileNotFound)?;

    Ok(latest_file.path())
}

#[allow(clippy::result_large_err)]
fn clean_daemon_logs(log_folder: &AbsoluteSystemPath) -> Result<(), DaemonError> {
    trace!("cleaning up daemon logs");
    log_folder.remove_dir_all().map_err(|error| {
        println!("Failed to remove log files: {error}");
        println!("Please remove manually: {log_folder}");
        DaemonError::CleanFailed
    })
}

#[cfg(test)]
mod tests {
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn latest_log_file_from_dir_returns_non_utf8_path() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        use turbopath::AbsoluteSystemPathBuf;

        let tempdir = tempfile::tempdir().unwrap();
        let log_folder = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let log_file = log_folder
            .as_std_path()
            .join(OsString::from_vec(b"turbo-\xFF.log".to_vec()));
        std::fs::write(&log_file, "").unwrap();

        assert_eq!(
            super::latest_log_file_from_dir(&log_folder).unwrap(),
            log_file
        );
    }
}
