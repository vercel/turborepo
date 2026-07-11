use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime},
};
#[cfg(unix)]
use std::{
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow};
#[cfg(unix)]
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use rand::Rng;
use serde::{Deserialize, Serialize};

const STALE_LOCK_THRESHOLD: Duration = Duration::from_secs(10);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const LOCK_RETRY_BASE: Duration = Duration::from_millis(10);
const LOCK_RETRY_CAP: Duration = Duration::from_millis(100);

pub const FILE_MODE: u32 = 0o644;
pub const DIR_MODE: u32 = 0o755;

/// A route as represented in `routes.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub hostname: String,
    pub port: u16,
    pub pid: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_https_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_funnel: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ngrok_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ngrok_pid: Option<i32>,
}

impl Route {
    pub fn new(hostname: impl Into<String>, port: u16, pid: i32) -> Self {
        Self {
            hostname: hostname.into(),
            port,
            pid,
            tailscale_url: None,
            tailscale_https_port: None,
            tailscale_funnel: None,
            ngrok_url: None,
            ngrok_pid: None,
        }
    }
}

/// A metadata patch uses two levels of `Option`: `None` leaves a field alone,
/// `Some(None)` removes it, and `Some(Some(value))` sets it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteMetadata {
    pub tailscale_url: Option<Option<String>>,
    pub tailscale_https_port: Option<Option<u16>>,
    pub tailscale_funnel: Option<Option<bool>>,
    pub ngrok_url: Option<Option<String>>,
    pub ngrok_pid: Option<Option<i32>>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "\"{hostname}\" is already registered by a running process (PID {existing_pid}). Use --force \
     to override."
)]
pub struct RouteConflict {
    pub hostname: String,
    pub existing_pid: i32,
}

type WarningHandler = Arc<dyn Fn(&str) + Send + Sync>;

/// Disk-backed route registry compatible with Portless 0.15.1.
pub struct RouteStore {
    pub dir: PathBuf,
    routes_path: PathBuf,
    lock_path: PathBuf,
    pub pid_path: PathBuf,
    pub port_file_path: PathBuf,
    on_warning: Option<WarningHandler>,
}

