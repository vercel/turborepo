use std::{
    sync::atomic::{AtomicI32, Ordering},
    time::Duration,
};

use crate::math::I32ValueRef;
use rand::Rng;
use turbo_tasks::Task;

#[turbo_tasks::function]
pub async fn random(id: RandomIdRef) -> I32ValueRef {
    let mut rng = rand::thread_rng();
    let invalidator = Task::get_invalidator();
    if id.get().counter.fetch_sub(1, Ordering::SeqCst) > 0 {
        async_std::task::spawn(async {
            async_std::task::sleep(Duration::from_secs(5)).await;
            println!("invalidate random number...");
            invalidator.invalidate();
        });
    }
    I32ValueRef::new(rng.gen_range(1..=6))
}

#[turbo_tasks::value]
pub struct RandomId {
    counter: AtomicI32,
}

#[turbo_tasks::value_impl]
impl RandomId {
    #[turbo_tasks::constructor(compare)]
    pub fn new() -> Self {
        Self {
            counter: AtomicI32::new(3),
        }
    }
}
