//! Turborepo's telemetry library. Handles sending anonymous telemetry events to
//! the Vercel API in the background.
//!
//! More detail is available at https://turbo.build/repo/docs/telemetry.

#![feature(error_generic_member_access)]

pub mod config;
pub mod errors;
pub mod events;

use std::time::Duration;

use config::{ConfigError, TelemetryConfig};
use events::TelemetryEvent;
use futures::{stream::FuturesUnordered, StreamExt};
use once_cell::sync::OnceCell;
use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::{JoinError, JoinHandle},
};
use tracing::{debug, error};
use turborepo_api_client::telemetry;
use turborepo_ui::{color, BOLD, GREY, UI};
use uuid::Uuid;

const BUFFER_THRESHOLD: usize = 10;

static EVENT_TIMEOUT: Duration = Duration::from_millis(1000);
static NO_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
static REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to initialize telemetry")]
    InitError(#[from] ConfigError),
    #[error("Failed to send telemetry event")]
    SendError(#[from] mpsc::error::SendError<TelemetryEvent>),
    #[error("Failed to record telemetry")]
    Join(#[from] JoinError),
    #[error("Telemetry already initialized")]
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
            if cfg!(debug_assertions) {
                panic!("[DEVELOPMENT ERROR] telemetry sender not initialized");
            }
            debug!("telemetry sender not initialized");
        }
    }
}

/// Starts the `Worker` on a separate tokio thread. Returns an `TelemetrySender`
/// and an `TelemetryHandle`.
///
/// We have two different types because the TelemetrySender should be shared
/// across threads (i.e. Clone + Send), while the TelemetryHandle cannot be
/// shared since it contains the structs necessary to shut down the worker.
pub fn init_telemetry(
    client: impl telemetry::TelemetryClient + Clone + Send + Sync + 'static,
    ui: UI,
) -> Result<TelemetryHandle, Error> {
    // make sure we're not already initialized
    if SENDER_INSTANCE.get().is_some() {
        debug!("telemetry already initialized");
        return Err(Error::AlreadyInitialized());
    }

    let (tx, rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let mut config = TelemetryConfig::new(ui)?;
    config.show_alert();

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
        ui,
    };
    let handle = worker.start();

    let telemetry_handle = TelemetryHandle {
        exit_ch: cancel_rx,
        handle,
    };

    // track the sender as global to avoid passing it around
    SENDER_INSTANCE.set(tx).unwrap();

    // return
    Ok(telemetry_handle)
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
    ui: UI,
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
            debug!(
                "Starting telemetry event queue flush (num_events={:?})",
                events.len()
            );
            let handle = self.send_events(events);
            if let Some(handle) = handle {
                self.senders.push(handle);
            }
            debug!("Done telemetry event queue flush");
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
                    color!(self.ui, BOLD, "{}", "[telemetry event]"),
                    color!(self.ui, GREY, "{}", pretty_event)
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
