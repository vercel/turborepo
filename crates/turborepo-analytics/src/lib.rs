#![deny(clippy::all)]

//! Turborepo's analytics library. Handles sending analytics events to the
//! Vercel API in the background. We only record cache usage events,
//! so when the cache is hit or missed for the file system or the HTTP cache.
//! Requires the user to be logged in to Vercel.

use std::time::Duration;

use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::{JoinError, JoinHandle, JoinSet},
};
use tracing::debug;
use turborepo_api_client::{APIAuth, analytics::AnalyticsClient};
pub use turborepo_vercel_api::AnalyticsEvent;
use uuid::Uuid;

// Batch frequent cache events without waiting for the idle timer on busy runs.
// The API's request body limit is not specified here; this is an event count,
// not a byte limit.
const BUFFER_THRESHOLD: usize = 100;

static EVENT_TIMEOUT: Duration = Duration::from_millis(200);
static NO_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
static REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to send analytics event.")]
    SendError(#[from] mpsc::error::SendError<AnalyticsEvent>),
    #[error("Failed to record analytics.")]
    Join(#[from] JoinError),
}

pub type AnalyticsSender = mpsc::UnboundedSender<AnalyticsEvent>;

/// The handle on the `Worker` tokio thread, along with a channel
/// to indicate to the thread that it should shut down.
pub struct AnalyticsHandle {
    exit_ch: oneshot::Receiver<()>,
    handle: JoinHandle<()>,
}

/// Starts the `Worker` on a separate tokio thread. Returns an `AnalyticsSender`
/// and an `AnalyticsHandle`.
///
/// We have two different types because the AnalyticsSender should be shared
/// across threads (i.e. Clone + Send), while the AnalyticsHandle cannot be
/// shared since it contains the structs necessary to shut down the worker.
pub fn start_analytics(
    api_auth: APIAuth,
    client: impl AnalyticsClient + Clone + Send + Sync + 'static,
) -> (AnalyticsSender, AnalyticsHandle) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let session_id = Uuid::new_v4();
    let worker = Worker {
        rx,
        buffer: Vec::new(),
        session_id,
        api_auth,
        exit_ch: cancel_tx,
        client,
        requests: JoinSet::new(),
    };
    let handle = worker.start();

    let analytics_handle = AnalyticsHandle {
        exit_ch: cancel_rx,
        handle,
    };

    (tx, analytics_handle)
}

impl AnalyticsHandle {
    async fn close(self) -> Result<(), Error> {
        drop(self.exit_ch);
        self.handle.await?;

        Ok(())
    }

    /// Closes the handle with an explicit timeout. If the handle fails to close
    /// within that timeout, it will log an error and drop the handle.
    #[tracing::instrument(skip_all)]
    pub async fn close_with_timeout(self) {
        if let Err(err) = tokio::time::timeout(EVENT_TIMEOUT, self.close()).await {
            debug!("failed to close analytics handle. error: {}", err)
        }
    }
}

struct Worker<C> {
    rx: mpsc::UnboundedReceiver<AnalyticsEvent>,
    buffer: Vec<AnalyticsEvent>,
    session_id: Uuid,
    api_auth: APIAuth,
    // Used to cancel the worker
    exit_ch: oneshot::Sender<()>,
    client: C,
    requests: JoinSet<()>,
}

