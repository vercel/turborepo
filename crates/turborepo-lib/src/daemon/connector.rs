use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use command_group::AsyncCommandGroup;
use log::{debug, error};
use notify::{Config, Event, EventKind, Watcher};
use sysinfo::{ProcessExt, ProcessRefreshKind, RefreshKind, SystemExt};
use thiserror::Error;
use tokio::{net::UnixStream, sync::mpsc, time::timeout};
use tonic::transport::Endpoint;

use super::{client::proto::turbod_client::TurbodClient, DaemonClient};
use crate::daemon::DaemonError;

#[derive(Error, Debug)]
pub enum DaemonConnectorError {
    /// There was a problem when forking to start the daemon.
    #[error("unable to fork")]
    Fork,
    /// There was a problem reading the pid file.
    #[error("could not read pid file")]
    PidFile,
    /// The daemon is not running and will not be started.
    #[error("daemon is not running")]
    NotRunning,
    /// There was an issue connecting to the socket.
    #[error("unable to connect to socket")]
    Socket,
    /// There was an issue performing the handshake.
    #[error("unable to make handshake")]
    Handshake,
    /// Waiting for the socket timed out.
    #[error("timeout while watchin directory: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    /// There was an issue in the file watcher.
    #[error("unable to watch directory: {0}")]
    Watcher(#[from] notify::Error),
}

#[derive(Debug)]
pub struct DaemonConnector {
    /// Whether the connector is allowed to start a daemon if it is not already
    /// running.
    pub can_start_server: bool,
    /// Whether the connector is allowed to kill a running daemon (for example,
    /// in the event of a version mismatch).
    pub can_kill_server: bool,
    pub pid_file: PathBuf,
    pub sock_file: PathBuf,
}

impl DaemonConnector {
    const CONNECT_RETRY_MAX: usize = 3;
    const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
    const SOCKET_TIMEOUT: Duration = Duration::from_secs(1);

    /// Attempt, with retries, to:
    /// 1. find (or start) the daemon process
    /// 2. locate its unix socket
    /// 3. connect to the socket
    /// 4. send the 'hello' message, negotiating versions
    ///
    /// A new server will be spawned (and the old one killed) if
    /// dont_kill is unset and one of these cases is hit:
    /// 1. the versions do not match
    /// 2. the server is not running
    /// 3. the server is unresponsive
    pub async fn connect(self) -> Result<DaemonClient<DaemonConnector>, DaemonConnectorError> {
        let time = Instant::now();
        for _ in 0..Self::CONNECT_RETRY_MAX {
            let pid = self.get_or_start_daemon().await?;
            debug!("got daemon with pid: {}", pid);

            let path = match self.wait_for_socket().await {
                Ok(p) => p,
                Err(_) => continue,
            };

            let conn = Self::get_connection(path.into()).await?;
            let mut client = DaemonClient {
                client: conn,
                connect_settings: (),
            };

            match client.handshake().await {
                Ok(_) => {
                    return {
                        debug!("connected in {}ns", time.elapsed().as_micros());
                        Ok(client.with_connect_settings(self))
                    }
                }
                Err(DaemonError::VersionMismatch) if self.can_kill_server => {
                    self.kill_live_server(client, pid).await?
                }
                Err(DaemonError::Unavailable) => self.kill_dead_server(pid).await?,
                Err(_) => return Err(DaemonConnectorError::Handshake),
            };
        }

        Err(DaemonConnectorError::Socket)
    }

    /// Gets the PID of the daemon process.
    ///
    /// If a daemon is not running, it starts one.
    async fn get_or_start_daemon(&self) -> Result<sysinfo::Pid, DaemonConnectorError> {
        debug!("looking for pid in lockfile: {:?}", self.pid_file);

        let pidfile = self.pid_lock()?;

        match pidfile.get_owner() {
            Some(pid) => {
                debug!("found pid: {}", pid);
                Ok(sysinfo::Pid::from(pid as usize))
            }
            None if self.can_start_server => {
                debug!("no pid found, starting daemon");
                Self::start_daemon().await
            }
            None => Err(DaemonConnectorError::NotRunning),
        }
    }

    /// Starts the daemon process, returning its PID.
    async fn start_daemon() -> Result<sysinfo::Pid, DaemonConnectorError> {
        let binary_path = std::env::current_exe().map_err(|_| DaemonConnectorError::Fork)?;

        // this creates a new process group for the given command
        // in a cross platform way, directing all output to /dev/null
        let mut group = tokio::process::Command::new(binary_path)
            .arg("daemon")
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .group_spawn()
            .map_err(|_| DaemonConnectorError::Fork)?;

        group
            .inner()
            .id()
            .map(|id| sysinfo::Pid::from(id as usize))
            .ok_or(DaemonConnectorError::Fork)
    }

