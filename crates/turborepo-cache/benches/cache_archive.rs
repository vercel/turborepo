//! Benchmarks for cache archive create and restore operations.
//!
//! These benchmarks measure the hot-path performance of writing task output
//! files into compressed tar archives and restoring them back to disk.

use std::fs;

use codspeed_criterion_compat::{
    BatchSize, BenchmarkId, Criterion, black_box, criterion_group, criterion_main,
};
use tempfile::TempDir;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_cache::cache_archive::{CacheReader, CacheWriter};

/// Create a temporary directory with a set of files of specified sizes.
/// Returns (temp_dir, anchored_file_paths).
fn setup_source_files(
    file_specs: &[(&str, usize)],
) -> (TempDir, Vec<AnchoredSystemPathBuf>) {
    let tmp = TempDir::with_prefix("cache-bench").unwrap();
    let mut paths = Vec::new();

    for &(name, size) in file_specs {
        let full_path = tmp.path().join(name);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        // Fill with repeating byte pattern for realistic compression behavior
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        fs::write(&full_path, &data).unwrap();
        paths.push(AnchoredSystemPathBuf::from_raw(name).unwrap());
    }

    (tmp, paths)
}

/// Create a cache archive from the given source files and return the archive
/// bytes.
fn create_archive(
    anchor: &AbsoluteSystemPath,
    files: &[AnchoredSystemPathBuf],
    compressed: bool,
) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut writer = CacheWriter::from_writer(&mut buf, compressed).unwrap();
    for file in files {
        writer.add_file(anchor, file).unwrap();
    }
    writer.finish().unwrap();
    buf
}

/// Restore an archive from bytes into a fresh temporary directory.
fn restore_archive(archive_bytes: &[u8], compressed: bool) -> (TempDir, Vec<AnchoredSystemPathBuf>) {
    let output = TempDir::with_prefix("cache-bench-restore").unwrap();
    let anchor = AbsoluteSystemPath::from_std_path(output.path()).unwrap();
    let mut reader = CacheReader::from_reader(archive_bytes, compressed).unwrap();
    let files = reader.restore(anchor).unwrap();
    (output, files)
}

// --- File specs for different workloads ---

/// Simulates a typical JS build output: many small files
fn many_small_files() -> Vec<(&'static str, usize)> {
    let mut files = Vec::new();
    for i in 0..100 {
        // Leak is fine in benchmarks — these are static for the process lifetime
        let name: &'static str =
            Box::leak(format!("dist/chunk-{i:03}.js").into_boxed_str());
        files.push((name, 4 * 1024)); // 4KB each
    }
    files
}

/// Simulates a build with fewer but larger files
fn few_large_files() -> Vec<(&'static str, usize)> {
    vec![
        ("dist/main.js", 512 * 1024),
        ("dist/vendor.js", 1024 * 1024),
        ("dist/main.css", 256 * 1024),
        ("dist/index.html", 8 * 1024),
    ]
}

/// Mixed workload: directories + small and large files
fn mixed_workload() -> Vec<(&'static str, usize)> {
    let mut files = vec![
        ("dist/main.js", 256 * 1024),
        ("dist/vendor.js", 512 * 1024),
        ("dist/styles.css", 64 * 1024),
        ("dist/index.html", 2 * 1024),
    ];
    for i in 0..20 {
        let name: &'static str =
            Box::leak(format!("dist/assets/image-{i:02}.dat").into_boxed_str());
        files.push((name, 16 * 1024));
    }
    files
}

// --- Benchmarks ---

fn bench_create_archive(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_create");

    for (label, file_specs) in [
        ("many_small_files", many_small_files()),
        ("few_large_files", few_large_files()),
        ("mixed_workload", mixed_workload()),
    ] {
        let (src_dir, files) = setup_source_files(&file_specs);
        let anchor = AbsoluteSystemPath::from_std_path(src_dir.path()).unwrap();

        group.bench_function(BenchmarkId::new("compressed", label), |b| {
            b.iter(|| {
                let archive = create_archive(black_box(anchor), black_box(&files), true);
                black_box(archive);
            });
        });

        group.bench_function(BenchmarkId::new("uncompressed", label), |b| {
            b.iter(|| {
                let archive = create_archive(black_box(anchor), black_box(&files), false);
                black_box(archive);
            });
        });
    }

    group.finish();
}

fn bench_restore_archive(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_restore");

    for (label, file_specs) in [
        ("many_small_files", many_small_files()),
        ("few_large_files", few_large_files()),
        ("mixed_workload", mixed_workload()),
    ] {
        let (src_dir, files) = setup_source_files(&file_specs);
        let anchor = AbsoluteSystemPath::from_std_path(src_dir.path()).unwrap();

        let compressed_archive = create_archive(anchor, &files, true);
        let uncompressed_archive = create_archive(anchor, &files, false);

        group.bench_function(BenchmarkId::new("compressed", label), |b| {
            b.iter_batched(
                || compressed_archive.clone(),
                |archive| {
                    let (dir, restored) = restore_archive(black_box(&archive), true);
                    black_box((dir, restored));
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("uncompressed", label), |b| {
            b.iter_batched(
                || uncompressed_archive.clone(),
                |archive| {
                    let (dir, restored) = restore_archive(black_box(&archive), false);
                    black_box((dir, restored));
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_roundtrip");

    for (label, file_specs) in [
        ("many_small_files", many_small_files()),
        ("few_large_files", few_large_files()),
        ("mixed_workload", mixed_workload()),
    ] {
        let (src_dir, files) = setup_source_files(&file_specs);
        let anchor = AbsoluteSystemPath::from_std_path(src_dir.path()).unwrap();

        group.bench_function(BenchmarkId::new("compressed", label), |b| {
            b.iter(|| {
                let archive = create_archive(black_box(anchor), black_box(&files), true);
                let (dir, restored) = restore_archive(black_box(&archive), true);
                black_box((dir, restored));
            });
        });
    }

    group.finish();
}

fn bench_get_sha(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_sha");

    let (src_dir, files) = setup_source_files(&few_large_files());
    let anchor = AbsoluteSystemPath::from_std_path(src_dir.path()).unwrap();
    let archive = create_archive(anchor, &files, true);

    group.bench_function("sha512_compressed_archive", |b| {
        b.iter(|| {
            let reader =
                CacheReader::from_reader(black_box(archive.as_slice()), true).unwrap();
            let sha = reader.get_sha().unwrap();
            black_box(sha);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_create_archive,
    bench_restore_archive,
    bench_roundtrip,
    bench_get_sha,
);
criterion_main!(benches);
