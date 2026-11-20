//! Differential test against cargo's dep-info: for every crate
//! cargo compiled (`target/debug/deps/*.d`,
//! `target/debug/build/*/build_script_build-*.d`), compare the source files
//! rustc read with our build-phase module closure from the same crate root
//! (`mod` edges, `include*!` embeds, the build-phase workspace `Cargo.toml`;
//! dependency crates are separate roots).
//!
//! Ignored by default. Build a project first, then:
//!
//! ```text
//! TURBO_TRACE_DEBUG_DEPINFO_ROOT=/path/to/ripgrep [TURBO_TRACE_DEBUG_DEPINFO_TARGET=/path/to/target] \
//!   cargo test -p turborepo-trace-lang-rust --test depinfo -- --ignored --nocapture
//! ```
//!
//! Mismatches are categorized:
//!
//! | Category | Meaning |
//! |---|---|
//! | `generated` | rustc read a file below `OUT_DIR`; we report `GeneratedFile` |
//! | `uncertain` | missing, but the crate has an uncertainty record we emitted (computed path, macro) |
//! | `conditional` | extra file reached only through CONDITIONAL edges (unknown `cfg`: `test`, `feature`, `doc`, …) |
//! | `changed since build` | the file is newer than the dep-info file (edited after the build) |
//! | `uncategorized` | anything else: a bug, or a case to categorize |
//!
//! The test fails if any rustc-read file is missing without a category.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::Instant,
};

use turborepo_trace_core::{RefFlags, RefKind, Target, Tracer, UncertaintyKind};
use turborepo_trace_lang_rust::RustLanguage;

/// One dep-info file: the crate root and every source file it lists.
struct DepInfo {
    root: PathBuf,
    mtime: std::time::SystemTime,
    deps: BTreeSet<PathBuf>,
}

fn unescape_split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&' ') => {
                cur.push(' ');
                chars.next();
            }
            ' ' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_depinfo(path: &Path, ws_root: &Path) -> Option<DepInfo> {
    let text = std::fs::read_to_string(path).ok()?;
    let first = text
        .lines()
        .find(|l| !l.starts_with('#') && l.contains(": "))?;
    let (_, rest) = first.split_once(": ")?;
    let mut deps = BTreeSet::new();
    let mut root = None;
    for d in unescape_split(rest) {
        let p = PathBuf::from(&d);
        let p = if p.is_absolute() { p } else { ws_root.join(p) };
        let p = turborepo_trace_core::fs::normalize(&p);
        if root.is_none() {
            root = Some(p.clone());
        }
        deps.insert(p);
    }
    Some(DepInfo {
        mtime: std::fs::metadata(path).and_then(|m| m.modified()).ok()?,
        root: root?,
        deps,
    })
}

fn collect_depinfo(target: &Path, ws_root: &Path) -> Vec<DepInfo> {
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(target.join("debug/deps")) {
        files.extend(
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "d")),
        );
    }
    if let Ok(rd) = std::fs::read_dir(target.join("debug/build")) {
        for e in rd.filter_map(Result::ok) {
            if let Ok(inner) = std::fs::read_dir(e.path()) {
                files.extend(inner.filter_map(Result::ok).map(|e| e.path()).filter(|p| {
                    p.extension().is_some_and(|e| e == "d")
                        && p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with("build_script_build"))
                }));
            }
        }
    }
    files.sort();
    files
        .iter()
        .filter_map(|f| parse_depinfo(f, ws_root))
        .filter(|d| d.root.extension().is_some_and(|e| e == "rs"))
        .collect()
}

#[derive(Default)]
struct Report {
    crates: usize,
    exact: usize,
    rustc_files: BTreeSet<PathBuf>,
    our_files: BTreeSet<PathBuf>,
    by_category: BTreeMap<&'static str, Vec<(PathBuf, PathBuf)>>,
}

