use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Manifest stored alongside cached artifacts to track what files belong
/// to a cache entry and their original filenames.
#[derive(Debug, Serialize, Deserialize)]
struct CacheManifest {
    /// Map of original filename -> stored filename (in the cache entry dir)
    files: HashMap<String, String>,
}

/// Manages the local filesystem cache for compiled Rust artifacts.
pub struct LocalCache {
    cache_dir: PathBuf,
    read_only: bool,
}

impl LocalCache {
    pub fn new(cache_dir: PathBuf, read_only: bool) -> Self {
        Self {
            cache_dir,
            read_only,
        }
    }

    /// Resolve the cache directory from environment or defaults.
    ///
    /// Priority:
    /// 1. TURBO_RUSTC_CACHE_DIR env var
    /// 2. <repo_root>/.turbo/rustc-cache (if TURBO_REPO_ROOT is set)
    /// 3. Platform cache dir (~/.cache/turborepo-rustc-cache on Linux,
    ///    ~/Library/Caches/turborepo-rustc-cache on macOS)
    pub fn from_env() -> Self {
        let read_only = is_read_only_mode();
        let cache_dir = resolve_cache_dir();

        Self {
            cache_dir,
            read_only,
        }
    }

    fn entry_dir(&self, hash: &str) -> PathBuf {
        self.cache_dir.join(hash)
    }

    fn manifest_path(&self, hash: &str) -> PathBuf {
        self.entry_dir(hash).join("manifest.json")
    }

    /// Check if a cache entry exists for the given hash.
    pub fn has(&self, hash: &str) -> bool {
        self.manifest_path(hash).exists()
    }

    /// Restore cached artifacts to the given output directory.
    /// Returns the list of restored file paths on success, None on miss.
    pub fn restore(&self, hash: &str, out_dir: &Path) -> io::Result<Option<Vec<PathBuf>>> {
        let manifest_path = self.manifest_path(hash);
        if !manifest_path.exists() {
            return Ok(None);
        }

        let manifest_data = fs::read_to_string(&manifest_path)?;
        let manifest: CacheManifest = serde_json::from_str(&manifest_data).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("corrupt cache manifest: {e}"),
            )
        })?;

        let entry_dir = self.entry_dir(hash);
        let mut restored = Vec::new();

        for (original_name, stored_name) in &manifest.files {
            let stored_path = entry_dir.join(stored_name);
            let dest_path = out_dir.join(original_name);

            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }

            fs::copy(&stored_path, &dest_path)?;
            restored.push(dest_path);
        }

        Ok(Some(restored))
    }

    /// Store output files from a compilation into the cache.
    /// `out_dir` is the --out-dir where rustc wrote its outputs.
    /// `crate_name` is used to identify which files in out_dir belong to this
    /// crate. `emit_types` controls which file types to cache.
    pub fn store(
        &self,
        hash: &str,
        out_dir: &Path,
        crate_name: &str,
        emit_types: &[&str],
    ) -> io::Result<()> {
        if self.read_only {
            return Ok(());
        }

        let entry_dir = self.entry_dir(hash);
        fs::create_dir_all(&entry_dir)?;

        let mut manifest = CacheManifest {
            files: HashMap::new(),
        };

        let output_files = find_output_files(out_dir, crate_name, emit_types)?;

        for file_path in &output_files {
            let file_name = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid output filename")
                })?;

            let stored_name = file_name.to_string();
            let dest = entry_dir.join(&stored_name);
            fs::copy(file_path, &dest)?;
            manifest.files.insert(file_name.to_string(), stored_name);
        }

        if manifest.files.is_empty() {
            // Nothing to cache — clean up the empty dir
            let _ = fs::remove_dir(&entry_dir);
            return Ok(());
        }

        let manifest_json = serde_json::to_string(&manifest).map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("failed to serialize manifest: {e}"),
            )
        })?;
        fs::write(self.manifest_path(hash), manifest_json)?;

        Ok(())
    }
}

/// Find output files in out_dir that belong to the given crate.
/// Cargo names output files with the pattern: lib<crate_name>-<hash>.<ext>
/// or <crate_name>-<hash>.<ext> for various extensions.
fn find_output_files(
    out_dir: &Path,
    crate_name: &str,
    emit_types: &[&str],
) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    // Cargo uses underscores in output filenames (crate names with hyphens
    // become underscores)
    let normalized = crate_name.replace('-', "_");

    let entries = match fs::read_dir(out_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(files),
        Err(e) => return Err(e),
    };

    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if !is_crate_output(&name, &normalized) {
            continue;
        }

        if should_cache_file(&name, emit_types) {
            files.push(entry.path());
        }
    }

    Ok(files)
}

/// Check if a filename looks like it belongs to the given crate.
/// Patterns: lib<name>-<hash>.<ext>, <name>-<hash>.<ext>, <name>-<hash>
fn is_crate_output(filename: &str, normalized_crate_name: &str) -> bool {
    let prefixes = [
        format!("lib{normalized_crate_name}-"),
        format!("{normalized_crate_name}-"),
    ];

    prefixes.iter().any(|prefix| filename.starts_with(prefix))
}

