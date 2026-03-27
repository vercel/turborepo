use std::{
    env,
    fs::{self},
    io,
    path::Path,
    process::Command,
    sync::OnceLock,
};

use sha2::{Digest, Sha256};

use crate::args::ParsedArgs;

/// Cache version — increment to invalidate all cached artifacts when the
/// wrapper's hashing algorithm changes.
const CACHE_VERSION: &str = "1";

static COMPILER_VERSION: OnceLock<String> = OnceLock::new();

/// Compute a deterministic cache key for a rustc invocation.
///
/// The key incorporates:
/// 1. Cache version (invalidates on wrapper changes)
/// 2. Compiler version (`rustc -vV` output)
/// 3. Source file content
/// 4. All extern crate file contents
/// 5. Sorted compilation flags (excluding location-only flags)
/// 6. Relevant environment variables (CARGO_* that affect compilation)
pub fn compute_cache_key(parsed: &ParsedArgs) -> io::Result<String> {
    let mut hasher = Sha256::new();

    hasher.update(CACHE_VERSION.as_bytes());
    hasher.update(b"\0");

    let compiler_version = get_compiler_version(&parsed.rustc_path)?;
    hasher.update(compiler_version.as_bytes());
    hasher.update(b"\0");

    if let Some(source) = &parsed.source_file {
        let content = fs::read(source)?;
        hasher.update(&content);
    }
    hasher.update(b"\0");

    hash_extern_crates(&mut hasher, parsed)?;
    hasher.update(b"\0");

    for arg in &parsed.hash_relevant_args {
        hasher.update(arg.as_bytes());
        hasher.update(b"\x01");
    }
    hasher.update(b"\0");

    hash_relevant_env_vars(&mut hasher);

    let result = hasher.finalize();
    Ok(hex::encode(result))
}

fn get_compiler_version(rustc_path: &Path) -> io::Result<String> {
    let cached = COMPILER_VERSION.get_or_init(|| {
        Command::new(rustc_path)
            .arg("-vV")
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default()
    });
    Ok(cached.clone())
}

fn hash_extern_crates(hasher: &mut Sha256, parsed: &ParsedArgs) -> io::Result<()> {
    let mut externs: Vec<_> = parsed
        .externs
        .iter()
        .filter_map(|ext| ext.path.as_ref().map(|p| (&ext.name, p)))
        .collect();
    externs.sort_by_key(|(name, _)| *name);

    for (name, path) in externs {
        hasher.update(name.as_bytes());
        hasher.update(b"=");

        match fs::read(path) {
            Ok(content) => {
                let file_hash = Sha256::digest(&content);
                hasher.update(file_hash);
            }
            Err(_) => {
                // If we can't read an extern crate, include the path
                // as a fallback so the key still changes if the path changes
                hasher.update(path.to_string_lossy().as_bytes());
            }
        }
        hasher.update(b"\x01");
    }

    Ok(())
}

/// Hash environment variables that affect Rust compilation output.
/// Follows sccache's approach: include CARGO_* vars (with exclusions)
/// and a few specific vars.
fn hash_relevant_env_vars(hasher: &mut Sha256) {
    let mut env_pairs: Vec<(String, String)> = env::vars()
        .filter(|(key, _)| is_hash_relevant_env_var(key))
        .collect();

    env_pairs.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (key, val) in &env_pairs {
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update(val.as_bytes());
        hasher.update(b"\x01");
    }
}

fn is_hash_relevant_env_var(key: &str) -> bool {
    if key.starts_with("CARGO_") {
        // Exclude vars that don't affect compilation output
        !matches!(
            key,
            "CARGO_MAKEFLAGS"
                | "CARGO_BUILD_JOBS"
                | "CARGO_ENCODED_RUSTFLAGS"
                | "CARGO_LOG"
                | "CARGO_HOME"
                | "CARGO_TARGET_DIR"
        ) && !key.starts_with("CARGO_REGISTRIES_")
    } else {
        matches!(
            key,
            "RUSTFLAGS"
                | "RUSTC"
                | "TARGET"
                | "HOST"
                | "OPT_LEVEL"
                | "DEBUG"
                | "PROFILE"
                | "OUT_DIR"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_relevant_env_vars() {
        assert!(is_hash_relevant_env_var("CARGO_PKG_NAME"));
        assert!(is_hash_relevant_env_var("CARGO_PKG_VERSION"));
        assert!(is_hash_relevant_env_var("CARGO_FEATURE_DEFAULT"));
        assert!(is_hash_relevant_env_var("RUSTFLAGS"));
        assert!(is_hash_relevant_env_var("TARGET"));

        assert!(!is_hash_relevant_env_var("CARGO_MAKEFLAGS"));
        assert!(!is_hash_relevant_env_var("CARGO_BUILD_JOBS"));
        assert!(!is_hash_relevant_env_var("CARGO_HOME"));
        assert!(!is_hash_relevant_env_var("CARGO_TARGET_DIR"));
        assert!(!is_hash_relevant_env_var(
            "CARGO_REGISTRIES_CRATES_IO_TOKEN"
        ));
        assert!(!is_hash_relevant_env_var("HOME"));
        assert!(!is_hash_relevant_env_var("PATH"));
    }
}
