//! Daemon Server
//!
//! This module houses the daemon server. For more information, go to the
//! [daemon module](std::daemon).

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use futures::Future;
use prost::DecodeError;
use semver::Version;
use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tonic::{server::NamedService, transport::Server};
use tower::ServiceBuilder;
use tracing::{error, info, trace, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_filewatch::{
    cookies::CookieWriter,
    globwatcher::{Error as GlobWatcherError, GlobError, GlobSet, GlobWatcher},
    package_watcher::{PackageWatcher, WatchingPackageDiscovery},
    FileSystemWatcher, WatchError,
};
use turborepo_repository::discovery::{
    LocalPackageDiscoveryBuilder, PackageDiscovery, PackageDiscoveryBuilder,
};

use super::{bump_timeout::BumpTimeout, endpoint::SocketOpenError, proto};
use crate::daemon::{
    bump_timeout_layer::BumpTimeoutLayer, default_timeout_layer::DefaultTimeoutLayer,
    endpoint::listen_socket, Paths,
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

/// We may need to pass out references to a subset of these, so
/// we'll make them public Arcs. Eventually we can stabilize on
/// a general API and close this up.
#[derive(Clone)]
pub struct FileWatching {
    watcher: Arc<FileSystemWatcher>,
    pub glob_watcher: Arc<GlobWatcher>,
    pub package_watcher: Arc<PackageWatcher>,
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

impl FileWatching {
    /// This function is called in the constructor for the `TurboGrpcService`
    /// and should defer ALL heavy computation to the background, making use
    /// of `OptionalWatch` to ensure that the server can start up without
    /// waiting for the filewatcher to be ready. Using `OptionalWatch`,
    /// dependent services can wait for resources they need to become
    /// available, and the server can start up without waiting for them.
    pub fn new<PD: PackageDiscovery + Send + Sync + 'static>(
        repo_root: AbsoluteSystemPathBuf,
        backup_discovery: PD,
    ) -> Result<FileWatching, WatchError> {
        let watcher = Arc::new(FileSystemWatcher::new_with_default_cookie_dir(&repo_root)?);
        let recv = watcher.watch();

        let cookie_watcher = CookieWriter::new(
            watcher.cookie_dir(),
            Duration::from_millis(100),
            recv.clone(),
        );
        let glob_watcher = Arc::new(GlobWatcher::new(
            repo_root.clone(),
            cookie_watcher,
            recv.clone(),
        ));
        let package_watcher = Arc::new(
            PackageWatcher::new(repo_root.clone(), recv.clone(), backup_discovery)
                .map_err(|e| WatchError::Setup(format!("{:?}", e)))?,
        );

        Ok(FileWatching {
            watcher,
            glob_watcher,
            package_watcher,
        })
    }
}

/// Timeout for every RPC the server handles
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct TurboGrpcService<S, PDB> {
    repo_root: AbsoluteSystemPathBuf,
    paths: Paths,
    timeout: Duration,
    external_shutdown: S,

    package_discovery_backup: PDB,
}

impl<S> TurboGrpcService<S, LocalPackageDiscoveryBuilder>
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
        paths: Paths,
        timeout: Duration,
        external_shutdown: S,
    ) -> Self {
        let package_discovery_backup =
            LocalPackageDiscoveryBuilder::new(repo_root.clone(), None, None);

        // Run the actual service. It takes ownership of the struct given to it,
        // so we use a private struct with just the pieces of state needed to handle
        // RPCs.
        TurboGrpcService {
            repo_root,
            paths,
            timeout,
            external_shutdown,
            package_discovery_backup,
        }
    }
}