impl<C: AnalyticsClient + Clone + Send + Sync + 'static> Worker<C> {
    pub fn start(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut timeout = tokio::time::sleep(NO_TIMEOUT);
            loop {
                select! {
                    // We want the events to be prioritized over closing
                    biased;
                    event = self.rx.recv() => {
                        if let Some(event) = event {
                            self.buffer.push(event);
                        } else {
                            // There are no senders left so we can shut down
                            break;
                        }
                        if self.buffer.len() >= BUFFER_THRESHOLD {
                            self.flush_events();
                            timeout = tokio::time::sleep(NO_TIMEOUT);
                        } else {
                            timeout = tokio::time::sleep(EVENT_TIMEOUT);
                        }
                    }
                    _ = timeout => {
                        self.flush_events();
                        timeout = tokio::time::sleep(NO_TIMEOUT);
                    }
                    _ = self.exit_ch.closed() => {
                        break;
                    }
                }
            }
            // Stop accepting new events, then drain anything already queued before the
            // final flush. Closing a handle does not require dropping every sender.
            self.rx.close();
            while let Some(event) = self.rx.recv().await {
                self.buffer.push(event);
                if self.buffer.len() >= BUFFER_THRESHOLD {
                    self.flush_events();
                }
            }
            self.flush_events();
            // If close_with_timeout expires, dropping the JoinHandle detaches this
            // worker, allowing pending requests to finish in the background.
            while let Some(result) = self.requests.join_next().await {
                if let Err(err) = result {
                    debug!("analytics request task failed: {}", err);
                }
            }
        })
    }

    pub fn flush_events(&mut self) {
        // Reap completed requests during long runs rather than retaining every
        // task in the set until shutdown.
        while let Some(result) = self.requests.try_join_next() {
            if let Err(err) = result {
                debug!("analytics request task failed: {}", err);
            }
        }
        if !self.buffer.is_empty() {
            let events = std::mem::take(&mut self.buffer);
            self.send_events(events);
        }
    }

    fn send_events(&mut self, mut events: Vec<AnalyticsEvent>) {
        let session_id = self.session_id;
        let client = self.client.clone();
        let api_auth = self.api_auth.clone();
        add_session_id(session_id, &mut events);

        self.requests.spawn(async move {
            // We don't log an error for a timeout because
            // that's what the Go code does.
            if let Ok(Err(err)) =
                tokio::time::timeout(REQUEST_TIMEOUT, client.record_analytics(&api_auth, events))
                    .await
            {
                debug!("failed to record cache usage analytics. error: {}", err)
            }
        });
    }
}

