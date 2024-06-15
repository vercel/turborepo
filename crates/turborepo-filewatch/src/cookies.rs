//! Cookies are the file watcher's way of synchronizing file system events. They
//! are files that are added to the file system that are named with the format
//! `[id].cookie`, where `[id]` is an increasing serial number, e.g.
//! `1.cookie`, `2.cookie`, and so on. The daemon can then watch for the
//! file creation event for this cookie file. Once it sees this event,
//! the daemon knows that the file system events are up to date and we
//! won't get any stale events.
//!
//! Here's the `CookieWriter` flow:
//! - `CookieWriter` spins up a `watch_for_cookie_requests` task and creates a
//!   `cookie_requests` mpsc channel to send a cookie request to that task. The
//!   cookie request consists of a oneshot `Sender` that the task can use to
//!   send back the serial number.
//! - The `watch_for_cookie_requests` task watches for cookie requests on
//!   `cookie_requests_rx`. When one occurs, it creates the cookie file and
//!   bumps the serial. It then sends the serial back using the `Sender`
//! - When `CookieWriter::cookie_request` is called, it sends the cookie request
//!   to the `watch_for_cookie_request` channel and then waits for the serial as
//!   a response (with a timeout). Upon getting the serial, a `CookiedRequest`
//!   gets returned with the serial number attached.
//!
//! And here's the `CookieWatcher` flow:
//! - `GlobWatcher` creates a `CookieWatcher`.
//! - `GlobWatcher` gets queries about globs that are wrapped in
//!   `CookiedRequest`. It passes these requests to
//!   `CookieWatcher::check_request`
//! - If the serial number attached to `CookiedRequest` has already been seen,
//!   `CookieWatcher::check_request` returns the inner query immediately.
//!   Otherwise, it gets stored in `CookieWatcher`.
//! - `GlobWatcher` waits for file system events on `recv`. When it gets an
//!   event, it passes the event to `CookieWatcher::pop_ready_requests`. If this
//!   event is indeed a cookie event, we return all of the requests that are now
//!   allowed to be processed (i.e. their serial number is now less than or
//!   equal to the latest seen serial).

use std::{collections::BinaryHeap, fs::OpenOptions, time::Duration};

use futures::FutureExt;
use notify::EventKind;
use thiserror::Error;
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time::error::Elapsed,
};
use tracing::trace;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathRelation};

use crate::{optional_watch::SomeRef, NotifyError, OptionalWatch};

