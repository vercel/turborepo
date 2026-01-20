//! Benchmarks for globwalk operations

use std::str::FromStr;

use codspeed_criterion_compat::{Criterion, black_box, criterion_group, criterion_main};
use globwalk::{
    Settings, ValidatedGlob, WalkType, fix_glob_pattern, globwalk, globwalk_with_settings,
};
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


fn bench_fix_glob_pattern_simple(c: &mut Criterion) {
    c.bench_function("fix_glob_pattern_simple", |b| {
        b.iter(|| black_box(fix_glob_pattern(black_box("packages/*/src/**/*.ts"))))
    });
}

fn bench_fix_glob_pattern_double_doublestar(c: &mut Criterion) {
    // Tests the double_doublestar regex: **/** -> **
    c.bench_function("fix_glob_pattern_double_doublestar", |b| {
        b.iter(|| black_box(fix_glob_pattern(black_box("**/**/**/*.ts"))))
    });
}

fn bench_fix_glob_pattern_leading_doublestar(c: &mut Criterion) {
    // Tests the leading_doublestar regex: **token -> **/*token
    c.bench_function("fix_glob_pattern_leading_doublestar", |b| {
        b.iter(|| black_box(fix_glob_pattern(black_box("**token/foo/bar"))))
    });
}

fn bench_fix_glob_pattern_trailing_doublestar(c: &mut Criterion) {
    // Tests the trailing_doublestar regex: token** -> token*/**
    c.bench_function("fix_glob_pattern_trailing_doublestar", |b| {
        b.iter(|| black_box(fix_glob_pattern(black_box("foo/bar**"))))
    });
}

fn bench_fix_glob_pattern_complex(c: &mut Criterion) {
    // Pattern that triggers multiple regex replacements
    c.bench_function("fix_glob_pattern_complex", |b| {
        b.iter(|| black_box(fix_glob_pattern(black_box("**token/**/**/**/suffix**"))))
    });
}


fn bench_validated_glob_simple(c: &mut Criterion) {
    c.bench_function("validated_glob_simple", |b| {
        b.iter(|| black_box(ValidatedGlob::from_str(black_box("**/*.ts")).unwrap()))
    });
}

fn bench_validated_glob_with_traversal(c: &mut Criterion) {
    // Tests path cleaning with .. and .
    c.bench_function("validated_glob_with_traversal", |b| {
        b.iter(|| {
            black_box(
                ValidatedGlob::from_str(black_box("packages/../apps/./web/**/*.tsx")).unwrap(),
            )
        })
    });
}

fn bench_validated_glob_deep_path(c: &mut Criterion) {
    c.bench_function("validated_glob_deep_path", |b| {
        b.iter(|| {
            black_box(
                ValidatedGlob::from_str(black_box(
                    "packages/ui/src/components/buttons/primary/**/*.ts",
                ))
                .unwrap(),
            )
        })
    });
}

#[cfg(unix)]
fn bench_validated_glob_with_colon(c: &mut Criterion) {
    // Unix-only: colons get escaped
    c.bench_function("validated_glob_with_colon", |b| {
        b.iter(|| black_box(ValidatedGlob::from_str(black_box("packages/foo:bar/**")).unwrap()))
    });
}


fn bench_globwalk_ignore_nested_packages(c: &mut Criterion) {
    let tmp = setup_test_dir();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

    c.bench_function("globwalk_ignore_nested_packages", |b| {
        let include = vec![ValidatedGlob::from_str("**/*.ts").unwrap()];
        let exclude: Vec<ValidatedGlob> = vec![];
        let settings = Settings::default().ignore_nested_packages();

        b.iter(|| {
            black_box(
                globwalk_with_settings(
                    black_box(&base_path),
                    black_box(&include),
                    black_box(&exclude),
                    WalkType::Files,
                    settings,
                )
                .unwrap(),
            )
        });
    });
}

fn bench_globwalk_path_with_special_chars(c: &mut Criterion) {
    let tmp = setup_test_dir();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

    // Base path escaping is tested when the path contains glob-special characters
    // We can't easily create paths with special chars, but we can benchmark
    // patterns that exercise the preprocessing pipeline heavily
    c.bench_function("globwalk_many_excludes", |b| {
        let include = vec![ValidatedGlob::from_str("**/*").unwrap()];
        let exclude = vec![
            ValidatedGlob::from_str("**/node_modules/**").unwrap(),
            ValidatedGlob::from_str("**/dist/**").unwrap(),
            ValidatedGlob::from_str("**/.turbo/**").unwrap(),
            ValidatedGlob::from_str("**/.git/**").unwrap(),
            ValidatedGlob::from_str("**/coverage/**").unwrap(),
            ValidatedGlob::from_str("**/.next/**").unwrap(),
            ValidatedGlob::from_str("**/build/**").unwrap(),
            ValidatedGlob::from_str("**/.cache/**").unwrap(),
        ];

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

fn bench_globwalk_traversal_patterns(c: &mut Criterion) {
    let tmp = setup_test_dir();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

    // Tests collapse_path indirectly
    c.bench_function("globwalk_traversal_patterns", |b| {
        let include = vec![
            ValidatedGlob::from_str("packages/../packages/*/src/**/*.ts").unwrap(),
            ValidatedGlob::from_str("apps/./web/src/**/*.tsx").unwrap(),
        ];
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

fn bench_globwalk_many_includes(c: &mut Criterion) {
    let tmp = setup_test_dir();
    let base_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

    c.bench_function("globwalk_many_includes", |b| {
        let include = vec![
            ValidatedGlob::from_str("packages/ui/src/**/*.ts").unwrap(),
            ValidatedGlob::from_str("packages/utils/src/**/*.ts").unwrap(),
            ValidatedGlob::from_str("packages/config/src/**/*.ts").unwrap(),
            ValidatedGlob::from_str("apps/web/src/**/*.tsx").unwrap(),
            ValidatedGlob::from_str("apps/docs/src/**/*.mdx").unwrap(),
        ];
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

criterion_group!(
    benches,
    // Original benchmarks
    bench_simple_glob,
    bench_complex_glob_with_excludes,
    bench_package_json_discovery,
    bench_doublestar_pattern,
    // fix_glob_pattern benchmarks
    bench_fix_glob_pattern_simple,
    bench_fix_glob_pattern_double_doublestar,
    bench_fix_glob_pattern_leading_doublestar,
    bench_fix_glob_pattern_trailing_doublestar,
    bench_fix_glob_pattern_complex,
    // ValidatedGlob benchmarks
    bench_validated_glob_simple,
    bench_validated_glob_with_traversal,
    bench_validated_glob_deep_path,
    // globwalk_with_settings benchmarks
    bench_globwalk_ignore_nested_packages,
    // Preprocessing/pipeline benchmarks
    bench_globwalk_path_with_special_chars,
    bench_globwalk_traversal_patterns,
    bench_globwalk_many_includes,
);

#[cfg(unix)]
criterion_group!(unix_benches, bench_validated_glob_with_colon,);

#[cfg(unix)]
criterion_main!(benches, unix_benches);

#[cfg(not(unix))]
criterion_main!(benches);
