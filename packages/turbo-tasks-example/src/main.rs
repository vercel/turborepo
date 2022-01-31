#![feature(once_cell)]

use std::{env::current_dir, fs, thread::sleep, time::Duration};

use async_std::task::block_on;
use math::add;
use turbo_tasks::{
    viz::{GraphViz, Visualizable},
    Task, TurboTasks,
};
use turbo_tasks_fs::{
    read, DiskFileSystemRef, FileContentRef, FileSystemPathRef, FileSystemRef, PathInFileSystemRef,
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
            make_math().await;

            let content = ls().await;
            content.into()
        })
    }));
    // println!("{:#?}", task);
    block_on(task.wait_output());
    let mut graph_viz = GraphViz::new();
    task.visualize(&mut graph_viz);
    fs::write("graph.html", GraphViz::wrap_html(&graph_viz.to_string())).unwrap();
    // println!("{:#?}", task);
    sleep(Duration::from_secs(30));
}

#[turbo_tasks::function]
async fn make_math() -> I32ValueRef {
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
    rz
}

#[turbo_tasks::function]
async fn ls() -> FileContentRef {
    let disk_fs = DiskFileSystemRef::new(
        "project".to_string(),
        current_dir().unwrap().to_str().unwrap().to_string(),
    );
    // TODO add casts to Smart Pointers
    let fs = FileSystemRef::from_node(disk_fs.into()).unwrap();
    let path = PathInFileSystemRef::new("Cargo.toml".to_string());
    let file_ref = FileSystemPathRef::new(fs, path.clone());
    let content = read(file_ref).await;
    print_size(path, content.clone()).await
}

#[turbo_tasks::function]
async fn print_size(path: PathInFileSystemRef, content: FileContentRef) -> FileContentRef {
    println!(
        "Size of {}: {}",
        path.get().path,
        content.get().buffer.len()
    );
    Task::side_effect();
    content
}
