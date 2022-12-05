use std::{
    collections::{HashMap, HashSet},
    mem::take,
    path::PathBuf,
    sync::{
        mpsc::{channel, Receiver, RecvError, TryRecvError},
        Arc,
    },
};

use anyhow::Result;
use notify::{
    event::ModifyKind, recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode,
    Watcher,
};
use turbo_tasks::{spawn_thread, Invalidator};

use crate::{path_to_key, InvalidatorMap};

pub fn start_watching(
    root: PathBuf,
    invalidator_map: Arc<InvalidatorMap>,
    dir_invalidator_map: Arc<InvalidatorMap>,
) -> Result<RecommendedWatcher> {
    // Create a channel to receive the events.
    let (tx, rx) = channel();

    // Create a watcher object, the notification back-end is selected based on the
    // platform.
    let mut watcher = recommended_watcher(tx)?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(&root, RecursiveMode::Recursive)?;

    // We need to invalidate all reads that happened before watching
    // Best is to start_watching before starting to read
    for (_, invalidators) in take(&mut *invalidator_map.lock().unwrap()).into_iter() {
        invalidators.into_iter().for_each(|i| i.invalidate());
    }
    for (_, invalidators) in take(&mut *dir_invalidator_map.lock().unwrap()).into_iter() {
        invalidators.into_iter().for_each(|i| i.invalidate());
    }

    spawn_thread(move || watch(root, rx, invalidator_map, dir_invalidator_map));

    Ok(watcher)
}

fn watch(
    root: PathBuf,
    rx: Receiver<notify::Result<Event>>,
    invalidator_map: Arc<InvalidatorMap>,
    dir_invalidator_map: Arc<InvalidatorMap>,
) {
    let mut batched_invalidate_path = HashSet::new();
    let mut batched_invalidate_path_dir = HashSet::new();
    let mut batched_invalidate_path_and_children = HashSet::new();
    let mut batched_invalidate_path_and_children_dir = HashSet::new();

    loop {
        let mut event_res = rx.recv().map_err(|e| match e {
            RecvError => TryRecvError::Disconnected,
        });
        loop {
            let event = match event_res {
                Ok(Ok(event)) => event,
                // notify error
                Ok(Err(e)) => {
                    println!("watch error: {}", e);
                    if e.paths.is_empty() {
                        batched_invalidate_path_and_children.insert(path_to_key(&root));
                        batched_invalidate_path_and_children_dir.insert(path_to_key(&root));
                    } else {
                        let paths = Vec::from_iter(e.paths.iter().map(path_to_key));
                        batched_invalidate_path_and_children.extend(paths.clone());
                        batched_invalidate_path_and_children_dir.extend(paths);
                    }
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    // Sender has been disconnected
                    // which means DiskFileSystem has been dropped
                    // exit thread
                    println!("stopped watching {}", root.display());
                    return;
                }
                Err(TryRecvError::Empty) => {
                    break;
                }
            };

            let paths = Vec::from_iter(event.paths.iter().map(path_to_key));
            match event.kind {
                EventKind::Modify(ModifyKind::Data(_)) => {
                    batched_invalidate_path.extend(paths.into_iter());
                }
                EventKind::Create(_) | EventKind::Remove(_) => {
                    batched_invalidate_path_and_children.extend(paths.clone());
                    batched_invalidate_path_and_children_dir.extend(paths);
                    for parent in event.paths.iter().filter_map(|path| path.parent()) {
                        batched_invalidate_path_dir.insert(path_to_key(parent));
                    }
                }
                EventKind::Modify(ModifyKind::Name(_)) => {
                    batched_invalidate_path_and_children.extend(paths);
                    for parent in event.paths.iter().filter_map(|path| path.parent()) {
                        batched_invalidate_path_dir.insert(path_to_key(parent));
                    }
                }
                EventKind::Any | EventKind::Modify(ModifyKind::Any) => {
                    batched_invalidate_path.extend(paths.clone());
                    batched_invalidate_path_and_children.extend(paths.clone());
                    batched_invalidate_path_and_children_dir.extend(paths);
                    for parent in event.paths.iter().filter_map(|path| path.parent()) {
                        batched_invalidate_path_dir.insert(path_to_key(parent));
                    }
                }
                EventKind::Modify(ModifyKind::Metadata(_) | ModifyKind::Other)
                | EventKind::Access(_)
                | EventKind::Other => {
                    // ignored
                }
            }
            event_res = rx.try_recv();
        }
        {
            let mut invalidator_map = invalidator_map.lock().unwrap();
            invalidate_path(&mut invalidator_map, batched_invalidate_path.drain());
            invalidate_path_and_children_execute(
                &mut invalidator_map,
                &mut batched_invalidate_path_and_children,
            );
        }
        {
            let mut dir_invalidator_map = dir_invalidator_map.lock().unwrap();
            invalidate_path(
                &mut dir_invalidator_map,
                batched_invalidate_path_dir.drain(),
            );
            invalidate_path_and_children_execute(
                &mut dir_invalidator_map,
                &mut batched_invalidate_path_and_children_dir,
            );
        }
    }
}

fn invalidate_path(
    invalidator_map: &mut HashMap<String, HashSet<Invalidator>>,
    paths: impl Iterator<Item = String>,
) {
    for path in paths {
        if let Some(invalidators) = invalidator_map.remove(&path) {
            invalidators.into_iter().for_each(|i| i.invalidate());
        }
    }
}

fn invalidate_path_and_children_execute(
    invalidator_map: &mut HashMap<String, HashSet<Invalidator>>,
    paths: &mut HashSet<String>,
) {
    for (_, invalidators) in invalidator_map
        .drain_filter(|key, _| paths.iter().any(|path_key| key.starts_with(path_key)))
    {
        invalidators.into_iter().for_each(|i| i.invalidate());
    }
    paths.clear()
}
