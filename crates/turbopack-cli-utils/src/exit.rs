use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

/// A guard for the exit handler. When dropped, the exit guard will be dropped.
/// It might also be dropped on Ctrl-C.
pub struct ExitGuard<T>(Arc<Mutex<Option<T>>>);

impl<T> Drop for ExitGuard<T> {
    fn drop(&mut self) {
        drop(self.0.lock().unwrap().take())
    }
}

/// Drop a guard when Ctrl-C is pressed or the [ExitGuard] is dropped.
pub fn exit_guard<T: Send + 'static>(guard: T) -> Result<ExitGuard<T>> {
    let guard = Arc::new(Mutex::new(Some(guard)));
    {
        let guard = guard.clone();
        ctrlc::set_handler(move || {
            drop(guard.lock().unwrap().take());
            std::process::exit(0);
        })
        .context("Unable to set ctrl-c handler")?;
    }
    Ok(ExitGuard(guard))
}
