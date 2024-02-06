use std::{collections::BinaryHeap, fs::OpenOptions, time::Duration};

use notify::EventKind;
use thiserror::Error;
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::error::Elapsed,
};
use tracing::trace;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathRelation};

use crate::NotifyError;

#[derive(Clone, Debug, Error)]
pub enum WatchError {
    #[error(transparent)]
    RecvError(#[from] broadcast::error::RecvError),
    #[error("filewatching encountered errors: {0}")]
    NotifyError(#[from] NotifyError),
    #[error("filewatching has closed, cannot watch cookies")]
    Closed,
}

#[derive(Debug, Error)]
pub enum CookieError {
    #[error(transparent)]
    Watch(#[from] WatchError),
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
}

/// CookieWriter is responsible for assigning filesystem cookies to a request
/// for a downstream, filewatching-backed service.
pub struct CookieWriter {
    root: AbsoluteSystemPathBuf,
    timeout: Duration,
    cookie_request_tx: mpsc::Sender<oneshot::Sender<Result<usize, CookieError>>>,
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_ch: tokio::sync::oneshot::Sender<()>,
}

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
        // Lower serials should be sorted higher, since the heap pops the highest values
        // first
        other.serial.partial_cmp(&self.serial)
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
    root: AbsoluteSystemPathBuf,
    // We don't necessarily get requests in serial-order, but we want to keep them
    // in order so we don't have to scan all requests every time we get a new cookie.
    pending_requests: BinaryHeap<CookiedRequest<T>>,
    latest: usize,
}

impl<T> CookieWatcher<T> {
    pub(crate) fn new(root: AbsoluteSystemPathBuf) -> Self {
        Self {
            root,
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
        if let Some(serial) = self.serial_for_path(path) {
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

    fn serial_for_path(&self, path: &AbsoluteSystemPath) -> Option<usize> {
        if self.root.relation_to_path(path) == PathRelation::Parent {
            let filename = path.file_name()?;
            filename.strip_suffix(".cookie")?.parse().ok()
        } else {
            None
        }
    }
}

impl CookieWriter {
    pub fn new(root: &AbsoluteSystemPath, timeout: Duration) -> Self {
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();
        let (cookie_requests_tx, cookie_requests_rx) = mpsc::channel(16);
        tokio::spawn(watch_cookies(
            root.to_owned(),
            cookie_requests_rx,
            exit_signal,
        ));
        Self {
            root: root.to_owned(),
            timeout,
            cookie_request_tx: cookie_requests_tx,
            _exit_ch: exit_ch,
        }
    }

    pub(crate) fn root(&self) -> &AbsoluteSystemPath {
        &self.root
    }

    pub(crate) async fn cookie_request<T>(
        &self,
        request: T,
    ) -> Result<CookiedRequest<T>, CookieError> {
        // we need to write the cookie from a single task so as to serialize them
        let (resp_tx, resp_rx) = oneshot::channel();
        self.cookie_request_tx.clone().send(resp_tx).await?;
        let serial = tokio::time::timeout(self.timeout, resp_rx).await???;
        Ok(CookiedRequest { request, serial })
    }
}

async fn watch_cookies(
    root: AbsoluteSystemPathBuf,
    mut cookie_requests: mpsc::Receiver<oneshot::Sender<Result<usize, CookieError>>>,
    mut exit_signal: tokio::sync::oneshot::Receiver<()>,
) {
    let mut serial: usize = 0;
    loop {
        tokio::select! {
            biased;
            _ = &mut exit_signal => return,
            //event = file_events.recv() => handle_file_event(&root, &watches, event),
            req = cookie_requests.recv() => handle_cookie_request(&root, &mut serial, req),
        }
    }
}

fn handle_cookie_request(
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

#[cfg(test)]
mod test {
    use std::time::Duration;

    use futures::channel::oneshot;
    use notify::{event::CreateKind, Event, EventKind};
    use tokio::sync::{broadcast, mpsc};
    use turbopath::AbsoluteSystemPathBuf;

    use super::{CookieWatcher, CookiedRequest};
    use crate::{cookie_jar::CookieWriter, NotifyError};

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

    struct TestClient {
        reqs_tx: mpsc::Sender<CookiedRequest<TestQuery>>,
        cookie_jar: CookieWriter,
    }

    impl TestClient {
        async fn request(&self) {
            let (resp_tx, resp_rx) = oneshot::channel();
            let query = TestQuery { resp: resp_tx };
            let req = self.cookie_jar.cookie_request(query).await.unwrap();
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
        let (reqs_tx, reqs_rx) = mpsc::channel(16);
        let cookie_jar = CookieWriter::new(&path, Duration::from_millis(100));
        let (exit_tx, exit_rx) = oneshot::channel();

        let service = TestService {
            file_events,
            cookie_watcher: CookieWatcher::new(path.clone()),
            reqs_rx,
        };
        let service_handle = tokio::spawn(service.watch(exit_rx));

        let client = TestClient {
            reqs_tx,
            cookie_jar,
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
}
