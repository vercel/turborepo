use anyhow::{anyhow, Context};
use lazy_static::lazy_static;
use std::future::Future;
use turbo_tasks::{schedule_child, NativeFunctionStaticRef, Task};

use crate::math::I32ValueRef;

pub async fn log_impl(a: I32ValueRef) -> I32ValueRef {
    println!("{}", a.get().value);
    Task::side_effect();
    a
}

// TODO autogenerate that
lazy_static! {
    static ref LOG_FUNCTION: NativeFunctionStaticRef = NativeFunctionStaticRef::new(|inputs| {
        if inputs.len() > 1 {
            return Err(anyhow!("add() called with too many arguments"));
        }
        let mut iter = inputs.into_iter();
        let a = iter
            .next()
            .ok_or_else(|| anyhow!("add() first argument missing"))?;
        I32ValueRef::verify(&a).context("add() invalid 1st argument")?;
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
    let result = schedule_child(&LOG_FUNCTION, vec![a.into()]);
    return async { I32ValueRef::from_node(result.await).unwrap() };
}
