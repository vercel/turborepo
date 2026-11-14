//! nft fixture parity: runs the JavaScript tracer on every
//! case of [`@vercel/nft`](https://github.com/vercel/nft)'s `test/unit` suite (vendored under
//! `fixtures/js/nft/`, see its README) and compares the file set with nft's
//! `output.js`.
//!
//! The suite is a report, not a gate: each case is classified as PASS, SUPERSET
//! (we emit extra files), MISSING (we miss files) or ERROR, a table is printed
//! (`--nocapture`) and `target/nft-parity/report.json` + `summary.md` are
//! written. The test only fails when a case listed in
//! `fixtures/js/nft/baseline.json` no longer passes.
//!
//! nft runs each case with `base` = its repo root and `processCwd` = the case
//! directory, so expected paths look like `package.json`,
//! `test/unit/<case>/input.js` and `node_modules/<pkg>/...` (nft's own dev
//! dependencies, hoisted). The harness rebuilds that layout under
//! `target/nft-parity/root/`: `package.json` from `fixtures/js/nft/_root`, a
//! fresh copy of the cases in `test/unit/`, and `node_modules/` installed by
//! the trace's development tooling (pinned by nft's `pnpm-lock.yaml`). Cases
//! that need packages that are not installed are skipped.
//!
//! Environment:
//! - `NFT_PARITY_UPDATE_BASELINE=1`: rewrite `baseline.json` from this run.
//! - `NFT_PARITY_FILTER=a,b`: only run cases whose name contains one of the
//!   substrings.
//! - `NFT_PARITY_REQUIRE_DEPS=1`: fail if nft's npm dependencies are not
//!   installed.
//! - `NFT_PARITY_ROOT=<dir>`: use another root (default
//!   `target/nft-parity/root`).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod nft_parity {
    pub mod cases;
    pub mod lang;
    pub mod report;
}

use std::{
    collections::BTreeSet,
    fs, io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    time::Instant,
};

use nft_parity::{
    cases::{self, Case},
    lang::NftJs,
    report::{Baseline, CaseResult, Check, Status},
};
use serde_json::json;
use turborepo_trace_core::{Target, Trace, Tracer, os::symlink};
use turborepo_trace_lang_js::JsLanguage;

const WORKSPACE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// Stands for "some platform prebuild was emitted" in the node-gyp cases.
const PREBUILD_SENTINEL: &str = "<any prebuilds/**/*.node>";

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0")
}

fn target_dir() -> PathBuf {
    let ws = PathBuf::from(WORKSPACE);
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(t) => ws.join(t),
        None => ws.join("target"),
    }
}

/// Recursively copies `from` to `to`, recreating symlinks as symlinks.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        symlink(fs::read_link(from)?, to)
    } else if meta.is_dir() {
        fs::create_dir_all(to)?;
        let mut entries: Vec<_> = fs::read_dir(from)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(drop)
    }
}

/// Builds the nft-like root: `package.json` and a fresh `test/unit/`.
fn materialize(fixtures: &Path, root: &Path, cases: &[Case]) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let pkg = fs::read(fixtures.join("_root/package.json"))?;
    if fs::read(root.join("package.json")).ok().as_deref() != Some(&pkg[..]) {
        fs::write(root.join("package.json"), pkg)?;
    }
    let unit = root.join("test/unit");
    if fs::symlink_metadata(&unit).is_ok() {
        fs::remove_dir_all(&unit)?;
    }
    fs::create_dir_all(&unit)?;
    for c in cases {
        copy_tree(&fixtures.join(&c.name), &unit.join(&c.name))?;
    }
    Ok(())
}

/// Files of `trace` relative to the base, plus the ones outside it.
fn file_sets<'t>(
    trace: &Trace,
    files: impl Iterator<Item = &'t Path>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut inside = BTreeSet::new();
    let mut outside = BTreeSet::new();
    for p in files {
        match trace.relative(p) {
            Some(r) => inside.insert(r.to_string_lossy().into_owned()),
            None => outside.insert(p.to_string_lossy().into_owned()),
        };
    }
    (inside, outside)
}

fn compare(
    case: &Case,
    label: &str,
    expected: &BTreeSet<String>,
    mut actual: BTreeSet<String>,
) -> Check {
    let mut expected = expected.clone();
    let found_prebuild = cases::normalize(&case.name, &mut actual);
    if case.name == "webpack-wrapper-strs-namespaces-large" {
        expected = expected
            .iter()
            .map(|f| cases::normalize_react_is(f))
            .collect();
    }
    if cases::NODE_GYP_TESTS.contains(&case.name.as_str()) {
        expected.insert(PREBUILD_SENTINEL.to_owned());
        if found_prebuild {
            actual.insert(PREBUILD_SENTINEL.to_owned());
        }
    }
    Check::new(label, &expected, &actual)
}

