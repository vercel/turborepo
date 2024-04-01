use std::{process::Stdio, sync::Arc, time::Duration};

use futures::future::join_all;
use tokio::{
    process::Command,
    sync::{
        mpsc::{Receiver as MpscReceiver, Sender as MpscSender},
        oneshot::{self, Receiver, Sender},
        Mutex,
    },
};
use turbo_updater::check_for_updates;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_scm::Git;

use crate::{
    commands::{
        link::{self, link},
        CommandBase,
    },
    daemon::DaemonConnector,
    get_version, DaemonPaths,
};

#[derive(Debug)]
pub enum DiagnosticMessage {
    NotApplicable(String),
    /// internal name of diag and human readable name
    Started(String),
    LogLine(String),
    Done(String),
    Failed(String),
    /// a request to suspend terminal output. the renderer
    /// will notify the diagnostic when it is safe to render
    /// and the diagnostic notify in return when it is done
    Suspend(Sender<()>, Receiver<()>),
    /// a request to the user with options and a callback to send the response
    Request(String, Vec<String>, Sender<String>),
}

/// A light wrapper around a tokio channel with a must_use such that we ensure
/// the communication is always terminated and the diagnostic is always
/// notified, in absence of linear types. Unfortunately this lint is flaky
/// at best so while we can't solely rely on it to enforce the invariant,
/// it's better than nothing.
#[must_use]
#[derive(Clone)]
pub struct DiagnosticChannel(MpscSender<DiagnosticMessage>);
impl DiagnosticChannel {
    pub fn new() -> (Self, MpscReceiver<DiagnosticMessage>) {
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        (Self(tx), rx)
    }

    async fn started(&self, message: String) {
        self.send(DiagnosticMessage::Started(message)).await
    }

    async fn log_line(&self, message: String) {
        self.send(DiagnosticMessage::LogLine(message)).await
    }

    /// prompts the user with a message and options
    async fn request(&self, message: String, options: Vec<String>) -> Option<Receiver<String>> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(DiagnosticMessage::Request(message, options, tx))
            .await
            .ok()?; // if the channel is closed, we can't request
        Some(rx)
    }

    /// suspends the terminal output and returns a pair of channels
    ///
    /// the first one signifies to the diagnostic when it is safe to render
    /// and the second one is used by the diagnostic to notify the renderer
    /// when it is done
    async fn suspend(&self) -> Option<(Receiver<()>, Sender<()>)> {
        let (stopped_tx, stopped_rx) = oneshot::channel();
        let (resume_tx, resume_rx) = oneshot::channel();

        self.0
            .send(DiagnosticMessage::Suspend(stopped_tx, resume_rx))
            .await
            .ok()?; // if the channel is closed, we can't suspend

        Some((stopped_rx, resume_tx))
    }

    // types of exit

    /// the diagnostic is done
    async fn done(self, message: String) {
        self.send(DiagnosticMessage::Done(message.to_string()))
            .await
    }

    /// the diagnostic failed
    async fn failed(self, message: String) {
        self.send(DiagnosticMessage::Failed(message.to_string()))
            .await
    }

    /// the diagnostic is not applicable
    async fn not_applicable(self, message: String) {
        self.send(DiagnosticMessage::NotApplicable(message.to_string()))
            .await
    }

    pub async fn send(&self, message: DiagnosticMessage) {
        _ = self.0.send(message).await // channel closed, ignore
    }
}

// sadly, because we want to use this trait as a trait object, we can't use
// async fn in the trait definition since it desugars to associated types
// which aren't compatible with trait objects. we _can_ box the futures though,
// which is what async_trait does for us. rather than add a dep, lets just
// use the one we use for the daemon though tonic
#[tonic::async_trait]
pub trait Diagnostic {
    // this needs to have a self param so it can be a trait object
    fn name(&self) -> &'static str;

    /// Execute a diagnostic. You _must_ call `DiagnosticChannel::started` when
    /// the diag has begun, and dispose of the channel by calling one of the
    /// three exit methods identified by consuming `self`.
    fn execute(&self, chan: DiagnosticChannel);
}

pub struct GitDaemonDiagnostic;

