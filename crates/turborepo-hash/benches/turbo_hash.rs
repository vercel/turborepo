use std::collections::HashMap;

use codspeed_criterion_compat::{Criterion, black_box, criterion_group, criterion_main};
use turborepo_hash::{FileHashes, GlobalHashable, LockFilePackages, TaskHashable, TurboHash};
use turborepo_lockfiles::Package;
use turborepo_types::{EnvMode, TaskOutputs};

fn bench_task_hashable(c: &mut Criterion) {
    c.bench_function("task_hashable_hash", |b| {
        b.iter(|| {
            let task_hashable = TaskHashable {
                global_hash: "global_hash",
                task_dependency_hashes: vec!["dep1".into(), "dep2".into(), "dep3".into()],
                package_dir: Some(turbopath::RelativeUnixPathBuf::new("packages/ui").unwrap()),
                hash_of_files: "abc123def456",
                external_deps_hash: Some("ext_deps_hash".into()),
                task: "build",
                outputs: TaskOutputs {
                    inclusions: vec!["dist/**".into(), "build/**".into()],
                    exclusions: vec!["**/*.map".into()],
                },
                pass_through_args: &[],
                env: &["NODE_ENV=production".into(), "CI=true".into()],
                resolved_env_vars: vec![
                    "NODE_ENV=production".into(),
                    "CI=true".into(),
                ],
                pass_through_env: &[],
                env_mode: EnvMode::Strict,
            };
            black_box(task_hashable.calculate_task_hash())
        });
    });
}

fn bench_global_hashable(c: &mut Criterion) {
    let global_file_hash_map: HashMap<turbopath::RelativeUnixPathBuf, String> = (0..20)
        .map(|i| {
            (
                turbopath::RelativeUnixPathBuf::new(format!("packages/pkg-{i}/package.json"))
                    .unwrap(),
                format!("hash_{i:04x}"),
            )
        })
        .collect();

    let engines: HashMap<&str, &str> =
        [("node", ">=18.0.0"), ("pnpm", ">=8.0.0")].into_iter().collect();

    c.bench_function("global_hashable_hash", |b| {
        b.iter(|| {
            let global_hash = GlobalHashable {
                global_cache_key: "v1",
                global_file_hash_map: &global_file_hash_map,
                root_external_dependencies_hash: Some("0000000000000000"),
                root_internal_dependencies_hash: Some("0000000000000001"),
                engines: engines.clone(),
                env: &["CI=true".into()],
                resolved_env_vars: vec!["CI=true".into()],
                pass_through_env: &[],
                env_mode: EnvMode::Strict,
                framework_inference: true,
            };
            black_box(global_hash.hash())
        });
    });
}

fn bench_file_hashes_small(c: &mut Criterion) {
    c.bench_function("file_hashes_10_files", |b| {
        b.iter(|| {
            let pairs: HashMap<turbopath::RelativeUnixPathBuf, String> = (0..10)
                .map(|i| {
                    (
                        turbopath::RelativeUnixPathBuf::new(format!("src/file_{i}.ts")).unwrap(),
                        format!("e69de29bb2d1d6434b8b29ae775ad8c2e48c{i:05}"),
                    )
                })
                .collect();
            black_box(FileHashes(pairs).hash())
        });
    });
}

fn bench_file_hashes_large(c: &mut Criterion) {
    c.bench_function("file_hashes_500_files", |b| {
        b.iter(|| {
            let pairs: HashMap<turbopath::RelativeUnixPathBuf, String> = (0..500)
                .map(|i| {
                    (
                        turbopath::RelativeUnixPathBuf::new(format!(
                            "packages/pkg-{}/src/file_{}.ts",
                            i / 10,
                            i % 10
                        ))
                        .unwrap(),
                        format!("e69de29bb2d1d6434b8b29ae775ad8c2e{i:08}"),
                    )
                })
                .collect();
            black_box(FileHashes(pairs).hash())
        });
    });
}

fn bench_lock_file_packages(c: &mut Criterion) {
    c.bench_function("lock_file_packages_100", |b| {
        b.iter(|| {
            let packages: Vec<Package> = (0..100)
                .map(|i| Package {
                    key: format!("@scope/package-{i}"),
                    version: format!("{}.0.0", i % 20),
                })
                .collect();
            black_box(LockFilePackages(packages).hash())
        });
    });
}

criterion_group!(
    benches,
    bench_task_hashable,
    bench_global_hashable,
    bench_file_hashes_small,
    bench_file_hashes_large,
    bench_lock_file_packages,
);

criterion_main!(benches);
