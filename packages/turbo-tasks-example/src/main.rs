#![feature(once_cell)]

use std::{env::current_dir, thread::sleep, time::Duration};

use async_std::task::block_on;
use math::add;
use turbo_tasks::TurboTasks;
use turbo_tasks_fs::{
    read, DiskFileSystemRef, FileSystemPathRef, FileSystemRef, PathInFileSystemRef,
};

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

            let disk_fs = DiskFileSystemRef::new(
                "project".to_string(),
                current_dir().unwrap().to_str().unwrap().to_string(),
            );
            // TODO add casts to Smart Pointers
            let fs = FileSystemRef::from_node(disk_fs.into()).unwrap();
            let file_ref =
                FileSystemPathRef::new(fs, PathInFileSystemRef::new("Cargo.toml".to_string()));
            let content = read(file_ref).await;
            println!("Size of Cargo.toml: {}", content.get().buffer.len());
            content.into()
        })
    }));
    // println!("{:#?}", task);
    block_on(task.wait_output());
    // println!("{:#?}", task);
    sleep(Duration::from_secs(30));
}
