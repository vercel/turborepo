#![deny(clippy::all)]
#![feature(assert_matches)]

//! A crate for registering listeners for a given signal

pub mod listeners;
pub mod signals;

use std::{
    fmt::Debug,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use futures::{Stream, StreamExt, stream::FuturesUnordered};
use signals::Signal;
use tokio::{
    pin,
    sync::{Notify, mpsc, oneshot},
};

/// Why the signal handler started shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReason {
    /// A real OS signal, such as Ctrl+C or SIGTERM, started shutdown.
    Signal = 1,
    /// Shutdown started because `close()` was called or because the signal
    /// source ended without yielding a signal.
    Close = 2,
}

/// SignalHandler notifies subscribers when shutdown starts because of a real
/// signal, an explicit `close()`, or a signal source that ends without yielding
/// a signal.
#[derive(Debug, Clone)]
pub struct SignalHandler {
    state: Arc<Mutex<HandlerState>>,
    close: mpsc::Sender<()>,
    shutdown_reason: Arc<AtomicU8>,
    started: Arc<Notify>,
}

#[derive(Debug, Default)]
struct HandlerState {
    subscribers: Vec<oneshot::Sender<oneshot::Sender<Signal>>>,
    is_closing: bool,
}

pub struct SignalSubscriber(oneshot::Receiver<oneshot::Sender<Signal>>);

/// SubscriberGuard should be kept until a subscriber is done processing the
/// signal
pub struct SubscriberGuard {
    _guard: oneshot::Sender<Signal>,
}

impl SignalHandler {
    /// Construct a new SignalHandler that alerts subscribers when
    /// `signal_source` yields a signal, when `close()` is called, or when the
    /// signal source ends without yielding a signal.
    pub fn new(signal_source: impl Stream<Item = Option<Signal>> + Send + 'static) -> Self {
        // think about channel size
        let state = Arc::new(Mutex::new(HandlerState::default()));
        let worker_state = state.clone();
        let shutdown_reason = Arc::new(AtomicU8::new(0));
        let worker_shutdown_reason = shutdown_reason.clone();
        let started = Arc::new(Notify::new());
        let worker_started = started.clone();
        let (close, mut rx) = mpsc::channel::<()>(1);
        tokio::spawn(async move {
            pin!(signal_source);
            let shutdown_reason = tokio::select! {
                signal = signal_source.next() => match signal {
                    Some(Some(_signal)) => ShutdownReason::Signal,
                    Some(None) | None => ShutdownReason::Close,
                },
                // We don't care if a close message was sent or if all handlers are dropped.
                // Either way start the shutdown process.
                _ = rx.recv() => ShutdownReason::Close,
            };
            worker_shutdown_reason.store(shutdown_reason as u8, Ordering::Release);
            worker_started.notify_waiters();

            let mut callbacks = {
                let mut state = worker_state.lock().expect("lock poisoned");
                // Mark ourselves as closing to prevent any additional subscribers from being
                // added
                state.is_closing = true;
                state
                    .subscribers
                    .drain(..)
                    .filter_map(|callback| {
                        let (tx, rx) = oneshot::channel();
                        // If the subscriber is no longer around we don't wait for the callback
                        callback.send(tx).ok()?;
                        Some(rx)
                    })
                    .collect::<FuturesUnordered<_>>()
            };

            // We don't care if callback gets dropped or if the done signal is sent.
            while let Some(_fut) = callbacks.next().await {}
        });

        Self {
            state,
            close,
            shutdown_reason,
            started,
        }
    }

    /// Register a new subscriber
    /// Will return `None` if SignalHandler is in the process of shutting down
    /// or if it has already shut down.
    pub fn subscribe(&self) -> Option<SignalSubscriber> {
        self.state
            .lock()
            .expect("poisoned lock")
            .add_subscriber()
            .map(SignalSubscriber)
    }

    /// Send message to signal handler that it should shut down and alert
    /// subscribers
    pub async fn close(&self) {
        if self.close.send(()).await.is_err() {
            // watcher has already closed
            return;
        }
        self.done().await;
    }

    /// Wait until handler is finished and all subscribers finish their cleanup
    /// work
    pub async fn done(&self) {
        // Receiver is dropped once the worker task completes
        self.close.closed().await;
    }

    /// Wait until shutdown starts for any reason.
    pub async fn started(&self) {
        let started = self.started.notified();
        if self.shutdown_reason().is_some() {
            return;
        }

        started.await;
    }

    /// Wait until shutdown starts because of a real OS signal.
    pub async fn signal_started(&self) {
        loop {
            let started = self.started.notified();
            match self.shutdown_reason() {
                Some(ShutdownReason::Signal) => return,
                Some(ShutdownReason::Close) => std::future::pending::<()>().await,
                None => started.await,
            }
        }
    }

    /// Return the reason shutdown started, if shutdown has started.
    pub fn shutdown_reason(&self) -> Option<ShutdownReason> {
        match self.shutdown_reason.load(Ordering::Acquire) {
            1 => Some(ShutdownReason::Signal),
            2 => Some(ShutdownReason::Close),
            _ => None,
        }
    }

    // Check if the worker thread is done, only meant to be used for assertions in
    // testing
    #[cfg(test)]
    fn is_done(&self) -> bool {
        self.close.is_closed()
    }
}

impl SignalSubscriber {
    /// Wait until signal is received by the signal handler
    pub async fn listen(self) -> SubscriberGuard {
        let _guard = self
            .0
            .await
            .expect("signal handler worker thread exited without alerting subscribers");
        SubscriberGuard { _guard }
    }
}

