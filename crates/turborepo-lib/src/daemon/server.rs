//! Daemon Server
//!
//! This module houses the daemon server, some implementation notes for which
//! are below.
//!
//! ## Implementation Notes
//!
//! The basic goals of the daemon are to watch for, and be able to provide
//! details about, filesystem changes. It is organised as an async server, which
//! holds a `HashGlobWatcher` which holds data about hashes, globs to watch for
//! that hash, and files that have been updated for that hash. In addition, this
//! server can be interrogated over grpc to register interest in particular
//! globs, and to query for changes for those globs.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use futures::Future;
use semver::Version;
use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot, watch, Mutex as AsyncMutex},
};
use tonic::transport::{NamedService, Server};
use tower::ServiceBuilder;
use tracing::{error, info, trace, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_filewatch::{
    cookie_jar::CookieJar,
    globwatcher::{Error as GlobWatcherError, GlobError, GlobSet, GlobWatcher},
    package_watcher::PackageWatcher,
    FileSystemWatcher, WatchError,
};
use turborepo_repository::discovery::{
    LocalPackageDiscoveryBuilder, PackageDiscovery, PackageDiscoveryBuilder,
};

use super::{
    bump_timeout::BumpTimeout,
    endpoint::SocketOpenError,
    proto::{self},
};
use crate::{
    daemon::{bump_timeout_layer::BumpTimeoutLayer, endpoint::listen_socket},
    run::package_discovery::WatchingPackageDiscovery,
};

#[derive(Debug)]
#[allow(dead_code)]
pub enum CloseReason {
    Timeout,
    Shutdown,
    WatcherClosed,
    ServerClosed,
    Interrupt,
    SocketOpenError(SocketOpenError),
}

pub struct FileWatching {
    _watcher: FileSystemWatcher,
    pub glob_watcher: GlobWatcher,
    pub package_watcher: PackageWatcher,
}

#[derive(Debug, Error)]
enum RpcError {
    #[error("deadline exceeded")]
    DeadlineExceeded,
    #[error("invalid glob: {0}")]
    InvalidGlob(#[from] GlobError),
    #[error("globwatching failed: {0}")]
    GlobWatching(#[from] GlobWatcherError),
    #[error("filewatching unavailable")]
    NoFileWatching,
}

impl From<RpcError> for tonic::Status {
    fn from(value: RpcError) -> Self {
        match value {
            RpcError::DeadlineExceeded => {
                tonic::Status::deadline_exceeded("failed to load filewatching in time")
            }
            RpcError::InvalidGlob(e) => tonic::Status::invalid_argument(e.to_string()),
            RpcError::GlobWatching(e) => tonic::Status::unavailable(e.to_string()),
            RpcError::NoFileWatching => tonic::Status::unavailable("filewatching unavailable"),
        }
    }
}

async fn start_filewatching<PD: PackageDiscovery + Send + 'static>(
    repo_root: AbsoluteSystemPathBuf,
    watcher_tx: watch::Sender<Option<Arc<FileWatching>>>,
    backup_discovery: PD,
) -> Result<(), WatchError> {
    let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).await?;
    let cookie_jar = CookieJar::new(
        watcher.cookie_dir(),
        Duration::from_millis(100),
        watcher.subscribe(),
    );
    let glob_watcher = GlobWatcher::new(&repo_root, cookie_jar, watcher.subscribe());
    let package_watcher =
        PackageWatcher::new(repo_root.clone(), watcher.subscribe(), backup_discovery)
            .await
            .map_err(|e| WatchError::Setup(format!("{:?}", e)))?;
    // We can ignore failures here, it means the server is shutting down and
    // receivers have gone out of scope.
    let _ = watcher_tx.send(Some(Arc::new(FileWatching {
        _watcher: watcher,
        glob_watcher,
        package_watcher,
    })));
    Ok(())
}

/// Timeout for every RPC the server handles
const REQUEST_TIMEOUT: Duration = Duration::from_millis(100);

pub struct TurboGrpcService<S, PDA, PDB> {
    watcher_tx: watch::Sender<Option<Arc<FileWatching>>>,
    watcher_rx: watch::Receiver<Option<Arc<FileWatching>>>,
    repo_root: AbsoluteSystemPathBuf,
    daemon_root: AbsoluteSystemPathBuf,
    log_file: AbsoluteSystemPathBuf,
    timeout: Duration,
    external_shutdown: S,

    package_discovery: PDA,
    package_discovery_backup: PDB,
}

impl<S> TurboGrpcService<S, WatchingPackageDiscovery, LocalPackageDiscoveryBuilder>
where
    S: Future<Output = CloseReason>,
{
    /// Create a gRPC server providing the Turbod interface. external_shutdown
    /// can be used to deliver a signal to shutdown the server. This is expected
    /// to be wired to signal handling. By default, the server will set up a
    /// file system watcher for the purposes of managing package discovery
    /// state, and use a `LocalPackageDiscovery` instance to refresh the
    /// state if the filewatcher encounters errors.
    pub fn new(
        repo_root: AbsoluteSystemPathBuf,
        daemon_root: AbsoluteSystemPathBuf,
        log_file: AbsoluteSystemPathBuf,
        timeout: Duration,
        external_shutdown: S,
    ) -> Self {
        let (watcher_tx, watcher_rx) = watch::channel(None);

        let package_discovery = WatchingPackageDiscovery::new(watcher_rx.clone());
        let package_discovery_backup =
            LocalPackageDiscoveryBuilder::new(repo_root.clone(), None, None);

        // Run the actual service. It takes ownership of the struct given to it,
        // so we use a private struct with just the pieces of state needed to handle
        // RPCs.
        TurboGrpcService {
            watcher_tx,
            watcher_rx,
            repo_root,
            daemon_root,
            log_file,
            timeout,
            external_shutdown,
            package_discovery,
            package_discovery_backup,
        }
    }
}

impl<S, PDA, PDB> TurboGrpcService<S, PDA, PDB>
where
    S: Future<Output = CloseReason>,
    PDA: PackageDiscovery + Send + 'static,
    PDB: PackageDiscoveryBuilder,
    PDB::Output: PackageDiscovery + Send + 'static,
{
    /// If errors are encountered when loading the package discovery, this
    /// builder will be used as a backup to refresh the state.
    pub fn with_package_discovery_backup<PDB2: PackageDiscoveryBuilder>(
        self,
        package_discovery_backup: PDB2,
    ) -> TurboGrpcService<S, PDA, PDB2> {
        TurboGrpcService {
            package_discovery: self.package_discovery,
            daemon_root: self.daemon_root,
            external_shutdown: self.external_shutdown,
            log_file: self.log_file,
            repo_root: self.repo_root,
            timeout: self.timeout,
            watcher_rx: self.watcher_rx,
            watcher_tx: self.watcher_tx,
            package_discovery_backup,
        }
    }

    pub async fn serve(self) -> Result<CloseReason, PDB::Error> {
        let Self {
            watcher_tx,
            watcher_rx,
            daemon_root,
            external_shutdown,
            log_file,
            repo_root,
            timeout,
            package_discovery,
            package_discovery_backup,
        } = self;

        let running = Arc::new(AtomicBool::new(true));
        let (_pid_lock, stream) = match listen_socket(&daemon_root, running.clone()).await {
            Ok((pid_lock, stream)) => (pid_lock, stream),
            Err(e) => return Ok(CloseReason::SocketOpenError(e)),
        };
        trace!("acquired connection stream for socket");

        let watcher_repo_root = repo_root.to_owned();
        // A channel to trigger the shutdown of the gRPC server. This is handed out
        // to components internal to the server process such as root watching, as
        // well as available to the gRPC server itself to handle the shutdown RPC.
        let (trigger_shutdown, mut shutdown_signal) = mpsc::channel::<()>(1);

        let backup_discovery = package_discovery_backup.build()?;

        // watch receivers as a group own the filewatcher, which will exit when
        // all references are dropped.
        let fw_shutdown = trigger_shutdown.clone();
        let fw_handle = tokio::task::spawn(async move {
            if let Err(e) =
                start_filewatching(watcher_repo_root, watcher_tx, backup_discovery).await
            {
                error!("filewatching failed to start: {}", e);
                let _ = fw_shutdown.send(()).await;
            }
            info!("filewatching started");
        });
        // exit_root_watch delivers a signal to the root watch loop to exit.
        // In the event that the server shuts down via some other mechanism, this
        // cleans up root watching task.
        let (exit_root_watch, root_watch_exit_signal) = oneshot::channel();
        let watch_root_handle = tokio::task::spawn(watch_root(
            watcher_rx.clone(),
            repo_root.to_owned(),
            trigger_shutdown.clone(),
            root_watch_exit_signal,
        ));

        let bump_timeout = Arc::new(BumpTimeout::new(timeout));
        let timeout_fut = bump_timeout.wait();

        // when one of these futures complete, let the server gracefully shutdown
        let (grpc_shutdown_tx, shutdown_reason) = oneshot::channel();
        let shutdown_fut = async move {
            select! {
                _ = shutdown_signal.recv() => grpc_shutdown_tx.send(CloseReason::Shutdown).ok(),
                _ = timeout_fut => grpc_shutdown_tx.send(CloseReason::Timeout).ok(),
                reason = external_shutdown => grpc_shutdown_tx.send(reason).ok(),
            };
        };

        // Run the actual service. It takes ownership of the struct given to it,
        // so we use a private struct with just the pieces of state needed to handle
        // RPCs.
        let service = TurboGrpcServiceInner {
            package_discovery: AsyncMutex::new(package_discovery),
            shutdown: trigger_shutdown,
            watcher_rx,
            times_saved: Arc::new(Mutex::new(HashMap::new())),
            start_time: Instant::now(),
            log_file: log_file.to_owned(),
        };
        let server_fut = {
            let service = ServiceBuilder::new()
                .layer(BumpTimeoutLayer::new(bump_timeout.clone()))
                .service(crate::daemon::proto::turbod_server::TurbodServer::new(
                    service,
                ));

            Server::builder()
                // set a max timeout for RPCs
                .timeout(REQUEST_TIMEOUT)
                .add_service(service)
                .serve_with_incoming_shutdown(stream, shutdown_fut)
        };
        // Wait for the server to exit.
        // This can be triggered by timeout, root watcher, or an RPC
        let _ = server_fut.await;
        info!("gRPC server exited");
        // Ensure our timer will exit
        running.store(false, Ordering::SeqCst);
        // We expect to have a signal from the grpc server on what triggered the exit
        let close_reason = shutdown_reason.await.unwrap_or(CloseReason::ServerClosed);
        // Now that the server has exited, the TurboGrpcService instance should be
        // dropped. The root watcher still has a reference to a receiver, keeping
        // the filewatcher alive. Trigger the root watcher to exit. We don't care
        // if we fail to send, root watching may have exited already
        let _ = exit_root_watch.send(());
        let _ = watch_root_handle.await;
        trace!("root watching exited");
        // Clean up the filewatching handle in the event that we never even got
        // started with filewatching. Again, we don't care about the error here.
        let _ = fw_handle.await;
        trace!("filewatching handle joined");
        Ok(close_reason)
    }
}

struct TurboGrpcServiceInner<PD> {
    //shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    shutdown: mpsc::Sender<()>,
    watcher_rx: watch::Receiver<Option<Arc<FileWatching>>>,
    times_saved: Arc<Mutex<HashMap<String, u64>>>,
    start_time: Instant,
    log_file: AbsoluteSystemPathBuf,
    package_discovery: AsyncMutex<PD>,
}

impl<PD> TurboGrpcServiceInner<PD> {
    async fn trigger_shutdown(&self) {
        info!("triggering shutdown");
        let _ = self.shutdown.send(()).await;
    }

    async fn wait_for_filewatching(&self) -> Result<Arc<FileWatching>, RpcError> {
        let rx = self.watcher_rx.clone();
        wait_for_filewatching(rx, Duration::from_millis(100)).await
    }

    async fn watch_globs(
        &self,
        hash: String,
        output_globs: Vec<String>,
        output_glob_exclusions: Vec<String>,
        time_saved: u64,
    ) -> Result<(), RpcError> {
        let glob_set = GlobSet::from_raw(output_globs, output_glob_exclusions)?;
        let fw = self.wait_for_filewatching().await?;
        fw.glob_watcher.watch_globs(hash.clone(), glob_set).await?;
        {
            let mut times_saved = self.times_saved.lock().expect("times saved lock poisoned");
            times_saved.insert(hash, time_saved);
        }
        Ok(())
    }

    async fn get_changed_outputs(
        &self,
        hash: String,
        candidates: HashSet<String>,
    ) -> Result<(HashSet<String>, u64), RpcError> {
        let time_saved = {
            let times_saved = self.times_saved.lock().expect("times saved lock poisoned");
            times_saved.get(hash.as_str()).copied().unwrap_or_default()
        };
        let fw = self.wait_for_filewatching().await?;
        let changed_globs = fw.glob_watcher.get_changed_globs(hash, candidates).await?;
        Ok((changed_globs, time_saved))
    }
}

async fn wait_for_filewatching(
    mut rx: watch::Receiver<Option<Arc<FileWatching>>>,
    timeout: Duration,
) -> Result<Arc<FileWatching>, RpcError> {
    let fw = tokio::time::timeout(timeout, rx.wait_for(|opt| opt.is_some()))
        .await
        .map_err(|_| RpcError::DeadlineExceeded)? // timeout case
        .map_err(|_| RpcError::NoFileWatching)?; // sender dropped

    return Ok(fw.as_ref().expect("guaranteed some above").clone());
}

async fn watch_root(
    filewatching_access: watch::Receiver<Option<Arc<FileWatching>>>,
    root: AbsoluteSystemPathBuf,
    trigger_shutdown: mpsc::Sender<()>,
    mut exit_signal: oneshot::Receiver<()>,
) -> Result<(), WatchError> {
    let mut recv_events = {
        let Ok(fw) = wait_for_filewatching(filewatching_access, Duration::from_secs(5)).await
        else {
            return Ok(());
        };

        fw._watcher.subscribe()
    };

    loop {
        // Ignore the outer layer of Result, if the sender has closed, filewatching has
        // gone away and we can return.
        select! {
            _ = &mut exit_signal => return Ok(()),
            event = recv_events.recv() => {
                let Ok(event) = event else {
                    return Ok(());
                };
                let should_trigger_shutdown = match event {
                    // filewatching can throw some weird events, so check that the root is actually gone
                    // before triggering a shutdown
                    Ok(event) if event.paths.iter().any(|p| p == (&root as &AbsoluteSystemPath)) => !root.exists(),
                    Ok(_) => false,
                    Err(_) => true
                };
                if should_trigger_shutdown {
                    warn!("Root watcher triggering shutdown");
                    // We don't care if a shutdown has already been triggered,
                    // so we can ignore the error.
                    let _ = trigger_shutdown.send(()).await;
                    return Ok(());
                }
            }
        }
    }
}

#[tonic::async_trait]
impl<PD> proto::turbod_server::Turbod for TurboGrpcServiceInner<PD>
where
    PD: PackageDiscovery + Send + 'static,
{
    async fn hello(
        &self,
        request: tonic::Request<proto::HelloRequest>,
    ) -> Result<tonic::Response<proto::HelloResponse>, tonic::Status> {
        let request = request.into_inner();

        let client_version = request.version;
        let server_version = proto::VERSION;

        let passes_version_check = match (
            proto::VersionRange::from_i32(request.supported_version_range),
            Version::parse(&client_version),
            Version::parse(server_version),
        ) {
            // if we fail to parse, or the constraint is invalid, we have a version mismatch
            (_, Err(_), _) | (_, _, Err(_)) | (None, _, _) => false,
            (Some(range), Ok(client), Ok(server)) => compare_versions(client, server, range),
        };

        if passes_version_check {
            Ok(tonic::Response::new(proto::HelloResponse {}))
        } else {
            Err(tonic::Status::failed_precondition(format!(
                "version mismatch. Client {} Server {}",
                client_version, server_version
            )))
        }
    }

    async fn shutdown(
        &self,
        _request: tonic::Request<proto::ShutdownRequest>,
    ) -> Result<tonic::Response<proto::ShutdownResponse>, tonic::Status> {
        self.trigger_shutdown().await;

        // if Some(Ok), then the server is shutting down now
        // if Some(Err), then the server is already shutting down
        // if None, then someone has already called shutdown
        Ok(tonic::Response::new(proto::ShutdownResponse {}))
    }

    async fn status(
        &self,
        _request: tonic::Request<proto::StatusRequest>,
    ) -> Result<tonic::Response<proto::StatusResponse>, tonic::Status> {
        Ok(tonic::Response::new(proto::StatusResponse {
            daemon_status: Some(proto::DaemonStatus {
                uptime_msec: self.start_time.elapsed().as_millis() as u64,
                log_file: self.log_file.to_string(),
            }),
        }))
    }

    async fn notify_outputs_written(
        &self,
        request: tonic::Request<proto::NotifyOutputsWrittenRequest>,
    ) -> Result<tonic::Response<proto::NotifyOutputsWrittenResponse>, tonic::Status> {
        let inner = request.into_inner();

        self.watch_globs(
            inner.hash,
            inner.output_globs,
            inner.output_exclusion_globs,
            inner.time_saved,
        )
        .await?;
        Ok(tonic::Response::new(proto::NotifyOutputsWrittenResponse {}))
    }

    async fn get_changed_outputs(
        &self,
        request: tonic::Request<proto::GetChangedOutputsRequest>,
    ) -> Result<tonic::Response<proto::GetChangedOutputsResponse>, tonic::Status> {
        let inner = request.into_inner();
        let (changed, time_saved) = self
            .get_changed_outputs(inner.hash, HashSet::from_iter(inner.output_globs))
            .await?;
        Ok(tonic::Response::new(proto::GetChangedOutputsResponse {
            changed_output_globs: changed.into_iter().collect(),
            time_saved,
        }))
    }

    async fn discover_packages(
        &self,
        _request: tonic::Request<proto::DiscoverPackagesRequest>,
    ) -> Result<tonic::Response<proto::DiscoverPackagesResponse>, tonic::Status> {
        self.package_discovery
            .lock()
            .await
            .discover_packages()
            .await
            .map(|packages| {
                tonic::Response::new(proto::DiscoverPackagesResponse {
                    package_files: packages
                        .workspaces
                        .into_iter()
                        .map(|d| proto::PackageFiles {
                            package_json: d.package_json.to_string(),
                            turbo_json: d.turbo_json.map(|t| t.to_string()),
                        })
                        .collect(),
                    package_manager: proto::PackageManager::from(packages.package_manager).into(),
                })
            })
            .map_err(|e| tonic::Status::internal(format!("{}", e)))
    }
}

/// Determine whether a server can serve a client's request based on its
/// version.
///
/// When the `proto::VersionRange` is anything other than `Exact` it means that
/// the server's version must exceed the client's version. For example, if the
/// client is `1.2.3` and the server is `1.2.4`, then the client's request can
/// be served if the `proto::VersionRange` is `Patch`, `Minor`, or `Major`.
/// However, if the server is `1.3.0`, then the client's request can only be
/// served if the `proto::VersionRange` is `Minor` or `Major`.
fn compare_versions(client: Version, server: Version, constraint: proto::VersionRange) -> bool {
    match constraint {
        proto::VersionRange::Exact => client == server,
        proto::VersionRange::Patch => {
            client.major == server.major
                && client.minor == server.minor
                && client.patch <= server.patch
        }
        proto::VersionRange::Minor => client.major == server.major && client.minor <= server.minor,
        // changes to major version is always incompatible
        proto::VersionRange::Major => client.major == server.major,
    }
}

impl<T> NamedService for TurboGrpcServiceInner<T> {
    const NAME: &'static str = "turborepo.Daemon";
}

#[cfg(test)]
mod test {
    use std::{
        assert_matches::{self, assert_matches},
        time::{Duration, Instant},
    };

    use futures::FutureExt;
    use semver::Version;
    use test_case::test_case;
    use tokio::sync::oneshot;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{
        discovery::{DiscoveryResponse, PackageDiscovery},
        package_manager::PackageManager,
    };

    use super::compare_versions;
    use crate::daemon::{proto::VersionRange, CloseReason, TurboGrpcService};

    #[test_case("1.2.3", "1.2.3", VersionRange::Exact, true ; "exact match")]
    #[test_case("1.2.3", "1.2.3", VersionRange::Patch, true ; "patch match")]
    #[test_case("1.2.3", "1.2.3", VersionRange::Minor, true ; "minor match")]
    #[test_case("1.2.3", "1.2.3", VersionRange::Major, true ; "major match")]
    #[test_case("1.2.3", "1.2.4", VersionRange::Exact, false ; "exact mismatch")]
    #[test_case("1.2.3", "1.2.4", VersionRange::Patch, true ; "patch greater match")]
    #[test_case("1.2.3", "1.2.4", VersionRange::Minor, true ; "minor greater match")]
    #[test_case("1.2.3", "1.2.4", VersionRange::Major, true ; "major greater match")]
    #[test_case("1.2.3", "1.2.2", VersionRange::Patch, false ; "patch lesser mismatch")]
    #[test_case("1.2.3", "1.1.3", VersionRange::Patch, false ; "patch lesser minor mismatch")]
    #[test_case("1.2.3", "1.1.0", VersionRange::Minor, false ; "minor lesser mismatch")]
    #[test_case("1.2.3", "0.1.0", VersionRange::Major, false ; "major lesser mismatch")]
    #[test_case("1.10.17-canary.0", "1.10.17-canary.1", VersionRange::Exact, false ; "canary mismatch")]
    #[test_case("1.10.17-canary.0", "1.10.17-canary.1", VersionRange::Patch, true ; "canary match")]
    #[test_case("1.0.0", "2.0.0", VersionRange::Major, false ; "major breaking changes")]

    fn version_match(a: &str, b: &str, constraint: VersionRange, expected: bool) {
        assert_eq!(
            compare_versions(
                Version::parse(a).unwrap(),
                Version::parse(b).unwrap(),
                constraint
            ),
            expected
        )
    }

    struct MockDiscovery;
    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &mut self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            Ok(DiscoveryResponse {
                package_manager: PackageManager::Yarn,
                workspaces: vec![],
            })
        }
    }

    // the windows runner starts a new thread to accept uds requests,
    // so we need a multi-threaded runtime
    #[tokio::test(flavor = "multi_thread")]
    #[tracing_test::traced_test]
    async fn lifecycle() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let repo_root = path.join_component("repo");
        let daemon_root = path.join_component("daemon");
        let log_file = daemon_root.join_component("log");
        tracing::info!("start");

        let pid_path = daemon_root.join_component("turbod.pid");

        let (tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);

        let service = TurboGrpcService::new(
            repo_root.clone(),
            daemon_root,
            log_file,
            Duration::from_secs(60 * 60),
            exit_signal,
        )
        .with_package_discovery_backup(MockDiscovery);

        // the package watcher reads data from the package.json file
        // so we need to create it
        repo_root.create_dir_all().unwrap();
        let package_json = repo_root.join_component("package.json");
        std::fs::write(package_json, r#"{"workspaces": ["packages/*"]}"#).unwrap();

        let handle = tokio::task::spawn(service.serve());

        tokio::time::sleep(Duration::from_millis(2000)).await;
        assert!(
            pid_path.exists(),
            "pid file must be present at {:?}",
            pid_path
        );
        // signal server exit
        tx.send(CloseReason::Interrupt).unwrap();
        handle.await.unwrap().unwrap();

        // The serve future should be dropped here, closing the server.
        tracing::info!("yay we are done");

        assert!(!pid_path.exists(), "pid file must be deleted");

        tracing::info!("and files cleaned up");
    }

    // the windows runner starts a new thread to accept uds requests,
    // so we need a multi-threaded runtime
    #[tokio::test(flavor = "multi_thread")]
    #[tracing_test::traced_test]
    async fn timeout() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let repo_root = path.join_component("repo");
        let daemon_root = path.join_component("daemon");
        let log_file = daemon_root.join_component("log");

        let pid_path = daemon_root.join_component("turbod.pid");

        let now = Instant::now();
        let (_tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);

        let server = TurboGrpcService::new(
            repo_root.clone(),
            daemon_root,
            log_file,
            Duration::from_millis(10),
            exit_signal,
        )
        .with_package_discovery_backup(MockDiscovery);

        // the package watcher reads data from the package.json file
        // so we need to create it
        repo_root.create_dir_all().unwrap();
        let package_json = repo_root.join_component("package.json");
        std::fs::write(package_json, r#"{"workspaces": ["packages/*"]}"#).unwrap();

        let close_reason = server.serve().await;

        assert!(
            now.elapsed() >= Duration::from_millis(10),
            "must wait at least 5ms"
        );
        assert_matches::assert_matches!(
            close_reason,
            Ok(CloseReason::Timeout),
            "must close due to timeout"
        );
        assert!(!pid_path.exists(), "pid file must be deleted");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[tracing_test::traced_test]
    async fn test_delete_root() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let repo_root = path.join_component("repo");
        let daemon_root = path.join_component("daemon");
        daemon_root.create_dir_all().unwrap();
        let log_file = daemon_root.join_component("log");

        let (_tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);

        let server = TurboGrpcService::new(
            repo_root.clone(),
            daemon_root,
            log_file,
            Duration::from_secs(60 * 60),
            exit_signal,
        )
        .with_package_discovery_backup(MockDiscovery);

        let handle = tokio::task::spawn(server.serve());

        // give filewatching some time to bootstrap
        tokio::time::sleep(Duration::from_secs(1)).await;
        // Remove the root
        repo_root.remove_dir_all().unwrap();

        let close_reason = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("no timeout")
            .expect("server exited");
        assert_matches!(close_reason, Ok(CloseReason::Shutdown));
    }
}