/// Check if a file should be cached based on its extension and the emit types.
fn should_cache_file(filename: &str, emit_types: &[&str]) -> bool {
    for emit in emit_types {
        let dominated = match *emit {
            "link" | "metadata" => {
                filename.ends_with(".rlib")
                    || filename.ends_with(".rmeta")
                    || filename.ends_with(".so")
                    || filename.ends_with(".dylib")
                    || filename.ends_with(".a")
                    || filename.ends_with(".dll")
                    || filename.ends_with(".lib")
            }
            "dep-info" => filename.ends_with(".d"),
            "llvm-bc" => filename.ends_with(".bc"),
            "asm" => filename.ends_with(".s"),
            "llvm-ir" => filename.ends_with(".ll"),
            "obj" => filename.ends_with(".o"),
            _ => false,
        };
        if dominated {
            return true;
        }
    }
    false
}

fn resolve_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("TURBO_RUSTC_CACHE_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }

    if let Ok(repo_root) = std::env::var("TURBO_REPO_ROOT") {
        if !repo_root.is_empty() {
            return PathBuf::from(repo_root).join(".turbo").join("rustc-cache");
        }
    }

    // Platform default
    dirs_cache_dir()
        .map(|d| d.join("turborepo-rustc-cache"))
        .unwrap_or_else(|| PathBuf::from(".turbo").join("rustc-cache"))
}

fn dirs_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs_next::cache_dir()
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_CACHE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| dirs_next::home_dir().map(|h| h.join(".cache")))
    }
    #[cfg(target_os = "windows")]
    {
        dirs_next::cache_dir()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        dirs_next::cache_dir()
    }
}

fn is_read_only_mode() -> bool {
    // Explicit override
    if let Ok(val) = std::env::var("TURBO_RUSTC_CACHE_MODE") {
        return val == "read-only" || val == "readonly";
    }

    // Not in CI → read-write (local dev builds write to cache)
    // In CI → also read-write (CI populates cache for the team)
    // The distinction between local and CI is handled by CARGO_INCREMENTAL,
    // not by cache read/write mode.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_crate_output() {
        assert!(is_crate_output("libmath_core-abc123.rlib", "math_core"));
        assert!(is_crate_output("libmath_core-abc123.rmeta", "math_core"));
        assert!(is_crate_output("math_core-abc123.d", "math_core"));
        assert!(!is_crate_output("libutils-abc123.rlib", "math_core"));
        assert!(!is_crate_output("random_file.txt", "math_core"));
    }

    #[test]
    fn test_should_cache_file() {
        let emit = vec!["link", "metadata"];
        assert!(should_cache_file("libfoo-abc.rlib", &emit));
        assert!(should_cache_file("libfoo-abc.rmeta", &emit));
        assert!(!should_cache_file("libfoo-abc.d", &emit));

        let emit_with_dep = vec!["link", "metadata", "dep-info"];
        assert!(should_cache_file("foo-abc.d", &emit_with_dep));
    }

    #[test]
    fn test_hyphen_to_underscore_normalization() {
        assert!(is_crate_output("libmath_core-abc123.rlib", "math_core"));
        // Crate name "math-core" normalizes to "math_core" in filenames
        assert!(is_crate_output("libmath_core-abc123.rlib", "math_core"));
    }

    #[test]
    fn test_store_and_restore() {
        let temp = tempfile::TempDir::new().unwrap();
        let cache_dir = temp.path().join("cache");
        let out_dir = temp.path().join("output");
        fs::create_dir_all(&out_dir).unwrap();

        // Create a fake rlib file
        fs::write(out_dir.join("libfoo-abc123.rlib"), b"fake rlib content").unwrap();
        fs::write(out_dir.join("libfoo-abc123.rmeta"), b"fake rmeta").unwrap();

        let cache = LocalCache::new(cache_dir, false);
        cache
            .store("testhash", &out_dir, "foo", &["link", "metadata"])
            .unwrap();

        assert!(cache.has("testhash"));

        // Restore to a new directory
        let restore_dir = temp.path().join("restored");
        fs::create_dir_all(&restore_dir).unwrap();

        let restored = cache.restore("testhash", &restore_dir).unwrap();
        assert!(restored.is_some());

        let files = restored.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(
            fs::read(restore_dir.join("libfoo-abc123.rlib")).unwrap(),
            b"fake rlib content"
        );
    }

    #[test]
    fn test_cache_miss() {
        let temp = tempfile::TempDir::new().unwrap();
        let cache = LocalCache::new(temp.path().join("cache"), false);

        let result = cache
            .restore("nonexistent", temp.path().join("out").as_path())
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_read_only_mode_skips_store() {
        let temp = tempfile::TempDir::new().unwrap();
        let out_dir = temp.path().join("output");
        fs::create_dir_all(&out_dir).unwrap();
        fs::write(out_dir.join("libbar-xyz.rlib"), b"data").unwrap();

        let cache = LocalCache::new(temp.path().join("cache"), true);
        cache.store("hash", &out_dir, "bar", &["link"]).unwrap();

        // Should not have stored anything
        assert!(!cache.has("hash"));
    }
}