fn add_session_id(id: Uuid, events: &mut Vec<AnalyticsEvent>) {
    for event in events {
        event.set_session_id(id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use tokio::{
        select,
        sync::{mpsc, mpsc::UnboundedReceiver},
    };
    use turborepo_api_client::{APIAuth, analytics::AnalyticsClient};
    use turborepo_types::SecretString;
    use turborepo_vercel_api::{AnalyticsEvent, CacheEvent, CacheSource};
    use uuid::Uuid;

    use crate::{BUFFER_THRESHOLD, add_session_id, start_analytics};

    #[derive(Clone)]
    struct DummyClient {
        // A vector that stores each batch of events
        events: Arc<Mutex<RefCell<Vec<Vec<AnalyticsEvent>>>>>,
        tx: mpsc::UnboundedSender<()>,
    }

    impl DummyClient {
        pub fn events(&self) -> Vec<Vec<AnalyticsEvent>> {
            self.events.lock().unwrap().borrow().clone()
        }
    }

    impl AnalyticsClient for DummyClient {
        async fn record_analytics(
            &self,
            _api_auth: &APIAuth,
            events: Vec<AnalyticsEvent>,
        ) -> Result<(), turborepo_api_client::Error> {
            self.events.lock().unwrap().borrow_mut().push(events);
            self.tx.send(()).unwrap();

            Ok(())
        }
    }

    // Asserts that we get the message after the timeout
    async fn expect_timeout_then_message(rx: &mut UnboundedReceiver<()>) {
        let timeout = tokio::time::sleep(std::time::Duration::from_millis(150));

        select! {
            _ = rx.recv() => {
                panic!("Expected to wait out the flush timeout")
            }
            _ = timeout => {
            }
        }

        rx.recv().await;
    }

    // Asserts that we get the message immediately before the timeout
    async fn expected_immediate_message(rx: &mut UnboundedReceiver<()>) {
        let timeout = tokio::time::sleep(std::time::Duration::from_millis(150));

        select! {
            _ = rx.recv() => {
            }
            _ = timeout => {
                panic!("expected to not wait out the flush timeout")
            }
        }
    }

    #[tokio::test]
    async fn test_batching() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        let client = DummyClient {
            events: Default::default(),
            tx,
        };

        let (analytics_sender, analytics_handle) = start_analytics(
            APIAuth {
                token: SecretString::new("foo".to_string()),
                team_id: Some("bar".to_string()),
                team_slug: None,
            },
            client.clone(),
        );

        for _ in 0..2 {
            analytics_sender
                .send(AnalyticsEvent {
                    session_id: None,
                    source: CacheSource::Local,
                    event: CacheEvent::Hit,
                    hash: "".to_string(),
                    duration: 0,
                })
                .unwrap();
        }
        let found = client.events();
        // Should have no events since we haven't flushed yet
        assert_eq!(found.len(), 0);

        expect_timeout_then_message(&mut rx).await;
        let found = client.events();
        assert_eq!(found.len(), 1);
        let payloads = &found[0];
        assert_eq!(payloads.len(), 2);

        drop(analytics_handle);
    }

    #[tokio::test]
    async fn test_batching_across_two_batches() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        let client = DummyClient {
            events: Default::default(),
            tx,
        };

        let (analytics_sender, analytics_handle) = start_analytics(
            APIAuth {
                token: SecretString::new("foo".to_string()),
                team_id: Some("bar".to_string()),
                team_slug: None,
            },
            client.clone(),
        );

        for _ in 0..BUFFER_THRESHOLD + 2 {
            analytics_sender
                .send(AnalyticsEvent {
                    session_id: None,
                    source: CacheSource::Local,
                    event: CacheEvent::Hit,
                    hash: "".to_string(),
                    duration: 0,
                })
                .unwrap();
        }

        expected_immediate_message(&mut rx).await;

        let found = client.events();
        assert_eq!(found.len(), 1);

        let payloads = &found[0];
        assert_eq!(payloads.len(), BUFFER_THRESHOLD);

        expect_timeout_then_message(&mut rx).await;
        let found = client.events();
        assert_eq!(found.len(), 2);

        let payloads = &found[1];
        assert_eq!(payloads.len(), 2);

        drop(analytics_handle);
    }

    #[derive(Clone, Copy)]
    struct SlowClient;

    impl AnalyticsClient for SlowClient {
        async fn record_analytics(
            &self,
            _api_auth: &APIAuth,
            _events: Vec<AnalyticsEvent>,
        ) -> Result<(), turborepo_api_client::Error> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_close_does_not_block_on_slow_client() {
        let (analytics_sender, analytics_handle) = start_analytics(
            APIAuth {
                token: SecretString::new("foo".to_string()),
                team_id: Some("bar".to_string()),
                team_slug: None,
            },
            SlowClient,
        );

        for _ in 0..2 {
            analytics_sender
                .send(AnalyticsEvent {
                    session_id: None,
                    source: CacheSource::Local,
                    event: CacheEvent::Hit,
                    hash: "".to_string(),
                    duration: 0,
                })
                .unwrap();
        }
        drop(analytics_sender);

        // The bounded public close should return even when HTTP is slow.
        tokio::time::timeout(
            Duration::from_secs(1),
            analytics_handle.close_with_timeout(),
        )
        .await
        .expect("bounded close blocked waiting for slow HTTP response");
    }

    #[derive(Clone)]
    struct DelayedClient {
        tx: mpsc::UnboundedSender<()>,
    }

    impl AnalyticsClient for DelayedClient {
        async fn record_analytics(
            &self,
            _api_auth: &APIAuth,
            _events: Vec<AnalyticsEvent>,
        ) -> Result<(), turborepo_api_client::Error> {
            tokio::time::sleep(Duration::from_millis(50)).await;
            self.tx.send(()).unwrap();
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_close_waits_for_final_request() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (analytics_sender, analytics_handle) = start_analytics(
            APIAuth {
                token: SecretString::new("foo".to_string()),
                team_id: Some("bar".to_string()),
                team_slug: None,
            },
            DelayedClient { tx },
        );
        analytics_sender
            .send(AnalyticsEvent {
                session_id: None,
                source: CacheSource::Local,
                event: CacheEvent::Hit,
                hash: "final".to_string(),
                duration: 0,
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), analytics_handle.close())
            .await
            .expect("timeout before close")
            .expect("analytics worker panicked");
        assert_eq!(rx.try_recv(), Ok(()));
    }

    #[tokio::test]
    async fn test_closing() {
        let (tx, _rx) = mpsc::unbounded_channel();

        let client = DummyClient {
            events: Default::default(),
            tx,
        };

        let (analytics_sender, analytics_handle) = start_analytics(
            APIAuth {
                token: SecretString::new("foo".to_string()),
                team_id: Some("bar".to_string()),
                team_slug: None,
            },
            client.clone(),
        );

        for _ in 0..2 {
            analytics_sender
                .send(AnalyticsEvent {
                    session_id: None,
                    source: CacheSource::Local,
                    event: CacheEvent::Hit,
                    hash: "".to_string(),
                    duration: 0,
                })
                .unwrap();
        }
        drop(analytics_sender);

        let found = client.events();
        assert!(found.is_empty());

        tokio::time::timeout(Duration::from_secs(1), analytics_handle.close())
            .await
            .expect("timeout before close")
            .expect("analytics worker panicked");
        let found = client.events();
        assert_eq!(found.len(), 1);
        let payloads = &found[0];
        assert_eq!(payloads.len(), 2);
    }

    #[tokio::test]
    async fn test_close_drains_queued_events_with_sender_alive() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = DummyClient {
            events: Default::default(),
            tx,
        };
        let (analytics_sender, analytics_handle) = start_analytics(
            APIAuth {
                token: SecretString::new("foo".to_string()),
                team_id: Some("bar".to_string()),
                team_slug: None,
            },
            client.clone(),
        );

        let count = BUFFER_THRESHOLD * 2 + 3;
        for index in 0..count {
            analytics_sender
                .send(AnalyticsEvent {
                    session_id: None,
                    source: CacheSource::Local,
                    event: CacheEvent::Hit,
                    hash: index.to_string(),
                    duration: 0,
                })
                .unwrap();
        }
        // Don't drop the sender: closing must still drain the channel and wait
        // for all the requests, including the final partial batch.
        tokio::time::timeout(Duration::from_secs(1), analytics_handle.close())
            .await
            .expect("timeout before close")
            .expect("analytics worker panicked");

        let batches = client.events();
        assert!(batches.iter().all(|batch| batch.len() <= BUFFER_THRESHOLD));
        let mut hashes: Vec<_> = batches
            .iter()
            .flat_map(|batch| batch.iter().map(|event| event.hash.clone()))
            .collect();
        hashes.sort();
        let mut expected: Vec<_> = (0..count).map(|index| index.to_string()).collect();
        expected.sort();
        assert_eq!(hashes, expected);
        assert!(
            batches
                .iter()
                .flatten()
                .all(|event| event.session_id.is_some())
        );
        assert!(analytics_sender.is_closed());
    }

    #[tokio::test]
    async fn test_client_error_handling() {
        let client = ErrorClient;

        let (analytics_sender, analytics_handle) = start_analytics(
            APIAuth {
                token: SecretString::new("foo".to_string()),
                team_id: Some("bar".to_string()),
                team_slug: None,
            },
            client,
        );

        // Send an event that will cause the client to error
        assert!(
            analytics_sender
                .send(AnalyticsEvent {
                    session_id: None,
                    source: CacheSource::Local,
                    event: CacheEvent::Hit,
                    hash: "".to_string(),
                    duration: 0,
                })
                .is_ok(),
            "sender should swallow client errors"
        );

        // Wait a bit for the error to be handled
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should not panic when closing
        analytics_handle.close_with_timeout().await;
    }

    #[test]
    fn test_add_session_id() {
        let mut events = vec![
            AnalyticsEvent {
                session_id: None,
                source: CacheSource::Local,
                event: CacheEvent::Hit,
                hash: "hash1".to_string(),
                duration: 100,
            },
            AnalyticsEvent {
                session_id: Some("existing".to_string()),
                source: CacheSource::Remote,
                event: CacheEvent::Miss,
                hash: "hash2".to_string(),
                duration: 200,
            },
        ];

        let session_id = Uuid::new_v4();
        add_session_id(session_id, &mut events);

        assert_eq!(events[0].session_id, Some(session_id.to_string()));
        assert_eq!(events[1].session_id, Some(session_id.to_string()));
    }

    #[derive(Clone, Copy)]
    struct ErrorClient;

    impl AnalyticsClient for ErrorClient {
        async fn record_analytics(
            &self,
            _api_auth: &APIAuth,
            _events: Vec<AnalyticsEvent>,
        ) -> Result<(), turborepo_api_client::Error> {
            // Simulate an error using a simple error type
            Err(turborepo_api_client::Error::ReadError(
                std::io::Error::other("test error"),
            ))
        }
    }
}
