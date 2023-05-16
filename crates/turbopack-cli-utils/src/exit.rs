use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

pub struct CloseGuard<T>(Arc<Mutex<Option<T>>>);

impl<T> Drop for CloseGuard<T> {
    fn drop(&mut self) {
        drop(self.0.lock().unwrap().take())
    }
}

pub fn exit_guard<T: Send + 'static>(guard: T) -> Result<CloseGuard<T>> {
    let guard = Arc::new(Mutex::new(Some(guard)));
    {
        let guard = guard.clone();
        ctrlc::set_handler(move || {
            drop(guard.lock().unwrap().take());
            std::process::exit(0);
        })
        .context("Unable to set ctrl-c handler")?;
    }
    Ok(CloseGuard(guard))
}