#[derive(Debug, Error)]
pub enum CookieError {
    #[error("cookie timeout expired")]
    Timeout(#[from] Elapsed),
    #[error("failed to receiver cookie notification: {0}")]
    RecvError(#[from] oneshot::error::RecvError),
    #[error("failed to send cookie file request: {0}")]
    SendError(#[from] mpsc::error::SendError<oneshot::Sender<Result<usize, CookieError>>>),
    #[error("failed to write cookie file at {path}: {io_err}")]
    IO {
        io_err: std::io::Error,
        path: AbsoluteSystemPathBuf,
    },
    #[error("cookie queue is not available")]
    Unavailable(#[from] watch::error::RecvError),
}

/// CookieWriter is responsible for assigning filesystem cookies to a request
/// for a downstream, filewatching-backed service.
#[derive(Clone)]
pub struct CookieWriter {
    // Where we put the cookie files, usually `<repo_root>/.turbo/cookies`
    cookie_root: AbsoluteSystemPathBuf,
    timeout: Duration,
    cookie_request_sender_lazy:
        OptionalWatch<mpsc::Sender<oneshot::Sender<Result<usize, CookieError>>>>,
    // _exit_ch exists to trigger a close on the receiver when all instances
    // of this struct are dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_ch: mpsc::Sender<()>,
}

/// A request that can only be processed after the `serial` has been seen by the
/// `CookieWatcher`.
#[derive(Debug)]
pub struct CookiedRequest<T> {
    request: T,
    serial: usize,
}

impl<T> PartialEq for CookiedRequest<T> {
    fn eq(&self, other: &Self) -> bool {
        self.serial == other.serial
    }
}

impl<T> Eq for CookiedRequest<T> {}

impl<T> PartialOrd for CookiedRequest<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for CookiedRequest<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Lower serials should be sorted higher, since the heap pops the highest values
        // first
        other.serial.cmp(&self.serial)
    }
}

/// CookieWatcher is used by downstream filewatching-backed services to
/// know when it is safe to handle a particular request.
pub(crate) struct CookieWatcher<T> {
    // Where we expect to find the cookie files, usually `<repo_root>/.turbo/cookies`
    cookie_root: AbsoluteSystemPathBuf,
    // We don't necessarily get requests in serial-order, but we want to keep them
    // in order so we don't have to scan all requests every time we get a new cookie.
    pending_requests: BinaryHeap<CookiedRequest<T>>,
    latest: usize,
}

impl<T> CookieWatcher<T> {
    pub(crate) fn new(cookie_root: AbsoluteSystemPathBuf) -> Self {
        Self {
            cookie_root,
            pending_requests: BinaryHeap::new(),
            latest: 0,
        }
    }

    /// Check if this request can be handled immediately. If so, return it. If
    /// not, queue it
    pub(crate) fn check_request(&mut self, cookied_request: CookiedRequest<T>) -> Option<T> {
        if cookied_request.serial <= self.latest {
            // We've already seen the cookie for this request, handle it now
            Some(cookied_request.request)
        } else {
            // We haven't seen the cookie for this request yet, hold onto it
            self.pending_requests.push(cookied_request);
            None
        }
    }

    /// If this is a cookie file, pop all requests that are ready to be handled.
    /// The returned vector might be empty if this was a cookie file but we
    /// don't have any requests that are ready to be handled. None is
    /// returned if this is not a cookie file being created.
    pub(crate) fn pop_ready_requests(
        &mut self,
        event_kind: EventKind,
        path: &AbsoluteSystemPath,
    ) -> Option<Vec<T>> {
        if !matches!(event_kind, EventKind::Create(_)) {
            return None;
        }
        if let Some(serial) = serial_for_path(&self.cookie_root, path) {
            self.latest = serial;
            let mut ready_requests = Vec::new();
            while let Some(cookied_request) = self.pending_requests.pop() {
                if cookied_request.serial <= serial {
                    ready_requests.push(cookied_request.request);
                } else {
                    self.pending_requests.push(cookied_request);
                    break;
                }
            }
            Some(ready_requests)
        } else {
            None
        }
    }
}

fn serial_for_path(root: &AbsoluteSystemPath, path: &AbsoluteSystemPath) -> Option<usize> {
    if root.relation_to_path(path) == PathRelation::Parent {
        let filename = path.file_name()?;
        filename.strip_suffix(".cookie")?.parse().ok()
    } else {
        None
    }
}

impl CookieWriter {
    #[cfg(test)]
    pub fn new_with_default_cookie_dir(
        repo_root: &AbsoluteSystemPath,
        timeout: Duration,
        recv: OptionalWatch<broadcast::Receiver<Result<notify::Event, NotifyError>>>,
    ) -> Self {
        let cookie_root = repo_root.join_components(&[".turbo", "cookies"]);
        Self::new(&cookie_root, timeout, recv)
    }

    pub fn new(
        cookie_root: &AbsoluteSystemPath,
        timeout: Duration,
        mut recv: OptionalWatch<broadcast::Receiver<Result<notify::Event, NotifyError>>>,
    ) -> Self {
        let (cookie_request_sender_tx, cookie_request_sender_lazy) = OptionalWatch::new();
        let (exit_ch, exit_signal) = mpsc::channel(16);
        tokio::spawn({
            let root = cookie_root.to_owned();
            async move {
                if recv.get().await.is_err() {
                    // here we need to wait for confirmation that the watching end is ready
                    // before we start sending requests. this has the side effect of not
                    // enabling the cookie writing mechanism until the watcher is ready
                    return;
                }

                let (cookie_requests_tx, cookie_requests_rx) = mpsc::channel(16);

                if cookie_request_sender_tx
                    .send(Some(cookie_requests_tx))
                    .is_err()
                {
                    // the receiver has already been dropped
                    tracing::debug!("nobody listening for cookie requests, exiting");
                    return;
                };
                watch_for_cookie_file_requests(root.to_owned(), cookie_requests_rx, exit_signal)
                    .await;
            }
        });
        Self {
            cookie_root: cookie_root.to_owned(),
            timeout,
            cookie_request_sender_lazy,
            _exit_ch: exit_ch,
        }
    }

    pub(crate) fn root(&self) -> &AbsoluteSystemPath {
        &self.cookie_root
    }

    /// Sends a request to make a cookie file to the
    /// `watch_for_cookie_file_requests` task. Waits on a response from the
    /// task, and returns a `CookiedRequest` with the expected serial
    /// number.
    pub(crate) async fn cookie_request<T>(
        &self,
        request: T,
    ) -> Result<CookiedRequest<T>, CookieError> {
        // we need to write the cookie from a single task so as to serialize them
        tokio::time::timeout(self.timeout, self.cookie_request_inner(request, None)).await?
    }

    /// Internal services may want to wait for the cookie
    /// system to be set up, before issuing a timeout for the
    /// cookie request.
    pub(crate) async fn initialized_cookie_request<T>(
        &self,
        request: T,
    ) -> Result<CookiedRequest<T>, CookieError> {
        self.cookie_request_inner(request, Some(self.timeout)).await
    }

    async fn cookie_request_inner<T>(
        &self,
        request: T,
        timeout: Option<Duration>,
    ) -> Result<CookiedRequest<T>, CookieError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let mut cookie_request_sender_lazy = self.cookie_request_sender_lazy.clone();
        let cookie_request_sender_lazy = cookie_request_sender_lazy
            .get()
            .await
            .map(|s| s.to_owned())?;
        cookie_request_sender_lazy.send(resp_tx).await?;
        let serial = match timeout {
            Some(timeout) => {
                let resp_rx = tokio::time::timeout(timeout, resp_rx).await??;
                resp_rx?
            }
            None => resp_rx.await??,
        };
        Ok(CookiedRequest { request, serial })
    }
}

async fn watch_for_cookie_file_requests(
    root: AbsoluteSystemPathBuf,
    mut cookie_requests: mpsc::Receiver<oneshot::Sender<Result<usize, CookieError>>>,
    mut exit_signal: mpsc::Receiver<()>,
) {
    let mut serial: usize = 0;
    loop {
        tokio::select! {
            biased;
            _ = exit_signal.recv() => return,
            req = cookie_requests.recv() => handle_cookie_file_request(&root, &mut serial, req),
        }
    }
}

fn handle_cookie_file_request(
    root: &AbsoluteSystemPath,
    serial: &mut usize,
    req: Option<oneshot::Sender<Result<usize, CookieError>>>,
) {
    if let Some(req) = req {
        *serial += 1;
        let cookie_path = root.join_component(&format!("{}.cookie", serial));
        let mut opts = OpenOptions::new();
        opts.truncate(true).create(true).write(true);
        let result = {
            // dropping the resulting file closes the handle
            trace!("writing cookie {}", cookie_path);
            cookie_path
                .ensure_dir()
                .and_then(|_| cookie_path.open_with_options(opts))
                .map_err(|io_err| CookieError::IO {
                    io_err,
                    path: cookie_path.clone(),
                })
        };
        let result = result.map(|_| *serial);
        // We don't care if the client has timed out and gone away
        let _ = req.send(result);
    }
}

/// a lightweight wrapper around OptionalWatch that embeds cookie ids into the
/// get call. for requests that require cookies (ie, waiting for filesystem
/// flushes) then a cookie watch is ideal
pub struct CookiedOptionalWatch<T, P: CookieReady> {
    value: watch::Receiver<Option<T>>,
    cookie_index: watch::Receiver<usize>,
    cookie_writer: CookieWriter,
    parent: P,
}

impl<T, P: CookieReady + Clone> Clone for CookiedOptionalWatch<T, P> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            cookie_index: self.cookie_index.clone(),
            cookie_writer: self.cookie_writer.clone(),
            parent: self.parent.clone(),
        }
    }
}

