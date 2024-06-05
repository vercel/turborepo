use std::{fs, panic, path::PathBuf, sync::Mutex};

use mockito::{mock, Mock};
use once_cell::sync::Lazy;

use crate::Package;

static LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::default());

pub(crate) fn within_test_dir(f: fn(path: PathBuf)) {
    // To avoid problems when working in parallel with the file system
    let mutex = LOCK.lock().expect("unlock mutex");

    let test_dir: PathBuf = std::env::temp_dir().join("update-informer-test");
    fs::create_dir_all(&test_dir).expect("create test dir");

    let result = panic::catch_unwind(|| {
        let path: PathBuf = test_dir.join("crates-repo");

        f(path);
    });

    fs::remove_dir_all(test_dir).expect("remove test dir");

    if let Err(e) = result {
        // If we panic while holding the mutex, it becomes poisoned, and future
        // tests fail in a unexpected way. So release lock before the panic.
        drop(mutex);
        panic::resume_unwind(e);
    }
}

#[cfg(feature = "crates")]
pub(crate) fn mock_crates(pkg: &Package, status: usize, data_path: &str) -> (Mock, String) {
    let mock_path = format!("/api/v1/crates/{}/versions", pkg);
    let data = fs::read_to_string(data_path).expect("read file to string");

    (mock_http(&mock_path, status, &data), data)
}

#[cfg(feature = "github")]
pub(crate) fn mock_github(pkg: &Package, status: usize, data_path: &str) -> (Mock, String) {
    let mock_path = format!("/repos/{}/releases/latest", pkg);
    let data = fs::read_to_string(data_path).expect("read file to string");

    (mock_http(&mock_path, status, &data), data)
}

#[cfg(feature = "npm")]
pub(crate) fn mock_npm(pkg: &Package, status: usize, data_path: &str) -> (Mock, String) {
    let mock_path = format!("/{}/latest", pkg);
    let data = fs::read_to_string(data_path).expect("read file to string");

    (mock_http(&mock_path, status, &data), data)
}

#[cfg(feature = "pypi")]
pub(crate) fn mock_pypi(pkg: &Package, status: usize, data_path: &str) -> (Mock, String) {
    let mock_path = format!("/pypi/{}/json", pkg);
    let data = fs::read_to_string(data_path).expect("read file to string");

    (mock_http(&mock_path, status, &data), data)
}

pub(crate) fn mock_http(path: &str, status: usize, body: &str) -> Mock {
    mock("GET", path)
        .with_status(status)
        .with_header("Content-Type", "application/json; charset=utf-8")
        .with_body(body)
        .create()
}