#[test]
#[ignore = "needs a cargo-built project: set TURBO_TRACE_DEBUG_DEPINFO_ROOT"]
fn depinfo_differential() {
    let Some(root) = std::env::var_os("TURBO_TRACE_DEBUG_DEPINFO_ROOT").map(PathBuf::from) else {
        eprintln!("TURBO_TRACE_DEBUG_DEPINFO_ROOT not set; skipping");
        return;
    };
    let root = root.canonicalize().expect("root exists");
    let target = std::env::var_os("TURBO_TRACE_DEBUG_DEPINFO_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let infos = collect_depinfo(&target, &root);
    assert!(
        !infos.is_empty(),
        "no dep-info files below {}",
        target.display()
    );

    // One trace from every crate root (dependency crates included through Package
    // edges).
    let roots: BTreeSet<PathBuf> = infos.iter().map(|d| d.root.clone()).collect();
    let roots: Vec<PathBuf> = roots.into_iter().filter(|r| r.is_file()).collect();
    let tracer = Tracer::builder()
        .base(&root)
        .cwd(&root)
        .language(RustLanguage::new())
        .build()
        .expect("tracer");
    let t0 = Instant::now();
    let trace = tracer.trace(&roots).expect("trace");
    let trace_time = t0.elapsed();

    // Adjacency over module-tree edges only.
    let mut adj: HashMap<PathBuf, Vec<(PathBuf, bool)>> = HashMap::new();
    let mut uncertain_files: BTreeSet<PathBuf> = BTreeSet::new();
    for e in trace.edges() {
        let Some(from) = trace.path(e.from).map(Path::to_path_buf) else {
            continue;
        };
        if let Some(u) = &e.uncertainty
            && matches!(
                u.kind,
                UncertaintyKind::ComputedSpecifier
                    | UncertaintyKind::GeneratedFile
                    | UncertaintyKind::EnvDependent
                    | UncertaintyKind::UnresolvedSpecifier
                    | UncertaintyKind::PartialSpecifier
            )
            && matches!(e.kind, RefKind::Mod | RefKind::Embed)
        {
            uncertain_files.insert(from.clone());
        }
        // Module tree plus the workspace `Cargo.toml` (a build-phase manifest): rustc's
        // dep-info lists it when a crate inherits `[workspace.package]` fields.
        let manifest = e.kind == RefKind::Manifest
            && e.phase == turborepo_trace_core::Phase::Build
            && &*e.spec == "workspace Cargo.toml";
        if !matches!(e.kind, RefKind::Mod | RefKind::Embed) && !manifest {
            continue;
        }
        if let Target::File(id) = &e.to
            && let Some(p) = trace.path(*id)
        {
            adj.entry(from)
                .or_default()
                .push((p.to_path_buf(), e.flags.contains(RefFlags::CONDITIONAL)));
        }
    }
    // Per root: files reached, and whether only conditionally.
    let closure = |r: &Path| -> (BTreeMap<PathBuf, bool>, bool) {
        let mut seen: BTreeMap<PathBuf, bool> = BTreeMap::new();
        let mut stack = vec![(r.to_path_buf(), false)];
        let mut uncertain = false;
        while let Some((p, cond)) = stack.pop() {
            match seen.get(&p) {
                Some(&c) if c <= cond => continue,
                _ => {}
            }
            seen.insert(p.clone(), cond);
            uncertain |= uncertain_files.contains(&p);
            for (q, c) in adj.get(&p).into_iter().flatten() {
                stack.push((q.clone(), cond || *c));
            }
        }
        (seen, uncertain)
    };

    let mut rep = Report::default();
    let mut by_root: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    let mut built: BTreeMap<PathBuf, std::time::SystemTime> = BTreeMap::new();
    for d in &infos {
        by_root
            .entry(d.root.clone())
            .or_default()
            .extend(d.deps.iter().cloned());
        let t = built.entry(d.root.clone()).or_insert(d.mtime);
        *t = (*t).min(d.mtime);
    }
    let newer = |f: &Path, r: &Path| -> bool {
        let m = std::fs::metadata(f).and_then(|m| m.modified()).ok();
        matches!((m, built.get(r)), (Some(m), Some(b)) if m > *b)
    };
    for (r, rustc) in &by_root {
        rep.crates += 1;
        let (ours, uncertain) = closure(r);
        let mut ok = true;
        for f in rustc {
            rep.rustc_files.insert(f.clone());
            if ours.contains_key(f) {
                continue;
            }
            ok = false;
            let cat = if f.components().any(|c| c.as_os_str() == "out")
                && f.starts_with(target.join("debug/build"))
            {
                "generated"
            } else if newer(r, r) || !f.exists() {
                "changed since build"
            } else if uncertain {
                "uncertain (missing)"
            } else {
                "uncategorized (missing)"
            };
            rep.by_category
                .entry(cat)
                .or_default()
                .push((r.clone(), f.clone()));
        }
        for (f, cond) in &ours {
            rep.our_files.insert(f.clone());
            if rustc.contains(f) {
                continue;
            }
            ok = false;
            let cat = if *cond {
                "conditional (extra)"
            } else if newer(f, r) || newer(r, r) {
                "changed since build"
            } else {
                "uncategorized (extra)"
            };
            rep.by_category
                .entry(cat)
                .or_default()
                .push((r.clone(), f.clone()));
        }
        if ok {
            rep.exact += 1;
        }
    }

    let short = |p: &Path| -> String {
        let s = p.display().to_string();
        let home = std::env::var("HOME").unwrap_or_default();
        s.replace(&root.display().to_string(), "<root>")
            .replace(&format!("{home}/.cargo/registry/src/"), "<registry>/")
    };
    println!("dep-info differential for {}", root.display());
    println!(
        "  dep-info files: {}, crate roots: {}, exact matches: {}",
        infos.len(),
        rep.crates,
        rep.exact
    );
    println!(
        "  rustc source files: {}, our module-closure files: {}, trace total files: {}, trace \
         time: {:?}",
        rep.rustc_files.len(),
        rep.our_files.len(),
        trace.files().len(),
        trace_time
    );
    for (cat, items) in &rep.by_category {
        println!("  {cat}: {}", items.len());
        for (r, f) in items.iter().take(40) {
            println!("    {} <- {}", short(f), short(r));
        }
    }
    let missing = rep
        .by_category
        .get("uncategorized (missing)")
        .map_or(0, Vec::len);
    assert_eq!(missing, 0, "rustc read files we did not trace (see above)");
}
