use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use command_group::AsyncCommandGroup;
use log::{debug, error};
use notify::{Config, Event, EventKind, RecommendedWatcher, Watcher};
use sysinfo::{ProcessExt, ProcessRefreshKind, RefreshKind, SystemExt};
use tokio::{net::UnixStream, sync::mpsc, time::timeout};
use tonic::transport::Endpoint;

use super::{client::proto::turbod_client::TurbodClient, DaemonClient, DaemonError};

#[derive(Debug)]
pub struct DaemonConnector {
    pub dont_start: bool,
    pub dont_kill: bool,
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
    pub async fn connect(self) -> Result<DaemonClient<DaemonConnector>, DaemonError> {
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

            match (client.handshake().await, &self.dont_kill) {
                (Ok(_), _) => return Ok(client.with_connect_settings(self)),
                // should be able to opt out of kill
                (Err(DaemonError::VersionMismatch), true) => {
                    return Err(DaemonError::VersionMismatch)
                }
                (Err(DaemonError::VersionMismatch), false) => {
                    self.kill_live_server(client, pid).await?;
                    continue;
                }
                (Err(DaemonError::Connection), _) => {
                    self.kill_dead_server(pid).await?;
                    continue;
                }
                // unhandled error
                (Err(e), _) => return Err(e),
            };
        }

        Err(DaemonError::Connection)
    }

    /// Gets the PID of the daemon process.
    ///
    /// If a daemon is not running, it starts one.
    async fn get_or_start_daemon(&self) -> Result<sysinfo::Pid, DaemonError> {
        debug!("looking for pid in lockfile: {:?}", self.pid_file);

        let pidfile = pidlock::Pidlock::new(self.pid_file.to_str().ok_or(DaemonError::PidFile)?);

        match (pidfile.get_owner(), self.dont_start) {
            (Some(pid), _) => {
                debug!("found pid: {}", pid);
                Ok(sysinfo::Pid::from(pid as usize))
            }
            (None, false) => {
                debug!("no pid found, starting daemon");
                Self::start_daemon().await
            }
            (None, true) => Err(DaemonError::NotRunning),
        }
    }

    /// Starts the daemon process, returning its PID.
    async fn start_daemon() -> Result<sysinfo::Pid, DaemonError> {
        let binary_path = std::env::current_exe().map_err(|_| DaemonError::Fork)?;

        // this creates a new process group for the given command
        // in a cross platform way, directing all output to /dev/null
        let mut group = tokio::process::Command::new(binary_path)
            .arg("daemon")
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .group_spawn()
            .map_err(|_| DaemonError::Fork)?;

        group
            .inner()
            .id()
            .map(|id| sysinfo::Pid::from(id as usize))
            .ok_or(DaemonError::Fork)
    }

    async fn get_connection(
        path: PathBuf,
    ) -> Result<TurbodClient<tonic::transport::Channel>, DaemonError> {
        debug!("connecting to socket: {}", path.to_string_lossy());
        let arc = Arc::new(path);

        // note, this path is just a dummy. the actual path is passed in
        let channel = match Endpoint::try_from("http://[::]:50051")?
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
                return Err(DaemonError::Connection);
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
    ) -> Result<(), DaemonError> {
        if client.stop().await.is_err() {
            self.kill_dead_server(pid).await?;
        }

        match timeout(
            Self::SHUTDOWN_TIMEOUT,
            wait_for_file(&self.pid_file, WaitAction::Missing),
        )
        .await?
        {
            Ok(_) => Ok(()),
            Err(_) => self.kill_dead_server(pid).await,
        }
    }

    /// Kills a server that is not responding.
    async fn kill_dead_server(&self, pid: sysinfo::Pid) -> Result<(), DaemonError> {
        let lock = pidlock::Pidlock::new(self.pid_file.to_str().ok_or(DaemonError::PidFile)?);

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
                Err(DaemonError::PidFile)
            }
        }
    }

    async fn wait_for_socket(&self) -> Result<&Path, DaemonError> {
        timeout(
            Self::SOCKET_TIMEOUT,
            wait_for_file(&self.sock_file, WaitAction::Exists),
        )
        .await?
        .map(|_| self.sock_file.as_path())
        .map_err(|_| DaemonError::Connection)
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

    match (action, path.exists()) {
        (WaitAction::Exists, false) => {}
        (WaitAction::Missing, true) => {}
        _ => return Ok(()),
    };

    let (tx, mut rx) = mpsc::channel(1);

    let mut watcher = RecommendedWatcher::new(
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
                WaitAction::Missing,
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
        Config::default(),
    )?;

    debug!("creating {:?}", parent);
    std::fs::create_dir_all(parent)?;

    debug!("watching {:?}", parent);
    watcher.watch(parent, notify::RecursiveMode::NonRecursive)?;
    rx.recv().await.expect("will receive a message");

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitAction {
    /// Wait for the file to exist.
    Exists,
    /// Wait for the file to be deleted.
    Missing,
}
