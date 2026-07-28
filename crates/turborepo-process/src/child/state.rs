use std::io;

use portable_pty::MasterPty as PtyController;
use tokio::sync::{mpsc, watch};
use tracing::{debug, trace};

use super::{ChildExit, ChildHandle, ShutdownStyle};

/// The structure that holds logic regarding interacting with the underlying
/// child process
#[derive(Debug)]
pub(super) struct ChildStateManager {
    pub(super) shutdown_style: ShutdownStyle,
    pub(super) exit_tx: watch::Sender<Option<ChildExit>>,
    pub(super) shutdown_initiated: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ChildCommandChannel(pub(super) mpsc::Sender<ChildCommand>);

impl ChildCommandChannel {
    pub(super) fn new() -> (Self, mpsc::Receiver<ChildCommand>) {
        let (tx, rx) = mpsc::channel(1);
        (ChildCommandChannel(tx), rx)
    }

    pub async fn shutdown(
        &self,
        shutdown_style: ShutdownStyle,
    ) -> Result<(), mpsc::error::SendError<ChildCommand>> {
        self.0.send(ChildCommand::Shutdown(shutdown_style)).await
    }

    pub async fn kill(&self) -> Result<(), mpsc::error::SendError<ChildCommand>> {
        self.0.send(ChildCommand::Kill).await
    }
}

pub(super) enum ChildCommand {
    Shutdown(ShutdownStyle),
    Kill,
}

impl ChildStateManager {
    pub(super) async fn handle_child_command(
        &self,
        command: Option<ChildCommand>,
        command_rx: &mut mpsc::Receiver<ChildCommand>,
        child: &mut ChildHandle,
        controller: Option<Box<dyn PtyController + Send>>,
    ) {
        let exit = match command.unwrap_or(ChildCommand::Shutdown(self.shutdown_style)) {
            ChildCommand::Shutdown(shutdown_style) => {
                debug!("stopping child process");
                shutdown_style.process(child, command_rx).await
            }
            ChildCommand::Kill => {
                debug!("killing child process");
                ShutdownStyle::Kill.process(child, command_rx).await
            }
        };
        // ignore the send error, failure means the channel is dropped
        trace!("sending child exit after shutdown");
        self.exit_tx.send(Some(exit)).ok();
        drop(controller);
    }

    pub(super) async fn handle_child_exit(&self, status: io::Result<Option<i32>>) {
        // If a shutdown was initiated we defer to the exit returned by
        // `ShutdownStyle::process` as that will have information if the child
        // responded to a SIGINT or a SIGKILL. The `wait` response this function
        // gets in that scenario would make it appear that the child was killed by an
        // external process.
        if self.shutdown_initiated {
            return;
        }

        debug!("child process exited normally");
        // the child process exited
        let child_exit = match status {
            Ok(Some(c)) => ChildExit::Finished(Some(c)),
            // if we hit this case, it means that the child process was killed
            // by someone else, and we should report that it was killed
            Ok(None) => ChildExit::KilledExternal,
            Err(_e) => ChildExit::Failed,
        };

        // ignore the send error, the channel is dropped anyways
        trace!("sending child exit");
        self.exit_tx.send(Some(child_exit)).ok();
    }
}