impl Diagnostic for GitDaemonDiagnostic {
    fn name(&self) -> &'static str {
        "git.daemon"
    }

    fn execute(&self, chan: DiagnosticChannel) {
        tokio::task::spawn(async move {
            chan.started("Git FS Monitor".to_string()).await;

            if !(cfg!(target_os = "windows") || cfg!(target_os = "macos")) {
                // the git daemon is not implemented on unix
                chan.not_applicable("Git FS Monitor (not available on linux)".to_string())
                    .await;
                return;
            }

            let commands: Result<Vec<Vec<u8>>, _> = join_all(
                [
                    &["--version"][..],
                    &["config", "--get", "core.fsmonitor"][..],
                    &["config", "--get", "core.untrackedcache"][..],
                ]
                .into_iter()
                .map(|args| async move {
                    // get the current setting
                    let stdout = Stdio::piped();

                    let Ok(git_path) = Git::find_bin() else {
                        return Err("git not found");
                    };

                    let command = Command::new(git_path.as_path())
                        .args(args)
                        .stdout(stdout)
                        .stdin(Stdio::null())
                        .spawn()
                        .expect("too many processes"); // this can only fail if we run out of processes on unix

                    command
                        .wait_with_output()
                        .await
                        .map(|d| d.stdout)
                        .map_err(|_| "failed to get git metadata")
                }),
            )
            .await
            .into_iter()
            .collect(); // transpose

            chan.log_line("Collecting metadata".to_string()).await;

            match commands.as_ref().map(|v| v.as_slice()) {
                Ok([version, fsmonitor, untrackedcache]) => {
                    let version = String::from_utf8_lossy(version);
                    let Some(version) = version.trim().strip_prefix("git version ") else {
                        chan.failed("Failed to get git version".to_string()).await;
                        return;
                    };

                    // attempt to split out the apple git suffix
                    let version = if let Some((version, _)) = version.split_once(" (Apple") {
                        version
                    } else {
                        version
                    };

                    let Ok(version) = semver::Version::parse(version) else {
                        chan.failed("Failed to parse git version".to_string()).await;
                        return;
                    };

                    if version.major == 2 && version.minor < 37 || version.major == 1 {
                        chan.not_applicable(format!(
                            "Git version {} is too old, please upgrade to 2.37 or newer",
                            version
                        ))
                        .await;
                        return;
                    } else {
                        chan.log_line(
                            format!("Using supported Git version - {}", version).to_string(),
                        )
                        .await;
                    }

                    let fsmonitor = String::from_utf8_lossy(fsmonitor);
                    let untrackedcache = String::from_utf8_lossy(untrackedcache);

                    if fsmonitor.trim() != "true" || untrackedcache.trim() != "true" {
                        chan.log_line("Git FS Monitor not configured".to_string())
                            .await;
                        chan.log_line( "For more information, see https://turbo.build/repo/docs/reference/command-line-reference/scan#fs-monitor".to_string()).await;
                        let Some(resp) = chan
                            .request(
                                "Configure it for this repo now?".to_string(),
                                vec!["Yes".to_string(), "No".to_string()],
                            )
                            .await
                        else {
                            // the sender (terminal) was shut, ignore
                            return;
                        };
                        match resp.await.as_ref().map(|s| s.as_str()) {
                            Ok("Yes") => {
                                chan.log_line("Setting Git FS Monitor settings".to_string())
                                    .await;

                                let futures = [
                                    ("core.fsmonitor", fsmonitor),
                                    ("core.untrackedcache", untrackedcache),
                                ]
                                .into_iter()
                                .filter(|(_, value)| value.trim() != "true")
                                .map(|(key, _)| async {
                                    let stdout = Stdio::piped();

                                    let command = Command::new("git")
                                        .args(["config", key, "true"])
                                        .stdout(stdout)
                                        .spawn()
                                        .expect("too many processes"); // this can only fail if we run out of processes on unix

                                    command.wait_with_output().await
                                });

                                let results: Result<Vec<_>, _> =
                                    join_all(futures).await.into_iter().collect();

                                match results {
                                    Ok(_) => {
                                        chan.log_line("Git FS Monitor settings set".to_string())
                                            .await;
                                    }
                                    Err(e) => {
                                        chan.failed(format!("Failed to set git settings: {}", e))
                                            .await;
                                        return;
                                    }
                                }
                            }
                            Ok("No") => {
                                chan.failed("Git FS Monitor not configured".to_string())
                                    .await;
                                return;
                            }
                            Ok(_) => unreachable!(),
                            Err(_) => {
                                // the sender (terminal) was shut, ignore
                            }
                        }
                    } else {
                        chan.log_line("Git FS Monitor settings set".to_string())
                            .await;
                    }
                }
                Ok(_) => unreachable!(), // the vec of futures has exactly 3 elements
                Err(e) => {
                    chan.failed(format!("Failed to get git version: {}", e))
                        .await;
                    return;
                }
            }

            chan.done("Git FS Monitor enabled".to_string()).await;
        });
    }
}

