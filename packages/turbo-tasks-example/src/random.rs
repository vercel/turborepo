use std::{future::Future, time::Duration};

use crate::math::I32ValueRef;
use anyhow::anyhow;
use lazy_static::lazy_static;
use rand::Rng;
use turbo_tasks::{dynamic_call, NativeFunction, Task};

pub async fn random_impl() -> I32ValueRef {
    let mut rng = rand::thread_rng();
    let invalidator = Task::get_invalidator();
    async_std::task::spawn(async {
        async_std::task::sleep(Duration::from_secs(5)).await;
        println!("invalidate random number...");
        invalidator.invalidate();
    });
    I32ValueRef::new(rng.gen_range(1..100))
}

// TODO autogenerate that
lazy_static! {
    static ref RANDOM_FUNCTION: NativeFunction = NativeFunction::new(|inputs| {
        if inputs.len() != 0 {
            return Err(anyhow!("random() called with too many arguments"));
        }
        Ok(Box::new(move || {
            Box::pin(async move { random_impl().await.into() })
        }))
    });
}

pub fn random() -> impl Future<Output = I32ValueRef> {
    // TODO decide if we want to schedule or execute directly
    // directly would be `random_impl()`
    let result = dynamic_call(&RANDOM_FUNCTION, Vec::new()).unwrap();
    return async { I32ValueRef::from_node(result.await).unwrap() };
}
