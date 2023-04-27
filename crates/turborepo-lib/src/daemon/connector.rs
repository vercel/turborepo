use std::{
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use command_group::AsyncCommandGroup;
use notify::{Config, Event, EventKind, Watcher};
use sysinfo::{Pid, ProcessExt, ProcessRefreshKind, RefreshKind, SystemExt};
use thiserror::Error;
use tokio::{sync::mpsc, time::timeout};
use tonic::transport::Endpoint;
use tracing::debug;

use super::{client::proto::turbod_client::TurbodClient, DaemonClient};
use crate::daemon::DaemonError;

#[derive(Error, Debug)]
pub enum DaemonConnectorError {
    /// There was a problem when forking to start the daemon.
    #[error("unable to fork")]
    Fork(ForkError),
    /// There was a problem reading the pid file.
    #[error("the process in the pid is not the daemon ({0}) and still running ({1})")]
    WrongPidProcess(Pid, Pid),
    /// The daemon is not running and will not be started.
    #[error("daemon is not running")]
    NotRunning,
    /// There was an issue connecting to the socket.
    #[error("unable to connect to socket: {0}")]
    Socket(#[from] tonic::transport::Error),
    /// There was an issue performing the handshake.
    #[error("unable to make handshake: {0}")]
    Handshake(#[from] Box<DaemonError>),
    /// Waiting for the socket timed out.
    #[error("timeout while watchin directory: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    /// There was an issue in the file watcher.
    #[error("unable to watch directory: {0}")]
    Watcher(#[from] FileWaitError),

    #[error("unable to connect to daemon after {0} retries")]
    ConnectRetriesExceeded(usize),
}

#[derive(Error, Debug)]
pub enum ForkError {
    #[error("daemon exited before we could connect")]
    Exited,
    #[error("unable to spawn daemon: {0}")]
    Spawn(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct DaemonConnector {
    /// Whether the connector is allowed to start a daemon if it is not already
    /// running.
    pub can_start_server: bool,
    /// Whether the connector is allowed to kill a running daemon (for example,
    /// in the event of a version mismatch).
    pub can_kill_server: bool,
    pub pid_file: turbopath::AbsoluteSystemPathBuf,
    pub sock_file: turbopath::AbsoluteSystemPathBuf,
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

            let conn = match self.get_connection(self.sock_file.clone()).await {
                Err(DaemonConnectorError::Watcher(_) | DaemonConnectorError::Socket(_)) => continue,
                rest => rest?,
            };

            let mut client = DaemonClient::new(conn);

            match client.handshake().await {
                Ok(_) => {
                    return {
                        debug!("connected in {}ms", time.elapsed().as_micros());
                        Ok(client.with_connect_settings(self))
                    }
                }
                Err(DaemonError::VersionMismatch) if self.can_kill_server => {
                    self.kill_live_server(client, pid).await?
                }
                Err(DaemonError::Unavailable) => self.kill_dead_server(pid).await?,
                Err(e) => return Err(DaemonConnectorError::Handshake(Box::new(e))),
            };
        }

        Err(DaemonConnectorError::ConnectRetriesExceeded(
            Self::CONNECT_RETRY_MAX,
        ))
    }

    /// Gets the PID of the daemon process.
    ///
    /// If a daemon is not running, it starts one.
    async fn get_or_start_daemon(&self) -> Result<sysinfo::Pid, DaemonConnectorError> {
        debug!("looking for pid in lockfile: {:?}", self.pid_file);

        let pidfile = self.pid_lock();

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
        let binary_path =
            std::env::current_exe().map_err(|e| DaemonConnectorError::Fork(e.into()))?;

        // this creates a new process group for the given command
        // in a cross platform way, directing all output to /dev/null
        let mut group = tokio::process::Command::new(binary_path)
            .arg("daemon")
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .group()
            .kill_on_drop(false)
            .spawn()
            .map_err(|e| DaemonConnectorError::Fork(e.into()))?;

        group
            .inner()
            .id()
            .map(|id| sysinfo::Pid::from(id as usize))
            .ok_or(DaemonConnectorError::Fork(ForkError::Exited))
    }

    /// Gets a connection to given path
    ///
    /// On Windows the socket file cannot be interacted with via any filesystem
    /// apis, due to this we need to just naively attempt to connect on that
    /// platform and retry in case of error.
    async fn get_connection(
        &self,
        path: turbopath::AbsoluteSystemPathBuf,
    ) -> Result<TurbodClient<tonic::transport::Channel>, DaemonConnectorError> {
        // windows doesn't treat sockets as files, so don't attempt to wait
        #[cfg(not(target_os = "windows"))]
        self.wait_for_socket().await?;

        debug!("connecting to socket: {}", path.to_string_lossy());
        let path = Arc::new(path);

        #[cfg(not(target_os = "windows"))]
        let make_service = move |_| {
            // we clone the reference counter here and move it into the async closure
            let path = path.clone();
            async move { tokio::net::UnixStream::connect(path.as_path()).await }
        };

        #[cfg(target_os = "windows")]
        let make_service = move |_| {
            let path = path.clone();
            async move { win(path) }
        };

        // note, this endpoint is just a dummy. the actual path is passed in
        Endpoint::try_from("http://[::]:50051")
            .expect("this is a valid uri")
            .connect_with_connector(tower::service_fn(make_service))
            .await
            .map(TurbodClient::new)
            .map_err(DaemonConnectorError::Socket)
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
        let lock = self.pid_lock();

        let system = sysinfo::System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::new()),
        );

        let owner = lock
            .get_owner()
            .and_then(|p| system.process(sysinfo::Pid::from(p as usize)));

        // if the pidfile is owned by the same pid as the one we found, kill it
        match owner {
            Some(owner) if pid == owner.pid() => {
                debug!("killing dead server with pid: {}", pid);
                owner.kill();

                Ok(())
            }
            Some(owner) => {
                debug!("pidfile is owned by another process, ignoring");
                Err(DaemonConnectorError::WrongPidProcess(pid, owner.pid()))
            }
            // pidfile has no owner and has been cleaned up so we're ok
            None => Ok(()),
        }
    }

    async fn wait_for_socket(&self) -> Result<(), DaemonConnectorError> {
        timeout(
            Self::SOCKET_TIMEOUT,
            wait_for_file(&self.sock_file, WaitAction::Exists),
        )
        .await?
        .map_err(Into::into)
    }

    fn pid_lock(&self) -> pidlock::Pidlock {
        pidlock::Pidlock::new(self.pid_file.clone().into())
    }
}

#[cfg(target_os = "windows")]
fn win(
    path: Arc<turbopath::AbsoluteSystemPathBuf>,
) -> Result<impl tokio::io::AsyncRead + tokio::io::AsyncWrite, std::io::Error> {
    use tokio_util::compat::FuturesAsyncReadCompatExt;
    uds_windows::UnixStream::connect(&*path)
        .and_then(async_io::Async::new)
        .map(FuturesAsyncReadCompatExt::compat)
}

#[derive(Debug, Error)]
pub enum FileWaitError {
    #[error("failed to register notifier {0}")]
    Notify(#[from] notify::Error),
    #[error("failed to wait for event {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid path {0}")]
    InvalidPath(turbopath::AbsoluteSystemPathBuf),
}

/// Waits for a file at some path on the filesystem to be created or deleted.
///
/// It does this by watching the parent directory of the path, and waiting for
/// events on that path.
async fn wait_for_file(
    path: &turbopath::AbsoluteSystemPathBuf,
    action: WaitAction,
) -> Result<(), FileWaitError> {
    let parent = path
        .parent()
        .ok_or_else(|| FileWaitError::InvalidPath(path.to_owned()))?;

    let file_name = path
        .file_name()
        .map(|f| f.to_owned())
        .ok_or_else(|| FileWaitError::InvalidPath(path.to_owned()))?;

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
                if paths
                    .iter()
                    .any(|p| p.file_name().map(|f| file_name.eq(f)).unwrap_or_default())
                {
                    futures::executor::block_on(async {
                        // if the receiver is dropped, it is because the future has
                        // been cancelled, so we don't need to do anything
                        tx.send(()).await.ok();
                    })
                }
            }
            _ => {}
        },
        Config::default().with_poll_interval(Duration::from_millis(10)),
    )?;

    debug!("creating {:?}", parent);
    std::fs::create_dir_all(parent.as_path())?;

    debug!("watching {:?}", parent);
    watcher.watch(parent.as_path(), notify::RecursiveMode::NonRecursive)?;

    match (action, path.exists()) {
        (WaitAction::Exists, false) => {}
        (WaitAction::Deleted, true) => {}
        _ => return Ok(()),
    };

    // this can only fail if the channel has been closed, which will
    // always happen either after this call ends, or after this future
    // is cancelled
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

#[cfg(test)]
mod test {
    use std::{
        assert_matches::assert_matches,
        path::{Path, PathBuf},
    };

    use sysinfo::Pid;
    use tokio::{
        select,
        sync::{oneshot::Sender, Mutex},
    };
    use tracing::info;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::daemon::client::proto;

    #[cfg(not(target_os = "windows"))]
    const NODE_EXE: &str = "node";
    #[cfg(target_os = "windows")]
    const NODE_EXE: &str = "node.exe";

    fn pid_path(tmp_path: &Path) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::new(tmp_path.join("turbod.pid")).unwrap()
    }

    fn sock_path(tmp_path: &Path) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::new(tmp_path.join("turbod.sock")).unwrap()
    }

    #[tokio::test]
    async fn handles_invalid_pid() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_owned();

        let pid = pid_path(&tmp_path);
        std::fs::write(&pid, "not a pid").unwrap();

        let connector = DaemonConnector {
            pid_file: pid,
            sock_file: sock_path(&tmp_path),
            can_kill_server: false,
            can_start_server: false,
        };

        assert_matches!(
            connector.get_or_start_daemon().await,
            Err(DaemonConnectorError::NotRunning)
        );
    }

    #[tokio::test]
    async fn handles_missing_server_connect() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_owned();

        let pid = pid_path(&tmp_path);
        let sock = sock_path(&tmp_path);

        let connector = DaemonConnector {
            pid_file: pid,
            sock_file: sock,
            can_kill_server: false,
            can_start_server: false,
        };

        assert_matches!(
            connector.connect().await,
            Err(DaemonConnectorError::NotRunning)
        );
    }

    #[tokio::test]
    async fn handles_kill_dead_server_missing_pid() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_owned();

        let pid = pid_path(&tmp_path);
        let sock = sock_path(&tmp_path);

        let connector = DaemonConnector {
            pid_file: pid,
            sock_file: sock,
            can_kill_server: false,
            can_start_server: false,
        };

        assert_matches!(
            connector.kill_dead_server(Pid::from(usize::MAX)).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn handles_kill_dead_server_missing_process() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_owned();

        let pid = pid_path(&tmp_path);
        std::fs::write(&pid, usize::MAX.to_string()).unwrap();
        let sock = sock_path(&tmp_path);
        std::fs::write(&sock, "").unwrap();

        let connector = DaemonConnector {
            pid_file: pid,
            sock_file: sock,
            can_kill_server: false,
            can_start_server: false,
        };

        assert_matches!(
            connector.kill_dead_server(Pid::from(usize::MAX)).await,
            Ok(())
        );

        assert!(connector.pid_file.exists(), "pid file should still exist");
    }

    #[tokio::test]
    async fn handles_kill_dead_server_wrong_process() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_owned();

        let proc = tokio::process::Command::new(NODE_EXE)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("-e")
            .arg("setInterval(() => {}, 1000)")
            .spawn()
            .unwrap();

        let pid = pid_path(&tmp_path);
        std::fs::write(&pid, proc.id().unwrap().to_string()).unwrap();
        let sock = sock_path(&tmp_path);
        std::fs::write(&sock, "").unwrap();

        let connector = DaemonConnector {
            pid_file: pid,
            sock_file: sock,
            can_kill_server: true,
            can_start_server: false,
        };

        let kill_pid = Pid::from(usize::MAX);
        let proc_id = Pid::from(proc.id().unwrap() as usize);

        assert_matches!(
            connector.kill_dead_server(kill_pid).await,
            Err(DaemonConnectorError::WrongPidProcess(daemon, running)) if daemon == kill_pid && running == proc_id
        );

        assert!(connector.pid_file.exists(), "pid file should still exist");
    }

    #[tokio::test]
    async fn handles_kill_dead_server() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_owned();

        let proc = tokio::process::Command::new(NODE_EXE)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("-e")
            .arg("setInterval(() => {}, 1000)")
            .spawn()
            .unwrap();

        let pid = pid_path(&tmp_path);
        std::fs::write(&pid, proc.id().unwrap().to_string()).unwrap();
        let sock = sock_path(&tmp_path);
        std::fs::write(&sock, "").unwrap();

        let connector = DaemonConnector {
            pid_file: pid,
            sock_file: sock,
            can_kill_server: true,
            can_start_server: false,
        };

        assert_matches!(
            connector
                .kill_dead_server(Pid::from(proc.id().unwrap() as usize))
                .await,
            Ok(())
        );

        assert!(connector.pid_file.exists(), "pid file should still exist");
    }

    struct DummyServer {
        shutdown: Mutex<Option<Sender<bool>>>,
    }

    #[tonic::async_trait]
    impl proto::turbod_server::Turbod for DummyServer {
        async fn shutdown(
            &self,
            req: tonic::Request<proto::ShutdownRequest>,
        ) -> tonic::Result<tonic::Response<proto::ShutdownResponse>> {
            info!("shutdown request: {:?}", req);
            self.shutdown
                .lock()
                .await
                .take()
                .unwrap()
                .send(true)
                .unwrap();
            Ok(tonic::Response::new(proto::ShutdownResponse {}))
        }

        async fn hello(
            &self,
            _req: tonic::Request<proto::HelloRequest>,
        ) -> tonic::Result<tonic::Response<proto::HelloResponse>> {
            unimplemented!()
        }

        async fn status(
            &self,
            _req: tonic::Request<proto::StatusRequest>,
        ) -> tonic::Result<tonic::Response<proto::StatusResponse>> {
            unimplemented!()
        }

        async fn notify_outputs_written(
            &self,
            _req: tonic::Request<proto::NotifyOutputsWrittenRequest>,
        ) -> tonic::Result<tonic::Response<proto::NotifyOutputsWrittenResponse>> {
            unimplemented!()
        }

        async fn get_changed_outputs(
            &self,
            _req: tonic::Request<proto::GetChangedOutputsRequest>,
        ) -> tonic::Result<tonic::Response<proto::GetChangedOutputsResponse>> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn handles_kill_live_server() {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        // set up the server
        let stream = async_stream::stream! {
            while let Some(item) = rx.recv().await {
                yield item;
            }
        };

        let server_fut = tonic::transport::Server::builder()
            .add_service(proto::turbod_server::TurbodServer::new(DummyServer {
                shutdown: Mutex::new(Some(shutdown_tx)),
            }))
            .serve_with_incoming(stream);

        let (pid_file, sock_file) = if cfg!(windows) {
            (
                AbsoluteSystemPathBuf::new(PathBuf::from("C:\\pid")).unwrap(),
                AbsoluteSystemPathBuf::new(PathBuf::from("C:\\sock")).unwrap(),
            )
        } else {
            (
                AbsoluteSystemPathBuf::new(PathBuf::from("/pid")).unwrap(),
                AbsoluteSystemPathBuf::new(PathBuf::from("/sock")).unwrap(),
            )
        };

        // set up the client
        let conn = DaemonConnector {
            pid_file,
            sock_file,
            can_kill_server: false,
            can_start_server: false,
        };

        let client = Endpoint::try_from("http://[::]:50051")
            .expect("this is a valid uri")
            .connect_with_connector(tower::service_fn(move |_| {
                // when a connection is made, create a duplex stream and send it to the server
                let tx = tx.clone();
                async move {
                    let (client, server) = tokio::io::duplex(1024);
                    let server: Result<_, anyhow::Error> = Ok(server);
                    let client: Result<_, anyhow::Error> = Ok(client);
                    tx.send(server).await.unwrap();
                    client
                }
            }))
            .await
            .map(TurbodClient::new)
            .unwrap();

        let client = DaemonClient::new(client);

        let shutdown_fut = conn.kill_live_server(client, Pid::from(1000));

        // drive the futures to completion
        select! {
            _ = shutdown_fut => {}
            _ = server_fut => panic!("server should not have shut down first"),
        }

        assert!(
            shutdown_rx.await.is_ok(),
            "shutdown should have been received"
        )
    }
}
