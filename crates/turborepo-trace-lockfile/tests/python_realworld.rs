//! Parses real Python lockfiles from `$TURBO_TRACE_DEBUG_PY_LOCKS` (a directory
//! with any of `uv.lock`, `poetry.lock`, `Pipfile.lock`, `requirements*.txt`),
//! as produced by the tools themselves. Skipped when the variable is unset.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_lockfile::{diff, parse_path};

#[test]
fn python_lockfiles_from_tools() {
    let Ok(dir) = std::env::var("TURBO_TRACE_DEBUG_PY_LOCKS") else {
        eprintln!("TURBO_TRACE_DEBUG_PY_LOCKS not set; skipping");
        return;
    };
    let mut seen = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("lock dir")
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let interesting = matches!(name.as_str(), "uv.lock" | "poetry.lock" | "Pipfile.lock")
            || (name.starts_with("requirements") && name.ends_with(".txt"));
        if !interesting {
            continue;
        }
        let lock = parse_path(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let with_hash = lock
            .packages()
            .iter()
            .filter(|p| p.integrity.is_some())
            .count();
        let root = lock
            .importers()
            .iter()
            .map(|i| format!("{}({} deps)", i.path, i.dependencies.len()))
            .collect::<Vec<_>>()
            .join(" ");
        let closure = lock.importer_closure(".", true).len();
        eprintln!(
            "{name}: format={} version={} packages={} with_hash={with_hash} importers=[{root}] \
             closure(.)={closure}",
            lock.format(),
            lock.lockfile_version(),
            lock.packages().len(),
        );
        assert!(!lock.packages().is_empty(), "{name}: no packages");
        assert!(diff(&lock, &lock).is_empty(), "{name}: self-diff not empty");
        seen += 1;
    }
    assert!(seen > 0, "no Python lockfiles in {dir}");
}