impl<S, PDB> TurboGrpcService<S, PDB>
where
    S: Future<Output = CloseReason>,
    PDB: PackageDiscoveryBuilder,
    PDB::Output: PackageDiscovery + Send + Sync + 'static,
{
    /// If errors are encountered when loading the package discovery, this
    /// builder will be used as a backup to refresh the state.
    pub fn with_package_discovery_backup<PDB2: PackageDiscoveryBuilder>(
        self,
        package_discovery_backup: PDB2,
    ) -> TurboGrpcService<S, PDB2> {
        TurboGrpcService {
            external_shutdown: self.external_shutdown,
            paths: self.paths,
            repo_root: self.repo_root,
            timeout: self.timeout,
            package_discovery_backup,
        }
    }

    pub async fn serve(self) -> Result<CloseReason, PDB::Error> {
        let Self {
            external_shutdown,
            paths,
            repo_root,
            timeout,
            package_discovery_backup,
        } = self;

        // A channel to trigger the shutdown of the gRPC server. This is handed out
        // to components internal to the server process such as root watching, as
        // well as available to the gRPC server itself to handle the shutdown RPC.
        let (trigger_shutdown, mut shutdown_signal) = mpsc::channel::<()>(1);

        let package_discovery_backup = package_discovery_backup.build()?;
        let (service, exit_root_watch, watch_root_handle) = TurboGrpcServiceInner::new(
            package_discovery_backup,
            repo_root.clone(),
            trigger_shutdown,
            paths.log_file,
        );

        let running = Arc::new(AtomicBool::new(true));
        let (_pid_lock, stream) =
            match listen_socket(&paths.pid_file, &paths.sock_file, running.clone()).await {
                Ok((pid_lock, stream)) => (pid_lock, stream),
                Err(e) => return Ok(CloseReason::SocketOpenError(e)),
            };
        trace!("acquired connection stream for socket");

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

        let server_fut = {
            let service = ServiceBuilder::new()
                .layer(BumpTimeoutLayer::new(bump_timeout.clone()))
                .layer(DefaultTimeoutLayer)
                .service(crate::daemon::proto::turbod_server::TurbodServer::new(
                    service,
                ));

            Server::builder()
                // we respect the timeout specified by the client if it is set, but
                // have a default timeout for non-blocking calls of 100ms, courtesy of
                // `DefaultTimeoutLayer`. the REQUEST_TIMEOUT, however, is the
                // maximum time we will wait for a response, regardless of the client's
                // preferences. it cannot be exceeded.
                .timeout(REQUEST_TIMEOUT)
                .add_service(service)
                .serve_with_incoming_shutdown(stream, shutdown_fut)
        };
        // Wait for the server to exit.
        // This can be triggered by timeout, root watcher, or an RPC
        tracing::debug!("server started");
        let _ = server_fut.await;
        tracing::debug!("server exited");
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
        Ok(close_reason)
    }
}

struct TurboGrpcServiceInner<PD> {
    shutdown: mpsc::Sender<()>,
    file_watching: FileWatching,
    times_saved: Arc<Mutex<HashMap<String, u64>>>,
    start_time: Instant,
    log_file: AbsoluteSystemPathBuf,
    package_discovery: PD,
}

// we have a grpc service that uses watching package discovery, and where the
// watching package hasher also uses watching package discovery as well as
// falling back to a local package hasher
impl TurboGrpcServiceInner<Arc<WatchingPackageDiscovery>> {
    pub fn new<PD: Sync + PackageDiscovery + Send + 'static>(
        package_discovery_backup: PD,
        repo_root: AbsoluteSystemPathBuf,
        trigger_shutdown: mpsc::Sender<()>,
        log_file: AbsoluteSystemPathBuf,
    ) -> (
        Self,
        oneshot::Sender<()>,
        JoinHandle<Result<(), WatchError>>,
    ) {
        let file_watching = FileWatching::new(repo_root.clone(), package_discovery_backup).unwrap();

        tracing::debug!("initing package discovery");
        let package_discovery = Arc::new(WatchingPackageDiscovery::new(
            file_watching.package_watcher.clone(),
        ));

        // exit_root_watch delivers a signal to the root watch loop to exit.
        // In the event that the server shuts down via some other mechanism, this
        // cleans up root watching task.
        let (exit_root_watch, root_watch_exit_signal) = oneshot::channel();
        let watch_root_handle = tokio::task::spawn(watch_root(
            file_watching.clone(),
            repo_root.clone(),
            trigger_shutdown.clone(),
            root_watch_exit_signal,
        ));

        (
            TurboGrpcServiceInner {
                package_discovery,
                shutdown: trigger_shutdown,
                file_watching,
                times_saved: Arc::new(Mutex::new(HashMap::new())),
                start_time: Instant::now(),
                log_file,
            },
            exit_root_watch,
            watch_root_handle,
        )
    }
}

impl<PD> TurboGrpcServiceInner<PD>
where
    PD: PackageDiscovery + Send + Sync + 'static,
{
    async fn trigger_shutdown(&self) {
        info!("triggering shutdown");
        let _ = self.shutdown.send(()).await;
    }

    async fn watch_globs(
        &self,
        hash: String,
        output_globs: Vec<String>,
        output_glob_exclusions: Vec<String>,
        time_saved: u64,
    ) -> Result<(), RpcError> {
        let glob_set = GlobSet::from_raw(output_globs, output_glob_exclusions)?;
        self.file_watching
            .glob_watcher
            .watch_globs(hash.clone(), glob_set, REQUEST_TIMEOUT)
            .await?;
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
        let changed_globs = self
            .file_watching
            .glob_watcher
            .get_changed_globs(hash, candidates, REQUEST_TIMEOUT)
            .await?;
        Ok((changed_globs, time_saved))
    }
}

