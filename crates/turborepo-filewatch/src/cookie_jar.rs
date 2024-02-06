use std::{
    collections::{HashMap, VecDeque},
    fs::OpenOptions,
    path::PathBuf,
    sync::{atomic::AtomicUsize, Arc, Mutex},
    time::Duration,
};

use notify::{Event, EventKind};
use thiserror::Error;
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::error::Elapsed,
};
use tracing::{debug, trace};
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

type CookieResponse = Result<(), WatchError>;

pub struct CookieJar {
    root: AbsoluteSystemPathBuf,
    serial: AtomicUsize,
    timeout: Duration,
    watches: Arc<Mutex<Watches>>,
    cookie_request_tx: mpsc::Sender<oneshot::Sender<Result<usize, CookieError>>>,
    // _exit_ch exists to trigger a close on the receiver when an instance
    // of this struct is dropped. The task that is receiving events will exit,
    // dropping the other sender for the broadcast channel, causing all receivers
    // to be notified of a close.
    _exit_ch: tokio::sync::oneshot::Sender<()>,
}

pub struct CookiedRequest<T> {
    request: T,
    serial: usize,
}

pub(crate) struct CookieWatcher<T> {
    root: AbsoluteSystemPathBuf,
    pending_requests: VecDeque<CookiedRequest<T>>,
    latest: usize,
}

impl<T> CookieWatcher<T> {
    pub(crate) fn new(root: AbsoluteSystemPathBuf) -> Self {
        Self {
            root,
            pending_requests: VecDeque::new(),
            latest: 0,
        }
    }

