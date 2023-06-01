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
        Arc, Mutex as StdMutux,
    },
    time::{Duration, Instant},
};

use globwatch::{StopSource, Watcher};
use tokio::{
    select,
    signal::ctrl_c,
    sync::{
        oneshot::{self, Receiver, Sender},
        Mutex,
    },
};
use tonic::transport::{NamedService, Server};
use tower::ServiceBuilder;
use tracing::{error, trace};
use turbopath::AbsoluteSystemPathBuf;

use super::{
    bump_timeout::BumpTimeout,
    endpoint::SocketOpenError,
    proto::{self},
    DaemonError,
};
use crate::{
    commands::CommandBase, daemon::bump_timeout_layer::BumpTimeoutLayer, get_version,
    globwatcher::HashGlobWatcher,
};

pub struct DaemonServer<T: Watcher> {
    daemon_root: AbsoluteSystemPathBuf,
    log_file: AbsoluteSystemPathBuf,

    start_time: Instant,
    timeout: Arc<BumpTimeout>,

    watcher: Arc<HashGlobWatcher<T>>,
    shutdown: Mutex<Option<Sender<()>>>,
    shutdown_rx: Option<Receiver<()>>,

    running: Arc<AtomicBool>,

    times_saved: Arc<std::sync::Mutex<HashMap<String, u64>>>,
}

#[derive(Debug)]
pub enum CloseReason {
    Timeout,
    Shutdown,
    WatcherClosed,
    ServerClosed,
    Interrupt,
    SocketOpenError(SocketOpenError),
}

impl DaemonServer<notify::RecommendedWatcher> {
    #[tracing::instrument(skip(base), fields(repo_root = %base.repo_root))]
    pub fn new(
        base: &CommandBase,
        timeout: Duration,
        log_file: AbsoluteSystemPathBuf,
    ) -> Result<Self, DaemonError> {
        let daemon_root = base.daemon_file_root();

        let watcher = Arc::new(HashGlobWatcher::new(
            AbsoluteSystemPathBuf::new(base.repo_root.clone()).expect("valid repo root"),
            daemon_root.join_component("flush").as_path().to_owned(),
        )?);

        let (send_shutdown, recv_shutdown) = tokio::sync::oneshot::channel::<()>();

        Ok(Self {
            daemon_root,
            log_file,

            start_time: Instant::now(),
            timeout: Arc::new(BumpTimeout::new(timeout)),

            watcher,
            shutdown: Mutex::new(Some(send_shutdown)),
            shutdown_rx: Some(recv_shutdown),

            running: Arc::new(AtomicBool::new(true)),
            times_saved: Arc::new(StdMutux::new(HashMap::new())),
        })
    }
}

impl<T: Watcher> Drop for DaemonServer<T> {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl<T: Watcher + Send + 'static> DaemonServer<T> {
    /// Serve the daemon server, while also watching for filesystem changes.
    #[tracing::instrument(skip(self))]
    pub async fn serve(mut self) -> CloseReason {
        let stop = StopSource::new();
        let watcher = self.watcher.clone();
        let watcher_fut = watcher.watch(stop.token());
        tokio::pin!(watcher_fut);

        let timer = self.timeout.clone();
        let timeout_fut = timer.wait();

        // if shutdown is available, then listen. otherwise just wait forever
        let shutdown_rx = self.shutdown_rx.take();
        let shutdown_fut = async move {
            match shutdown_rx {
                Some(rx) => {
                    rx.await.ok();
                }
                None => {
                    futures::pending!();
                }
            }
        };

        // when one of these futures complete, let the server gracefully shutdown
        let (shutdown_tx, shutdown_reason) = oneshot::channel();
        let shutdown_fut = async move {
            select! {
                _ = shutdown_fut => shutdown_tx.send(CloseReason::Shutdown).ok(),
                _ = timeout_fut => shutdown_tx.send(CloseReason::Timeout).ok(),
                _ = ctrl_c() => shutdown_tx.send(CloseReason::Interrupt).ok(),
            };
        };

        #[cfg(feature = "http")]
        let server_fut = {
            // set up grpc reflection
            let efd = include_bytes!("file_descriptor_set.bin");
            let reflection = tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(efd)
                .build()
                .unwrap();

            let service = ServiceBuilder::new()
                .layer(BumpTimeoutLayer::new(self.timeout.clone()))
                .service(crate::daemon::proto::turbod_server::TurbodServer::new(self));

            Server::builder()
                .add_service(reflection)
                .add_service(service)
                .serve_with_shutdown("127.0.0.1:5000".parse().unwrap(), shutdown_fut)
        };

        #[cfg(not(feature = "http"))]
        let (_lock, server_fut) = {
            let (lock, stream) = match crate::daemon::endpoint::listen_socket(
                self.daemon_root.clone(),
                self.running.clone(),
            )
            .await
            {
                Ok(val) => val,
                Err(e) => return CloseReason::SocketOpenError(e),
            };

            trace!("acquired connection stream for socket");

            let service = ServiceBuilder::new()
                .layer(BumpTimeoutLayer::new(self.timeout.clone()))
                .service(crate::daemon::proto::turbod_server::TurbodServer::new(self));

            (
                lock,
                Server::builder()
                    .add_service(service)
                    .serve_with_incoming_shutdown(stream, shutdown_fut),
            )
        };
        tokio::pin!(server_fut);

        // necessary to make sure we don't try to poll the watcher_fut once it
        // has completed
        let mut watcher_done = false;
        loop {
            select! {
                    _ = &mut server_fut => {
                    return shutdown_reason.await.unwrap_or(CloseReason::ServerClosed);
                },
                watch_res = &mut watcher_fut, if !watcher_done => {
                    match watch_res {
                        Ok(()) => return CloseReason::WatcherClosed,
                        Err(e) => {
                            error!("Globwatch config error: {:?}", e);
                            watcher_done = true;
                        },
                    }
                },
            }
        }

        // here the stop token is dropped, and the pid lock is dropped
        // causing them to be cleaned up
    }
}

