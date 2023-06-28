use std::{fs::canonicalize, path::PathBuf};

use once_cell::sync::Lazy;

/// The turbo repo root. Should be used as the root when building with turbopack
/// against fixtures in this crate.
pub static REPO_ROOT: Lazy<String> =
    Lazy::new(|| get_repo_root_path().to_str().unwrap().to_string());

/// The repo's node_modules directory. This is where other node_modules
/// directories are canonicalized (realpath-ed) to
// pub static NODE_MODULES_PATH: Lazy<PathBuf> =
//     Lazy::new(|| get_repo_root_path().join("node_modules"));

fn get_repo_root_path() -> PathBuf {
    let package_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    canonicalize(package_root)
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}