impl RouteStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self::with_warning_handler(dir, None)
    }

    pub fn with_warning(
        dir: impl Into<PathBuf>,
        on_warning: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self::with_warning_handler(dir, Some(Arc::new(on_warning)))
    }

    fn with_warning_handler(dir: impl Into<PathBuf>, on_warning: Option<WarningHandler>) -> Self {
        let dir = dir.into();
        Self {
            routes_path: dir.join("routes.json"),
            lock_path: dir.join("routes.lock"),
            pid_path: dir.join("proxy.pid"),
            port_file_path: dir.join("proxy.port"),
            dir,
            on_warning,
        }
    }

    pub fn ensure_dir(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        // As in the TypeScript implementation, chmod/chown are best effort:
        // an existing shared state directory may belong to another user.
        #[cfg(unix)]
        let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(DIR_MODE));
        fix_ownership(&self.dir);
        Ok(())
    }

    pub fn routes_path(&self) -> &Path {
        &self.routes_path
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    fn warn(&self, message: String) {
        if let Some(handler) = &self.on_warning {
            handler(&message);
        }
    }

    fn acquire_lock(&self) -> bool {
        let started = Instant::now();
        let mut delay = LOCK_RETRY_BASE;
        while started.elapsed() < LOCK_TIMEOUT {
            match fs::create_dir(&self.lock_path) {
                Ok(()) => return true,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    match fs::metadata(&self.lock_path)
                        .and_then(|metadata| metadata.modified())
                        .and_then(|modified| {
                            SystemTime::now()
                                .duration_since(modified)
                                .map_err(io::Error::other)
                        }) {
                        Ok(age) if age > STALE_LOCK_THRESHOLD => {
                            let _ = fs::remove_dir_all(&self.lock_path);
                            continue;
                        }
                        Err(_) => continue,
                        _ => {}
                    }
                    let upper = u64::try_from(delay.as_millis()).unwrap_or(0);
                    let jitter = if upper == 0 {
                        0
                    } else {
                        rand::rng().random_range(0..upper)
                    };
                    thread::sleep(delay + Duration::from_millis(jitter));
                    delay = (delay * 2).min(LOCK_RETRY_CAP);
                }
                Err(_) => return false,
            }
        }
        false
    }

    fn release_lock(&self) {
        let _ = fs::remove_dir_all(&self.lock_path);
    }

    fn process_is_alive(pid: i32) -> bool {
        process_is_alive(pid)
    }

    fn parse_routes(&self) -> Vec<Route> {
        let raw = match fs::read_to_string(&self.routes_path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                self.warn(format!("Could not read routes file: {error}"));
                return Vec::new();
            }
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(parsed) => parsed,
            Err(_) => {
                self.warn(format!(
                    "Corrupted routes file (invalid JSON): {}",
                    self.routes_path.display()
                ));
                return Vec::new();
            }
        };
        let Some(entries) = parsed.as_array() else {
            self.warn(format!(
                "Corrupted routes file (expected array): {}",
                self.routes_path.display()
            ));
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
            .collect()
    }

    /// Loads live routes. This never writes cleanup back without a lock.
    pub fn load_routes(&self) -> Vec<Route> {
        self.load_routes_inner(false)
    }

    fn load_routes_inner(&self, persist_cleanup: bool) -> Vec<Route> {
        let routes = self.parse_routes();
        let alive: Vec<_> = routes
            .iter()
            .filter(|route| route.pid == 0 || Self::process_is_alive(route.pid))
            .cloned()
            .collect();
        if persist_cleanup && alive.len() != routes.len() {
            // Portless treats cleanup write failures as non-fatal.
            let _ = self.save_routes(&alive);
        }
        alive
    }

    /// Loads routes without stale-PID filtering, for prune operations.
    pub fn load_routes_raw(&self) -> Vec<Route> {
        self.parse_routes()
    }

    fn save_routes(&self, routes: &[Route]) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(routes)?;
        fs::write(&self.routes_path, bytes)
            .with_context(|| format!("failed to write {}", self.routes_path.display()))?;
        #[cfg(unix)]
        fs::set_permissions(&self.routes_path, fs::Permissions::from_mode(FILE_MODE))
            .with_context(|| format!("failed to chmod {}", self.routes_path.display()))?;
        fix_ownership(&self.routes_path);
        Ok(())
    }

    /// Adds or replaces a route and returns the PID sent SIGTERM by `force`.
    pub fn add_route(
        &self,
        hostname: impl Into<String>,
        port: u16,
        pid: i32,
        force: bool,
    ) -> Result<Option<i32>> {
        self.ensure_dir()?;
        if !self.acquire_lock() {
            return Err(anyhow!("Failed to acquire route lock"));
        }
        let hostname = hostname.into();
        let result = (|| {
            let routes = self.load_routes_inner(true);
            let existing = routes.iter().find(|route| route.hostname == hostname);
            let mut killed_pid = None;
            if let Some(existing) =
                existing.filter(|route| route.pid != pid && Self::process_is_alive(route.pid))
            {
                if !force {
                    return Err(RouteConflict {
                        hostname: hostname.clone(),
                        existing_pid: existing.pid,
                    }
                    .into());
                }
                if terminate_process(existing.pid) {
                    killed_pid = Some(existing.pid);
                }
            }
            let mut updated: Vec<_> = routes
                .into_iter()
                .filter(|route| route.hostname != hostname)
                .collect();
            updated.push(Route::new(hostname, port, pid));
            self.save_routes(&updated)?;
            Ok(killed_pid)
        })();
        self.release_lock();
        result
    }

    pub fn prune_stale_routes(&self) -> Result<Vec<Route>> {
        self.ensure_dir()?;
        if !self.acquire_lock() {
            return Err(anyhow!("Failed to acquire route lock"));
        }
        let result = (|| {
            let (alive, stale): (Vec<_>, Vec<_>) = self
                .load_routes_raw()
                .into_iter()
                .partition(|route| route.pid == 0 || Self::process_is_alive(route.pid));
            if !stale.is_empty() {
                self.save_routes(&alive)?;
            }
            Ok(stale)
        })();
        self.release_lock();
        result
    }

    pub fn update_route(&self, hostname: &str, patch: RouteMetadata) -> Result<()> {
        self.ensure_dir()?;
        if !self.acquire_lock() {
            return Err(anyhow!("Failed to acquire route lock"));
        }
        let result = (|| {
            let mut routes = self.load_routes_inner(true);
            let Some(route) = routes.iter_mut().find(|route| route.hostname == hostname) else {
                return Ok(());
            };
            apply_patch(&mut route.tailscale_url, patch.tailscale_url);
            apply_patch(&mut route.tailscale_https_port, patch.tailscale_https_port);
            apply_patch(&mut route.tailscale_funnel, patch.tailscale_funnel);
            apply_patch(&mut route.ngrok_url, patch.ngrok_url);
            apply_patch(&mut route.ngrok_pid, patch.ngrok_pid);
            self.save_routes(&routes)
        })();
        self.release_lock();
        result
    }

    /// Removes `hostname`; with `owner_pid`, removal occurs only if it still
    /// belongs to that PID (preventing an old forced-out process from deleting
    /// its replacement's route during exit cleanup).
    pub fn remove_route(&self, hostname: &str, owner_pid: Option<i32>) -> Result<()> {
        self.ensure_dir()?;
        if !self.acquire_lock() {
            return Err(anyhow!("Failed to acquire route lock"));
        }
        let result = {
            let routes: Vec<_> = self
                .load_routes_inner(true)
                .into_iter()
                .filter(|route| {
                    route.hostname != hostname || owner_pid.is_some_and(|owner| route.pid != owner)
                })
                .collect();
            self.save_routes(&routes)
        };
        self.release_lock();
        result
    }
}