pub trait CookieReady {
    fn ready(&mut self, id: usize) -> impl std::future::Future<Output = ()>;
}

impl CookieReady for () {
    async fn ready(&mut self, _id: usize) {}
}

impl<T, U: CookieReady> CookieReady for CookiedOptionalWatch<T, U> {
    async fn ready(&mut self, id: usize) {
        tracing::debug!("waiting for cookie {}", id);
        self.parent.ready(id).await;
        _ = self.cookie_index.wait_for(|v| v >= &id).await;
    }
}

impl<T> CookiedOptionalWatch<T, ()> {
    pub fn new(
        update: CookieWriter,
    ) -> (
        watch::Sender<Option<T>>,
        CookieRegister,
        CookiedOptionalWatch<T, ()>,
    ) {
        let (tx, rx) = watch::channel(None);
        let (cookie_tx, cookie_rx) = watch::channel(0);
        tracing::debug!("starting cookied optional watch in {}", update.root());
        (
            tx,
            CookieRegister(cookie_tx, update.root().to_owned()),
            CookiedOptionalWatch {
                value: rx,
                cookie_index: cookie_rx,
                cookie_writer: update,
                parent: (),
            },
        )
    }
}

impl<T, U: CookieReady + Clone> CookiedOptionalWatch<T, U> {
    /// Create a new sibling cookie watcher that inherits the same fs source as
    /// this one.
    pub fn new_sibling<T2>(&self) -> (watch::Sender<Option<T2>>, CookiedOptionalWatch<T2, U>) {
        let (tx, rx) = watch::channel(None);
        (
            tx,
            CookiedOptionalWatch {
                value: rx,
                cookie_index: self.cookie_index.clone(),
                cookie_writer: self.cookie_writer.clone(),
                parent: self.parent.clone(),
            },
        )
    }

