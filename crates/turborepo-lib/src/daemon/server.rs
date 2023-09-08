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
use thiserror::Error;
use tokio::{
    select,
    sync::{oneshot, watch},
};
use tonic::transport::{NamedService, Server};
use tower::ServiceBuilder;
use tracing::{error, trace, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_filewatch::{
    cookie_jar::CookieJar,
    globwatcher::{Error as GlobWatcherError, GlobSet, GlobWatcher},
    FileSystemWatcher, WatchError,
};

use super::{
    bump_timeout::BumpTimeout,
    endpoint::SocketOpenError,
    proto::{self},
};
use crate::{
    daemon::{bump_timeout_layer::BumpTimeoutLayer, endpoint::listen_socket},
    get_version,
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

struct FileWatching {
    _watcher: FileSystemWatcher,
    glob_watcher: GlobWatcher,
}

#[derive(Debug, Error)]
enum RpcError {
    #[error("deadline exceeded")]
    DeadlineExceeded,
    #[error("invalid glob: {0}")]
    InvalidGlob(#[from] wax::BuildError),
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

async fn start_filewatching(
    repo_root: AbsoluteSystemPathBuf,
    watcher_tx: watch::Sender<Option<Arc<FileWatching>>>,
) -> Result<(), WatchError> {
    let watcher = FileSystemWatcher::new(&repo_root).await?;
    // TODO: be more methodical about this choice:
    let cookie_dir = repo_root.join_component(".git");
    let cookie_jar = CookieJar::new(&cookie_dir, Duration::from_millis(100), watcher.subscribe());
    let glob_watcher = GlobWatcher::new(&repo_root, cookie_jar, watcher.subscribe());
    // We can ignore failures here, it means the server is shutting down and
    // receivers have gone out of scope.
    let _ = watcher_tx.send(Some(Arc::new(FileWatching {
        _watcher: watcher,
        glob_watcher,
    })));
    Ok(())
}

pub async fn serve<S>(
    repo_root: &AbsoluteSystemPath,
    daemon_root: &AbsoluteSystemPath,
    log_file: AbsoluteSystemPathBuf,
    timeout: Duration,
    external_shutdown: S,
) -> CloseReason
where
    S: Future<Output = CloseReason>,
{
    let running = Arc::new(AtomicBool::new(true));
    let (_pid_lock, stream) = match listen_socket(daemon_root.clone(), running.clone()).await {
        Ok((pid_lock, stream)) => (pid_lock, stream),
        Err(e) => return CloseReason::SocketOpenError(e),
    };
    trace!("acquired connection stream for socket");

    let watcher_repo_root = repo_root.to_owned();
    let (watcher_tx, watcher_rx) = watch::channel(None);
    let (trigger_shutdown, shutdown_signal) = {
        let (tx, rx) = oneshot::channel::<()>();
        (Arc::new(Mutex::new(Some(tx))), rx)
    };

    // watch receivers as a group own the filewatcher, which will exit when
    // all references are dropped.
    let fw_shutdown = trigger_shutdown.clone();
    let fw_handle = tokio::task::spawn(async move {
        if let Err(e) = start_filewatching(watcher_repo_root, watcher_tx).await {
            fw_shutdown
                .lock()
                .expect("mutex poisoned")
                .take()
                .map(|tx| {
                    error!("filewatching failed to start: {}", e);
                    let _ = tx.send(());
                });
        }
    });
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
    let (gprc_shutdown_tx, shutdown_reason) = oneshot::channel();
    let shutdown_fut = async move {
        select! {
            _ = shutdown_signal => gprc_shutdown_tx.send(CloseReason::Shutdown).ok(),
            _ = timeout_fut => gprc_shutdown_tx.send(CloseReason::Timeout).ok(),
            reason = external_shutdown => gprc_shutdown_tx.send(reason).ok(),
        };
    };

    let service = TurboGrpcService {
        shutdown: trigger_shutdown,
        watcher_rx,
        times_saved: Arc::new(Mutex::new(HashMap::new())),
        start_time: Instant::now(),
        log_file,
    };
    let server_fut = {
        let service = ServiceBuilder::new()
            .layer(BumpTimeoutLayer::new(bump_timeout.clone()))
            .service(crate::daemon::proto::turbod_server::TurbodServer::new(
                service,
            ));

        Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(stream, shutdown_fut)
    };
    // Wait for the server to exit.
    let _ = server_fut.await;
    trace!("gRPC server exited");
    running.store(false, Ordering::SeqCst);
    // We expect to have a signal from the grpc server on what triggered the exit
    let close_reason = shutdown_reason.await.unwrap_or(CloseReason::ServerClosed);
    // Now that the server has exited, the TurboGrpcService instance should be
    // dropped. The root watcher still has a reference to a receiver, keeping
    // the filewatcher alive. We don't care if we fail to send, root watching
    // may have exited already
    let _ = exit_root_watch.send(());
    let _ = watch_root_handle.await;
    trace!("root watching exited");
    let _ = fw_handle.await;
    trace!("filewatching handle joined");
    close_reason
}

struct TurboGrpcService {
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    watcher_rx: watch::Receiver<Option<Arc<FileWatching>>>,
    times_saved: Arc<Mutex<HashMap<String, u64>>>,
    start_time: Instant,
    log_file: AbsoluteSystemPathBuf,
}

impl TurboGrpcService {
    fn trigger_shutdown(&self) {
        self.shutdown
            .lock()
            .expect("mutex poisoned")
            .take()
            .map(|s| s.send(()));
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
    if let Some(fw) = rx.borrow().as_ref().map(|fw| fw.clone()) {
        return Ok(fw);
    }
    tokio::time::timeout(timeout, rx.changed())
        .await
        .map_err(|_| RpcError::DeadlineExceeded)? // timeout case
        .map_err(|_| RpcError::NoFileWatching)?; // sender dropped with no receivers
    let result = rx
        .borrow()
        .as_ref()
        .map(|fw| fw.clone())
        // This error should never happen, we got the change notification
        // above, and we only ever go from None to Some filewatcher
        .ok_or_else(|| RpcError::NoFileWatching)?;
    Ok(result)
}

async fn watch_root(
    filewatching_access: watch::Receiver<Option<Arc<FileWatching>>>,
    root: AbsoluteSystemPathBuf,
    trigger_shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
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
                    Ok(event) if event.paths.iter().any(|p| p == (&root as &AbsoluteSystemPath)) => true,
                    Ok(_) => false,
                    Err(_) => true
                };
                if should_trigger_shutdown {
                    warn!("Root watcher triggering shutdown");
                    trigger_shutdown.lock().expect("mutex poisoned").take().map(|s| s.send(()));
                    return Ok(());
                }
            }
        }
    }
}

#[tonic::async_trait]
impl proto::turbod_server::Turbod for TurboGrpcService {
    async fn hello(
        &self,
        request: tonic::Request<proto::HelloRequest>,
    ) -> Result<tonic::Response<proto::HelloResponse>, tonic::Status> {
        let client_version = request.into_inner().version;
        let server_version = get_version();
        if client_version != server_version {
            return Err(tonic::Status::failed_precondition(format!(
                "version mismatch. Client {} Server {}",
                client_version, server_version
            )));
        } else {
            Ok(tonic::Response::new(proto::HelloResponse {}))
        }
    }

    async fn shutdown(
        &self,
        _request: tonic::Request<proto::ShutdownRequest>,
    ) -> Result<tonic::Response<proto::ShutdownResponse>, tonic::Status> {
        self.trigger_shutdown();

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
}

impl NamedService for TurboGrpcService {
    const NAME: &'static str = "turborepo.Daemon";
}

#[cfg(test)]
mod test {
    use std::{
        assert_matches::{self, assert_matches},
        time::{Duration, Instant},
    };

    use futures::FutureExt;
    use tokio::sync::oneshot;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::daemon::{server::serve, CloseReason};

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
        repo_root.create_dir_all().unwrap();
        let daemon_root = path.join_component("daemon");
        let log_file = daemon_root.join_component("log");
        tracing::info!("start");

        let pid_path = daemon_root.join_component("turbod.pid");

        let (tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);
        let handle = tokio::task::spawn(async move {
            let repo_root = repo_root;
            let daemon_root = daemon_root;
            serve(
                &repo_root,
                &daemon_root,
                log_file,
                Duration::from_secs(60 * 60),
                exit_signal,
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(2000)).await;
        assert!(pid_path.exists(), "pid file must be present");
        // signal server exit
        tx.send(CloseReason::Interrupt).unwrap();
        handle.await.unwrap();

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
        repo_root.create_dir_all().unwrap();
        let daemon_root = path.join_component("daemon");
        let log_file = daemon_root.join_component("log");

        let pid_path = daemon_root.join_component("turbod.pid");

        let now = Instant::now();
        let (_tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);
        let close_reason = serve(
            &repo_root,
            &daemon_root,
            log_file,
            Duration::from_millis(5),
            exit_signal,
        )
        .await;

        assert!(
            now.elapsed() >= Duration::from_millis(5),
            "must wait at least 5ms"
        );
        assert_matches::assert_matches!(
            close_reason,
            super::CloseReason::Timeout,
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
        repo_root.create_dir_all().unwrap();
        let daemon_root = path.join_component("daemon");
        daemon_root.create_dir_all().unwrap();
        let log_file = daemon_root.join_component("log");

        let (_tx, rx) = oneshot::channel::<CloseReason>();
        let exit_signal = rx.map(|_result| CloseReason::Interrupt);

        let server_repo_root = repo_root.clone();
        let handle = tokio::task::spawn(async move {
            let repo_root = server_repo_root;
            let daemon_root = daemon_root;
            serve(
                &repo_root,
                &daemon_root,
                log_file,
                Duration::from_secs(60 * 60),
                exit_signal,
            )
            .await
        });

        // give filewatching some time to bootstrap
        tokio::time::sleep(Duration::from_secs(1)).await;
        // Remove the root
        repo_root.remove_dir_all().unwrap();

        let close_reason = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("no timeout")
            .expect("server exited");
        assert_matches!(close_reason, CloseReason::Shutdown);
    }
}
