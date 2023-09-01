use std::sync::{Arc, Mutex};

use itertools::Either;

use super::{Closed, Open, ProcessManager};

/// A simple adaptor over `ProcessManager` that allows it to be shared between
/// threads.
#[derive(Debug)]
pub struct SharedProcessManager(Arc<Mutex<Either<ProcessManager<Open>, ProcessManager<Closed>>>>);

impl SharedProcessManager {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Either::Right(
            ProcessManager::<Closed>::new(),
        ))))
    }

    pub fn start(&self) {
        let mut lock = self.0.lock().unwrap();
        // we need ownership of the data in the lock, so replace it with a default value
        let old = std::mem::replace(&mut *lock, Either::Right(ProcessManager::<Closed>::new()));
        *lock = old.right_and_then(|v| Either::Left(v.start()));
    }

    pub fn is_started(&self) -> bool {
        let lock = self.0.lock().unwrap();
        lock.is_left()
    }

    pub async fn wait(&self) {
        let mut lock = self.0.lock().unwrap();
        // we need ownership of the data in the lock, so replace it with a default value
        let old = std::mem::replace(&mut *lock, Either::Right(ProcessManager::<Closed>::new()));
        *lock = match old {
            Either::Left(manager) => Either::Right(manager.wait().await),
            closed => closed,
        }
    }

    pub async fn stop(&self) {
        let mut lock = self.0.lock().unwrap();
        // we need ownership of the data in the lock, so replace it with a default value
        let old = std::mem::replace(&mut *lock, Either::Right(ProcessManager::<Closed>::new()));
        *lock = match old {
            Either::Left(manager) => Either::Right(manager.stop().await),
            closed => closed,
        }
    }

    pub fn spawn(
        &self,
        command: super::child::Command,
        timeout: std::time::Duration,
    ) -> Option<super::child::Child> {
        let mut lock = self.0.lock().unwrap();
        lock.as_mut().map_left(|l| l.spawn(command, timeout)).left()
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_spawn() {
        let manager = SharedProcessManager::new();

        let mut command = super::super::child::Command::new("sleep");
        command.arg("1");

        manager.start();
        let child = manager.spawn(command, Duration::from_secs(1));
        assert!(child.is_some());
        manager.stop().await;
    }

    #[tokio::test]
    async fn test_stop() {
        let manager = SharedProcessManager::new();
        manager.start();
        manager.stop().await;
    }

    #[tokio::test]
    async fn test_stop_then_spawn() {
        let manager = SharedProcessManager::new();

        let mut command = super::super::child::Command::new("sleep");
        command.arg("1");

        manager.start();
        manager.stop().await;
        let child = manager.spawn(command, Duration::from_secs(1));
        assert!(child.is_none());
        manager.stop().await;
    }

    #[tokio::test]
    async fn test_wait() {
        let manager = SharedProcessManager::new();

        let mut command = super::super::child::Command::new("sleep");
        command.arg("1");

        manager.start();
        let child = manager.spawn(command, Duration::from_secs(1));
        assert!(child.is_some());
        manager.wait().await;
        assert!(!manager.is_started());
        manager.stop().await;
        assert!(!manager.is_started());
    }
}
