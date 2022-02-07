#![feature(trivial_bounds)]
#![feature(once_cell)]

use async_std::task::block_on;
use math::{add, max_new};
use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use random::RandomIdRef;
use std::{env::current_dir, fs, sync::mpsc::channel, thread, time::Duration};
use turbo_tasks::{
    viz::{GraphViz, Visualizable},
    Task, TurboTasks,
};

use turbo_tasks_fs::{
    DirectoryContent, DirectoryEntry, DiskFileSystemRef, FileContent, FileContentRef,
    FileSystemPathRef, FileSystemRef,
};

use crate::trace::{copy_all, CopyAllOptions};
use crate::{
    log::{log, LoggingOptionsRef},
    math::I32ValueRef,
    random::random,
};

mod log;
mod math;
mod random;
mod trace;
mod utils;

fn main() {
    let tt = TurboTasks::new();
    let task = tt.spawn_root_task(|| {
        Box::pin(async {
            // make_math().await;

            let root = current_dir().unwrap().to_str().unwrap().to_string();
            let disk_fs = DiskFileSystemRef::new("project".to_string(), root);

            // TODO add casts to Smart Pointers
            let fs = FileSystemRef::from_node(disk_fs.into()).unwrap();

            thread::spawn(|| {
                // Create a channel to receive the events.
                let (tx, rx) = channel();
                let root = current_dir().unwrap().to_str().unwrap().to_string();
                // Create a watcher object, delivering debounced events.
                // The notification back-end is selected based on the platform.
                let mut watcher = watcher(tx, Duration::from_secs(1)).unwrap();
                // Add a path to be watched. All files and directories at that path and
                // below will be monitored for changes.
                watcher
                    .watch(root.clone(), RecursiveMode::Recursive)
                    .unwrap();
                let disk_fs = DiskFileSystemRef::new("project".to_string(), root);
                loop {
                    match rx.recv() {
                        Ok(DebouncedEvent::Write(event)) => {
                            if let Some(invalidator) = disk_fs
                                .get()
                                .invalidators
                                .lock()
                                .unwrap()
                                .remove(&event.into_os_string().into_string().unwrap())
                            {
                                invalidator.invalidate()
                            }
                        }
                        Ok(_event) => unimplemented!(),
                        Err(e) => println!("watch error: {:?}", e),
                    }
                }
            });

            // ls(fs).await;
            let input = FileSystemPathRef::new(fs.clone(), "demo".to_string());
            let output = FileSystemPathRef::new(fs.clone(), "out".to_string());
            let entry = FileSystemPathRef::new(fs.clone(), "demo/index.txt".to_string());

            copy_all(
                entry,
                CopyAllOptions {
                    input_dir: input,
                    output_dir: output,
                }
                .into(),
            )
            .await;

            None
        })
    });
    block_on(task.wait_output());
    sleep(Duration::from_secs(30));

    // create a graph
    let mut graph_viz = GraphViz::new(false);

    // graph root node
    task.visualize(&mut graph_viz);

    // graph unconnected nodes
    tt.visualize(&mut graph_viz);

    // write HTML
    fs::write("graph.html", GraphViz::wrap_html(&graph_viz.to_string())).unwrap();
    println!("graph.html written");
}

#[turbo_tasks::function]
async fn make_math() {
    let r1 = random(RandomIdRef::new(Duration::from_secs(5), 4));
    let r2 = random(RandomIdRef::new(Duration::from_secs(7), 3));
    let r1 = r1.await;
    let max = max_new(r1.clone(), r2.await);
    let a = add(I32ValueRef::new(42), I32ValueRef::new(1));
    let b = add(I32ValueRef::new(2), I32ValueRef::new(3));
    let a = a.await;
    log(a.clone(), LoggingOptionsRef::new("value of a".to_string())).await;
    let max = max.await;
    let c = add(max.clone(), a);
    let d = add(max, b.await);
    let e = add(c.await, d.await);
    let r = add(r1, e.await);
    log(r.await, LoggingOptionsRef::new("value of r".to_string())).await;
}

#[turbo_tasks::function]
async fn ls(fs: FileSystemRef) {
    let directory_ref = FileSystemPathRef::new(fs, ".".to_string());
    print_sizes(directory_ref.clone()).await;
}

#[turbo_tasks::function]
async fn print_sizes(directory: FileSystemPathRef) {
    let content = directory.clone().read_dir().await;
    match &*content.get() {
        DirectoryContent::Entries(entries) => {
            for entry in entries.iter() {
                match &*entry.get() {
                    DirectoryEntry::File(path) => {
                        print_size(path.clone(), path.clone().read().await).await;
                    }
                    DirectoryEntry::Directory(path) => {
                        print_sizes(path.clone()).await;
                    }
                    _ => {}
                }
            }
        }
        DirectoryContent::NotFound => {
            println!("{}: not found", directory.get().path);
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