#[tonic::async_trait]
impl<T: Watcher + Send + 'static> proto::turbod_server::Turbod for DaemonServer<T> {
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
        self.shutdown.lock().await.take().map(|s| s.send(()));

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
                log_file: self.log_file.to_str().unwrap().to_string(),
            }),
        }))
    }

    async fn notify_outputs_written(
        &self,
        request: tonic::Request<proto::NotifyOutputsWrittenRequest>,
    ) -> Result<tonic::Response<proto::NotifyOutputsWrittenResponse>, tonic::Status> {
        let inner = request.into_inner();

        {
            let mut times_saved = self.times_saved.lock().expect("times saved lock poisoned");
            times_saved.insert(inner.hash.clone(), inner.time_saved);
        }
        match self
            .watcher
            .watch_globs(
                Arc::new(inner.hash),
                inner.output_globs,
                inner.output_exclusion_globs,
            )
            .await
        {
            Ok(_) => Ok(tonic::Response::new(proto::NotifyOutputsWrittenResponse {})),
            Err(e) => {
                error!("failed to watch globs: {:?}", e);
                Err(tonic::Status::internal("failed to watch globs"))
            }
        }
    }

    async fn get_changed_outputs(
        &self,
        request: tonic::Request<proto::GetChangedOutputsRequest>,
    ) -> Result<tonic::Response<proto::GetChangedOutputsResponse>, tonic::Status> {
        let inner = request.into_inner();
        let hash = Arc::new(inner.hash);
        let changed = self
            .watcher
            .changed_globs(&hash, HashSet::from_iter(inner.output_globs))
            .await;

        let time_saved = {
            let times_saved = self.times_saved.lock().expect("times saved lock poisoned");
            times_saved.get(hash.as_str()).copied().unwrap_or_default()
        };

        match changed {
            Ok(changed) => Ok(tonic::Response::new(proto::GetChangedOutputsResponse {
                changed_output_globs: changed.into_iter().collect(),
                time_saved: time_saved,
            })),
            Err(e) => {
                error!("flush directory operation failed: {:?}", e);
                Err(tonic::Status::internal("failed to watch flush directory"))
            }
        }
    }
}

impl<T: Watcher> NamedService for DaemonServer<T> {
    const NAME: &'static str = "turborepo.Daemon";
}

#[cfg(test)]
mod test {
    use std::{
        assert_matches,
        time::{Duration, Instant},
    };

    use tokio::select;
    use turbopath::AbsoluteSystemPathBuf;

    use super::DaemonServer;
    use crate::{commands::CommandBase, ui::UI, Args};

    // the windows runner starts a new thread to accept uds requests,
    // so we need a multi-threaded runtime
    #[tokio::test(flavor = "multi_thread")]
    #[tracing_test::traced_test]
    async fn lifecycle() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::new(tempdir.path()).unwrap();

        tracing::info!("start");

        let daemon = DaemonServer::new(
            &CommandBase::new(
                Args {
                    ..Default::default()
                },
                path.clone(),
                "test",
                UI::new(true),
            )
            .unwrap(),
            Duration::from_secs(60 * 60),
            path.clone(),
        )
        .unwrap();

        tracing::info!("server started");

        let pid_path = path.join_component("turbod.pid");
        let sock_path = path.join_component("turbod.sock");

        select! {
            _ = daemon.serve() => panic!("must not close"),
            _ = tokio::time::sleep(Duration::from_millis(10)) => (),
        }

        tracing::info!("yay we are done");

        assert!(!pid_path.exists(), "pid file must be deleted");
        assert!(!sock_path.exists(), "socket file must be deleted");

        tracing::info!("and files cleaned up")
    }

    // the windows runner starts a new thread to accept uds requests,
    // so we need a multi-threaded runtime
    #[tokio::test(flavor = "multi_thread")]
    #[tracing_test::traced_test]
    async fn timeout() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::new(tempdir.path()).unwrap();

        let daemon = DaemonServer::new(
            &CommandBase::new(
                Args {
                    ..Default::default()
                },
                path.clone(),
                "test",
                UI::new(true),
            )
            .unwrap(),
            Duration::from_millis(5),
            path.clone(),
        )
        .unwrap();

        let pid_path = path.join_component("turbod.pid");

        let now = Instant::now();
        let close_reason = daemon.serve().await;

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
}
