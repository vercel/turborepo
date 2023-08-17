#[derive(Default)]
struct DiskWatcher {
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// Keeps track of which directories are currently watched. This is only
    /// used on a OS that doesn't support recursive watching.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    watching: dashmap::DashSet<PathBuf>,
}

impl DiskWatcher {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn restore_if_watching(&self, dir_path: &Path, root_path: &Path) -> Result<()> {
        if self.watching.contains(dir_path) {
            let mut watcher = self.watcher.lock().unwrap();
            self.start_watching(&mut watcher, dir_path, root_path)?;
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn ensure_watching(&self, dir_path: &Path, root_path: &Path) -> Result<()> {
        if self.watching.contains(dir_path) {
            return Ok(());
        }
        let mut watcher = self.watcher.lock().unwrap();
        if self.watching.insert(dir_path.to_path_buf()) {
            self.start_watching(&mut watcher, dir_path, root_path)?;
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn start_watching(
        &self,
        watcher: &mut std::sync::MutexGuard<Option<RecommendedWatcher>>,
        dir_path: &Path,
        root_path: &Path,
    ) -> Result<()> {
        if let Some(watcher) = watcher.as_mut() {
            let mut path = dir_path;
            while let Err(err) = watcher.watch(path, RecursiveMode::NonRecursive) {
                if path == root_path {
                    return Err(err).context(format!(
                        "Unable to watch {} (tried up to {})",
                        dir_path.display(),
                        path.display()
                    ));
                }
                let Some(parent_path) = path.parent() else {
                    return Err(err).context(format!(
                        "Unable to watch {} (tried up to {})",
                        dir_path.display(),
                        path.display()
                    ));
                };
                path = parent_path;
            }
        }
        Ok(())
    }
}
