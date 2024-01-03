//! Implementation of process group extensions for [Tokio](https://tokio.rs)’s
//! asynchronous [`Command` type](::tokioprocesspty::Command).

use std::{
    io::Result,
    process::{ExitStatus, Output},
};

use crate::{
    command_group::{builder::CommandGroupBuilder, AsyncGroupChild},
    tokioprocesspty::Command,
};

#[cfg(target_family = "windows")]
mod windows;

#[cfg(target_family = "unix")]
mod unix;

pub(crate) mod child;

/// Extensions for [`Command`](::tokioprocesspty::Command) adding support for
/// process groups.
///
/// This uses [`async_trait`] for now to provide async methods as a trait.
#[async_trait::async_trait]
pub trait AsyncCommandGroup {
    /// Executes the command as a child process group, returning a handle to it.
    ///
    /// By default, stdin, stdout and stderr are inherited from the parent.
    ///
    /// On Windows, this creates a job object instead of a POSIX process group.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() {
    /// use tokioprocesspty::Command;
    /// use command_group::AsyncCommandGroup;
    ///
    /// Command::new("ls")
    ///         .group_spawn()
    ///         .expect("ls command failed to start");
    /// # }
    /// ```
    fn group_spawn(&mut self) -> Result<AsyncGroupChild> {
        self.group().spawn()
    }

    /// Converts the implementor into a
    /// [`CommandGroupBuilder`](crate::CommandGroupBuilder), which can be used
    /// to set flags that are not available on the `Command` type.
    fn group(&mut self) -> crate::builder::CommandGroupBuilder<crate::tokioprocesspty::Command>;

    /// Executes the command as a child process group, waiting for it to finish
    /// and collecting all of its output.
    ///
    /// By default, stdout and stderr are captured (and used to provide the
    /// resulting output). Stdin is not inherited from the parent and any
    /// attempt by the child process to read from the stdin stream will result
    /// in the stream immediately closing.
    ///
    /// On Windows, this creates a job object instead of a POSIX process group.
    ///
    /// # Examples
    ///
    /// ```should_panic
    /// # #[tokio::main]
    /// # async fn main() {
    /// use tokioprocesspty::Command;
    /// use std::io::{self, Write};
    /// use command_group::AsyncCommandGroup;
    ///
    /// let output = Command::new("/bin/cat")
    ///                      .arg("file.txt")
    ///                      .group_output()
    ///                      .await
    ///                      .expect("failed to execute process");
    ///
    /// println!("status: {}", output.status);
    /// io::stdout().write_all(&output.stdout).unwrap();
    /// io::stderr().write_all(&output.stderr).unwrap();
    ///
    /// assert!(output.status.success());
    /// # }
    /// ```
    async fn group_output(&mut self) -> Result<Output> {
        let child = self.group_spawn()?;
        child.wait_with_output().await
    }

    /// Executes a command as a child process group, waiting for it to finish
    /// and collecting its status.
    ///
    /// By default, stdin, stdout and stderr are inherited from the parent.
    ///
    /// On Windows, this creates a job object instead of a POSIX process group.
    ///
    /// # Examples
    ///
    /// ```should_panic
    /// # #[tokio::main]
    /// # async fn main() {
    /// use tokioprocesspty::Command;
    /// use command_group::AsyncCommandGroup;
    ///
    /// let status = Command::new("/bin/cat")
    ///                      .arg("file.txt")
    ///                      .group_status()
    ///                      .await
    ///                      .expect("failed to execute process");
    ///
    /// println!("process finished with: {}", status);
    ///
    /// assert!(status.success());
    /// # }
    /// ```
    async fn group_status(&mut self) -> Result<ExitStatus> {
        let mut child = self.group_spawn()?;
        child.wait().await
    }
}

#[async_trait::async_trait]
impl AsyncCommandGroup for Command {
    fn group<'a>(&'a mut self) -> CommandGroupBuilder<'a, Command> {
        CommandGroupBuilder::new(self)
    }
}
