use std::{
    ffi::OsStr,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use command_group::AsyncCommandGroup;
use notify::{Config, Event, EventKind, Watcher};
use pidlock::PidFileError;
use sysinfo::{Pid, ProcessExt, ProcessRefreshKind, RefreshKind, SystemExt};
use thiserror::Error;
use tokio::{sync::mpsc, time::timeout};
use tonic::transport::Endpoint;
use tracing::debug;
use turbopath::AbsoluteSystemPath;

use super::{proto::turbod_client::TurbodClient, DaemonClient, Paths};
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
    #[error("timeout while watching directory: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    /// There was an issue in the file watcher.
    #[error("unable to watch directory: {0}")]
    Watcher(#[from] FileWaitError),

    #[error("unable to connect to daemon after {0} retries")]
    ConnectRetriesExceeded(usize),

    #[error("unable to use pid file: {0}")]
    PidFile(#[from] PidFileError),
}

#[derive(Error, Debug)]
pub enum ForkError {
    #[error("daemon exited before we could connect")]
    Exited,
    #[error("unable to spawn daemon: {0}")]
    Spawn(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct DaemonConnector {
    /// Whether the connector is allowed to start a daemon if it is not already
    /// running.
    pub can_start_server: bool,
    /// Whether the connector is allowed to kill a running daemon (for example,
    /// in the event of a version mismatch).
    pub can_kill_server: bool,
    pub paths: Paths,
    /// Optional custom turbo.json path to watch
    pub custom_turbo_json_path: Option<turbopath::AbsoluteSystemPathBuf>,
}

impl DaemonConnector {
    pub fn new(
        can_start_server: bool,
        can_kill_server: bool,
        repo_root: &AbsoluteSystemPath,
        custom_turbo_json_path: Option<&turbopath::AbsoluteSystemPathBuf>,
    ) -> Self {
        let paths = Paths::from_repo_root(repo_root);
        Self {
            can_start_server,
            can_kill_server,
            paths,
            custom_turbo_json_path: custom_turbo_json_path.cloned(),
        }
    }

    pub fn with_custom_turbo_json_path(mut self, path: turbopath::AbsoluteSystemPathBuf) -> Self {
        self.custom_turbo_json_path = Some(path);
        self
    }

    const CONNECT_RETRY_MAX: usize = 3;
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
    const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
    const SOCKET_TIMEOUT: Duration = Duration::from_secs(1);
    const SOCKET_ERROR_WAIT: Duration = Duration::from_millis(50);

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
    #[tracing::instrument(skip(self))]
    pub async fn connect(self) -> Result<DaemonClient<DaemonConnector>, DaemonConnectorError> {
        let time = Instant::now();
        for _ in 0..Self::CONNECT_RETRY_MAX {
            let pid = self.get_or_start_daemon().await?;
            debug!("got daemon with pid: {}", pid);

            let conn = match self.get_connection(self.paths.sock_file.clone()).await {
                Err(DaemonConnectorError::Watcher(_)) => continue,
                Err(DaemonConnectorError::Socket(e)) => {
                    // assume the server is not yet ready
                    debug!("socket error: {}", e);
                    tokio::time::sleep(DaemonConnector::SOCKET_ERROR_WAIT).await;
                    continue;
                }
                rest => rest?,
            };

            let mut client = DaemonClient::new(conn);

            match client.handshake().await {
                Ok(_) => {
                    return {
                        debug!("connected in {}µs", time.elapsed().as_micros());
                        Ok(client.with_connect_settings(self))
                    }
                }
                Err(DaemonError::VersionMismatch(_)) if self.can_kill_server => {
                    self.kill_live_server(client, pid).await?
                }
                Err(DaemonError::Unavailable(_)) => self.kill_dead_server(pid).await?,
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
        debug!("looking for pid in lockfile: {:?}", self.paths.pid_file);

        let pidfile = self.pid_lock();

        match pidfile.get_owner()? {
            Some(pid) => {
                debug!("found pid: {}", pid);
                Ok(sysinfo::Pid::from(pid as usize))
            }
            None if self.can_start_server => {
                debug!("no pid found, starting daemon");
                Self::start_daemon(&self.custom_turbo_json_path).await
            }
            None => Err(DaemonConnectorError::NotRunning),
        }
    }

    /// Starts the daemon process, returning its PID.
    async fn start_daemon(
        custom_turbo_json_path: &Option<turbopath::AbsoluteSystemPathBuf>,
    ) -> Result<sysinfo::Pid, DaemonConnectorError> {
        let binary_path =
            std::env::current_exe().map_err(|e| DaemonConnectorError::Fork(e.into()))?;
        // this creates a new process group for the given command
        // in a cross platform way, directing all output to /dev/null
        let mut command = tokio::process::Command::new(binary_path);
        command.arg("--skip-infer").arg("daemon");

        // Pass custom turbo.json path if specified
        if let Some(path) = custom_turbo_json_path {
            command.arg("--turbo-json-path").arg(path.as_str());
        }

        let mut group = command
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
    #[tracing::instrument(skip(self))]
    async fn get_connection(
        &self,
        path: turbopath::AbsoluteSystemPathBuf,
    ) -> Result<TurbodClient<tonic::transport::Channel>, DaemonConnectorError> {
        // windows doesn't treat sockets as files, so don't attempt to wait
        #[cfg(not(target_os = "windows"))]
        self.wait_for_socket().await?;

        debug!("connecting to socket: {}", path);
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

        // note, this endpoint is just a placeholder. the actual path is passed in via
        // make_service
        Endpoint::try_from("http://[::]:50051")
            .expect("this is a valid uri")
            .timeout(Self::CONNECT_TIMEOUT)
            .connect_with_connector(tower::service_fn(make_service))
            .await
            .map(TurbodClient::new)
            .map_err(DaemonConnectorError::Socket)
    }

    /// Kills a currently active server by shutting it down and waiting for it
    /// to exit.
    #[tracing::instrument(skip(self, client))]
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
            wait_for_file(&self.paths.pid_file, WaitAction::Deleted),
        )
        .await?
        {
            Ok(_) => Ok(()),
            Err(_) => self.kill_dead_server(pid).await,
        }
    }

    /// Kills a server that is not responding.
    #[tracing::instrument(skip(self))]
    async fn kill_dead_server(&self, pid: sysinfo::Pid) -> Result<(), DaemonConnectorError> {
        let lock = self.pid_lock();

        let system = sysinfo::System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::new()),
        );

        let owner = lock
            .get_owner()?
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

    #[tracing::instrument(skip(self))]
    async fn wait_for_socket(&self) -> Result<(), DaemonConnectorError> {
        // Note that we don't care if this is our daemon
        // or not. We started a process, but someone else could beat
        // use to listening. That's fine, we'll check the version
        // later. However, we need to ensure that _some_ pid file
        // exists to protect against stale .sock files
        timeout(
            Self::SOCKET_TIMEOUT,
            wait_for_file(&self.paths.pid_file, WaitAction::Exists),
        )
        .await??;
        timeout(
            Self::SOCKET_TIMEOUT,
            wait_for_file(&self.paths.sock_file, WaitAction::Exists),
        )
        .await?
        .map_err(Into::into)
    }

    fn pid_lock(&self) -> pidlock::Pidlock {
        pidlock::Pidlock::new(self.paths.pid_file.clone().into())
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
#[tracing::instrument]
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
                if paths.iter().any(|p| {
                    p.file_name()
                        .map(|f| OsStr::new(&file_name).eq(f))
                        .unwrap_or_default()
                }) {
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
    watcher.watch(parent.as_std_path(), notify::RecursiveMode::NonRecursive)?;

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
    use std::assert_matches::assert_matches;

    use tokio::{
        select,
        sync::{oneshot::Sender, Mutex},
    };
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::{Request, Response, Status};
    use tower::ServiceBuilder;
    use tracing::info;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::daemon::{
        default_timeout_layer::DefaultTimeoutLayer,
        proto::{self, PackageChangesRequest},
    };

    #[cfg(not(target_os = "windows"))]
    const NODE_EXE: &str = "node";
    #[cfg(target_os = "windows")]
    const NODE_EXE: &str = "node.exe";

    #[tokio::test]
    async fn handles_invalid_pid() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();

        let connector = DaemonConnector {
            can_start_server: false,
            can_kill_server: false,
            paths: Paths::from_repo_root(&repo_root),
            custom_turbo_json_path: None,
        };
        connector.paths.pid_file.ensure_dir().unwrap();
        connector
            .paths
            .pid_file
            .create_with_contents("not a pid")
            .unwrap();

        assert_matches!(
            connector.get_or_start_daemon().await,
            Err(DaemonConnectorError::PidFile(PidFileError::Invalid { .. }))
        );
    }

    #[tokio::test]
    async fn handles_missing_server_connect() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let connector = DaemonConnector {
            can_start_server: false,
            can_kill_server: false,
            paths: Paths::from_repo_root(&repo_root),
            custom_turbo_json_path: None,
        };

        assert_matches!(
            connector.connect().await,
            Err(DaemonConnectorError::NotRunning)
        );
    }

    #[tokio::test]
    async fn handles_kill_dead_server_missing_pid() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let connector = DaemonConnector {
            can_start_server: false,
            can_kill_server: false,
            paths: Paths::from_repo_root(&repo_root),
            custom_turbo_json_path: None,
        };

        assert_matches!(
            connector.kill_dead_server(Pid::from(usize::MAX)).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn handles_kill_dead_server_missing_process() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let connector = DaemonConnector {
            can_start_server: false,
            can_kill_server: false,
            paths: Paths::from_repo_root(&repo_root),
            custom_turbo_json_path: None,
        };

        connector.paths.pid_file.ensure_dir().unwrap();
        connector
            .paths
            .pid_file
            .create_with_contents(i32::MAX.to_string())
            .unwrap();
        connector.paths.sock_file.ensure_dir().unwrap();
        connector.paths.sock_file.create_with_contents("").unwrap();

        assert_matches!(
            connector.kill_dead_server(Pid::from(usize::MAX)).await,
            Ok(())
        );

        assert!(
            !connector.paths.pid_file.exists(),
            "pid file should be cleaned up when getting the owner of a stale pid"
        );
    }

    #[tokio::test]
    async fn handles_kill_dead_server_wrong_process() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let connector = DaemonConnector {
            can_start_server: false,
            can_kill_server: false,
            paths: Paths::from_repo_root(&repo_root),
            custom_turbo_json_path: None,
        };

        let proc = tokio::process::Command::new(NODE_EXE)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("-e")
            .arg("setInterval(() => {}, 1000)")
            .spawn()
            .unwrap();

        connector.paths.pid_file.ensure_dir().unwrap();
        connector
            .paths
            .pid_file
            .create_with_contents(proc.id().unwrap().to_string())
            .unwrap();
        connector.paths.sock_file.ensure_dir().unwrap();
        connector.paths.sock_file.create_with_contents("").unwrap();

        let kill_pid = Pid::from(usize::MAX);
        let proc_id = Pid::from(proc.id().unwrap() as usize);

        assert_matches!(
            connector.kill_dead_server(kill_pid).await,
            Err(DaemonConnectorError::WrongPidProcess(daemon, running)) if daemon == kill_pid && running == proc_id
        );

        assert!(
            connector.paths.pid_file.exists(),
            "pid file should still exist"
        );
    }

    #[tokio::test]
    async fn handles_kill_dead_server() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let connector = DaemonConnector {
            can_start_server: false,
            can_kill_server: true,
            paths: Paths::from_repo_root(&repo_root),
            custom_turbo_json_path: None,
        };

        let proc = tokio::process::Command::new(NODE_EXE)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("-e")
            .arg("setInterval(() => {}, 1000)")
            .spawn()
            .unwrap();

        connector.paths.pid_file.ensure_dir().unwrap();
        connector
            .paths
            .pid_file
            .create_with_contents(proc.id().unwrap().to_string())
            .unwrap();
        connector.paths.sock_file.ensure_dir().unwrap();
        connector.paths.sock_file.create_with_contents("").unwrap();

        assert_matches!(
            connector
                .kill_dead_server(Pid::from(proc.id().unwrap() as usize))
                .await,
            Ok(())
        );

        assert!(
            connector.paths.pid_file.exists(),
            "pid file should still exist"
        );
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
            request: tonic::Request<proto::HelloRequest>,
        ) -> tonic::Result<tonic::Response<proto::HelloResponse>> {
            let client_version = request.into_inner().version;
            Err(tonic::Status::failed_precondition(format!(
                "version mismatch. Client {} Server test-version",
                client_version
            )))
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

        async fn discover_packages(
            &self,
            _req: tonic::Request<proto::DiscoverPackagesRequest>,
        ) -> Result<tonic::Response<proto::DiscoverPackagesResponse>, tonic::Status> {
            unimplemented!()
        }

        async fn discover_packages_blocking(
            &self,
            _req: tonic::Request<proto::DiscoverPackagesRequest>,
        ) -> Result<tonic::Response<proto::DiscoverPackagesResponse>, tonic::Status> {
            unimplemented!()
        }

        type PackageChangesStream = ReceiverStream<Result<proto::PackageChangeEvent, Status>>;
        async fn package_changes(
            &self,
            _req: Request<PackageChangesRequest>,
        ) -> Result<Response<Self::PackageChangesStream>, Status> {
            unimplemented!()
        }

        async fn get_file_hashes(
            &self,
            _req: tonic::Request<proto::GetFileHashesRequest>,
        ) -> Result<tonic::Response<proto::GetFileHashesResponse>, tonic::Status> {
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

        let service = ServiceBuilder::new().layer(DefaultTimeoutLayer).service(
            proto::turbod_server::TurbodServer::new(DummyServer {
                shutdown: Mutex::new(Some(shutdown_tx)),
            }),
        );

        let server_fut = tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(stream);

        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let connector = DaemonConnector {
            can_start_server: false,
            can_kill_server: false,
            paths: Paths::from_repo_root(&repo_root),
            custom_turbo_json_path: None,
        };

        let mut client = Endpoint::try_from("http://[::]:50051")
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

        // spawn the future for the server so that it responds to
        // the hello request
        let server_fut = tokio::spawn(server_fut);

        let hello_resp: DaemonError = client
            .hello(proto::HelloRequest {
                version: "version-mismatch".to_string(),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .into();
        assert_matches!(hello_resp, DaemonError::VersionMismatch(_));
        let client = DaemonClient::new(client);

        let shutdown_fut = connector.kill_live_server(client, Pid::from(1000));

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
