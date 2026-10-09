//! Which output patterns `turbo clean` will expand.
//!
//! An allowlist, so that a sweeping or escaping pattern is refused instead of
//! deleting source:
//!
//! - no `..` segment anywhere, and no absolute paths;
//! - the first segment is a literal name (`dist/**`, `.next/**`,
//!   `tsconfig.tsbuildinfo`) or a brace set of literal names
//!   (`{dist,build}/**`);
//! - or the whole pattern is `*.<literal extension>` (`*.tsbuildinfo`);
//! - and wildcards never apply to a package directory or one of its ancestors
//!   (a root task's `packages/**`).

use crate::targets::ProtectedDirectories;

const GLOB_CHARACTERS: &[char] = &['*', '?', '[', ']', '{', '}', '!'];

fn is_literal(segment: &str) -> bool {
    !segment.contains(GLOB_CHARACTERS)
}

fn segments(pattern: &str) -> Vec<&str> {
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    pattern
        .split(separators)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect()
}

/// `{dist,build}` → `["dist", "build"]`, when every alternative is literal.
fn literal_alternatives(segment: &str) -> Option<Vec<&str>> {
    let inner = segment.strip_prefix('{')?.strip_suffix('}')?;
    let alternatives: Vec<&str> = inner.split(',').collect();
    alternatives
        .iter()
        .all(|alternative| {
            !alternative.is_empty()
                && is_literal(alternative)
                && *alternative != "."
                && *alternative != ".."
        })
        .then_some(alternatives)
}

/// `*.tsbuildinfo`, `*.d.ts`: any name with a literal extension.
fn is_extension_glob(segment: &str) -> bool {
    segment
        .strip_prefix("*.")
        .is_some_and(|extension| !extension.is_empty() && is_literal(extension))
}

/// Why `pattern`, relative to the package at `package_directory`, is refused.
pub(crate) fn refusal(
    pattern: &str,
    package_directory: &[String],
    protected: &ProtectedDirectories,
) -> Option<&'static str> {
    if pattern.starts_with('/') || pattern.starts_with('\\') || pattern.contains(':') {
        return Some("it is an absolute path");
    }
    let segments = segments(pattern);
    if segments.iter().any(|segment| {
        segment.contains("..")
            && (*segment == ".."
                || segment
                    .split([',', '{', '}'])
                    .any(|alternative| alternative == ".."))
    }) {
        return Some("it contains `..`");
    }
    let Some((first, rest)) = segments.split_first() else {
        return Some("it matches the whole package directory");
    };

    let bases: Vec<&str> = if is_literal(first) {
        vec![first]
    } else if let Some(alternatives) = literal_alternatives(first) {
        alternatives
    } else if rest.is_empty() && is_extension_glob(first) {
        return None;
    } else {
        return Some(
            "it starts with a wildcard; anchor it at an output directory such as `dist/**`",
        );
    };

    // The literal directories the wildcards apply to.
    let literal_rest = rest.iter().take_while(|segment| is_literal(segment));
    let literal_rest: Vec<&str> = literal_rest.copied().collect();
    let has_wildcards = literal_rest.len() < rest.len();
    if !has_wildcards {
        return None;
    }
    let sweeps_a_package = bases.iter().any(|base| {
        let directory: Vec<String> = package_directory
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(*base))
            .chain(literal_rest.iter().copied())
            .map(str::to_owned)
            .collect();
        protected.covers(&directory)
    });
    sweeps_a_package.then_some("it would sweep a package directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(pattern: &str) -> bool {
        refusal(
            pattern,
            &["packages".to_owned(), "web".to_owned()],
            &ProtectedDirectories::from_dirs(&["packages/web", "packages/web/nested"]),
        )
        .is_some()
    }

    #[test]
    fn allows_anchored_outputs() {
        for pattern in [
            "dist/**",
            "dist/**/*.js",
            ".next/**",
            "{dist,build}/**",
            "*.tsbuildinfo",
            "*.d.ts",
            "tsconfig.tsbuildinfo",
            "dist",
            "./dist/**",
            "out/nested/*.js",
        ] {
            assert!(!refused(pattern), "{pattern} should be allowed");
        }
    }

    #[test]
    fn refuses_escaping_or_sweeping_outputs() {
        for pattern in [
            "**",
            "**/*.js",
            "*",
            "*.*",
            "?*",
            "[a-z]*",
            "*.ts*",
            "",
            ".",
            "../b/*.*",
            "../../*.*",
            "dist/*/../../**",
            "dist/../../x",
            "{dist,..}/**",
            "{dist,*}/**",
            "/abs/**",
            "nested/**",
        ] {
            assert!(refused(pattern), "{pattern} should be refused");
        }
    }

    #[test]
    fn root_tasks_cannot_sweep_package_directories() {
        let protected = ProtectedDirectories::from_dirs(&["packages/web"]);
        assert!(refusal("packages/**", &[], &protected).is_some());
        assert!(refusal("packages/web/*.js", &[], &protected).is_some());
        assert!(refusal("packages/web/dist/**", &[], &protected).is_none());
        assert!(refusal("generated/**", &[], &protected).is_none());
        assert!(refusal("*.log", &[], &protected).is_none());
    }
}
