//! Turborepo's telemetry library. Handles sending anonymous telemetry events to
//! the Vercel API in the background.
//!
//! More detail is available at https://turborepo.dev/docs/telemetry.

#![feature(error_generic_member_access)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod config;
pub mod errors;
pub mod events;

use std::{sync::OnceLock, time::Duration};

use config::{ConfigError, TelemetryConfig};
use events::TelemetryEvent;
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
    SendError(#[from] Box<mpsc::error::SendError<TelemetryEvent>>),
    #[error("Failed to record telemetry.")]
    Join(#[from] JoinError),
    #[error("Telemetry already initialized.")]
    AlreadyInitialized(),
}

pub type TelemetrySender = mpsc::UnboundedSender<TelemetryEvent>;

/// The handle on the `Worker` tokio thread, along with a channel
/// to indicate to the thread that it should shut down. Inert when disabled.
pub struct TelemetryHandle {
    worker: Option<WorkerHandle>,
}

struct WorkerHandle {
    exit_ch: oneshot::Receiver<()>,
    handle: JoinHandle<()>,
}

// None means initialization has not happened, not that the user opted out.
enum TelemetryState {
    Disabled,
    Enabled(TelemetrySender),
}

static TELEMETRY_STATE: OnceLock<TelemetryState> = OnceLock::new();

// Only explicit opt-out makes builders inert. Before initialization, preserve
// event construction and the missing-initialization diagnostic in `telem`.
fn is_disabled() -> bool {
    matches!(TELEMETRY_STATE.get(), Some(TelemetryState::Disabled))
}

// A global instance of the TelemetrySender.
pub fn telem(event: events::TelemetryEvent) {
    match TELEMETRY_STATE.get() {
        Some(TelemetryState::Disabled) => {}
        Some(TelemetryState::Enabled(sender)) => {
            let result = sender.send(event);
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
) -> Result<(TelemetryHandle, TelemetrySender, bool), Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::unbounded_channel();
    if !config.is_enabled() {
        return Ok((TelemetryHandle { worker: None }, tx, false));
    }
    let (cancel_tx, cancel_rx) = oneshot::channel();
    config.show_alert(color_config);
    let enabled = true;

    let session_id = Uuid::new_v4();
    let worker = Worker {
        rx,
        buffer: Vec::new(),
        exit_ch: cancel_tx,
        client,
        session_id: session_id.to_string(),
        telemetry_id: config.get_id().to_string(),
        enabled,
        color_config,
    };
    let handle = worker.start();

    let telemetry_handle = TelemetryHandle {
        worker: Some(WorkerHandle {
            exit_ch: cancel_rx,
            handle,
        }),
    };

    // return
    Ok((telemetry_handle, tx, enabled))
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
) -> Result<(TelemetryHandle, bool), Box<dyn std::error::Error>> {
    init_with_state(
        &TELEMETRY_STATE,
        client,
        color_config,
        TelemetryConfig::with_default_config_path,
    )
}

fn init_with_state(
    state: &OnceLock<TelemetryState>,
    client: impl telemetry::TelemetryClient + Clone + Send + Sync + 'static,
    color_config: ColorConfig,
    load_config: impl FnOnce() -> Result<TelemetryConfig, ConfigError>,
) -> Result<(TelemetryHandle, bool), Box<dyn std::error::Error>> {
    if state.get().is_some() {
        debug!("telemetry already initialized");
        return Err(Box::new(Error::AlreadyInitialized()));
    }

    let (handle, new_state, enabled) = if config::is_disabled_by_env() {
        (
            TelemetryHandle { worker: None },
            TelemetryState::Disabled,
            false,
        )
    } else {
        let (handle, sender, enabled) = init(load_config()?, client, color_config)?;
        let new_state = if enabled {
            TelemetryState::Enabled(sender)
        } else {
            TelemetryState::Disabled
        };
        (handle, new_state, enabled)
    };
    state
        .set(new_state)
        .map_err(|_| Box::new(Error::AlreadyInitialized()) as Box<dyn std::error::Error>)?;
    Ok((handle, enabled))
}

impl TelemetryHandle {
    async fn close(self) -> Result<(), Error> {
        if let Some(worker) = self.worker {
            drop(worker.exit_ch);
            worker.handle.await?;
        }

        Ok(())
    }

    /// Closes the handle with an explicit timeout. If the handle fails to close
    /// within that timeout, it will log an error and drop the handle.
    pub async fn close_with_timeout(self) {
        if self.worker.is_none() {
            return;
        }
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
        })
    }

    pub fn flush_events(&mut self) {
        if !self.buffer.is_empty() {
            let events = std::mem::take(&mut self.buffer);
            let num_events = events.len();
            self.send_events(events);
            trace!(
                "Flushed telemetry event queue (num_events={:?})",
                num_events
            );
        }
    }

    fn send_events(&self, events: Vec<TelemetryEvent>) {
        if !self.enabled {
            return;
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
        tokio::spawn(async move {
            if let Ok(Err(err)) = tokio::time::timeout(
                REQUEST_TIMEOUT,
                client.record_telemetry(events, telemetry_id.as_str(), session_id.as_str()),
            )
            .await
            {
                debug!("failed to record cache usage telemetry. error: {}", err)
            }
        });
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

    // Each scenario gets its own process so environment variables and the global
    // OnceLock cannot race with other tests.
    #[test]
    fn test_telemetry_state() {
        use std::{fmt, process::Command};

        use crate::{
            TELEMETRY_STATE, TelemetryState,
            events::{
                EventBuilder, EventType, Identifiable, command::CommandEventBuilder,
                generic::GenericEventBuilder, repo::RepoEventBuilder,
                task::PackageTaskEventBuilder,
            },
            init_telemetry,
        };

        let Ok(scenario) = std::env::var("TURBO_TELEMETRY_TEST_SCENARIO") else {
            for scenario in [
                "uninitialized",
                "failed-init",
                "config",
                "enabled",
                "dnt-1",
                "dnt-true",
                "turbo-1",
                "turbo-true",
            ] {
                let (_tmp, dir) = temp_dir();
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args(["--exact", "tests::test_telemetry_state", "--nocapture"])
                    .env_remove("DO_NOT_TRACK")
                    .env_remove("TURBO_TELEMETRY_DISABLED")
                    .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
                    .env("TURBO_CONFIG_DIR_PATH", dir.as_str())
                    .env("TURBO_TELEMETRY_TEST_SCENARIO", scenario);
                if let Some(value) = scenario.strip_prefix("dnt-") {
                    command.env("DO_NOT_TRACK", value);
                } else if let Some(value) = scenario.strip_prefix("turbo-") {
                    command.env("TURBO_TELEMETRY_DISABLED", value);
                }
                let output = command.output().unwrap();
                assert!(output.status.success(), "{scenario}: {output:?}");
            }
            return;
        };

        let config_path =
            AbsoluteSystemPathBuf::new(std::env::var("TURBO_CONFIG_DIR_PATH").unwrap())
                .unwrap()
                .join_components(&["turborepo", "telemetry.json"]);
        if scenario == "uninitialized" {
            assert!(TELEMETRY_STATE.get().is_none());
            assert!(!crate::is_disabled());
            assert!(!GenericEventBuilder::new().get_id().is_empty());
            return;
        }
        if scenario == "failed-init" {
            let state = std::sync::OnceLock::new();
            for _ in 0..2 {
                let result =
                    super::init_with_state(&state, SlowClient, ColorConfig::new(false), || {
                        Err(crate::config::ConfigError::Message("test failure".into()))
                    });
                assert!(result.is_err());
                assert!(state.get().is_none());
            }
            return;
        }
        if scenario == "config" {
            TelemetryConfig::new(config_path.clone())
                .unwrap()
                .disable()
                .unwrap();
        }
        let runtime = (scenario == "enabled").then(|| tokio::runtime::Runtime::new().unwrap());
        let _guard = runtime.as_ref().map(|runtime| runtime.enter());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = DummyClient {
            events: Default::default(),
            tx,
        };
        let (handle, enabled) = init_telemetry(client.clone(), ColorConfig::new(false)).unwrap();
        assert_eq!(enabled, scenario == "enabled");
        assert_eq!(handle.worker.is_some(), enabled);
        assert!(init_telemetry(client.clone(), ColorConfig::new(false)).is_err());

        if enabled {
            assert!(matches!(
                TELEMETRY_STATE.get(),
                Some(TelemetryState::Enabled(_))
            ));
            let parent = GenericEventBuilder::new();
            let task = PackageTaskEventBuilder::new("package", "build").with_parent(&parent);
            assert!(!task.get_id().is_empty());
            task.track_env_mode("strict");
            task.child().track_env_mode("loose");
            runtime.as_ref().unwrap().block_on(handle.close()).unwrap();
            runtime.as_ref().unwrap().block_on(async {
                tokio::time::timeout(Duration::from_secs(1), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
            });
            let events = client.events();
            assert_eq!(events.len(), 1);
            let [TelemetryEvent::Task(first), TelemetryEvent::Task(child)] = events[0].as_slice()
            else {
                panic!("expected two task events");
            };
            assert_eq!(first.parent_id.as_ref(), Some(parent.get_id()));
            assert_eq!(child.parent_id.as_ref(), Some(task.get_id()));
            assert_eq!(first.package, child.package);
            assert_eq!(first.task, "build");
            assert_eq!(child.task, "build");
            assert_eq!(first.key, "env_mode");
            assert_eq!(first.value, "strict");
            assert_eq!(child.value, "loose");
            // The existing process cache must survive removal of the config.
            let hash = TelemetryConfig::one_way_hash("sensitive");
            config_path.remove_file().unwrap();
            assert_eq!(hash, TelemetryConfig::one_way_hash("sensitive"));
            assert!(!config_path.exists());
            return;
        }

        assert!(matches!(
            TELEMETRY_STATE.get(),
            Some(TelemetryState::Disabled)
        ));
        struct MustNotFormat;
        impl fmt::Display for MustNotFormat {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("disabled telemetry formatted an event");
            }
        }
        let generic = GenericEventBuilder::new();
        let command = CommandEventBuilder::new("run").with_parent(&generic);
        let repo = RepoEventBuilder::new("private-repo").with_parent(&generic);
        let task =
            PackageTaskEventBuilder::new("private-package", "private-task").with_parent(&repo);
        generic.track_arg_value("test", MustNotFormat, EventType::Sensitive);
        command.track_arg_value("test", MustNotFormat, EventType::Sensitive);
        command.track_ui_mode(MustNotFormat);
        repo.track_size(2);
        task.track_env_mode("strict");
        assert!(generic.child().get_id().is_empty());
        assert!(command.child().get_id().is_empty());
        assert!(repo.child().get_id().is_empty());
        assert!(task.child().get_id().is_empty());
        // Direct events are also silently ignored, not queued.
        crate::telem(TelemetryEvent::Generic(TelemetryGenericEvent {
            id: String::new(),
            parent_id: None,
            key: String::new(),
            value: String::new(),
        }));
        futures::executor::block_on(handle.close_with_timeout());
        assert!(client.events().is_empty());
        if scenario != "config" {
            assert!(!config_path.exists(), "environment opt-out touched config");
        }
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

        let (telemetry_handle, telemetry_sender, _) = result.unwrap();

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

        let (telemetry_handle, telemetry_sender, _) = result.unwrap();

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

    #[derive(Clone)]
    struct SlowClient;

    impl TelemetryClient for SlowClient {
        async fn record_telemetry(
            &self,
            _events: Vec<TelemetryEvent>,
            _telemetry_id: &str,
            _session_id: &str,
        ) -> Result<(), turborepo_api_client::Error> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_close_does_not_block_on_slow_client() {
        let (_tmp, temp_dir) = temp_dir();
        let config =
            TelemetryConfig::new(temp_dir.join_components(&["turborepo", "telemetry.json"]))
                .unwrap();

        let result = init(config, SlowClient, ColorConfig::new(false));
        let (telemetry_handle, telemetry_sender, _) = result.unwrap();

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

        // close() should return near-instantly even though the client takes 5s
        tokio::time::timeout(Duration::from_millis(200), telemetry_handle.close())
            .await
            .expect("close() blocked waiting for slow HTTP response")
            .expect("worker panicked");
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

        let (telemetry_handle, telemetry_sender, _) = result.unwrap();

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