    /// Create a new child cookie watcher that inherits the same fs source as
    /// this one, but also has its own cookie source. This allows you to
    /// synchronize two independent cookie streams.
    pub fn new_child<T2>(
        &self,
    ) -> (
        watch::Sender<Option<T2>>,
        CookieRegister,
        CookiedOptionalWatch<T2, Self>,
    ) {
        let (tx, rx) = watch::channel(None);
        let (cookie_tx, cookie_rx) = watch::channel(0);

        (
            tx,
            CookieRegister(cookie_tx, self.cookie_writer.root().to_owned()),
            CookiedOptionalWatch {
                value: rx,
                cookie_index: cookie_rx,
                cookie_writer: self.cookie_writer.clone(),
                parent: self.clone(),
            },
        )
    }

    #[tracing::instrument(skip(self))]
    pub async fn get(&mut self) -> Result<SomeRef<'_, T>, CookieError> {
        let next_id = self
            .cookie_writer
            .initialized_cookie_request(())
            .await?
            .serial;
        self.ready(next_id).await;
        tracing::debug!("got cookie, waiting for data");
        Ok(self.get_inner().await?)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_change(&mut self) -> Result<SomeRef<'_, T>, watch::error::RecvError> {
        self.value.changed().await?;
        self.get_inner().await
    }

    /// Please do not use this data from a user-facing query. It should only
    /// really be used for internal state management. Equivalent to
    /// `OptionalWatch::get`
    ///
    /// For an example as to why we need this, sometimes file event processing
    /// needs to access data but issuing a cookie request would deadlock.
    ///
    /// `_reason` is purely for documentation purposes and is not used.
    pub async fn get_raw(
        &mut self,
        _reason: &str,
    ) -> Result<SomeRef<'_, T>, watch::error::RecvError> {
        self.get_inner().await
    }

    /// Get the current value, if it is available.
    ///
    /// Unlike `OptionalWatch::get_immediate`, this method will block until the
    /// cookie has been seen, at which point it will call `now_or_never` on the
    /// value watch.
    #[tracing::instrument(skip(self))]
    pub async fn get_immediate(
        &mut self,
    ) -> Option<Result<SomeRef<'_, T>, watch::error::RecvError>> {
        let next_id = self.cookie_writer.cookie_request(()).await.ok()?.serial;
        self.cookie_index.wait_for(|v| v >= &next_id).await.ok()?;
        self.get_inner().now_or_never()
    }

    /// Please do not use this data from a user-facing query. It should only
    /// really be used for internal state management. Equivalent to
    /// `OptionalWatch::get`
    ///
    /// For an example as to why we need this, sometimes file event processing
    /// needs to access data but issuing a cookie request would deadlock.
    ///
    /// `_reason` is purely for documentation purposes and is not used.
    pub async fn get_immediate_raw(
        &mut self,
        reason: &str,
    ) -> Option<Result<SomeRef<'_, T>, watch::error::RecvError>> {
        self.get_raw(reason).now_or_never()
    }

    async fn get_inner(&mut self) -> Result<SomeRef<'_, T>, watch::error::RecvError> {
        self.value.wait_for(|f| f.is_some()).await?;
        Ok(SomeRef(self.value.borrow()))
    }

    pub(crate) fn watch(&self) -> watch::Receiver<Option<T>> {
        self.value.clone()
    }
}

