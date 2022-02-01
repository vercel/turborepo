#![feature(once_cell)]

use async_std::task::block_on;
use math::add;
use std::thread::sleep;
use std::{env::current_dir, fs, time::Duration};
use turbo_tasks::{
    viz::{GraphViz, Visualizable},
    Task, TurboTasks,
};
use turbo_tasks_fs::{
    DirectoryContent, DirectoryEntry, DiskFileSystemRef, FileContent, FileContentRef,
    FileSystemPathRef, FileSystemRef, PathInFileSystemRef,
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
    let task = tt.spawn_root_task(|| {
        Box::pin(async {
            make_math().await;

            let disk_fs = DiskFileSystemRef::new(
                "project".to_string(),
                current_dir().unwrap().to_str().unwrap().to_string(),
            );
            // TODO add casts to Smart Pointers
            let fs = FileSystemRef::from_node(disk_fs.into()).unwrap();

            ls(fs).await;
            None
        })
    });
    // println!("{:#?}", task);
    // println!("{:#?}", task);
    sleep(Duration::from_secs(30));
    block_on(task.wait_output());
    let mut graph_viz = GraphViz::new();
    task.visualize(&mut graph_viz);
    fs::write("graph.html", GraphViz::wrap_html(&graph_viz.to_string())).unwrap();
}

#[turbo_tasks::function]
async fn make_math() {
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
}

#[turbo_tasks::function]
async fn ls(fs: FileSystemRef) {
    let path = PathInFileSystemRef::new(".".to_string());
    let directory_ref = FileSystemPathRef::new(fs, path.clone());
    print_sizes(directory_ref.clone()).await;
}

#[turbo_tasks::function]
async fn print_sizes(directory: FileSystemPathRef) {
    let content = directory.read_dir().await;
    match &*content.get() {
        DirectoryContent::Entries(entries) => {
            for entry in entries.iter() {
                match &*entry.get() {
                    DirectoryEntry::File(path) => {
                        print_size(path.clone(), path.read().await).await;
                    }
                    DirectoryEntry::Directory(path) => {
                        print_sizes(path.clone()).await;
                    }
                    _ => {}
                }
            }
        }
        DirectoryContent::NotFound => {
            println!("{}: not found", directory.get().path.get().path);
        }
    };
}

#[turbo_tasks::function]
async fn print_size(path: FileSystemPathRef, content: FileContentRef) {
    match &*content.get() {
        FileContent::Content(buffer) => {
            println!("{:?}: Size {}", *path.get(), buffer.len());
        }
        FileContent::NotFound => {
            println!("{:?}: not found", *path.get());
        }
    }
    Task::side_effect();
}
