//! Turborepo's telemetry library. Handles sending anonymous telemetry events to
//! the Vercel API in the background.
//!
//! More detail is available at https://turborepo.com/docs/telemetry.

#![feature(error_generic_member_access)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]

pub mod config;
pub mod errors;
pub mod events;

use std::time::Duration;

use config::{ConfigError, TelemetryConfig};
use events::TelemetryEvent;
use futures::{StreamExt, stream::FuturesUnordered};
use once_cell::sync::OnceCell;
use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::{JoinError, JoinHandle},
};
use tracing::{debug, trace};
use turborepo_api_client::telemetry;
use turborepo_ui::{BOLD, ColorConfig, GREY, color};
use uuid::Uuid;

const BUFFER_THRESHOLD: usize = 10;

static EVENT_TIMEOUT: Duration = Duration::from_millis(1000);
static NO_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
static REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to initialize telemetry.")]
    InitError(#[from] ConfigError),
    #[error("Failed to send telemetry event.")]
    SendError(#[from] mpsc::error::SendError<TelemetryEvent>),
    #[error("Failed to record telemetry.")]
    Join(#[from] JoinError),
    #[error("Telemetry already initialized.")]
    AlreadyInitialized(),
}

pub type TelemetrySender = mpsc::UnboundedSender<TelemetryEvent>;

/// The handle on the `Worker` tokio thread, along with a channel
/// to indicate to the thread that it should shut down.
pub struct TelemetryHandle {
    exit_ch: oneshot::Receiver<()>,
    handle: JoinHandle<()>,
}

static SENDER_INSTANCE: OnceCell<TelemetrySender> = OnceCell::new();

// A global instance of the TelemetrySender.
pub fn telem(event: events::TelemetryEvent) {
    let sender = SENDER_INSTANCE.get();
    match sender {
        Some(s) => {
            let result = s.send(event);
            if let Err(err) = result {
                debug!("failed to send telemetry event. error: {}", err)
            }
        }
        None => {
            // If we're in debug mode - log a warning
            if cfg!(debug_assertions) && !cfg!(test) {
                println!("\n[DEVELOPMENT ERROR] telemetry sender not initialized\n");
            }
            debug!("telemetry sender not initialized");
        }
    }
}

fn init(
    mut config: TelemetryConfig,
    client: impl telemetry::TelemetryClient + Clone + Send + Sync + 'static,
    color_config: ColorConfig,
) -> Result<(TelemetryHandle, TelemetrySender), Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    config.show_alert(color_config);

    let session_id = Uuid::new_v4();
    let worker = Worker {
        rx,
        buffer: Vec::new(),
        senders: FuturesUnordered::new(),
        exit_ch: cancel_tx,
        client,
        session_id: session_id.to_string(),
        telemetry_id: config.get_id().to_string(),
        enabled: config.is_enabled(),
        color_config,
    };
    let handle = worker.start();

    let telemetry_handle = TelemetryHandle {
        exit_ch: cancel_rx,
        handle,
    };

    // return
    Ok((telemetry_handle, tx))
}

/// Starts the `Worker` on a separate tokio thread. Returns an `TelemetrySender`
/// and an `TelemetryHandle`.
///
/// We have two different types because the TelemetrySender should be shared
/// across threads (i.e. Clone + Send), while the TelemetryHandle cannot be
/// shared since it contains the structs necessary to shut down the worker.
pub fn init_telemetry(
    client: impl telemetry::TelemetryClient + Clone + Send + Sync + 'static,
    color_config: ColorConfig,
) -> Result<TelemetryHandle, Box<dyn std::error::Error>> {
    // make sure we're not already initialized
    if SENDER_INSTANCE.get().is_some() {
        debug!("telemetry already initialized");
        return Err(Box::new(Error::AlreadyInitialized()));
    }
    let config = TelemetryConfig::with_default_config_path()?;
    let (handle, sender) = init(config, client, color_config)?;
    SENDER_INSTANCE.set(sender).unwrap();
    Ok(handle)
}

impl TelemetryHandle {
    async fn close(self) -> Result<(), Error> {
        drop(self.exit_ch);
        self.handle.await?;

        Ok(())
    }

    /// Closes the handle with an explicit timeout. If the handle fails to close
    /// within that timeout, it will log an error and drop the handle.
    pub async fn close_with_timeout(self) {
        if let Err(err) = tokio::time::timeout(EVENT_TIMEOUT, self.close()).await {
            debug!("failed to close telemetry handle. error: {}", err)
        } else {
            debug!("telemetry handle closed")
        }
    }
}

struct Worker<C> {
    rx: mpsc::UnboundedReceiver<TelemetryEvent>,
    buffer: Vec<TelemetryEvent>,
    senders: FuturesUnordered<JoinHandle<()>>,
    // Used to cancel the worker
    exit_ch: oneshot::Sender<()>,
    client: C,
    telemetry_id: String,
    session_id: String,
    enabled: bool,
    color_config: ColorConfig,
}

impl<C: telemetry::TelemetryClient + Clone + Send + Sync + 'static> Worker<C> {
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
                        if self.buffer.len() == BUFFER_THRESHOLD {
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
            self.flush_events();
            while let Some(result) = self.senders.next().await {
                if let Err(err) = result {
                    debug!("failed to send telemetry event. error: {}", err)
                }
            }
        })
    }

    pub fn flush_events(&mut self) {
        if !self.buffer.is_empty() {
            let events = std::mem::take(&mut self.buffer);
            let num_events = events.len();
            let handle = self.send_events(events);
            if let Some(handle) = handle {
                self.senders.push(handle);
            }
            trace!(
                "Flushed telemetry event queue (num_events={:?})",
                num_events
            );
        }
    }

    fn send_events(&self, events: Vec<TelemetryEvent>) -> Option<JoinHandle<()>> {
        if !self.enabled {
            return None;
        }

        if config::is_debug() {
            for event in &events {
                let pretty_event = serde_json::to_string_pretty(&event)
                    .unwrap_or("Error serializing event".to_string());
                println!(
                    "\n{}\n{}\n",
                    color!(self.color_config, BOLD, "{}", "[telemetry event]"),
                    color!(self.color_config, GREY, "{}", pretty_event)
                );
            }
        }

        let client = self.client.clone();
        let session_id = self.session_id.clone();
        let telemetry_id = self.telemetry_id.clone();
        Some(tokio::spawn(async move {
            if let Ok(Err(err)) = tokio::time::timeout(
                REQUEST_TIMEOUT,
                client.record_telemetry(events, telemetry_id.as_str(), session_id.as_str()),
            )
            .await
            {
                debug!("failed to record cache usage telemetry. error: {}", err)
            }
        }))
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
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_api_client::telemetry::TelemetryClient;
    use turborepo_ui::ColorConfig;
    use turborepo_vercel_api::telemetry::{TelemetryEvent, TelemetryGenericEvent};

    use crate::{config::TelemetryConfig, init};

    #[derive(Clone)]
    struct DummyClient {
        // A vector that stores each batch of events
        events: Arc<Mutex<RefCell<Vec<Vec<TelemetryEvent>>>>>,
        tx: mpsc::UnboundedSender<()>,
    }

    impl DummyClient {
        pub fn events(&self) -> Vec<Vec<TelemetryEvent>> {
            self.events.lock().unwrap().borrow().clone()
        }
    }

    impl TelemetryClient for DummyClient {
        async fn record_telemetry(
            &self,
            events: Vec<TelemetryEvent>,
            _telemetry_id: &str,
            _session_id: &str,
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

    fn temp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        (temp_dir, path)
    }

    #[tokio::test]
    async fn test_batching() {
        let (_tmp, temp_dir) = temp_dir();
        let config =
            TelemetryConfig::new(temp_dir.join_components(&["turborepo", "telemetry.json"]))
                .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();

        let client = DummyClient {
            events: Default::default(),
            tx,
        };

        let result = init(config, client.clone(), ColorConfig::new(false));

        let (telemetry_handle, telemetry_sender) = result.unwrap();

        for _ in 0..2 {
            telemetry_sender
                .send(TelemetryEvent::Generic(TelemetryGenericEvent {
                    id: "id".to_string(),
                    key: "key".to_string(),
                    value: "value".to_string(),
                    parent_id: None,
                }))
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

        drop(telemetry_handle);
    }

    #[tokio::test]
    async fn test_batching_across_two_batches() {
        let (_tmp, temp_dir) = temp_dir();
        let config =
            TelemetryConfig::new(temp_dir.join_components(&["turborepo", "telemetry.json"]))
                .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();

        let client = DummyClient {
            events: Default::default(),
            tx,
        };

        let result = init(config, client.clone(), ColorConfig::new(false));

        let (telemetry_handle, telemetry_sender) = result.unwrap();

        for _ in 0..12 {
            telemetry_sender
                .send(TelemetryEvent::Generic(TelemetryGenericEvent {
                    id: "id".to_string(),
                    key: "key".to_string(),
                    value: "value".to_string(),
                    parent_id: None,
                }))
                .unwrap();
        }

        expected_immediate_message(&mut rx).await;

        let found = client.events();
        assert_eq!(found.len(), 1);

        let payloads = &found[0];
        assert_eq!(payloads.len(), 10);

        expect_timeout_then_message(&mut rx).await;
        let found = client.events();
        assert_eq!(found.len(), 2);

        let payloads = &found[1];
        assert_eq!(payloads.len(), 2);

        drop(telemetry_handle);
    }

    #[tokio::test]
    async fn test_closing() {
        let (_tmp, temp_dir) = temp_dir();
        let config =
            TelemetryConfig::new(temp_dir.join_components(&["turborepo", "telemetry.json"]))
                .unwrap();
        let (tx, mut _rx) = mpsc::unbounded_channel();

        let client = DummyClient {
            events: Default::default(),
            tx,
        };

        let result = init(config, client.clone(), ColorConfig::new(false));

        let (telemetry_handle, telemetry_sender) = result.unwrap();

        for _ in 0..2 {
            telemetry_sender
                .send(TelemetryEvent::Generic(TelemetryGenericEvent {
                    id: "id".to_string(),
                    key: "key".to_string(),
                    value: "value".to_string(),
                    parent_id: None,
                }))
                .unwrap();
        }
        drop(telemetry_sender);

        let found = client.events();
        assert!(found.is_empty());

        tokio::time::timeout(Duration::from_millis(5), telemetry_handle.close())
            .await
            .expect("timeout before close")
            .expect("analytics worker panicked");
        let found = client.events();
        assert_eq!(found.len(), 1);
        let payloads = &found[0];
        assert_eq!(payloads.len(), 2);
    }
}
