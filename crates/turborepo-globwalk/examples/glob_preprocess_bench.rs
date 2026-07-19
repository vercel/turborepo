//! Micro-benchmark for the glob-preprocessing fast paths.
//!
//! Compares the previous regex-based implementations of `fix_glob_pattern` and
//! `escape_glob_literals` against the current ones (which skip the regex engine
//! when the input has no `**` token / no glob metacharacters).
//!
//! Run with:
//!
//! ```sh
//! cargo run -p globwalk --release --example glob_preprocess_bench
//! ```
//!
//! The example is dependency-free (uses `std::time` and the `regex` crate that
//! `globwalk` already depends on) so it runs offline and produces directly
//! comparable before/after numbers.

// This is a throwaway benchmark harness; the regex reference impls genuinely
// need to unwrap their (statically valid) patterns.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    borrow::Cow,
    hint::black_box,
    sync::OnceLock,
    time::{Duration, Instant},
};

use globwalk::fix_glob_pattern;
use regex::Regex;

// --- Previous ("old") implementations, copied verbatim from the pre-change
// --- source so the benchmark measures the same work the old code did. ---

fn old_double_doublestar() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\*\*(?:/\*\*)+").unwrap())
}
fn old_leading_doublestar() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\*\*(?P<suffix>[^*/]+)").unwrap())
}
fn old_trailing_doublestar() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?P<prefix>[^*/]+)\*\*").unwrap())
}
fn old_glob_literals() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?<literal>[\?\*\$:<>\(\)\[\]{},])").unwrap())
}

fn old_fix_glob_pattern(pattern: &str) -> Cow<'_, str> {
    let p0: Cow<'_, str> = Cow::Borrowed(pattern);
    let p1 = old_double_doublestar().replace(&p0, "**");
    let p2 = old_leading_doublestar().replace(&p1, "**/*$suffix");
    let p3 = old_trailing_doublestar().replace(&p2, "$prefix*/**");
    match p3 {
        Cow::Borrowed(s) if std::ptr::eq(s, pattern) => Cow::Borrowed(pattern),
        Cow::Borrowed(s) => Cow::Owned(s.to_string()),
        Cow::Owned(s) => Cow::Owned(s),
    }
}

fn old_escape_glob_literals(literal_glob: &str) -> Cow<'_, str> {
    old_glob_literals().replace_all(literal_glob, "\\$literal")
}

// --- Current ("new") escape implementation. `fix_glob_pattern` is exercised
// --- through the public API; `escape_glob_literals` is private, so its current
// --- logic is mirrored here (kept in sync with `src/lib.rs`). ---

#[inline]
fn is_glob_literal(c: char) -> bool {
    matches!(
        c,
        '?' | '*' | '$' | ':' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ','
    )
}

fn new_escape_glob_literals(literal_glob: &str) -> Cow<'_, str> {
    if !literal_glob.contains(is_glob_literal) {
        return Cow::Borrowed(literal_glob);
    }
    let mut escaped = String::with_capacity(literal_glob.len() + 8);
    for c in literal_glob.chars() {
        if is_glob_literal(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    Cow::Owned(escaped)
}

/// Globs representative of what turbo feeds through preprocessing: task
/// input/output globs, workspace globs, and base paths. The majority contain no
/// `**` token / no metacharacters, which is exactly the case the fast paths
/// target.
fn corpus() -> Vec<String> {
    let globs = [
        "package.json",
        "tsconfig.json",
        "src/index.ts",
        "dist",
        "dist/",
        ".next",
        "build",
        "src/*.ts",
        "*.js",
        "lib/**",
        "**/*.ts",
        "src/**/*.tsx",
        "packages/*/package.json",
        "apps/*/dist/**",
        "!**/*.test.ts",
        "coverage/**",
        "node_modules",
        "README.md",
        "public/assets/images/logo.svg",
        "very/deeply/nested/directory/structure/without/any/globs/file.json",
    ];
    globs.iter().map(|s| s.to_string()).collect()
}

/// Absolute-ish base paths, the typical input to `escape_glob_literals`.
fn base_path_corpus() -> Vec<String> {
    [
        "/home/user/projects/monorepo",
        "/home/user/projects/monorepo/apps/web",
        "/home/user/projects/monorepo/packages/ui/src",
        "/var/lib/ci/workspace/build/output",
        "/Users/dev/code/company/very/deep/package/path",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn bench<F>(iters: u32, corpus: &[String], mut f: F) -> Duration
where
    F: FnMut(&str) -> usize,
{
    // Warm up.
    for _ in 0..5 {
        for s in corpus {
            black_box(f(black_box(s)));
        }
    }
    let start = Instant::now();
    for _ in 0..iters {
        for s in corpus {
            black_box(f(black_box(s)));
        }
    }
    start.elapsed()
}

fn report(label: &str, calls: u64, old: Duration, new: Duration) {
    let old_ns = old.as_nanos() as f64 / calls as f64;
    let new_ns = new.as_nanos() as f64 / calls as f64;
    let speedup = old_ns / new_ns;
    println!("{label}");
    println!("  old (regex): {old_ns:8.1} ns/call  ({old:?} total)");
    println!("  new (fast) : {new_ns:8.1} ns/call  ({new:?} total)");
    println!(
        "  speedup    : {speedup:.2}x  ({:.1}% faster)\n",
        (1.0 - new_ns / old_ns) * 100.0
    );
}

fn main() {
    // Correctness gate: the benchmark must only compare identical behavior.
    for s in corpus().iter().chain(base_path_corpus().iter()) {
        assert_eq!(
            old_fix_glob_pattern(s).as_ref(),
            fix_glob_pattern(s).as_ref(),
            "fix_glob_pattern diverged for {s:?}"
        );
        assert_eq!(
            old_escape_glob_literals(s).as_ref(),
            new_escape_glob_literals(s).as_ref(),
            "escape_glob_literals diverged for {s:?}"
        );
    }

    let iters: u32 = 200_000;

    let fix_corpus = corpus();
    let calls = iters as u64 * fix_corpus.len() as u64;
    let old = bench(iters, &fix_corpus, |s| old_fix_glob_pattern(s).len());
    let new = bench(iters, &fix_corpus, |s| fix_glob_pattern(s).len());
    report("fix_glob_pattern (mixed task glob corpus)", calls, old, new);

    let esc_corpus = base_path_corpus();
    let calls = iters as u64 * esc_corpus.len() as u64;
    let old = bench(iters, &esc_corpus, |s| old_escape_glob_literals(s).len());
    let new = bench(iters, &esc_corpus, |s| new_escape_glob_literals(s).len());
    report("escape_glob_literals (base path corpus)", calls, old, new);
}
