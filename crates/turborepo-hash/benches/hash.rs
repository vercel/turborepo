//! Benchmarks for turborepo-hash operations
//!
//! These benchmarks cover the hot path of hash computation that runs for every
//! task in a turborepo build: Cap'n Proto serialization, sorting, and xxHash64.

use std::collections::HashMap;

use codspeed_criterion_compat::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion,
};
use turborepo_hash::{
    FileHashes, GlobalHashable, LockFilePackages, LockFilePackagesRef, TaskHashable, TurboHash,
};
use turborepo_lockfiles::Package;
use turborepo_types::{EnvMode, TaskOutputs};

fn make_file_hashes(n: usize) -> FileHashes {
    FileHashes(
        (0..n)
            .map(|i| {
                (
                    turbopath::RelativeUnixPathBuf::new(format!("packages/pkg-{i}/src/file_{i}.ts"))
                        .unwrap(),
                    format!("{:016x}", i * 0xDEADBEEF),
                )
            })
            .collect(),
    )
}

fn make_packages(n: usize) -> Vec<Package> {
    (0..n)
        .map(|i| Package {
            key: format!("@scope/package-{i}"),
            version: format!("{}.{}.{}", i / 100, (i / 10) % 10, i % 10),
        })
        .collect()
}

fn bench_file_hashes(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_hashes");

    for size in [10, 100, 500, 1000] {
        let file_hashes = make_file_hashes(size);

        group.bench_with_input(
            BenchmarkId::new("owned", size),
            &file_hashes,
            |b, fh| {
                b.iter(|| black_box(fh.clone().hash()));
            },
        );

        group.bench_with_input(BenchmarkId::new("ref", size), &file_hashes, |b, fh| {
            b.iter(|| black_box(fh.hash()));
        });
    }

    group.finish();
}

fn bench_lock_file_packages(c: &mut Criterion) {
    let mut group = c.benchmark_group("lock_file_packages");

    for size in [10, 100, 500] {
        let packages = make_packages(size);

        group.bench_with_input(
            BenchmarkId::new("owned", size),
            &packages,
            |b, pkgs| {
                b.iter(|| black_box(LockFilePackages(pkgs.clone()).hash()));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("ref", size),
            &packages,
            |b, pkgs| {
                let refs: Vec<&Package> = pkgs.iter().collect();
                b.iter(|| black_box(LockFilePackagesRef(refs.clone()).hash()));
            },
        );
    }

    group.finish();
}

fn bench_task_hashable(c: &mut Criterion) {
    c.bench_function("task_hashable", |b| {
        b.iter(|| {
            let task_hashable = TaskHashable {
                global_hash: "abc123def456",
                task_dependency_hashes: vec![
                    "dep_hash_1".to_string(),
                    "dep_hash_2".to_string(),
                    "dep_hash_3".to_string(),
                ],
                package_dir: Some(
                    turbopath::RelativeUnixPathBuf::new("packages/my-package").unwrap(),
                ),
                hash_of_files: "file_hash_abc",
                external_deps_hash: Some("ext_deps_hash".to_string()),
                task: "build",
                outputs: TaskOutputs {
                    inclusions: vec!["dist/**".to_string(), ".next/**".to_string()],
                    exclusions: vec!["dist/cache/**".to_string()],
                },
                pass_through_args: &["--verbose".to_string(), "--mode=production".to_string()],
                env: &[
                    "NODE_ENV".to_string(),
                    "CI".to_string(),
                    "VERCEL".to_string(),
                ],
                resolved_env_vars: vec![
                    "NODE_ENV=production".to_string(),
                    "CI=true".to_string(),
                    "VERCEL=1".to_string(),
                ],
                pass_through_env: &["HOME".to_string(), "PATH".to_string()],
                env_mode: EnvMode::Strict,
            };
            black_box(task_hashable.hash())
        });
    });
}

fn bench_global_hashable(c: &mut Criterion) {
    let global_file_hash_map: HashMap<turbopath::RelativeUnixPathBuf, String> = (0..50)
        .map(|i| {
            (
                turbopath::RelativeUnixPathBuf::new(format!("global/config_{i}.json")).unwrap(),
                format!("{:016x}", i as u64 * 0xCAFE_BABE),
            )
        })
        .collect();

    let engines: HashMap<&str, &str> = [("node", ">=18.0.0"), ("pnpm", ">=8.0.0")]
        .into_iter()
        .collect();

    let env = vec![
        "NODE_ENV".to_string(),
        "CI".to_string(),
        "VERCEL".to_string(),
    ];
    let resolved = vec![
        "NODE_ENV=production".to_string(),
        "CI=true".to_string(),
        "VERCEL=1".to_string(),
    ];
    let pass_through = vec!["HOME".to_string(), "PATH".to_string()];

    c.bench_function("global_hashable", |b| {
        b.iter(|| {
            let global_hashable = GlobalHashable {
                global_cache_key: "global_cache_key_v1",
                global_file_hash_map: &global_file_hash_map,
                root_external_dependencies_hash: Some("0000000000000000"),
                root_internal_dependencies_hash: Some("0000000000000001"),
                engines: engines.clone(),
                env: &env,
                resolved_env_vars: resolved.clone(),
                pass_through_env: &pass_through,
                env_mode: EnvMode::Strict,
                framework_inference: true,
            };
            black_box(global_hashable.hash())
        });
    });
}

criterion_group!(
    benches,
    bench_file_hashes,
    bench_lock_file_packages,
    bench_task_hashable,
    bench_global_hashable,
);

criterion_main!(benches);
