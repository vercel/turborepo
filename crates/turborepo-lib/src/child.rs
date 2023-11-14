use std::{io, process::Command, sync::Arc};

use ctrlc;
use shared_child::SharedChild;
use tracing::debug;

/// Spawns a child in a way where SIGINT is correctly forwarded to the child
pub fn spawn_child(mut command: Command) -> Result<Arc<SharedChild>, io::Error> {
    debug!("spawn_child entered");

    let shared_child = Arc::new(SharedChild::spawn(&mut command)?);

    debug!("shared_child assigned");

    let handler_shared_child = shared_child.clone();

    debug!("shared_child cloned");

    debug!("before setting handler");

    ctrlc::set_handler(move || {
        debug!("ctrlc handler called");
        // on windows, we can't send signals so just kill
        // we are quiting anyways so just ignore
        #[cfg(target_os = "windows")]
        unsafe {
            debug!("Calling special kill code for windows");
            handler_shared_child.kill().ok();
        }

        // on unix, we should send a SIGTERM to the child
        // so that go can gracefully shut down process groups
        // SAFETY: we could pull in the nix crate to handle this
        // 'safely' but nix::sys::signal::kill just calls libc::kill
        debug!("Calling special kill code for unix");
        #[cfg(not(target_os = "windows"))]
        unsafe {
            libc::kill(handler_shared_child.id() as i32, libc::SIGTERM);
        }
    })
    .expect("handler set");

    debug!("returning shared child");

    Ok(shared_child)
}
