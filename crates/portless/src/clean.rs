use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

/// Exact filename allowlist created under a Portless 0.15.1 state directory.
pub const PORTLESS_STATE_FILES: &[&str] = &[
    "routes.json",
    "routes.lock",
    "proxy.pid",
    "proxy.port",
    "proxy.log",
    "proxy.tls",
    "proxy.custom-cert",
    "proxy.tld",
    "proxy.tlds",
    "proxy.lan",
    "ca-key.pem",
    "ca.pem",
    "server-key.pem",
    "server.pem",
    "server.csr",
    "server-ext.cnf",
    "ca.srl",
];

pub const HOST_CERTS_DIR: &str = "host-certs";

pub fn user_state_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = env::var_os("HOME");
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".portless"))
}

pub fn legacy_system_state_dir() -> PathBuf {
    #[cfg(windows)]
    {
        env::temp_dir().join("portless")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/tmp/portless")
    }
}

/// Returns unique, existing user, legacy, and environment-selected state dirs.
pub fn collect_state_dirs_for_cleanup() -> Vec<PathBuf> {
    let candidates = [
        user_state_dir(),
        Some(legacy_system_state_dir()),
        env::var_os("PORTLESS_STATE_DIR")
            .and_then(|value| value.into_string().ok())
            .and_then(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
            }),
    ];
    let mut seen = HashSet::new();
    let mut dirs = Vec::new();
    for path in candidates.into_iter().flatten() {
        let resolved = resolve_path(&path);
        if resolved.exists() && seen.insert(resolved.clone()) {
            dirs.push(resolved);
        }
    }
    dirs
}

/// Best-effort cleanup. Unknown files and the state directory itself remain.
pub fn remove_portless_state_files(dir: impl AsRef<Path>) {
    let dir = dir.as_ref();
    for filename in PORTLESS_STATE_FILES {
        let path = dir.join(filename);
        // `routes.lock` is normally a directory, while all other allowlisted
        // entries are files. Handle either shape without broadening cleanup.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::IsADirectory => {
                let _ = fs::remove_dir_all(&path);
            }
            Err(_) => {}
        }
    }
    let _ = fs::remove_dir_all(dir.join(HOST_CERTS_DIR));
}

fn resolve_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tempfile::TempDir;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn removes_only_allowlisted_state() {
        let temp = TempDir::new().expect("tempdir");
        fs::write(temp.path().join("routes.json"), "[]").expect("route");
        fs::write(temp.path().join("proxy.custom-cert"), "1").expect("marker");
        fs::write(temp.path().join("proxy.tlds"), "localhost\ntest\n").expect("tlds");
        fs::write(temp.path().join("user-notes.txt"), "keep me").expect("notes");
        fs::create_dir(temp.path().join("routes.lock")).expect("lock");
        fs::create_dir(temp.path().join(HOST_CERTS_DIR)).expect("host certs");
        fs::write(temp.path().join(HOST_CERTS_DIR).join("x.pem"), "x").expect("cert");

        remove_portless_state_files(temp.path());

        assert!(!temp.path().join("routes.json").exists());
        assert!(!temp.path().join("routes.lock").exists());
        assert!(!temp.path().join("proxy.custom-cert").exists());
        assert!(!temp.path().join("proxy.tlds").exists());
        assert!(!temp.path().join(HOST_CERTS_DIR).exists());
        assert_eq!(
            fs::read_to_string(temp.path().join("user-notes.txt")).expect("notes"),
            "keep me"
        );
    }

    #[test]
    fn collect_includes_existing_environment_dir_and_deduplicates() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let temp = TempDir::new().expect("tempdir");
        // SAFETY: environment access is serialized within this module.
        unsafe { env::set_var("PORTLESS_STATE_DIR", temp.path()) };
        let dirs = collect_state_dirs_for_cleanup();
        // SAFETY: environment access is serialized within this module.
        unsafe { env::remove_var("PORTLESS_STATE_DIR") };
        assert!(dirs.contains(&temp.path().to_path_buf()));
        let unique: HashSet<_> = dirs.iter().collect();
        assert_eq!(unique.len(), dirs.len());
    }

    #[test]
    fn missing_paths_are_non_fatal() {
        let temp = TempDir::new().expect("tempdir");
        remove_portless_state_files(temp.path());
    }
}
