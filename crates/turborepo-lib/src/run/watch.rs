use std::{
    collections::HashSet,
    ops::DerefMut as _,
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tokio::{select, sync::Notify, task::JoinHandle};
use tracing::{instrument, trace, warn};
use turborepo_repository::package_graph::PackageName;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::sender::UISender;

use crate::{
    commands::CommandBase,
    daemon::{proto, DaemonConnectorError, DaemonError},
    get_version, opts,
    run::{self, builder::RunBuilder, scope::target_selector::InvalidSelectorError, Run},
    turbo_json::{CONFIG_FILE, CONFIG_FILE_JSONC},
    DaemonConnector, DaemonPaths,
};

#[derive(Debug)]
enum ChangedPackages {
    All,
    Some(HashSet<PackageName>),
}

impl Default for ChangedPackages {
    fn default() -> Self {
        ChangedPackages::Some(HashSet::new())
    }
}

impl ChangedPackages {
    pub fn is_empty(&self) -> bool {
        match self {
            ChangedPackages::All => false,
            ChangedPackages::Some(pkgs) => pkgs.is_empty(),
        }
    }
}

pub struct WatchClient {
    run: Arc<Run>,
    watched_packages: HashSet<PackageName>,
    persistent_tasks_handle: Option<RunHandle>,
    connector: DaemonConnector,
    base: CommandBase,
    telemetry: CommandEventBuilder,
    handler: SignalHandler,
    ui_sender: Option<UISender>,
    ui_handle: Option<JoinHandle<Result<(), turborepo_ui::Error>>>,
    experimental_write_cache: bool,
}

struct RunHandle {
    stopper: run::RunStopper,
    run_task: JoinHandle<Result<i32, run::Error>>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Failed to connect to daemon.")]
    #[diagnostic(transparent)]
    Daemon(#[from] DaemonError),
    #[error("Failed to connect to daemon.")]
    DaemonConnector(#[from] DaemonConnectorError),
    #[error("Failed to decode message from daemon.")]
    Decode(#[from] prost::DecodeError),
    #[error("Could not get current executable.")]
    CurrentExe(std::io::Error),
    #[error("Could not start `turbo`.")]
    Start(std::io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Run(#[from] run::Error),
    #[error("`--since` is not supported in Watch Mode.")]
    SinceNotSupported,
    #[error(transparent)]
    Opts(#[from] opts::Error),
    #[error("Invalid filter pattern")]
    InvalidSelector(#[from] InvalidSelectorError),
    #[error("Filter cannot contain a git range in Watch Mode.")]
    GitRangeInFilter {
        #[source_code]
        filter: String,
        #[label]
        span: SourceSpan,
    },
    #[error("Daemon connection closed.")]
    ConnectionClosed,
    #[error("Failed to subscribe to signal handler. Shutting down.")]
    NoSignalHandler,
    #[error("Watch interrupted due to signal.")]
    SignalInterrupt,
    #[error("Package change error.")]
    PackageChange(#[from] tonic::Status),
    #[error(transparent)]
    UI(#[from] turborepo_ui::Error),
    #[error("Could not connect to UI thread: {0}")]
    UISend(String),
    #[error("Cannot use non-standard turbo configuration at {0} with Watch Mode.")]
    NonStandardTurboJsonPath(String),
    #[error("Invalid config: {0}")]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    SignalListener(#[from] turborepo_signals::listeners::Error),
}

impl WatchClient {
    pub async fn new(
        base: CommandBase,
        experimental_write_cache: bool,
        telemetry: CommandEventBuilder,
    ) -> Result<Self, Error> {
        let signal = get_signal()?;
        let handler = SignalHandler::new(signal);

        // Check if the turbo.json path is the standard one (either turbo.json or
        // turbo.jsonc)
        let standard_path_json = base.repo_root.join_component(CONFIG_FILE);
        let standard_path_jsonc = base.repo_root.join_component(CONFIG_FILE_JSONC);

        if base.opts.repo_opts.root_turbo_json_path != standard_path_json
            && base.opts.repo_opts.root_turbo_json_path != standard_path_jsonc
        {
            return Err(Error::NonStandardTurboJsonPath(
                base.opts.repo_opts.root_turbo_json_path.to_string(),
            ));
        }

        if matches!(base.opts.run_opts.daemon, Some(false)) {
            warn!("daemon is required for watch, ignoring request to disable daemon");
        }

        let new_base = base.clone();
        let run = Arc::new(
            RunBuilder::new(new_base)?
                .build(&handler, telemetry.clone())
                .await?,
        );

        let watched_packages = run.get_relevant_packages();

        let (ui_sender, ui_handle) = run.start_ui()?.unzip();

        let connector = DaemonConnector {
            can_start_server: true,
            can_kill_server: true,
            paths: DaemonPaths::from_repo_root(&base.repo_root),
        };

        Ok(Self {
            base,
            run,
            watched_packages,
            connector,
            handler,
            telemetry,
            experimental_write_cache,
            persistent_tasks_handle: None,
            ui_sender,
            ui_handle,
        })
    }

    pub async fn start(&mut self) -> Result<(), Error> {
        let connector = self.connector.clone();
        let mut client = connector.connect().await?;

        let mut events = client.package_changes().await?;

        let signal_subscriber = self.handler.subscribe().ok_or(Error::NoSignalHandler)?;

        // We explicitly use a tokio::sync::Mutex here to avoid deadlocks.
        // If we used a std::sync::Mutex, we could deadlock by spinning the lock
        // and not yielding back to the tokio runtime.
        let changed_packages = Mutex::new(ChangedPackages::default());
        let notify_run = Arc::new(Notify::new());
        let notify_event = notify_run.clone();

        let event_fut = async {
            while let Some(event) = events.next().await {
                let event = event?;
                Self::handle_change_event(&changed_packages, event.event.unwrap())?;
                notify_event.notify_one();
            }

            Err(Error::ConnectionClosed)
        };

        let run_fut = async {
            let mut run_handle: Option<RunHandle> = None;
            loop {
                notify_run.notified().await;
                let some_changed_packages = {
                    let mut changed_packages_guard =
                        changed_packages.lock().expect("poisoned lock");
                    (!changed_packages_guard.is_empty())
                        .then(|| std::mem::take(changed_packages_guard.deref_mut()))
                };

                if let Some(changed_packages) = some_changed_packages {
                    // Clean up currently running tasks
                    if let Some(RunHandle { stopper, run_task }) = run_handle.take() {
                        // Shut down the tasks for the run
                        stopper.stop().await;
                        // Run should exit shortly after we stop all child tasks, wait for it to
                        // finish to ensure all messages are flushed.
                        let _ = run_task.await;
                    }
                    run_handle = Some(self.execute_run(changed_packages).await?);
                }
            }
        };

        select! {
            biased;
            _ = signal_subscriber.listen() => {
                tracing::info!("shutting down");
                Err(Error::SignalInterrupt)
            }
            result = event_fut => {
                result
            }
            run_result = run_fut => {
                run_result
            }
        }
    }

    #[instrument(skip(changed_packages))]
    fn handle_change_event(
        changed_packages: &Mutex<ChangedPackages>,
        event: proto::package_change_event::Event,
    ) -> Result<(), Error> {
        // Should we recover here?
        match event {
            proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
                package_name,
            }) => {
                let package_name = PackageName::from(package_name);

                match changed_packages.lock().expect("poisoned lock").deref_mut() {
                    ChangedPackages::All => {
                        // If we've already changed all packages, ignore
                    }
                    ChangedPackages::Some(ref mut pkgs) => {
                        pkgs.insert(package_name);
                    }
                }
            }
            proto::package_change_event::Event::RediscoverPackages(_) => {
                *changed_packages.lock().expect("poisoned lock") = ChangedPackages::All;
            }
            proto::package_change_event::Event::Error(proto::PackageChangeError { message }) => {
                return Err(DaemonError::Unavailable(message).into());
            }
        }

        Ok(())
    }

    /// Shut down any resources that run as part of watch.
    pub async fn shutdown(&mut self) {
        if let Some(sender) = &self.ui_sender {
            sender.stop().await;
        }
        if let Some(RunHandle { stopper, run_task }) = self.persistent_tasks_handle.take() {
            // Shut down the tasks for the run
            stopper.stop().await;
            // Run should exit shortly after we stop all child tasks, wait for it to finish
            // to ensure all messages are flushed.
            let _ = run_task.await;
        }
    }

    /// Executes a run with the given changed packages. Splits the run into two
    /// parts:
    /// 1. The persistent tasks that are not allowed to be interrupted
    /// 2. The non-persistent tasks and the persistent tasks that are allowed to
    ///    be interrupted
    ///
    /// Returns a handle to the task running (2)
    async fn execute_run(&mut self, changed_packages: ChangedPackages) -> Result<RunHandle, Error> {
        // Should we recover here?
        trace!("handling run with changed packages: {changed_packages:?}");
        match changed_packages {
            ChangedPackages::Some(packages) => {
                let packages = packages
                    .into_iter()
                    .filter(|pkg| {
                        // If not in the watched packages set, ignore
                        self.watched_packages.contains(pkg)
                    })
                    .collect();

                let mut opts = self.base.opts().clone();
                if !self.experimental_write_cache {
                    opts.cache_opts.cache.remote.write = false;
                    opts.cache_opts.cache.remote.read = false;
                }

                let new_base = CommandBase::from_opts(
                    opts,
                    self.base.repo_root.clone(),
                    get_version(),
                    self.base.color_config,
                );

                let signal_handler = self.handler.clone();
                let telemetry = self.telemetry.clone();

                let run = RunBuilder::new(new_base)?
                    .with_entrypoint_packages(packages)
                    .hide_prelude()
                    .build(&signal_handler, telemetry)
                    .await?;

                if let Some(sender) = &self.ui_sender {
                    let task_names = run.engine.tasks_with_command(&run.pkg_dep_graph);
                    sender
                        .restart_tasks(task_names)
                        .map_err(|err| Error::UISend(format!("some packages changed: {err}")))?;
                }

                let ui_sender = self.ui_sender.clone();
                Ok(RunHandle {
                    stopper: run.stopper(),
                    run_task: tokio::spawn(async move { run.run(ui_sender, true).await }),
                })
            }
            ChangedPackages::All => {
                let mut opts = self.base.opts().clone();
                if !self.experimental_write_cache {
                    opts.cache_opts.cache.remote.write = false;
                    opts.cache_opts.cache.remote.read = false;
                }

                let base = CommandBase::from_opts(
                    opts,
                    self.base.repo_root.clone(),
                    get_version(),
                    self.base.color_config,
                );

                // rebuild run struct
                self.run = RunBuilder::new(base.clone())?
                    .hide_prelude()
                    .build(&self.handler, self.telemetry.clone())
                    .await?
                    .into();

                self.watched_packages = self.run.get_relevant_packages();

                // Clean up currently running persistent tasks
                if let Some(RunHandle { stopper, run_task }) = self.persistent_tasks_handle.take() {
                    // Shut down the tasks for the run
                    stopper.stop().await;
                    // Run should exit shortly after we stop all child tasks, wait for it to finish
                    // to ensure all messages are flushed.
                    let _ = run_task.await;
                }
                if let Some(sender) = &self.ui_sender {
                    let task_names = self.run.engine.tasks_with_command(&self.run.pkg_dep_graph);
                    sender
                        .update_tasks(task_names)
                        .map_err(|err| Error::UISend(format!("all packages changed {err}")))?;
                }

                if self.run.has_non_interruptible_tasks() {
                    debug_assert!(
                        self.persistent_tasks_handle.is_none(),
                        "persistent handle should be empty before creating a new one"
                    );
                    let persistent_run = self.run.create_run_for_non_interruptible_tasks();
                    let ui_sender = self.ui_sender.clone();
                    // If we have persistent tasks, we run them on a separate thread
                    // since persistent tasks don't finish
                    self.persistent_tasks_handle = Some(RunHandle {
                        stopper: persistent_run.stopper(),
                        run_task: tokio::spawn(
                            async move { persistent_run.run(ui_sender, true).await },
                        ),
                    });

                    let non_persistent_run = self.run.create_run_for_interruptible_tasks();
                    let ui_sender = self.ui_sender.clone();
                    Ok(RunHandle {
                        stopper: non_persistent_run.stopper(),
                        run_task: tokio::spawn(async move {
                            non_persistent_run.run(ui_sender, true).await
                        }),
                    })
                } else {
                    let ui_sender = self.ui_sender.clone();
                    let run = self.run.clone();
                    Ok(RunHandle {
                        stopper: run.stopper(),
                        run_task: tokio::spawn(async move { run.run(ui_sender, true).await }),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Mutex};

    use turborepo_repository::package_graph::PackageName;

    use super::{ChangedPackages, WatchClient};
    use crate::daemon::proto;

    #[test]
    fn test_changed_packages_is_empty() {
        // Test empty Some variant
        let empty = ChangedPackages::Some(HashSet::new());
        assert!(empty.is_empty());

        // Test non-empty Some variant
        let mut set = HashSet::new();
        set.insert(PackageName::from("test-package"));
        let non_empty = ChangedPackages::Some(set);
        assert!(!non_empty.is_empty());

        // Test All variant
        let all = ChangedPackages::All;
        assert!(!all.is_empty());
    }

    #[test]
    fn test_changed_packages_default() {
        let default = ChangedPackages::default();
        assert!(default.is_empty());
        if let ChangedPackages::Some(set) = default {
            assert!(set.is_empty());
        } else {
            panic!("Default should be Some variant with empty set");
        }
    }

    #[test]
    fn test_handle_change_event_package_changed_to_empty_set() {
        let changed_packages = Mutex::new(ChangedPackages::Some(HashSet::new()));
        let event = proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
            package_name: "test-package".to_string(),
        });

        let result = WatchClient::handle_change_event(&changed_packages, event);
        assert!(result.is_ok());

        let guard = changed_packages.lock().unwrap();
        if let ChangedPackages::Some(ref set) = *guard {
            assert_eq!(set.len(), 1);
            assert!(set.contains(&PackageName::from("test-package")));
        } else {
            panic!("Expected Some variant");
        }
    }

    #[test]
    fn test_handle_change_event_package_changed_to_existing_set() {
        let mut initial_set = HashSet::new();
        initial_set.insert(PackageName::from("existing-package"));
        let changed_packages = Mutex::new(ChangedPackages::Some(initial_set));

        let event = proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
            package_name: "new-package".to_string(),
        });

        let result = WatchClient::handle_change_event(&changed_packages, event);
        assert!(result.is_ok());

        let guard = changed_packages.lock().unwrap();
        if let ChangedPackages::Some(ref set) = *guard {
            assert_eq!(set.len(), 2);
            assert!(set.contains(&PackageName::from("existing-package")));
            assert!(set.contains(&PackageName::from("new-package")));
        } else {
            panic!("Expected Some variant");
        }
    }

    #[test]
    fn test_handle_change_event_package_changed_when_already_all() {
        let changed_packages = Mutex::new(ChangedPackages::All);
        let event = proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
            package_name: "test-package".to_string(),
        });

        let result = WatchClient::handle_change_event(&changed_packages, event);
        assert!(result.is_ok());

        let guard = changed_packages.lock().unwrap();
        // Should remain as All and ignore the specific package
        if let ChangedPackages::All = *guard {
            // This is expected
        } else {
            panic!("Expected to remain as All variant");
        }
    }

    #[test]
    fn test_handle_change_event_rediscover_packages_from_some() {
        let mut initial_set = HashSet::new();
        initial_set.insert(PackageName::from("test-package"));
        let changed_packages = Mutex::new(ChangedPackages::Some(initial_set));

        let event =
            proto::package_change_event::Event::RediscoverPackages(proto::RediscoverPackages {});

        let result = WatchClient::handle_change_event(&changed_packages, event);
        assert!(result.is_ok());

        let guard = changed_packages.lock().unwrap();
        if let ChangedPackages::All = *guard {
            // This is expected
        } else {
            panic!("Expected All variant after rediscover");
        }
    }

    #[test]
    fn test_handle_change_event_rediscover_packages_from_all() {
        let changed_packages = Mutex::new(ChangedPackages::All);
        let event =
            proto::package_change_event::Event::RediscoverPackages(proto::RediscoverPackages {});

        let result = WatchClient::handle_change_event(&changed_packages, event);
        assert!(result.is_ok());

        let guard = changed_packages.lock().unwrap();
        if let ChangedPackages::All = *guard {
            // This is expected - should remain All
        } else {
            panic!("Expected to remain as All variant");
        }
    }

    #[test]
    fn test_handle_change_event_error() {
        let changed_packages = Mutex::new(ChangedPackages::Some(HashSet::new()));
        let error_message = "Test daemon error".to_string();
        let event = proto::package_change_event::Event::Error(proto::PackageChangeError {
            message: error_message.clone(),
        });

        let result = WatchClient::handle_change_event(&changed_packages, event);
        assert!(result.is_err());

        // Verify it returns an error - the specific message format is tested elsewhere
        // The important part is that error events result in errors being returned
        let error = result.unwrap_err();
        assert!(matches!(error, super::Error::Daemon(_)));
    }

    #[test]
    fn test_handle_change_event_multiple_package_changes() {
        let changed_packages = Mutex::new(ChangedPackages::Some(HashSet::new()));

        // Add first package
        let event1 = proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
            package_name: "package-1".to_string(),
        });
        let result = WatchClient::handle_change_event(&changed_packages, event1);
        assert!(result.is_ok());

        // Add second package
        let event2 = proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
            package_name: "package-2".to_string(),
        });
        let result = WatchClient::handle_change_event(&changed_packages, event2);
        assert!(result.is_ok());

        // Verify both packages are in the set
        let guard = changed_packages.lock().unwrap();
        if let ChangedPackages::Some(ref set) = *guard {
            assert_eq!(set.len(), 2);
            assert!(set.contains(&PackageName::from("package-1")));
            assert!(set.contains(&PackageName::from("package-2")));
        } else {
            panic!("Expected Some variant");
        }
    }

    #[test]
    fn test_handle_change_event_duplicate_package_changes() {
        let changed_packages = Mutex::new(ChangedPackages::Some(HashSet::new()));

        // Add same package twice
        let event1 = proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
            package_name: "duplicate-package".to_string(),
        });
        let result = WatchClient::handle_change_event(&changed_packages, event1);
        assert!(result.is_ok());

        let event2 = proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
            package_name: "duplicate-package".to_string(),
        });
        let result = WatchClient::handle_change_event(&changed_packages, event2);
        assert!(result.is_ok());

        // Verify only one instance exists (HashSet deduplication)
        let guard = changed_packages.lock().unwrap();
        if let ChangedPackages::Some(ref set) = *guard {
            assert_eq!(set.len(), 1);
            assert!(set.contains(&PackageName::from("duplicate-package")));
        } else {
            panic!("Expected Some variant");
        }
    }
}
