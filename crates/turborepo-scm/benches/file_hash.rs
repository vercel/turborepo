use codspeed_criterion_compat::{Criterion, black_box, criterion_group, criterion_main};
use tempfile::TempDir;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_scm::manual;

fn create_test_files(tmp: &TempDir, count: usize, size_bytes: usize) -> Vec<AnchoredSystemPathBuf> {
    let content: Vec<u8> = (0..size_bytes).map(|i| (i % 256) as u8).collect();
    let mut files = Vec::with_capacity(count);
    for i in 0..count {
        let name = format!("file_{i}.txt");
        let path = tmp.path().join(&name);
        std::fs::write(&path, &content).unwrap();
        files.push(AnchoredSystemPathBuf::from_raw(&name).unwrap());
    }
    files
}

fn bench_hash_small_files(c: &mut Criterion) {
    let tmp = TempDir::with_prefix("scm-bench").unwrap();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let files = create_test_files(&tmp, 50, 1024); // 50 files, 1KB each

    c.bench_function("hash_50_small_files_1kb", |b| {
        b.iter(|| {
            black_box(manual::hash_files(&base_path, files.iter(), false).unwrap())
        });
    });
}

fn bench_hash_medium_files(c: &mut Criterion) {
    let tmp = TempDir::with_prefix("scm-bench").unwrap();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let files = create_test_files(&tmp, 20, 64 * 1024); // 20 files, 64KB each

    c.bench_function("hash_20_medium_files_64kb", |b| {
        b.iter(|| {
            black_box(manual::hash_files(&base_path, files.iter(), false).unwrap())
        });
    });
}

fn bench_hash_large_files(c: &mut Criterion) {
    let tmp = TempDir::with_prefix("scm-bench").unwrap();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let files = create_test_files(&tmp, 5, 1024 * 1024); // 5 files, 1MB each

    c.bench_function("hash_5_large_files_1mb", |b| {
        b.iter(|| {
            black_box(manual::hash_files(&base_path, files.iter(), false).unwrap())
        });
    });
}

fn bench_hash_many_tiny_files(c: &mut Criterion) {
    let tmp = TempDir::with_prefix("scm-bench").unwrap();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let files = create_test_files(&tmp, 200, 128); // 200 files, 128 bytes each

    c.bench_function("hash_200_tiny_files_128b", |b| {
        b.iter(|| {
            black_box(manual::hash_files(&base_path, files.iter(), false).unwrap())
        });
    });
}

criterion_group!(
    benches,
    bench_hash_small_files,
    bench_hash_medium_files,
    bench_hash_large_files,
    bench_hash_many_tiny_files,
);

criterion_main!(benches);