pub struct DaemonDiagnostic(pub DaemonPaths);

impl Diagnostic for DaemonDiagnostic {
    fn name(&self) -> &'static str {
        "turbo.daemon"
    }

    fn execute(&self, chan: DiagnosticChannel) {
        let paths = self.0.clone();
        tokio::task::spawn(async move {
            chan.started("Turbo Daemon".to_string()).await;
            chan.log_line("Connecting to daemon...".to_string()).await;

            let pid_path = paths.pid_file.as_std_path().to_owned();

            let connector = DaemonConnector {
                can_kill_server: false,
                can_start_server: true,
                paths,
            };

            let mut client = match connector.connect().await {
                Ok(client) => client,
                Err(e) => {
                    chan.failed(format!("Failed to connect to daemon: {}", e))
                        .await;
                    return;
                }
            };

            chan.log_line("Getting status...".to_string()).await;

            match client.status().await {
                Ok(status) => {
                    chan.log_line(format!("Daemon up for {}ms", status.uptime_msec))
                        .await;
                    let lock = pidlock::Pidlock::new(pid_path);
                    let pid = if let Ok(Some(owner)) = lock.get_owner() {
                        format!(" (pid {})", owner)
                    } else {
                        "".to_string()
                    };
                    chan.done(format!("Daemon is running{}", pid)).await;
                }
                Err(e) => {
                    chan.failed(format!("Failed to get daemon status: {}", e))
                        .await;
                }
            }
        });
    }
}

pub struct LSPDiagnostic(pub DaemonPaths);
impl Diagnostic for LSPDiagnostic {
    fn name(&self) -> &'static str {
        "turbo.lsp"
    }

    fn execute(&self, chan: DiagnosticChannel) {
        let lsp_root = self.0.lsp_pid_file.as_std_path().to_owned();
        tokio::task::spawn(async move {
            chan.started("Turborepo Extension".to_string()).await;
            chan.log_line("Checking if extension is running...".to_string())
                .await;
            let pidlock = pidlock::Pidlock::new(lsp_root);
            match pidlock.get_owner() {
                Ok(Some(pid)) => {
                    chan.done(format!("Turborepo Extension is running (pid {})", pid))
                        .await;
                }
                Ok(None) => {
                    chan.log_line("Unable to find LSP instance".to_string())
                        .await;
                    chan.log_line( "For more information, see https://turbo.build/repo/docs/reference/command-line-reference/scan#lsp".to_string()).await;
                    chan.failed("Turborepo Extension is not running".to_string())
                        .await;
                }
                Err(e) => {
                    chan.failed(format!("Failed to get LSP status: {}", e))
                        .await;
                }
            }
        });
    }
}

/// a struct that checks and prompts the user to enable remote cache
pub struct RemoteCacheDiagnostic(pub Arc<Mutex<CommandBase>>);
impl RemoteCacheDiagnostic {
    pub fn new(base: CommandBase) -> Self {
        Self(Arc::new(Mutex::new(base)))
    }
}

