use std::{ops::Deref, sync::Arc};

use tokio::sync::{Semaphore, SemaphorePermit};
use turborepo_scm::SCM;

#[derive(Debug, Clone)]
pub struct SCMResource {
    scm: SCM,
    semaphore: Arc<Semaphore>,
}

pub struct SCMPermit<'a> {
    scm: &'a SCM,
    _permit: SemaphorePermit<'a>,
}

impl SCMResource {
    pub fn new(scm: SCM) -> Self {
        // We want to only take at most NUM_CPUS - 3 for git processes.
        // Accounting for the `turbo` process itself and the daemon this leaves one core
        // available for the rest of the system.
        let num_permits = num_cpus::get().saturating_sub(3).max(1);
        Self::new_with_permits(scm, num_permits)
    }

    fn new_with_permits(scm: SCM, num_permits: usize) -> Self {
        let semaphore = Arc::new(Semaphore::new(num_permits));
        Self { scm, semaphore }
    }

    pub async fn acquire_scm(&self) -> SCMPermit<'_> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .expect("semaphore should not be closed");
        SCMPermit {
            scm: &self.scm,
            _permit,
        }
    }
}

impl Deref for SCMPermit<'_> {
    type Target = SCM;

    fn deref(&self) -> &Self::Target {
        self.scm
    }
}

#[cfg(test)]
mod test {
    use tokio::sync::oneshot;

    use super::*;

    #[tokio::test]
    async fn test_limits_access() {
        let scm = SCMResource::new_with_permits(SCM::Manual, 1);
        let scm_copy = scm.clone();
        let permit_1 = scm.acquire_scm().await;
        let (other_tx, mut other_rx) = oneshot::channel();
        tokio::task::spawn(async move {
            let _permit_2 = scm_copy.acquire_scm().await;
            other_tx.send(()).ok();
        });
        assert!(
            other_rx.try_recv().is_err(),
            "other should not have gotten a scm permit"
        );
        drop(permit_1);
        assert!(
            other_rx.await.is_ok(),
            "other should have gotten permit and exited"
        );
    }
}