fn apply_patch<T>(field: &mut Option<T>, patch: Option<Option<T>>) {
    if let Some(value) = patch {
        *field = value;
    }
}

#[cfg(unix)]
fn process_is_alive(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}

#[cfg(windows)]
fn process_is_alive(pid: i32) -> bool {
    windows_process::is_alive(pid)
}

#[cfg(unix)]
fn terminate_process(pid: i32) -> bool {
    kill(Pid::from_raw(pid), Signal::SIGTERM).is_ok()
}

#[cfg(windows)]
fn terminate_process(pid: i32) -> bool {
    windows_process::terminate(pid)
}

#[cfg(windows)]
mod windows_process {
    use std::ffi::c_void;

    type Handle = *mut c_void;

    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "CloseHandle"]
        fn close_handle(handle: Handle) -> i32;
        #[link_name = "GetExitCodeProcess"]
        fn get_exit_code_process(process: Handle, exit_code: *mut u32) -> i32;
        #[link_name = "OpenProcess"]
        fn open_process(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        #[link_name = "TerminateProcess"]
        fn terminate_process(process: Handle, exit_code: u32) -> i32;
    }

    pub(super) fn is_alive(pid: i32) -> bool {
        let Ok(pid) = u32::try_from(pid) else {
            return false;
        };
        // SAFETY: OpenProcess is called with a PID value and no borrowed pointers.
        let handle = unsafe { open_process(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0;
        // SAFETY: `handle` is valid until it is closed below and `exit_code` is
        // writable.
        let succeeded = unsafe { get_exit_code_process(handle, &mut exit_code) } != 0;
        // SAFETY: `handle` was returned by OpenProcess and is closed exactly once.
        unsafe { close_handle(handle) };
        succeeded && exit_code == STILL_ACTIVE
    }

    pub(super) fn terminate(pid: i32) -> bool {
        let Ok(pid) = u32::try_from(pid) else {
            return false;
        };
        // SAFETY: OpenProcess is called with a PID value and no borrowed pointers.
        let handle = unsafe { open_process(PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            return false;
        }
        // Node's process.kill maps SIGTERM to TerminateProcess on Windows.
        // SAFETY: `handle` has PROCESS_TERMINATE access and remains valid here.
        let succeeded = unsafe { terminate_process(handle, 1) } != 0;
        // SAFETY: `handle` was returned by OpenProcess and is closed exactly once.
        unsafe { close_handle(handle) };
        succeeded
    }
}

#[cfg(not(any(unix, windows)))]
fn process_is_alive(_pid: i32) -> bool {
    false
}

#[cfg(not(any(unix, windows)))]
fn terminate_process(_pid: i32) -> bool {
    false
}

#[cfg(unix)]
fn fix_ownership(path: &Path) {
    let uid = std::env::var("SUDO_UID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok());
    let gid = std::env::var("SUDO_GID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok());
    let owner = match (uid, gid) {
        (Some(uid), Some(gid)) => Some(format!("{uid}:{gid}")),
        (Some(uid), None) => Some(uid.to_string()),
        (None, Some(gid)) => Some(format!(":{gid}")),
        (None, None) => None,
    };
    if let Some(owner) = owner {
        let _ = Command::new("chown")
            .arg(owner)
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(not(unix))]
fn fix_ownership(_path: &Path) {}

#[cfg(test)]
mod tests {
    use std::{
        process::{self, Command},
        sync::Mutex,
    };

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn json_schema_and_metadata_patch_match_typescript() {
        let temp = TempDir::new().expect("tempdir");
        let store = RouteStore::new(temp.path());
        store
            .add_route("app.localhost", 4123, process::id() as i32, false)
            .expect("add route");
        store
            .update_route(
                "app.localhost",
                RouteMetadata {
                    tailscale_url: Some(Some("https://box.ts.net".into())),
                    ngrok_pid: Some(Some(42)),
                    ..RouteMetadata::default()
                },
            )
            .expect("update");
        let json = fs::read_to_string(store.routes_path()).expect("read");
        assert!(json.contains("\"tailscaleUrl\""));
        assert!(json.contains("\"ngrokPid\""));
        assert!(!json.contains("tailscale_url"));
        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(store.routes_path())
                    .expect("route metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                FILE_MODE
            );
            assert_eq!(
                fs::metadata(temp.path())
                    .expect("dir metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                DIR_MODE
            );
        }

        store
            .update_route(
                "app.localhost",
                RouteMetadata {
                    tailscale_url: Some(None),
                    ..RouteMetadata::default()
                },
            )
            .expect("clear");
        assert_eq!(store.load_routes()[0].tailscale_url, None);
    }

    #[test]
    fn stale_pids_are_filtered_and_cleaned_under_mutation_lock() {
        let temp = TempDir::new().expect("tempdir");
        let store = RouteStore::new(temp.path());
        store.ensure_dir().expect("dir");
        let routes = vec![
            Route::new("live.localhost", 4001, process::id() as i32),
            Route::new("dead.localhost", 4002, 999_999_999),
        ];
        store.save_routes(&routes).expect("save");
        assert_eq!(store.load_routes().len(), 1);
        assert_eq!(store.load_routes_raw().len(), 2);
        store
            .add_route("new.localhost", 4003, process::id() as i32, false)
            .expect("add");
        assert_eq!(store.load_routes_raw().len(), 2);
    }

    #[test]
    fn owner_pid_prevents_old_owner_cleanup() {
        let temp = TempDir::new().expect("tempdir");
        let store = RouteStore::new(temp.path());
        store
            .add_route("app.localhost", 4001, process::id() as i32, false)
            .expect("add");
        store
            .remove_route("app.localhost", Some(12345))
            .expect("owner-safe remove");
        assert_eq!(store.load_routes().len(), 1);
        store
            .remove_route("app.localhost", Some(process::id() as i32))
            .expect("owner remove");
        assert!(store.load_routes().is_empty());
    }

    #[test]
    fn stale_lock_is_recovered_and_corruption_warns() {
        let temp = TempDir::new().expect("tempdir");
        let store = RouteStore::new(temp.path());
        store.ensure_dir().expect("dir");
        fs::create_dir(store.lock_path()).expect("lock");
        let lock = fs::File::open(store.lock_path()).expect("open lock");
        lock.set_times(
            fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(11)),
        )
        .expect("mtime");
        store
            .add_route("app.localhost", 4001, process::id() as i32, false)
            .expect("recover lock");

        let errors = Mutex::new(Vec::new());
        let warned = RouteStore::with_warning(temp.path(), move |message| {
            errors.lock().expect("mutex").push(message.to_owned());
        });
        fs::write(warned.routes_path(), "not json").expect("corrupt");
        assert!(warned.load_routes().is_empty());
    }

    #[test]
    fn live_conflict_errors_and_force_terminates_process() {
        let temp = TempDir::new().expect("tempdir");
        let store = RouteStore::new(temp.path());
        #[cfg(unix)]
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleeper");
        #[cfg(windows)]
        let mut child = Command::new("cmd.exe")
            .args(["/d", "/s", "/c", "ping -n 30 127.0.0.1 >NUL"])
            .spawn()
            .expect("spawn sleeper");
        let child_pid = child.id() as i32;
        store
            .add_route("app.localhost", 4001, child_pid, false)
            .expect("register child");

        let error = store
            .add_route("app.localhost", 4002, process::id() as i32, false)
            .expect_err("must conflict");
        let conflict = error
            .downcast_ref::<RouteConflict>()
            .expect("route conflict type");
        assert_eq!(conflict.existing_pid, child_pid);

        assert_eq!(
            store
                .add_route("app.localhost", 4002, process::id() as i32, true)
                .expect("force"),
            Some(child_pid)
        );
        let status = child.wait().expect("wait sleeper");
        assert!(!status.success());
        assert_eq!(store.load_routes()[0].pid, process::id() as i32);
    }
}
