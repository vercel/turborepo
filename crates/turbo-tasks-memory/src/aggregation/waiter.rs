use std::sync::Arc;

use parking_lot::{Condvar, Mutex};

#[derive(Default)]
pub struct Waiter {
    mutex: Mutex<bool>,
    condvar: Condvar,
}

impl Waiter {
    pub fn wait(&self) {
        let mut guard = self.mutex.lock();
        while !*guard {
            self.condvar.wait(&mut guard);
        }
    }

    pub fn notify(&self) {
        *self.mutex.lock() = true;
        self.condvar.notify_all();
    }
}

#[derive(Default)]
pub struct PotentialWaiter {
    waiter: Option<Arc<Waiter>>,
}

impl PotentialWaiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn waiter(&mut self) -> Arc<Waiter> {
        self.waiter.get_or_insert_default().clone()
    }

    pub fn notify(&mut self) {
        if let Some(waiter) = self.waiter.take() {
            waiter.notify();
        }
    }
}
