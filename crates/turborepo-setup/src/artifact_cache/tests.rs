#![allow(clippy::unwrap_used)]
use std::os::unix::fs::{PermissionsExt, symlink};

use sha2::{Digest, Sha256};

use super::*;

const LIMIT: usize = 1024;

fn verified(bytes: &[u8]) -> (VerifiedArtifact, String) {
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let digest = ExpectedSha256::from_hex(&sha256).unwrap();
    (
        VerifiedArtifact::verify_bytes(bytes.to_vec(), digest).unwrap(),
        sha256,
    )
}

fn files(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn miss_then_verified_read_only_hit_without_premature_writes() {
    let base = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(base.path()).unwrap();
    let (artifact, sha256) = verified(b"archive");
    assert!(cache.get(Platform::MacosArm64, &sha256, LIMIT).is_none());
    // Lookups never create cache state.
    assert!(files(base.path()).is_empty());
    cache.put(Platform::MacosArm64, &artifact).unwrap();
    let hit = cache.get(Platform::MacosArm64, &sha256, LIMIT).unwrap();
    assert_eq!(hit.as_bytes(), b"archive");
    // Keyed by lock platform and digest: other keys stay misses.
    assert!(cache.get(Platform::Any, &sha256, LIMIT).is_none());
    assert!(
        cache
            .get(Platform::MacosArm64, &"0".repeat(64), LIMIT)
            .is_none()
    );
    let directory = base.path().join("turborepo/setup-artifacts/v1/macos-arm64");
    assert_eq!(files(&directory), std::slice::from_ref(&sha256));
    let mode = fs::metadata(directory.join(&sha256)).unwrap().permissions();
    assert_eq!(mode.mode() & 0o777, 0o444);
    let mode = fs::metadata(&directory).unwrap().permissions();
    assert_eq!(mode.mode() & 0o777, 0o700);
}

#[test]
fn corrupt_oversized_or_linked_entries_are_misses_and_put_repairs() {
    let base = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(base.path()).unwrap();
    let (artifact, sha256) = verified(b"complete archive bytes");
    cache.put(Platform::Any, &artifact).unwrap();
    let entry = base
        .path()
        .join("turborepo/setup-artifacts/v1/any")
        .join(&sha256);
    // A truncated or tampered entry never verifies.
    fs::remove_file(&entry).unwrap();
    fs::write(&entry, b"complete").unwrap();
    assert!(cache.get(Platform::Any, &sha256, LIMIT).is_none());
    cache.put(Platform::Any, &artifact).unwrap();
    assert!(cache.get(Platform::Any, &sha256, LIMIT).is_some());
    // Callers' download limits also bound cached reads.
    assert!(cache.get(Platform::Any, &sha256, 4).is_none());
    // A planted link is never followed, even to the right bytes.
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("bytes"), b"complete archive bytes").unwrap();
    fs::remove_file(&entry).unwrap();
    symlink(outside.path().join("bytes"), &entry).unwrap();
    assert!(cache.get(Platform::Any, &sha256, LIMIT).is_none());
    // A linked cache directory is rejected for reads and writes.
    fs::remove_dir_all(base.path().join("turborepo")).unwrap();
    fs::create_dir_all(outside.path().join("v1/any")).unwrap();
    fs::create_dir(base.path().join("turborepo")).unwrap();
    symlink(
        outside.path(),
        base.path().join("turborepo/setup-artifacts"),
    )
    .unwrap();
    assert!(cache.put(Platform::Any, &artifact).is_err());
    assert!(files(&outside.path().join("v1/any")).is_empty());
    assert!(cache.get(Platform::Any, &sha256, LIMIT).is_none());
}

#[test]
fn interrupted_writes_leave_only_unread_partials() {
    let base = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(base.path()).unwrap();
    let (artifact, sha256) = verified(b"archive");
    let directory = base
        .path()
        .join("turborepo/setup-artifacts/v1/linux-x64-gnu");
    fs::create_dir_all(&directory).unwrap();
    // A crash before rename leaves a private partial, never an entry.
    fs::write(directory.join(".partial-crashed"), b"arch").unwrap();
    assert!(cache.get(Platform::LinuxX64Gnu, &sha256, LIMIT).is_none());
    cache.put(Platform::LinuxX64Gnu, &artifact).unwrap();
    assert_eq!(
        cache
            .get(Platform::LinuxX64Gnu, &sha256, LIMIT)
            .unwrap()
            .as_bytes(),
        b"archive"
    );
    assert_eq!(files(&directory), [".partial-crashed".to_owned(), sha256]);
}

