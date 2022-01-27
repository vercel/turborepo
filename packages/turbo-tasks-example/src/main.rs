#![feature(once_cell)]

use std::{thread::sleep, time::Duration};

use async_std::task::block_on;
use math::add;
use turbo_tasks::TurboTasks;

use crate::{
    log::{log, LoggingOptionsRef},
    math::I32ValueRef,
    random::random,
};

mod log;
mod math;
mod random;

fn main() {
    let tt = TurboTasks::new();
    let task = tt.spawn_root_task(Box::new(|| {
        Box::pin(async {
            let a = I32ValueRef::new(42);
            let b = I32ValueRef::new(2);
            let c = I32ValueRef::new(7);
            let r = random().await;
            let x = add(a, b.clone());
            let y = add(b, c);
            let (x, y) = (x.await, y.await);
            let z = add(x.clone(), y.clone());
            let rz = add(r, y);
            let z = z.await;
            let rz = rz.await;
            log(x, LoggingOptionsRef::new("value of x".to_string())).await;
            log(z, LoggingOptionsRef::new("value of z".to_string())).await;
            log(
                rz.clone(),
                LoggingOptionsRef::new("value of rz".to_string()),
            )
            .await;
            rz.into()
        })
    }));
    println!("{:#?}", task);
    block_on(task.wait_output());
    println!("{:#?}", task);
    sleep(Duration::from_secs(30));
}