    pub(crate) fn check_request(&mut self, cookied_request: CookiedRequest<T>) -> Option<T> {
        if cookied_request.serial <= self.latest {
            // We've already seen the cookie for this request, handle it now
            Some(cookied_request.request)
        } else {
            // We haven't seen the cookie for this request yet, hold onto it
            self.pending_requests.push_back(cookied_request);
            None
        }
    }

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
            while let Some(cookied_request) = self.pending_requests.pop_front() {
                if cookied_request.serial <= serial {
                    ready_requests.push(cookied_request.request);
                } else {
                    self.pending_requests.push_front(cookied_request);
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

#[derive(Default)]
struct Watches {
    closed: bool,
    cookies: HashMap<PathBuf, oneshot::Sender<CookieResponse>>,
}

impl CookieJar {
    pub fn new(
        root: &AbsoluteSystemPath,
        timeout: Duration,
        file_events: broadcast::Receiver<Result<Event, NotifyError>>,
    ) -> Self {
        let (exit_ch, exit_signal) = tokio::sync::oneshot::channel();
        let watches = Arc::new(Mutex::new(Watches::default()));
        let (cookie_requests_tx, cookie_requests_rx) = mpsc::channel(16);
        tokio::spawn(watch_cookies(
            root.to_owned(),
            watches.clone(),
            file_events,
            cookie_requests_rx,
            exit_signal,
        ));
        Self {
            root: root.to_owned(),
            serial: AtomicUsize::new(0),
            timeout,
            watches,
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

    pub async fn wait_for_cookie(&self) -> Result<(), CookieError> {
        let serial = self
            .serial
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let cookie_path = self.root.join_component(&format!("{}.cookie", serial));
        let (tx, rx) = oneshot::channel();
        {
            let mut watches = self.watches.lock().expect("mutex poisoned");
            if watches.closed {
                return Err(CookieError::Watch(WatchError::Closed));
            }
            watches
                .cookies
                .insert(cookie_path.as_std_path().to_owned(), tx);
        }
        let mut opts = OpenOptions::new();
        opts.truncate(true).create(true).write(true);
        {
            // dropping the resulting file closes the handle
            trace!("writing cookie {}", cookie_path);
            _ = cookie_path
                .ensure_dir()
                .and_then(|_| cookie_path.open_with_options(opts))
                .map_err(|io_err| CookieError::IO {
                    io_err,
                    path: cookie_path.clone(),
                })?;
        }
        // ??? -> timeout, recv failure, actual cookie failure
        tokio::time::timeout(self.timeout, rx).await???;
        Ok(())
    }
}

async fn watch_cookies(
    root: AbsoluteSystemPathBuf,
    watches: Arc<Mutex<Watches>>,
    mut file_events: broadcast::Receiver<Result<Event, NotifyError>>,
    mut cookie_requests: mpsc::Receiver<oneshot::Sender<Result<usize, CookieError>>>,
    mut exit_signal: tokio::sync::oneshot::Receiver<()>,
) {
    let mut serial: usize = 0;
    loop {
        tokio::select! {
            biased;
            _ = &mut exit_signal => return,
            event = file_events.recv() => handle_file_event(&root, &watches, event),
            req = cookie_requests.recv() => handle_cookie_request(&root, &mut serial, req),
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

        fn handle_file_event(
            root: &AbsoluteSystemPath,
            watches: &Arc<Mutex<Watches>>,
            event: Result<Result<Event, NotifyError>, broadcast::error::RecvError>,
        ) {
            match flatten_event(event) {
                Ok(event) => {
                    if matches!(event.kind, EventKind::Create(_)) {
                        let mut watches = watches.lock().expect("mutex poisoned");
                        for path in event.paths {
                            let abs_path: &AbsoluteSystemPath = path
                                .as_path()
                                .try_into()
                                .expect("Non-absolute path from filewatching");
                            if root.relation_to_path(abs_path) == PathRelation::Parent {
                                trace!("saw cookie: {}", abs_path);
                                if let Some(responder) = watches.cookies.remove(&path) {
                                    if responder.send(Ok(())).is_err() {
                                        // Note that cookie waiters will time out if they don't get
                                        // a response, so we
                                        // don't necessarily
                                        // need to panic here, although we could decide to do that
                                        // in the future.
                                        debug!("failed to notify cookie waiter of cookie success");
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    // we got an error, notify all waiters that their cookie failed
                    let is_closing = matches!(
                        e,
                        WatchError::RecvError(broadcast::error::RecvError::Closed)
                    );
                    let resp = if is_closing { WatchError::Closed } else { e };
                    let mut watches = watches.lock().expect("mutex poisoned");
                    for (_, sender) in watches.cookies.drain() {
                        if sender.send(Err(resp.clone())).is_err() {
                            // Note that cookie waiters will time out if they don't get a response,
                            // so we don't necessarily need to panic
                            // here, although we could decide to do that
                            // in the future.
                            debug!("failed to notify cookie waiter of error: {}", resp);
                        }
                    }
                    if is_closing {
                        watches.closed = true;
                        return;
                    }
                }
            }
        }
    }
}

// result flattening is an unstable feature, so add a manual helper to do so.
// This version is for unwrapping events coming from filewatching
fn flatten_event(
    event: Result<Result<Event, NotifyError>, broadcast::error::RecvError>,
) -> Result<Event, WatchError> {
    Ok(event??)
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, sync::Arc, time::Duration};

    use notify::{event::CreateKind, ErrorKind, Event, EventKind};
    use tokio::{sync::broadcast, time};
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use crate::{
        cookie_jar::{CookieError, CookieJar, WatchError},
        NotifyError,
    };

    async fn ensure_tracked(cookie_jar: &CookieJar, path: &AbsoluteSystemPath) {
        let path = path.as_std_path();
        let mut interval = time::interval(Duration::from_millis(2));
        for _i in 0..50 {
            interval.tick().await;
            let watches = cookie_jar.watches.lock().expect("mutex poisoned");
            if watches.cookies.contains_key(path) {
                return;
            }
        }
        panic!("failed to find path in cookie_jar")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_wait_for_cookie() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let (send_file_events, file_events) = broadcast::channel(16);

        let cookie_jar = CookieJar::new(&path, Duration::from_millis(100), file_events);
        let cookie_path = path.join_component("0.cookie");
        tokio_scoped::scope(|scope| {
            scope.spawn(async { cookie_jar.wait_for_cookie().await.unwrap() });

            scope.block_on(ensure_tracked(&cookie_jar, &cookie_path));

            send_file_events
                .send(Ok(Event {
                    kind: EventKind::Create(CreateKind::File),
                    paths: vec![cookie_path.as_std_path().to_owned()],
                    ..Default::default()
                }))
                .unwrap();
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_wait_for_cookie_after_close() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let (send_file_events, file_events) = broadcast::channel(16);

        let cookie_jar = CookieJar::new(&path, Duration::from_millis(1000), file_events);
        tokio_scoped::scope(|scope| {
            scope.spawn(async {
                let result = cookie_jar.wait_for_cookie().await;
                assert_matches!(result, Err(CookieError::Watch(WatchError::Closed)));
            });
            // We don't care whether or not we're tracking the cookie yet, either codepath
            // should result in the same error

            // Dropping the [last, only] sender closes the channel, which closes
            // the loop watching the cookie folder
            drop(send_file_events);
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_wait_for_cookie_timeout() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let (_send_file_events, file_events) = broadcast::channel(16);

        let cookie_jar = CookieJar::new(&path, Duration::from_millis(10), file_events);
        tokio_scoped::scope(|scope| {
            scope.spawn(async {
                let result = cookie_jar.wait_for_cookie().await;
                assert_matches!(result, Err(CookieError::Timeout(_)));
            });

            // Don't send any events, expect to timeout.
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_wait_for_cookie_with_error() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        let (send_file_events, file_events) = broadcast::channel(16);

        let cookie_jar = CookieJar::new(&path, Duration::from_millis(10), file_events);
        let cookie_path = path.join_component("0.cookie");
        tokio_scoped::scope(|scope| {
            scope.spawn(async {
                let result = cookie_jar.wait_for_cookie().await;
                assert_matches!(result, Err(CookieError::Watch(WatchError::NotifyError(_))));
            });

            scope.block_on(ensure_tracked(&cookie_jar, &cookie_path));

            // send an error, assert that we fail to get our cookie
            send_file_events
                .send(Err(NotifyError(Arc::new(notify::Error {
                    kind: ErrorKind::Generic("test error".to_string()),
                    paths: vec![cookie_path.as_std_path().to_owned()],
                }))))
                .unwrap();
        });

        let cookie_path = path.join_component("1.cookie");
        tokio_scoped::scope(|scope| {
            scope.spawn(async {
                cookie_jar.wait_for_cookie().await.unwrap();
            });

            scope.block_on(ensure_tracked(&cookie_jar, &cookie_path));

            // ensure that we can still wait for new cookies even though an error occurred
            // previously
            send_file_events
                .send(Ok(Event {
                    kind: EventKind::Create(CreateKind::File),
                    paths: vec![cookie_path.as_std_path().to_owned()],
                    ..Default::default()
                }))
                .unwrap();
        });
    }
}