#[test]
fn concurrent_writers_and_readers_never_observe_partial_bytes() {
    let base = tempfile::tempdir().unwrap();
    let body = vec![7; LIMIT];
    let (artifact, sha256) = verified(&body);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                // Independent handles model separate repositories/processes.
                let cache = ArtifactCache::new(base.path()).unwrap();
                for _ in 0..16 {
                    cache.put(Platform::MacosX64, &artifact).unwrap();
                    if let Some(hit) = cache.get(Platform::MacosX64, &sha256, LIMIT) {
                        assert_eq!(hit.as_bytes(), body.as_slice());
                    }
                }
            });
        }
    });
    let directory = base.path().join("turborepo/setup-artifacts/v1/macos-x64");
    assert_eq!(files(&directory), [sha256]);
}

#[test]
fn only_absolute_caller_selected_bases_are_accepted() {
    assert!(ArtifactCache::new(Path::new("relative/cache")).is_none());
    assert!(ArtifactCache::new(Path::new("/tmp/../cache")).is_none());
    let base = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(base.path()).unwrap();
    // Uppercase or malformed digests never become cache paths.
    let (_, sha256) = verified(b"archive");
    assert!(
        cache
            .get(Platform::Any, &sha256.to_ascii_uppercase(), LIMIT)
            .is_none()
    );
    assert!(cache.get(Platform::Any, "../escape", LIMIT).is_none());
}

fn age(path: &Path, days: u64) {
    let at = SystemTime::now() - Duration::from_secs(days * 86_400);
    File::open(path).unwrap().set_modified(at).unwrap();
}

#[test]
fn pruning_keeps_entries_another_repository_used_recently() {
    let base = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(base.path()).unwrap();
    let (old, old_sha256) = verified(b"replaced version archive");
    let (shared, shared_sha256) = verified(b"archive another clone still uses");
    cache.put(Platform::Any, &old).unwrap();
    cache.put(Platform::Any, &shared).unwrap();
    let directory = base.path().join("turborepo/setup-artifacts/v1/any");
    fs::write(directory.join(".partial-stale"), b"torn").unwrap();
    fs::write(directory.join(".partial-live"), b"in progress").unwrap();
    fs::write(directory.join("notes"), b"not a cache entry").unwrap();
    let aged = [old_sha256.as_str(), shared_sha256.as_str()];
    for name in aged.into_iter().chain([".partial-stale", "notes"]) {
        age(&directory.join(name), 40);
    }
    // Another repository's setup reuses one entry, refreshing its last use.
    let other = ArtifactCache::new(base.path()).unwrap();
    assert!(other.get(Platform::Any, &shared_sha256, LIMIT).is_some());
    assert_eq!(cache.disk_use().unwrap().entries, 4);
    let removed = cache
        .prune_unused(Duration::from_secs(30 * 86_400))
        .unwrap();
    assert_eq!(removed.entries, 2);
    assert_eq!(removed.bytes, old.as_bytes().len() as u64 + 4);
    assert!(cache.get(Platform::Any, &old_sha256, LIMIT).is_none());
    assert!(cache.get(Platform::Any, &shared_sha256, LIMIT).is_some());
    let mut expected = vec![".partial-live".to_owned(), "notes".into(), shared_sha256];
    expected.sort();
    assert_eq!(files(&directory), expected);
}

#[test]
fn lock_references_keep_entries_whose_installs_were_reused() {
    let base = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(base.path()).unwrap();
    let (referenced, sha256) = verified(b"archive a reused install came from");
    cache.put(Platform::MacosArm64, &referenced).unwrap();
    let entry = base
        .path()
        .join("turborepo/setup-artifacts/v1/macos-arm64")
        .join(&sha256);
    age(&entry, 40);
    // Only the exact lock key is refreshed; misses create nothing.
    cache.mark_used(Platform::Any, &sha256);
    cache.mark_used(Platform::LinuxX64Gnu, &sha256);
    cache.mark_used(Platform::MacosArm64, &"0".repeat(64));
    assert_eq!(
        files(&base.path().join("turborepo/setup-artifacts/v1")),
        ["macos-arm64"]
    );
    let month = Duration::from_secs(30 * 86_400);
    assert!(fs::metadata(&entry).unwrap().modified().unwrap() < SystemTime::now() - month);
    cache.mark_used(Platform::MacosArm64, &sha256);
    assert_eq!(cache.prune_unused(month).unwrap(), DiskUse::default());
    assert!(cache.get(Platform::MacosArm64, &sha256, LIMIT).is_some());
}
