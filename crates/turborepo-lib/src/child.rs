use std::{io, process::Command, sync::Arc};

use shared_child::SharedChild;

/// Spawns a child in a way where SIGINT is correctly forwarded to the child
pub fn spawn_child(mut command: Command) -> Result<Arc<SharedChild>, io::Error> {
    let shared_child = Arc::new(SharedChild::spawn(&mut command)?);
    let handler_shared_child = shared_child.clone();

    ctrlc::set_handler(move || {
        // on windows, we can't send signals so just kill
        // we are quiting anyways so just ignore
        #[cfg(target_os = "windows")]
        handler_shared_child.kill().ok();

        // on unix, we should send a SIGTERM to the child
        // so that go can gracefully shut down process groups
        // SAFETY: we could pull in the nix crate to handle this
        // 'safely' but nix::sys::signal::kill just calls libc::kill
        #[cfg(not(target_os = "windows"))]
        unsafe {
            libc::kill(handler_shared_child.id() as i32, libc::SIGTERM);
        }
    })
    .expect("handler set");

    Ok(shared_child)
}