impl Diagnostic for RemoteCacheDiagnostic {
    fn name(&self) -> &'static str {
        "vercel.auth"
    }

    fn execute(&self, chan: DiagnosticChannel) {
        let base = self.0.clone();
        tokio::task::spawn(async move {
            chan.started("Remote Cache".to_string()).await;

            let result = {
                let base = base.lock().await;
                base.config()
                    .map(|c| (c.team_id().is_some(), c.team_slug().is_some()))
            };

            let Ok((has_team_id, has_team_slug)) = result else {
                chan.failed("Malformed config file".to_string()).await;
                return;
            };

            chan.log_line("Checking credentials".to_string()).await;

            if has_team_id || has_team_slug {
                chan.done("Remote Cache enabled".to_string()).await;
                return;
            }

            let result = {
                chan.log_line("Linking to remote cache".to_string()).await;
                let mut base = base.lock().await;
                let Some((stopped, resume)) = chan.suspend().await else {
                    // the sender (terminal) was shut, ignore
                    return;
                };
                stopped.await.unwrap();
                let link_res = link(&mut base, false, crate::cli::LinkTarget::RemoteCache).await;
                resume.send(()).unwrap();
                link_res
            };

            match result {
                Ok(_) => {
                    chan.log_line("Linked".to_string()).await;
                    chan.done("Remote Cache enabled".to_string()).await
                }
                Err(link::Error::NotLinking) => {
                    chan.not_applicable("Remote Cache opted out".to_string())
                        .await
                }
                Err(e) => {
                    chan.failed(format!("Failed to link: {}", e)).await;
                }
            }
        });
    }
}

pub struct UpdateDiagnostic(pub AbsoluteSystemPathBuf);

impl Diagnostic for UpdateDiagnostic {
    fn name(&self) -> &'static str {
        "turbo.update"
    }

    fn execute(&self, chan: DiagnosticChannel) {
        let repo_root = self.0.clone();
        tokio::task::spawn(async move {
            chan.started("Update Turborepo to latest version".to_string())
                .await;
            chan.log_line("Checking for updates...".to_string()).await;
            let version = tokio::task::spawn_blocking(|| {
                check_for_updates(
                    "turbo",
                    get_version(),
                    None,
                    Some(Duration::from_secs(0)), // check every time
                )
                .map_err(|e| e.to_string()) // not send
            })
            .await;

            match version {
                Ok(Ok(Some(version))) => {
                    chan.log_line(format!("Turborepo {} is available", version).to_string())
                        .await;

                    let Some(resp) = chan
                        .request(
                            "Would you like to run the codemod automatically?".to_string(),
                            vec!["Yes".to_string(), "No".to_string()],
                        )
                        .await
                    else {
                        // the sender (terminal) was shut, ignore
                        return;
                    };

                    match resp.await.as_ref().map(|s| s.as_str()) {
                        Ok("Yes") => {
                            chan.log_line("Updating Turborepo...".to_string()).await;
                            let mut command = Command::new("npx");
                            let command = command
                                .arg("--yes")
                                .arg("@turbo/codemod@latest")
                                .arg("update")
                                .arg("--force")
                                .arg(repo_root.as_path())
                                .stdout(Stdio::piped())
                                .stdin(Stdio::null())
                                .spawn()
                                .expect("too many processes"); // this can only fail if we run out of processes on unix

                            match command.wait_with_output().await {
                                Ok(output) if output.status.success() => {
                                    chan.done("Turborepo on latest version".to_string()).await;
                                }
                                Ok(output) => {
                                    chan.log_line(
                                        String::from_utf8(output.stdout).unwrap_or_default(),
                                    )
                                    .await;
                                    chan.failed("Unable to update Turborepo".to_string()).await
                                }
                                Err(_) => {
                                    chan.failed("Unable to update Turborepo".to_string()).await
                                }
                            }
                        }
                        Ok("No") => chan.failed("Turborepo on old version".to_string()).await,
                        Ok(_) => unreachable!(), // yes and no are the only options
                        Err(_) => {
                            // the sender (terminal) was shut, ignore
                        }
                    }
                }
                // no versions in the registry, just report success
                Ok(Ok(None)) => {
                    chan.log_line("No updates available".to_string()).await;
                    chan.done("Turborepo on latest version".to_string()).await
                }
                Ok(Err(message)) => {
                    chan.failed(format!("Failed to check for updates: {}", message))
                        .await;
                }
                Err(_) => {
                    chan.failed("Failed to check for updates".to_string()).await;
                }
            }
        });
    }
}