async fn watch_root(
    filewatching_access: FileWatching,
    root: AbsoluteSystemPathBuf,
    trigger_shutdown: mpsc::Sender<()>,
    mut exit_signal: oneshot::Receiver<()>,
) -> Result<(), WatchError> {
    let mut recv_events = filewatching_access
        .watcher
        .subscribe()
        .await
        // we can only encounter an error here if the file watcher is closed (a recv error)
        .map_err(|_| WatchError::Setup("file watching shut down".to_string()))?;

    tracing::debug!("watching root: {:?}", root);

    loop {
        // Ignore the outer layer of Result, if the sender has closed, filewatching has
        // gone away and we can return.
        select! {
            _ = &mut exit_signal => break,
            event = recv_events.recv() => {
                let Ok(event) = event else {
                    break;
                };
                tracing::debug!("root watcher received event: {:?}", event);
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
                    break;
                }
            }
        }
    }

    tracing::debug!("no longer watching root");

    Ok(())
}

#[tonic::async_trait]
impl<PD: PackageDiscovery + Send + Sync + 'static> proto::turbod_server::Turbod
    for TurboGrpcServiceInner<PD>
{
    async fn hello(
        &self,
        request: tonic::Request<proto::HelloRequest>,
    ) -> Result<tonic::Response<proto::HelloResponse>, tonic::Status> {
        let request = request.into_inner();

        let client_version = request.version;
        let server_version = proto::VERSION;

        let passes_version_check = match (
            proto::VersionRange::try_from(request.supported_version_range),
            Version::parse(&client_version),
            Version::parse(server_version),
        ) {
            // if we fail to parse, or the constraint is invalid, we have a version mismatch
            (_, Err(_), _) | (_, _, Err(_)) | (Err(DecodeError { .. }), _, _) => false,
            (Ok(range), Ok(client), Ok(server)) => compare_versions(client, server, range),
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
            .map_err(|e| match e {
                turborepo_repository::discovery::Error::Unavailable => {
                    tonic::Status::unavailable("package discovery unavailable")
                }
                turborepo_repository::discovery::Error::Failed(e) => {
                    tonic::Status::internal(format!("{}", e))
                }
            })
    }

    async fn discover_packages_blocking(
        &self,
        _request: tonic::Request<proto::DiscoverPackagesRequest>,
    ) -> Result<tonic::Response<proto::DiscoverPackagesResponse>, tonic::Status> {
        self.package_discovery
            .discover_packages_blocking()
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
            .map_err(|e| match e {
                turborepo_repository::discovery::Error::Unavailable => {
                    tonic::Status::unavailable("package discovery unavailable")
                }
                turborepo_repository::discovery::Error::Failed(e) => {
                    tonic::Status::internal(format!("{}", e))
                }
            })
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

impl<PD> NamedService for TurboGrpcServiceInner<PD> {
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
    use crate::daemon::{proto::VersionRange, CloseReason, Paths, TurboGrpcService};

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
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            Ok(DiscoveryResponse {
                package_manager: PackageManager::Yarn,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            self.discover_packages().await
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
        let paths = Paths::from_repo_root(&repo_root);
        tracing::info!("start");

        let (tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);

        let service = TurboGrpcService::new(
            repo_root.clone(),
            paths.clone(),
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
            paths.pid_file.exists(),
            "pid file must be present at {:?}",
            paths.pid_file
        );
        // signal server exit
        tx.send(CloseReason::Interrupt).unwrap();
        handle.await.unwrap().unwrap();

        // The serve future should be dropped here, closing the server.
        tracing::info!("yay we are done");

        assert!(!paths.pid_file.exists(), "pid file must be deleted");

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
        let paths = Paths::from_repo_root(&repo_root);

        let now = Instant::now();
        let (_tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);

        let server = TurboGrpcService::new(
            repo_root.clone(),
            paths.clone(),
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
        assert!(!paths.pid_file.exists(), "pid file must be deleted");
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
        let paths = Paths::from_repo_root(&repo_root);

        let (_tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);

        let server = TurboGrpcService::new(
            repo_root.clone(),
            paths,
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
