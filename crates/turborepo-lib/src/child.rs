use std::{io, process::Command, sync::Arc};

use shared_child::SharedChild;

/// Spawns a child in a way where SIGINT is correctly forwarded to the child
pub fn spawn_child(mut command: Command) -> Result<Arc<SharedChild>, io::Error> {
    let shared_child = Arc::new(SharedChild::spawn(&mut command)?);

    #[cfg(not(target_os = "windows"))]
    let handler_shared_child = shared_child.clone();

    ctrlc::set_handler(move || {
        // On Windows the child shares our console, so the OS delivers the
        // CTRL_C_EVENT to it directly and it can run its own graceful
        // shutdown. Killing the child here would race ahead of that
        // shutdown and terminate it immediately, so this handler only
        // swallows the event to keep this process alive long enough to
        // collect the child's exit status.

        // on unix, we should send a SIGTERM to the child
        // so that go can gracefully shut down process groups
        // SAFETY: we could pull in the nix crate to handle this
        // 'safely' but nix::sys::signal::kill just calls libc::kill
        #[cfg(not(target_os = "windows"))]
        unsafe {
            libc::kill(handler_shared_child.id() as i32, libc::SIGTERM);
        }
    })
    .map_err(io::Error::other)?;

    Ok(shared_child)
}