impl HandlerState {
    fn add_subscriber(&mut self) -> Option<oneshot::Receiver<oneshot::Sender<Signal>>> {
        (!self.is_closing).then(|| {
            let (tx, rx) = oneshot::channel();
            self.subscribers.push(tx);
            rx
        })
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, time::Duration};

    use futures::stream;

    use super::*;

    #[cfg(windows)]
    const DEFAULT_SIGNAL: Signal = Signal::CtrlC;
    #[cfg(not(windows))]
    const DEFAULT_SIGNAL: Signal = Signal::Interrupt;

    #[tokio::test]
    async fn test_subscribers_triggered_from_signal() {
        let (tx, rx) = oneshot::channel();
        let handler = SignalHandler::new(stream::once(async move {
            rx.await.ok();
            Some(DEFAULT_SIGNAL)
        }));
        let subscriber = handler.subscribe().unwrap();
        // Send mocked SIGINT
        tx.send(DEFAULT_SIGNAL).unwrap();

        let (done, mut is_done) = oneshot::channel();
        let handler2 = handler.clone();
        tokio::spawn(async move {
            handler2.done().await;
            done.send(()).ok();
        });

        let _guard = subscriber.listen().await;
        assert_eq!(handler.shutdown_reason(), Some(ShutdownReason::Signal));
        assert_matches!(
            is_done.try_recv(),
            Err(oneshot::error::TryRecvError::Empty),
            "done shouldn't be finished"
        );
        drop(_guard);
        tokio::time::sleep(Duration::from_millis(5)).await;
        handler.done().await;
    }

    #[tokio::test]
    async fn test_subscribers_triggered_from_close() {
        let (_tx, rx) = oneshot::channel::<()>();
        let handler = SignalHandler::new(stream::once(async move {
            rx.await.ok();
            Some(DEFAULT_SIGNAL)
        }));
        let subscriber = handler.subscribe().unwrap();
        let (close_done, mut is_close_done) = oneshot::channel();

        let h2 = handler.clone();
        let _handle = tokio::spawn(async move {
            h2.close().await;
            close_done.send(()).ok();
        });

        let _guard = subscriber.listen().await;
        assert_eq!(handler.shutdown_reason(), Some(ShutdownReason::Close));
        assert_matches!(
            is_close_done.try_recv(),
            Err(oneshot::error::TryRecvError::Empty),
            "close shouldn't be finished"
        );
        drop(_guard);
        handler.done().await;
    }

    #[tokio::test]
    async fn test_close_idempotent() {
        let (_tx, rx) = oneshot::channel::<()>();
        let handler = SignalHandler::new(stream::once(async move {
            rx.await.ok();
            Some(DEFAULT_SIGNAL)
        }));
        handler.close().await;
        handler.close().await;
    }

    #[tokio::test]
    async fn test_signal_source_none_treated_as_close() {
        let handler = SignalHandler::new(stream::iter([None]));
        let subscriber = handler.subscribe().unwrap();

        let _guard = subscriber.listen().await;
        assert_eq!(handler.shutdown_reason(), Some(ShutdownReason::Close));
        drop(_guard);
        handler.done().await;
    }

    #[tokio::test]
    async fn test_signal_started_only_resolves_for_signal() {
        let (_tx, rx) = oneshot::channel::<()>();
        let handler = SignalHandler::new(stream::once(async move {
            rx.await.ok();
            Some(DEFAULT_SIGNAL)
        }));

        let signal_started = {
            let handler = handler.clone();
            tokio::spawn(async move {
                handler.signal_started().await;
            })
        };

        handler.close().await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(
            !signal_started.is_finished(),
            "signal_started should stay pending for close-driven shutdown"
        );
        signal_started.abort();
    }

    #[tokio::test]
    async fn test_signal_started_resolves_for_signal() {
        let (tx, rx) = oneshot::channel();
        let handler = SignalHandler::new(stream::once(async move {
            rx.await.ok();
            Some(DEFAULT_SIGNAL)
        }));

        let signal_started = {
            let handler = handler.clone();
            tokio::spawn(async move {
                handler.signal_started().await;
            })
        };

        tx.send(DEFAULT_SIGNAL).unwrap();
        tokio::time::timeout(Duration::from_secs(1), signal_started)
            .await
            .expect("signal_started should resolve after a signal")
            .unwrap();
        assert_eq!(handler.shutdown_reason(), Some(ShutdownReason::Signal));
        handler.done().await;
    }

    #[tokio::test]
    async fn test_subscribe_after_close() {
        let (tx, rx) = oneshot::channel();
        let handler = SignalHandler::new(stream::once(async move {
            rx.await.ok();
            Some(DEFAULT_SIGNAL)
        }));
        let subscriber = handler.subscribe().unwrap();

        // Send SIGINT
        tx.send(DEFAULT_SIGNAL).unwrap();
        // Do a quick yield to give the worker a chance to read the sigint
        tokio::task::yield_now().await;
        assert!(
            !handler.is_done(),
            "handler should not finish until subscriber finishes"
        );
        assert!(
            handler.subscribe().is_none(),
            "handler that has received a signal should not accept new subscribers"
        );
        let _guard = subscriber.listen().await;
        assert_eq!(handler.shutdown_reason(), Some(ShutdownReason::Signal));
        drop(_guard);
        handler.done().await;
    }
}
