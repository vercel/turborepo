use std::{
    collections::{HashMap, HashSet},
    fs,
    hash::Hasher,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use notify::{
    Event, EventHandler, EventKind, RecursiveMode,
    event::{CreateKind, ModifyKind, RemoveKind},
};
use xxhash_rust::xxh64::Xxh64;

use crate::{RepositoryIgnore, SubscriptionRegistry};

pub(super) struct ScopedPoller {
    roots: Arc<Mutex<HashMap<PathBuf, RecursiveMode>>>,
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ScopedPoller {
    pub(super) fn new<F: EventHandler>(
        mut handler: F,
        ignore: RepositoryIgnore,
        registry: Arc<SubscriptionRegistry>,
        cookie_dir: PathBuf,
    ) -> notify::Result<Self> {
        let roots = Arc::new(Mutex::new(HashMap::new()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let scan_roots = roots.clone();
        let scan_cancelled = cancelled.clone();
        let thread = thread::Builder::new()
            .name("turbo-scoped-poll".into())
            .spawn(move || {
                let mut previous: HashMap<PathBuf, u64> = HashMap::new();
                let mut initialized = false;
                while !scan_cancelled.load(Ordering::Acquire) && !ignore.is_cancelled() {
                    let roots = scan_roots
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clone();
                    if roots.is_empty() {
                        thread::park_timeout(Duration::from_secs(1));
                        continue;
                    }
                    let current = scan(
                        &roots,
                        &ignore,
                        &registry.physical_paths(),
                        &cookie_dir,
                        &scan_cancelled,
                    );
                    match current {
                        Ok(current) => {
                            for (path, value) in &current {
                                if !initialized && *path != cookie_dir.join(".turbo-cookie") {
                                    continue;
                                }
                                let kind = match previous.get(path) {
                                    None => EventKind::Create(CreateKind::Any),
                                    Some(old) if old != value => EventKind::Modify(ModifyKind::Any),
                                    Some(_) => continue,
                                };
                                handler.handle_event(Ok(Event::new(kind).add_path(path.clone())));
                            }
                            for path in previous.keys().filter(|path| !current.contains_key(*path))
                            {
                                handler.handle_event(Ok(Event::new(EventKind::Remove(
                                    RemoveKind::Any,
                                ))
                                .add_path(path.clone())));
                            }
                            previous = current;
                            initialized = true;
                        }
                        Err(error) if !scan_cancelled.load(Ordering::Acquire) => {
                            handler.handle_event(Err(error.into()))
                        }
                        Err(_) => break,
                    }
                    thread::park_timeout(Duration::from_secs(1));
                }
            })
            .map_err(notify::Error::io)?;
        Ok(Self {
            roots,
            cancelled,
            thread: Some(thread),
        })
    }

    pub(super) fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        self.roots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(path.to_path_buf(), mode);
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
        Ok(())
    }

    pub(super) fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.roots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(path);
        Ok(())
    }
}

impl Drop for ScopedPoller {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn scan(
    roots: &HashMap<PathBuf, RecursiveMode>,
    ignore: &RepositoryIgnore,
    explicit: &[PathBuf],
    cookie_dir: &Path,
    cancelled: &AtomicBool,
) -> std::io::Result<HashMap<PathBuf, u64>> {
    let mut pending: Vec<_> = roots
        .iter()
        .map(|(path, mode)| (path.clone(), *mode, true, false))
        .collect();
    pending.extend(
        explicit
            .iter()
            .filter(|path| path.starts_with(ignore.root()))
            .map(|path| (path.clone(), RecursiveMode::Recursive, true, true)),
    );
    pending.extend(
        ignore
            .control_paths()
            .into_iter()
            .map(|path| (path, RecursiveMode::NonRecursive, false, true)),
    );
    pending.push((
        cookie_dir.to_path_buf(),
        RecursiveMode::Recursive,
        true,
        true,
    ));
    let mut result = HashMap::new();
    let mut visited = HashSet::new();
    while let Some((path, mode, children, forced)) = pending.pop() {
        check_cancelled(cancelled)?;
        if ignore.is_cancelled() {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        if !visited.insert((
            path.clone(),
            mode == RecursiveMode::Recursive,
            children,
            forced,
        )) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !forced && !ignore.is_relevant(&path, metadata.is_dir()) {
            continue;
        }
        if metadata.is_dir() {
            result.insert(path.clone(), 0);
            if children {
                for entry in fs::read_dir(&path)? {
                    check_cancelled(cancelled)?;
                    pending.push((
                        entry?.path(),
                        mode,
                        mode == RecursiveMode::Recursive,
                        forced,
                    ));
                }
            }
        } else if metadata.is_file() {
            let mut file = match fs::File::open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let mut hash = Xxh64::new(0);
            let mut buffer = [0; 65536];
            loop {
                check_cancelled(cancelled)?;
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.write(&buffer[..count]);
            }
            result.insert(path, hash.finish());
        } else if metadata.file_type().is_symlink() {
            use std::hash::Hash;
            let mut hash = Xxh64::new(0);
            fs::read_link(&path)?.hash(&mut hash);
            result.insert(path, hash.finish());
        }
    }
    Ok(result)
}

fn check_cancelled(cancelled: &AtomicBool) -> std::io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "filesystem scan cancelled",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_content_controls_tracked_files_and_explicit_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.join(".gitignore"), "ignored/\n.turbo/\n").unwrap();
        fs::create_dir(root.join("ignored")).unwrap();
        for name in ["tracked", "explicit", "irrelevant"] {
            fs::write(root.join("ignored").join(name), "one").unwrap();
        }
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["add", "-f", "ignored/tracked"])
                .status()
                .unwrap()
                .success()
        );
        let cookie = root.join(".turbo/cookies");
        fs::create_dir_all(&cookie).unwrap();
        fs::write(cookie.join(".turbo-cookie"), "cookie").unwrap();
        let model = RepositoryIgnore::new(&root);
        let roots = HashMap::from([(root.clone(), RecursiveMode::Recursive)]);
        let explicit = root.join("ignored/explicit");
        let cancelled = AtomicBool::new(false);
        let first = scan(
            &roots,
            &model,
            std::slice::from_ref(&explicit),
            &cookie,
            &cancelled,
        )
        .unwrap();
        assert!(first.contains_key(&root.join("ignored/tracked")));
        assert!(first.contains_key(&explicit));
        assert!(!first.contains_key(&root.join("ignored/irrelevant")));
        assert!(first.contains_key(&root.join(".git/index")));
        assert!(first.contains_key(&cookie.join(".turbo-cookie")));
        fs::write(&explicit, "two").unwrap();
        let second = scan(
            &roots,
            &model,
            std::slice::from_ref(&explicit),
            &cookie,
            &cancelled,
        )
        .unwrap();
        assert_ne!(first[&explicit], second[&explicit]);
        let renamed = root.join("renamed");
        fs::rename(&explicit, &renamed).unwrap();
        let third = scan(
            &roots,
            &model,
            std::slice::from_ref(&explicit),
            &cookie,
            &cancelled,
        )
        .unwrap();
        assert!(!third.contains_key(&explicit));
        assert_eq!(third[&renamed], second[&explicit]);
        cancelled.store(true, Ordering::Release);
        assert_eq!(
            scan(&roots, &model, &[], &cookie, &cancelled)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::Interrupted
        );
    }
}