pub struct CookieRegister(watch::Sender<usize>, AbsoluteSystemPathBuf);
impl CookieRegister {
    pub fn register(&self, paths: &[&AbsoluteSystemPath]) {
        tracing::trace!("registering cookie for {:?}", paths);
        for path in paths {
            if let Some(serial) = serial_for_path(&self.1, path) {
                tracing::trace!("updating cookie to {}", serial);
                let _ = self.0.send(serial);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use notify::{event::CreateKind, Event, EventKind};
    use tokio::{
        sync::{broadcast, mpsc, oneshot},
        task::JoinSet,
    };
    use turbopath::AbsoluteSystemPathBuf;

    use super::{CookieWatcher, CookiedRequest};
    use crate::{cookies::CookieWriter, NotifyError, OptionalWatch};

    struct TestQuery {
        resp: oneshot::Sender<()>,
    }

    struct TestService {
        file_events: broadcast::Receiver<Result<Event, NotifyError>>,
        cookie_watcher: CookieWatcher<TestQuery>,
        reqs_rx: mpsc::Receiver<CookiedRequest<TestQuery>>,
    }

    impl TestService {
        async fn watch(mut self, mut exit_ch: oneshot::Receiver<()>) {
            loop {
                tokio::select! {
                    biased;
                    _ = &mut exit_ch => return,
                    Some(req) = self.reqs_rx.recv() => {
                        if let Some(query) = self.cookie_watcher.check_request(req) {
                            query.resp.send(()).unwrap();
                        }
                    }
                    file_event = self.file_events.recv() => {
                        let event = file_event.unwrap().unwrap();
                        for path in event.paths {
                            let path = AbsoluteSystemPathBuf::try_from(path).unwrap();
                            if let Some(queries) = self.cookie_watcher.pop_ready_requests(event.kind, &path) {
                                for query in queries {
                                    query.resp.send(()).unwrap();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[derive(Clone)]
    struct TestClient {
        reqs_tx: mpsc::Sender<CookiedRequest<TestQuery>>,
        cookie_writer: CookieWriter,
    }

    impl TestClient {
        async fn request(&self) {
            let (resp_tx, resp_rx) = oneshot::channel();
            let query = TestQuery { resp: resp_tx };
            let req = self.cookie_writer.cookie_request(query).await.unwrap();
            self.reqs_tx.send(req).await.unwrap();
            resp_rx.await.unwrap();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_service_cookies() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let (send_file_events, file_events) = broadcast::channel(16);
        let recv = OptionalWatch::once(file_events.resubscribe());
        let (reqs_tx, reqs_rx) = mpsc::channel(16);
        let cookie_writer = CookieWriter::new(&path, Duration::from_secs(2), recv);
        let (exit_tx, exit_rx) = oneshot::channel();

        let service = TestService {
            file_events,
            cookie_watcher: CookieWatcher::new(path.clone()),
            reqs_rx,
        };
        let service_handle = tokio::spawn(service.watch(exit_rx));

        let client = TestClient {
            reqs_tx,
            cookie_writer,
        };
        // race request and file event. Either order should work.
        tokio_scoped::scope(|scope| {
            scope.spawn(client.request());
            send_file_events
                .send(Ok(Event {
                    kind: EventKind::Create(CreateKind::File),
                    paths: vec![path.join_component("1.cookie").as_std_path().to_owned()],
                    ..Default::default()
                }))
                .unwrap();
        });

        // explicitly send the cookie first
        tokio_scoped::scope(|scope| {
            send_file_events
                .send(Ok(Event {
                    kind: EventKind::Create(CreateKind::File),
                    paths: vec![path.join_component("2.cookie").as_std_path().to_owned()],
                    ..Default::default()
                }))
                .unwrap();
            scope.spawn(client.request());
        });

        // send a cookie with a much higher value
        tokio_scoped::scope(|scope| {
            send_file_events
                .send(Ok(Event {
                    kind: EventKind::Create(CreateKind::File),
                    paths: vec![path.join_component("20.cookie").as_std_path().to_owned()],
                    ..Default::default()
                }))
                .unwrap();
            scope.spawn(client.request());
        });
        exit_tx.send(()).unwrap();
        service_handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_out_of_order_requests() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let (send_file_events, file_events) = broadcast::channel(16);
        let recv = OptionalWatch::once(file_events.resubscribe());
        let (reqs_tx, reqs_rx) = mpsc::channel(16);
        let cookie_writer = CookieWriter::new(&path, Duration::from_secs(2), recv);
        let (exit_tx, exit_rx) = oneshot::channel();

        let service = TestService {
            file_events,
            cookie_watcher: CookieWatcher::new(path.clone()),
            reqs_rx,
        };
        let service_handle = tokio::spawn(service.watch(exit_rx));

        let client = TestClient {
            reqs_tx,
            cookie_writer,
        };

        let mut join_set = JoinSet::new();
        let client_1 = client.clone();
        join_set.spawn(async move { client_1.request().await });

        let client_2 = client.clone();
        join_set.spawn(async move { client_2.request().await });

        let client_3 = client.clone();
        join_set.spawn(async move { client_3.request().await });

        send_file_events
            .send(Ok(Event {
                kind: EventKind::Create(CreateKind::File),
                paths: vec![path.join_component("2.cookie").as_std_path().to_owned()],
                ..Default::default()
            }))
            .unwrap();

        // Expect 2 rpcs to be ready. We don't know which ones they will be
        // but we also don't care. We don't have ordering semantics on the client
        // side.
        join_set.join_next().await.unwrap().unwrap();
        join_set.join_next().await.unwrap().unwrap();

        send_file_events
            .send(Ok(Event {
                kind: EventKind::Create(CreateKind::File),
                paths: vec![path.join_component("3.cookie").as_std_path().to_owned()],
                ..Default::default()
            }))
            .unwrap();
        join_set.join_next().await.unwrap().unwrap();

        exit_tx.send(()).unwrap();
        service_handle.await.unwrap();
    }
}
