//! Fixture snapshot support: trace a fixture tree and render
//! machine-independent JSON for [`insta`].

use std::path::{Path, PathBuf};

use turborepo_trace_core::{TracerBuilder, fs::normalize, output};

/// Traces `entries` (relative to `fixture_dir`) with `builder`, using the
/// fixture directory as the base, and returns the `json` output with the
/// absolute base redacted as `<fixture>`.
///
/// Used by [`crate::fixture_snapshot!`]; call it directly to post-process the
/// JSON.
///
/// # Panics
///
/// Panics if the tracer cannot be built, the trace fails, or the output cannot
/// be written: this is a test helper.
// Invariant: test support: the working directory exists; test support: writing to a Vec cannot
// fail; test support: the JSON writer emits UTF-8.
#[allow(clippy::expect_used)]
pub fn fixture_json(
    builder: TracerBuilder,
    fixture_dir: impl AsRef<Path>,
    entries: &[&str],
) -> String {
    let dir = fixture_dir.as_ref();
    let dir = if dir.is_absolute() {
        normalize(dir)
    } else {
        normalize(&std::env::current_dir().expect("cwd").join(dir))
    };
    assert!(
        dir.is_dir(),
        "fixture directory {} does not exist",
        dir.display()
    );
    let tracer = builder
        .base(&dir)
        .cwd(&dir)
        .build()
        .unwrap_or_else(|e| panic!("cannot build tracer: {e}"));
    let entries: Vec<PathBuf> = entries.iter().map(|e| dir.join(e)).collect();
    let trace = tracer
        .trace(&entries)
        .unwrap_or_else(|e| panic!("trace of {entries:?} failed: {e}"));
    let mut out = Vec::new();
    output::write_json(&trace, &mut out).expect("write json");
    let json = String::from_utf8(out).expect("json output is UTF-8");
    redact_base(&json, &dir)
}

/// Replaces every occurrence of `base` (as a JSON string or path prefix) with
/// `<fixture>`.
// Invariant: test support: a string always serializes.
#[allow(clippy::expect_used)]
pub fn redact_base(json: &str, base: &Path) -> String {
    let base = base.to_string_lossy();
    // Paths are JSON-escaped in the output; escape the needle the same way.
    let escaped = serde_json::to_string(base.as_ref()).expect("serialize path");
    let needle = &escaped[1..escaped.len() - 1];
    let mut out = String::with_capacity(json.len());
    let mut rest = json;
    while let Some(i) = rest.find(needle) {
        let after = &rest[i + needle.len()..];
        out.push_str(&rest[..i]);
        // Only whole path components: `/a/b` must not match `/a/bc`.
        if after.starts_with('"') || after.starts_with('/') {
            out.push_str("<fixture>");
        } else {
            out.push_str(needle);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_base_prefixes() {
        let json = r#"{"base": "/a/b", "files": ["/a/b/c", "/a/bc", "x"]}"#;
        assert_eq!(
            redact_base(json, Path::new("/a/b")),
            r#"{"base": "<fixture>", "files": ["<fixture>/c", "/a/bc", "x"]}"#
        );
    }
}
