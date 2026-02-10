//! Benchmarks for turborepo-hash hot path operations.
//!
//! These benchmarks cover the core hashing pipeline that runs for every task
//! during a turbo build: Cap'n Proto serialization -> canonical form -> xxHash64.

use std::collections::HashMap;

use codspeed_criterion_compat::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion,
};
use turborepo_hash::{FileHashes, GlobalHashable, LockFilePackages, TaskHashable, TurboHash};
use turborepo_lockfiles::Package;
use turborepo_types::{EnvMode, TaskOutputs};

/// Build a FileHashes with `n` entries to benchmark sort + serialize + hash.
fn make_file_hashes(n: usize) -> FileHashes {
    let map: HashMap<turbopath::RelativeUnixPathBuf, String> = (0..n)
        .map(|i| {
            (
                turbopath::RelativeUnixPathBuf::new(format!("packages/pkg-{i}/src/file-{i}.ts"))
                    .unwrap(),
                format!("{:016x}", (i as u64).wrapping_mul(0x123456789abcdef0)),
            )
        })
        .collect();
    FileHashes(map)
}

/// Build a LockFilePackages with `n` entries.
fn make_lock_file_packages(n: usize) -> LockFilePackages {
    LockFilePackages(
        (0..n)
            .map(|i| Package {
                key: format!("@scope/package-{i}"),
                version: format!("{}.{}.{}", i / 100, (i / 10) % 10, i % 10),
            })
            .collect(),
    )
}

/// Build a TaskHashable with a realistic set of fields.
fn make_task_hashable<'a>(
    num_deps: usize,
    env_vars: &'a [String],
    resolved_env: Vec<String>,
    pass_through_args: &'a [String],
    pass_through_env: &'a [String],
) -> TaskHashable<'a> {
    TaskHashable {
        global_hash: "abc123def456789012345678",
        task_dependency_hashes: (0..num_deps)
            .map(|i| format!("{:016x}", (i as u64).wrapping_mul(0xdeadbeefcafe0000)))
            .collect(),
        package_dir: Some(turbopath::RelativeUnixPathBuf::new("packages/my-pkg").unwrap()),
        hash_of_files: "aaaa1111bbbb2222",
        external_deps_hash: Some("cccc3333dddd4444".to_string()),
        task: "build",
        outputs: TaskOutputs {
            inclusions: vec!["dist/**".to_string(), ".next/**".to_string()],
            exclusions: vec!["dist/**/*.map".to_string()],
        },
        pass_through_args,
        env: env_vars,
        resolved_env_vars: resolved_env,
        pass_through_env,
        env_mode: EnvMode::Strict,
    }
}

/// Build a GlobalHashable with references to the provided data.
fn make_global_hashable<'a>(
    file_hash_map: &'a HashMap<turbopath::RelativeUnixPathBuf, String>,
    env: &'a [String],
    resolved_env: Vec<String>,
    pass_through_env: &'a [String],
) -> GlobalHashable<'a> {
    GlobalHashable {
        global_cache_key: "turborepo-cache-v1",
        global_file_hash_map: file_hash_map,
        root_external_dependencies_hash: Some("eeee5555ffff6666"),
        root_internal_dependencies_hash: Some("7777888899990000"),
        engines: [("node", ">=18"), ("pnpm", ">=8")]
            .into_iter()
            .collect(),
        env,
        resolved_env_vars: resolved_env,
        pass_through_env,
        env_mode: EnvMode::Strict,
        framework_inference: true,
    }
}

fn bench_file_hashes(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_hashes");
    for size in [10, 50, 200, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            b.iter_with_setup(|| make_file_hashes(n), |fh| black_box(fh.hash()));
        });
    }
    group.finish();
}

fn bench_lock_file_packages(c: &mut Criterion) {
    let mut group = c.benchmark_group("lock_file_packages");
    for size in [10, 100, 500, 2000] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            b.iter_with_setup(|| make_lock_file_packages(n), |lp| black_box(lp.hash()));
        });
    }
    group.finish();
}

fn bench_task_hashable(c: &mut Criterion) {
    let env_vars: Vec<String> = (0..5).map(|i| format!("VAR_{i}=value_{i}")).collect();
    let resolved_env: Vec<String> = (0..3).map(|i| format!("RESOLVED_{i}=val_{i}")).collect();
    let pass_through_args: Vec<String> = vec!["--verbose".into(), "--output=json".into()];
    let pass_through_env: Vec<String> = vec!["PATH".into(), "HOME".into()];

    let mut group = c.benchmark_group("task_hashable");
    for num_deps in [0, 5, 20, 50] {
        group.bench_with_input(
            BenchmarkId::new("deps", num_deps),
            &num_deps,
            |b, &n| {
                b.iter(|| {
                    let th = make_task_hashable(
                        n,
                        &env_vars,
                        resolved_env.clone(),
                        &pass_through_args,
                        &pass_through_env,
                    );
                    black_box(th.hash())
                });
            },
        );
    }
    group.finish();
}

fn bench_global_hashable(c: &mut Criterion) {
    let env: Vec<String> = (0..10).map(|i| format!("ENV_{i}=value_{i}")).collect();
    let resolved_env: Vec<String> = (0..5).map(|i| format!("RESOLVED_{i}=val_{i}")).collect();
    let pass_through_env: Vec<String> = vec!["CI".into(), "NODE_ENV".into()];

    let mut group = c.benchmark_group("global_hashable");
    for num_files in [5, 20, 100, 500] {
        let file_hash_map: HashMap<turbopath::RelativeUnixPathBuf, String> = (0..num_files)
            .map(|i| {
                (
                    turbopath::RelativeUnixPathBuf::new(format!("config/file-{i}.json")).unwrap(),
                    format!("{:016x}", (i as u64).wrapping_mul(0xfedcba9876543210)),
                )
            })
            .collect();

        group.bench_with_input(
            BenchmarkId::new("files", num_files),
            &num_files,
            |b, &_n| {
                b.iter(|| {
                    let gh = make_global_hashable(
                        &file_hash_map,
                        &env,
                        resolved_env.clone(),
                        &pass_through_env,
                    );
                    black_box(gh.hash())
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_file_hashes,
    bench_lock_file_packages,
    bench_task_hashable,
    bench_global_hashable,
);
criterion_main!(benches);
