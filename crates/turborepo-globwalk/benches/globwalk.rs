//! Benchmarks for globwalk operations

use std::str::FromStr;

use codspeed_criterion_compat::{Criterion, black_box, criterion_group, criterion_main};
use globwalk::{ValidatedGlob, WalkType, globwalk};
use tempfile::TempDir;
use turbopath::AbsoluteSystemPathBuf;

/// Create a test directory structure for benchmarking
fn setup_test_dir() -> TempDir {
    let tmp = TempDir::with_prefix("globwalk-bench").unwrap();

    // Create a realistic monorepo-like structure
    let directories = [
        "packages/ui/src/components",
        "packages/ui/src/hooks",
        "packages/ui/dist",
        "packages/utils/src",
        "packages/utils/dist",
        "packages/config/src",
        "apps/web/src/pages",
        "apps/web/src/components",
        "apps/web/public",
        "apps/web/node_modules/react",
        "apps/web/node_modules/next",
        "apps/docs/src",
        "apps/docs/public",
        "node_modules/typescript/lib",
        "node_modules/eslint/lib",
        ".turbo/cache",
        ".git/objects/pack",
    ];

    let files = [
        "packages/ui/src/components/Button.tsx",
        "packages/ui/src/components/Input.tsx",
        "packages/ui/src/components/Modal.tsx",
        "packages/ui/src/hooks/useModal.ts",
        "packages/ui/src/index.ts",
        "packages/ui/dist/index.js",
        "packages/ui/package.json",
        "packages/utils/src/format.ts",
        "packages/utils/src/parse.ts",
        "packages/utils/src/index.ts",
        "packages/utils/dist/index.js",
        "packages/utils/package.json",
        "packages/config/src/eslint.ts",
        "packages/config/src/tsconfig.ts",
        "packages/config/package.json",
        "apps/web/src/pages/index.tsx",
        "apps/web/src/pages/about.tsx",
        "apps/web/src/components/Header.tsx",
        "apps/web/src/components/Footer.tsx",
        "apps/web/public/favicon.ico",
        "apps/web/package.json",
        "apps/web/node_modules/react/index.js",
        "apps/web/node_modules/next/index.js",
        "apps/docs/src/index.mdx",
        "apps/docs/public/logo.png",
        "apps/docs/package.json",
        "node_modules/typescript/lib/typescript.js",
        "node_modules/eslint/lib/eslint.js",
        ".turbo/cache/abc123.tar.gz",
        ".git/objects/pack/pack-123.pack",
        "package.json",
        "turbo.json",
        "pnpm-lock.yaml",
    ];

    for dir in directories.iter() {
        std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
    }

    for file in files.iter() {
        std::fs::File::create(tmp.path().join(file)).unwrap();
    }

    tmp
}

fn bench_simple_glob(c: &mut Criterion) {
    let tmp = setup_test_dir();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

    c.bench_function("globwalk_simple_pattern", |b| {
        let include = vec![ValidatedGlob::from_str("**/*.ts").unwrap()];
        let exclude: Vec<ValidatedGlob> = vec![];

        b.iter(|| {
            black_box(
                globwalk(
                    black_box(&base_path),
                    black_box(&include),
                    black_box(&exclude),
                    WalkType::Files,
                )
                .unwrap(),
            )
        });
    });
}

fn bench_complex_glob_with_excludes(c: &mut Criterion) {
    let tmp = setup_test_dir();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

    c.bench_function("globwalk_complex_with_excludes", |b| {
        let include = vec![
            ValidatedGlob::from_str("packages/*/src/**/*.ts").unwrap(),
            ValidatedGlob::from_str("apps/*/src/**/*.tsx").unwrap(),
        ];
        let exclude = vec![
            ValidatedGlob::from_str("**/node_modules/**").unwrap(),
            ValidatedGlob::from_str("**/dist/**").unwrap(),
            ValidatedGlob::from_str("**/.turbo/**").unwrap(),
        ];

        b.iter(|| {
            black_box(
                globwalk(
                    black_box(&base_path),
                    black_box(&include),
                    black_box(&exclude),
                    WalkType::Files,
                )
                .unwrap(),
            )
        });
    });
}

fn bench_package_json_discovery(c: &mut Criterion) {
    let tmp = setup_test_dir();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

    c.bench_function("globwalk_package_json_discovery", |b| {
        let include = vec![ValidatedGlob::from_str("**/package.json").unwrap()];
        let exclude = vec![ValidatedGlob::from_str("**/node_modules/**").unwrap()];

        b.iter(|| {
            black_box(
                globwalk(
                    black_box(&base_path),
                    black_box(&include),
                    black_box(&exclude),
                    WalkType::Files,
                )
                .unwrap(),
            )
        });
    });
}

fn bench_doublestar_pattern(c: &mut Criterion) {
    let tmp = setup_test_dir();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

    c.bench_function("globwalk_doublestar", |b| {
        let include = vec![ValidatedGlob::from_str("**/*").unwrap()];
        let exclude: Vec<ValidatedGlob> = vec![];

        b.iter(|| {
            black_box(
                globwalk(
                    black_box(&base_path),
                    black_box(&include),
                    black_box(&exclude),
                    WalkType::All,
                )
                .unwrap(),
            )
        });
    });
}

criterion_group!(
    benches,
    bench_simple_glob,
    bench_complex_glob_with_excludes,
    bench_package_json_discovery,
    bench_doublestar_pattern,
);

criterion_main!(benches);