/// Runs one case: a cold trace, then a warm one on the same tracer (nft's
/// `doTrace(true)`).
fn run_case(case: &Case, root: &Path) -> Result<(Vec<Check>, BTreeSet<String>, usize), String> {
    let dir = root.join("test/unit").join(&case.name);
    let paths: Vec<(String, String)> = cases::NFT_PATHS
        .iter()
        .map(|(from, to)| {
            (
                from.to_string(),
                root.join(to).to_string_lossy().into_owned(),
            )
        })
        .collect();
    let mut builder = Tracer::builder()
        .base(root)
        .cwd(&dir)
        // unit.test.js ignores its own `actual.js` dumps.
        .ignore("actual.js");
    if let Some(c) = &case.conditions {
        builder = builder.conditions(cases::turbotrace_conditions(c));
    }
    // Only the esm-paths cases use the `paths` aliases unit.test.js passes to every
    // case.
    let uses_paths = case.name.starts_with("esm-paths");
    builder = if case.exports_only || uses_paths {
        builder.language(NftJs {
            exports_only: case.exports_only,
            paths: if uses_paths { paths } else { Vec::new() },
        })
    } else {
        builder.language(JsLanguage)
    };
    let tracer = builder.build().map_err(|e| format!("build: {e}"))?;

    let entries: Vec<PathBuf> = case.inputs.iter().map(|i| dir.join(i)).collect();
    let cold = tracer.trace(&entries).map_err(|e| format!("trace: {e}"))?;
    let (cold_files, outside) = file_sets(&cold, cold.files());
    let uncertainties = cold.uncertainties().len();
    let mut checks = vec![compare(case, "cold", &case.expected, cold_files.clone())];

    if case.name == "multi-input" {
        for (entry, expected) in cases::MULTI_INPUT_CLOSURES {
            let entry_path = dir.join(entry);
            let label = format!("closure:{entry}");
            let Some(idx) = cold.entry_index(&entry_path) else {
                let entries: Vec<_> = cold.entries().collect();
                return Err(format!(
                    "{label}: {} is not an entry of the trace (entries: {entries:?})",
                    entry_path.display()
                ));
            };
            let (mut files, _) = file_sets(&cold, cold.entry_files(idx));
            // nft collects the files whose parents are in the closure, so the entry itself
            // only counts when another file of its closure references it (a cycle).
            let entry_id = cold.file_id(&entry_path);
            let closure: BTreeSet<_> = cold
                .entry_files(idx)
                .filter_map(|p| cold.file_id(p))
                .filter(|id| Some(*id) != entry_id)
                .collect();
            let referenced = cold.edges().iter().any(|e| {
                closure.contains(&e.from) && matches!(e.to, Target::File(t) if Some(t) == entry_id)
            });
            if !referenced {
                files.remove(&format!("test/unit/{}/{entry}", case.name));
            }
            let expected: BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
            checks.push(compare(case, &label, &expected, files));
        }
    }

    let entries: Vec<PathBuf> = case.cached_inputs.iter().map(|i| dir.join(i)).collect();
    let warm = tracer
        .trace(&entries)
        .map_err(|e| format!("cached trace: {e}"))?;
    let (warm_files, _) = file_sets(&warm, warm.files());
    if case.cached_inputs == case.inputs && warm_files != cold_files {
        let only_cold: Vec<_> = cold_files.difference(&warm_files).collect();
        let only_warm: Vec<_> = warm_files.difference(&cold_files).collect();
        return Err(format!(
            "warm-cache trace differs from cold trace: only cold {only_cold:?}, only warm \
             {only_warm:?}"
        ));
    }
    checks.push(compare(case, "cached", &case.cached_expected, warm_files));
    Ok((checks, outside, uncertainties))
}

fn missing_packages(case: &Case, root: &Path) -> Vec<String> {
    case.npm_packages
        .iter()
        .filter(|p| {
            !root
                .join("node_modules")
                .join(p)
                .join("package.json")
                .is_file()
        })
        .cloned()
        .collect()
}