    async fn get_connection(
        path: PathBuf,
    ) -> Result<TurbodClient<tonic::transport::Channel>, DaemonConnectorError> {
        debug!("connecting to socket: {}", path.to_string_lossy());
        let arc = Arc::new(path);

        // note, this endpoint is just a dummy. the actual path is passed in
        let channel = match Endpoint::try_from("http://[::]:50051")
            .expect("this is a valid uri")
            .connect_with_connector(tower::service_fn(move |_| {
                // we clone the reference counter here and move it into the async closure
                let arc = arc.clone();
                async move { UnixStream::connect::<&Path>(arc.as_path()).await }
            }))
            .await
        {
            Ok(c) => c,
            Err(e) => {
                error!("failed to connect to socket: {}", e);
                return Err(DaemonConnectorError::Socket);
            }
        };

        Ok(TurbodClient::new(channel))
    }

    /// Kills a currently active server but shutting it down and waiting for it
    /// to exit.
    async fn kill_live_server(
        &self,
        client: DaemonClient<()>,
        pid: sysinfo::Pid,
    ) -> Result<(), DaemonConnectorError> {
        if client.stop().await.is_err() {
            self.kill_dead_server(pid).await?;
        }

        match timeout(
            Self::SHUTDOWN_TIMEOUT,
            wait_for_file(&self.pid_file, WaitAction::Deleted),
        )
        .await?
        {
            Ok(_) => Ok(()),
            Err(_) => self.kill_dead_server(pid).await,
        }
    }

    /// Kills a server that is not responding.
    async fn kill_dead_server(&self, pid: sysinfo::Pid) -> Result<(), DaemonConnectorError> {
        let lock = self.pid_lock()?;

        let system = sysinfo::System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::new()),
        );

        let owner = lock
            .get_owner()
            .and_then(|p| system.process(sysinfo::Pid::from(p as usize)));

        // if the pidfile is owned by the same pid as the one we found, kill it
        match (pid, owner) {
            (pid, Some(owner)) if pid == owner.pid() => {
                debug!("killing dead server with pid: {}", pid);
                owner.kill();
                Ok(())
            }
            _ => {
                debug!("pidfile is stale, ignoring");
                Err(DaemonConnectorError::PidFile)
            }
        }
    }

    async fn wait_for_socket(&self) -> Result<&Path, DaemonConnectorError> {
        timeout(
            Self::SOCKET_TIMEOUT,
            wait_for_file(&self.sock_file, WaitAction::Exists),
        )
        .await?
        .map(|_| self.sock_file.as_path())
        .map_err(Into::into)
    }

    fn pid_lock(&self) -> Result<pidlock::Pidlock, DaemonConnectorError> {
        self.pid_file
            .to_str()
            .ok_or(DaemonConnectorError::PidFile)
            .map(pidlock::Pidlock::new)
    }
}

/// Waits for a file at some path on the filesystem to be created or deleted.
///
/// It does this by watching the parent directory of the path, and waiting for
/// events on that path.
async fn wait_for_file(path: &Path, action: WaitAction) -> Result<(), notify::Error> {
    let parent = match path.parent() {
        Some(p) => p,
        None => return Ok(()), // the root can neither be created nor deleted
    };

    let file_name = match path.file_name().map(|f| f.to_owned()) {
        Some(p) => Arc::new(p),
        None => return Ok(()), // you cannot watch `..`
    };

    let (tx, mut rx) = mpsc::channel(1);

    let mut watcher = notify::PollWatcher::new(
        move |res| match (res, action) {
            (
                Ok(Event {
                    // for some reason, socket _creation_ is not detected, however,
                    // we can assume that any event except delete implies readiness
                    kind: EventKind::Access(_) | EventKind::Create(_) | EventKind::Modify(_),
                    paths,
                    ..
                }),
                WaitAction::Exists,
            )
            | (
                Ok(Event {
                    kind: EventKind::Remove(_),
                    paths,
                    ..
                }),
                WaitAction::Deleted,
            ) => {
                if paths.iter().any(|p| {
                    p.file_name()
                        .map(|f| file_name.as_os_str().eq(f))
                        .unwrap_or_default()
                }) {
                    futures::executor::block_on(async {
                        tx.send(()).await.expect("will send a message");
                    })
                }
            }
            _ => {}
        },
        Config::default().with_poll_interval(Duration::from_millis(10)),
    )?;

    debug!("creating {:?}", parent);
    std::fs::create_dir_all(parent)?;

    debug!("watching {:?}", parent);
    watcher.watch(parent, notify::RecursiveMode::NonRecursive)?;

    match (action, path.exists()) {
        (WaitAction::Exists, false) => {}
        (WaitAction::Deleted, true) => {}
        _ => return Ok(()),
    };

    rx.recv().await.expect("will receive a message");

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitAction {
    /// Wait for the file to exist.
    Exists,
    /// Wait for the file to be deleted.
    Deleted,
}
