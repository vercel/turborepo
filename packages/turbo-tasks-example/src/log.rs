use anyhow::{anyhow, Context};
use lazy_static::lazy_static;
use std::future::Future;
use turbo_tasks::{dynamic_call, NativeFunction, Task};

use crate::math::I32ValueRef;

pub async fn log_impl(a: I32ValueRef) -> I32ValueRef {
    println!("{}", a.get().value);
    Task::side_effect();
    a
}

// TODO autogenerate that
lazy_static! {
    static ref LOG_FUNCTION: NativeFunction = NativeFunction::new(|inputs| {
        let mut iter = inputs.into_iter();
        let a = iter
            .next()
            .ok_or_else(|| anyhow!("log() first argument missing"))?;
        if iter.next().is_some() {
            return Err(anyhow!("log() called with too many arguments"));
        }
        I32ValueRef::verify(&a).context("log() invalid 1st argument")?;
        Ok(Box::new(move || {
            let a = a.clone();
            Box::pin(async move {
                let a = I32ValueRef::from_node(a).unwrap();
                log_impl(a).await.into()
            })
        }))
    });
}

pub fn log(a: I32ValueRef) -> impl Future<Output = I32ValueRef> {
    // TODO decide if we want to schedule or execute directly
    // directly would be `add_impl(a, b)`
    let result = dynamic_call(&LOG_FUNCTION, vec![a.into()]).unwrap();
    return async { I32ValueRef::from_node(result.await).unwrap() };
}
