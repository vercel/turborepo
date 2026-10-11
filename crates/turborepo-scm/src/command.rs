use std::{io, process::Command, time::Duration};

use turbopath::AbsoluteSystemPathBuf;
use turborepo_process::{ChildExit, ProcessManager};
use turborepo_task_id::TaskId;

/// Owns the subprocesses used by a single provenance capture. Closing the
/// scope atomically prevents further spawns and drains the existing children.
#[derive(Clone, Debug)]
pub struct ScmCommandScope {
    manager: ProcessManager,
    runtime: tokio::runtime::Handle,
}

impl ScmCommandScope {
    pub fn new() -> Self {
        Self {
            manager: ProcessManager::new(false),
            runtime: tokio::runtime::Handle::current(),
        }
    }

    /// Stop and reap the children. Provenance diffs use a private index so
    /// forced termination cannot leave the repository's index locked.
    pub async fn stop(&self) {
        self.manager.stop().await;
    }

    /// Called only from the blocking SCM worker, never on a runtime thread.
    pub(crate) fn run(
        &self,
        command: &Command,
        stdout: &mut (dyn io::Write + Send),
    ) -> io::Result<(bool, Vec<u8>)> {
        let mut managed = turborepo_process::Command::new(command.get_program());
        managed.args(command.get_args());
        if let Some(dir) = command.get_current_dir() {
            managed.current_dir(AbsoluteSystemPathBuf::try_from(dir).map_err(io::Error::other)?);
        }
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                managed.env(key, value);
            } else {
                return Err(io::Error::other(
                    "SCM command cannot remove environment variables",
                ));
            }
        }
        self.runtime.block_on(async {
            let mut child = self
                .manager
                .spawn(
                    managed,
                    Duration::from_secs(1),
                    TaskId::new("scm", "provenance"),
                )
                .ok_or_else(|| io::Error::other("SCM capture is shutting down"))??;
            let (status, stderr) = child.wait_with_raw_stdout(stdout).await?;
            Ok((status == Some(ChildExit::Finished(Some(0))), stderr))
        })
    }
}

impl Default for ScmCommandScope {
    fn default() -> Self {
        Self::new()
    }
}
