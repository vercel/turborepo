use std::time::Duration;

use crate::math::I32ValueRef;
use rand::Rng;
use turbo_tasks::Task;

#[turbo_tasks::function]
pub async fn random() -> I32ValueRef {
    let mut rng = rand::thread_rng();
    let invalidator = Task::get_invalidator();
    async_std::task::spawn(async {
        async_std::task::sleep(Duration::from_secs(5)).await;
        println!("invalidate random number...");
        invalidator.invalidate();
    });
    I32ValueRef::new(rng.gen_range(1..100))
}