#[test]
fn nft_parity() {
    let fixtures = PathBuf::from(WORKSPACE).join("crates/turborepo-trace-testkit/fixtures/js/nft");
    let out = target_dir().join("nft-parity");
    let root = std::env::var_os("NFT_PARITY_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("root"));
    let upstream = fs::read_to_string(fixtures.join("_root/UPSTREAM")).unwrap_or_default();
    let nft_rev = upstream
        .lines()
        .find_map(|l| l.strip_prefix("rev="))
        .unwrap_or("unknown")
        .to_owned();

    let all = cases::load(&fixtures).expect("load nft cases");
    let filter: Vec<String> = std::env::var("NFT_PARITY_FILTER")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let cases: Vec<Case> = all
        .into_iter()
        .filter(|c| filter.is_empty() || filter.iter().any(|f| c.name.contains(f.as_str())))
        .collect();
    materialize(&fixtures, &root, &cases).expect("materialize nft root");
    // Traces report real paths; compare against the canonical root (no `..`, no
    // symlinks).
    let root = fs::canonicalize(&root).expect("canonicalize nft root");
    let deps_installed = root.join("node_modules").is_dir();
    if env_flag("NFT_PARITY_REQUIRE_DEPS") {
        assert!(
            deps_installed,
            "nft npm dependencies missing in {}; the development tooling installs them",
            root.display()
        );
    }

    let started = Instant::now();
    let mut results = Vec::with_capacity(cases.len());
    for case in &cases {
        let mut emulated = Vec::new();
        if case.name.starts_with("esm-paths") {
            emulated.push("paths".to_owned());
        }
        if case.exports_only {
            emulated.push("exportsOnly".to_owned());
        }
        if case.conditions.is_some() {
            emulated.push("conditions".to_owned());
        }
        let mut r = CaseResult {
            name: case.name.clone(),
            status: Status::Skip,
            inputs: case.inputs.clone(),
            test_opts: case.test_opts.clone(),
            emulated,
            unsupported: case.unsupported.clone(),
            npm_packages: case.npm_packages.clone(),
            checks: Vec::new(),
            outside_base: BTreeSet::new(),
            uncertainties: 0,
            error: None,
            skip: case.skip.clone(),
            ms: 0.0,
        };
        let missing_pkgs = missing_packages(case, &root);
        if r.skip.is_none() && !missing_pkgs.is_empty() {
            r.skip = Some(format!(
                "requires-npm: {} not installed (the development tooling installs them)",
                missing_pkgs.join(", ")
            ));
        }
        if r.skip.is_none() {
            let t = Instant::now();
            let outcome = catch_unwind(AssertUnwindSafe(|| run_case(case, &root)));
            r.ms = t.elapsed().as_secs_f64() * 1000.0;
            match outcome {
                Ok(Ok((checks, outside, unc))) => {
                    r.status = checks
                        .iter()
                        .map(Check::status)
                        .max()
                        .unwrap_or(Status::Pass);
                    r.checks = checks;
                    r.outside_base = outside;
                    r.uncertainties = unc;
                }
                Ok(Err(e)) => {
                    r.status = Status::Error;
                    r.error = Some(e);
                }
                Err(panic) => {
                    r.status = Status::Error;
                    let msg = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "non-string panic".into());
                    r.error = Some(format!("panic: {msg}"));
                }
            }
        }
        results.push(r);
    }
    let elapsed = started.elapsed();

    let header = format!(
        "## nft parity (nft {nft_rev}, {} cases, deps {}, {:.2}s)",
        results.len(),
        if deps_installed {
            "installed"
        } else {
            "missing"
        },
        elapsed.as_secs_f64()
    );
    let table = nft_parity::report::table(&results, &header);
    println!("{table}");
    let meta = json!({
        "nft_repo": "https://github.com/vercel/nft",
        "nft_rev": nft_rev,
        "root": root,
        "deps_installed": deps_installed,
        "filter": filter,
        "elapsed_ms": elapsed.as_millis() as u64,
    });
    let report = nft_parity::report::report_json(&results, meta);
    fs::create_dir_all(&out).expect("create target/nft-parity");
    fs::write(
        out.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .expect("write report.json");
    fs::write(out.join("summary.md"), &table).expect("write summary.md");
    eprintln!(
        "nft parity report: {}",
        out.join("report.json").to_string_lossy()
    );

    let baseline_path = fixtures.join("baseline.json");
    let baseline = Baseline::load(&baseline_path);
    if env_flag("NFT_PARITY_UPDATE_BASELINE") {
        assert!(
            filter.is_empty(),
            "refusing to update the baseline from a filtered run"
        );
        let pass = Baseline::updated(baseline.as_ref(), &results);
        let doc = json!({
            "comment": "Cases that matched nft's output.js exactly. nft_parity fails if one of \
                        them stops passing. Update with NFT_PARITY_UPDATE_BASELINE=1.",
            "nft_rev": nft_rev,
            "pass": pass,
        });
        fs::write(
            &baseline_path,
            serde_json::to_string_pretty(&doc).unwrap() + "\n",
        )
        .expect("write baseline.json");
        eprintln!(
            "updated {} ({} passing)",
            baseline_path.display(),
            pass.len()
        );
        return;
    }
    if let Some(b) = baseline {
        let improvements = b.improvements(&results);
        if !improvements.is_empty() {
            eprintln!(
                "newly passing (update the baseline with NFT_PARITY_UPDATE_BASELINE=1): {}",
                improvements.join(", ")
            );
        }
        let regressions = b.regressions(&results);
        assert!(
            regressions.is_empty(),
            "nft parity regressions against fixtures/js/nft/baseline.json:\n  {}",
            regressions.join("\n  ")
        );
    }
}
