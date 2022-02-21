#![feature(trivial_bounds)]
#![feature(once_cell)]
#![feature(into_future)]

use async_std::task::{block_on, spawn};
use math::{add, max_new};
use random::RandomIdRef;
use std::time::Instant;
use std::{env::current_dir, fs, thread, time::Duration};
use turbo_pack::emit;
use turbo_pack::module::Module;
use turbo_tasks::viz::GraphViz;
use turbo_tasks::{SlotRef, TurboTasks};

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
    block_on(async {
        let start = Instant::now();

        let task = tt.spawn_root_task(|| {
            Box::pin(async {
                // make_math().await;

                let root = current_dir().unwrap().to_str().unwrap().to_string();
                let disk_fs = DiskFileSystemRef::new("project".to_string(), root);

                // Smart Pointer cast
                let fs: FileSystemRef = disk_fs.into();

                // ls(fs).await;
                let input = FileSystemPathRef::new(fs.clone(), "demo".to_string());
                let output = FileSystemPathRef::new(fs.clone(), "out".to_string());
                let entry = FileSystemPathRef::new(fs.clone(), "demo/index.js".to_string());

                emit(Module { path: entry }.into(), input, output);

                // copy_all(
                //     entry,
                //     CopyAllOptions {
                //         input_dir: input,
                //         output_dir: output,
                //     }
                //     .into(),
                // )
                // .await;

                SlotRef::Nothing
            })
        });
        task.wait_done().await;
        println!("done in {} ms", start.elapsed().as_millis());

        for task in tt.cached_tasks_iter() {
            task.reset_executions();
        }

        let mut i = 10;
        loop {
            // if i == 0 {
            //     println!("writing graph.html...");
            //     // create a graph
            //     let mut graph_viz = GraphViz::new();

            //     // graph root node
            //     graph_viz.add_task(&task);

            //     // graph tasks in cache
            //     for task in tt.cached_tasks_iter() {
            //         graph_viz.add_task(&task);
            //     }

            //     // prettify graph
            //     graph_viz.merge_edges();
            //     graph_viz.drop_unchanged_slots();
            //     graph_viz.skip_loney_resolve();
            //     graph_viz.drop_inactive_tasks();

            //     // write HTML
            //     fs::write("graph.html", GraphViz::wrap_html(&graph_viz.get_graph())).unwrap();
            //     println!("graph.html written");
            //     i = 10;
            // }

            task.root_wait_dirty().await;
            let start = Instant::now();
            task.wait_done().await;
            println!("updated in {} µs", start.elapsed().as_micros());
            i -= 1;
        }
    });
}

#[turbo_tasks::function]
fn make_math() {
    let r1 = random(RandomIdRef::new(Duration::from_secs(5), 4));
    let r2 = random(RandomIdRef::new(Duration::from_secs(7), 3));
    let max = max_new(r1.clone(), r2);
    let a = add(I32ValueRef::new(42), I32ValueRef::new(1));
    let b = add(I32ValueRef::new(2), I32ValueRef::new(3));
    log(a.clone(), LoggingOptionsRef::new("value of a".to_string()));
    let c = add(max.clone(), a);
    let d = add(max, b);
    let e = add(c, d);
    let r = add(r1, e);
    log(r, LoggingOptionsRef::new("value of r".to_string()));
}

#[turbo_tasks::function]
async fn ls(fs: FileSystemRef) {
    let directory_ref = FileSystemPathRef::new(fs, ".".to_string());
    print_sizes(directory_ref.clone());
}

#[turbo_tasks::function]
async fn print_sizes(directory: FileSystemPathRef) {
    let content = directory.clone().read_dir();
    match &*content.await {
        DirectoryContent::Entries(entries) => {
            for entry in entries.iter() {
                match &*entry.get().await {
                    DirectoryEntry::File(path) => {
                        print_size(path.clone(), path.clone().read());
                    }
                    DirectoryEntry::Directory(path) => {
                        print_sizes(path.clone());
                    }
                    _ => {}
                }
            }
        }
        DirectoryContent::NotFound => {
            println!("{}: not found", directory.await.path);
        }
    };
}

#[turbo_tasks::function]
async fn print_size(path: FileSystemPathRef, content: FileContentRef) {
    match &*content.await {
        FileContent::Content(buffer) => {
            println!("{:?}: Size {}", *path.await, buffer.len());
        }
        FileContent::NotFound => {
            println!("{:?}: not found", *path.await);
        }
    }
}
