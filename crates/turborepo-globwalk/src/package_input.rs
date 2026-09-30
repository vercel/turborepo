//! Resolution of package-relative input globs.
//!
//! Task `inputs` are written relative to the task's package directory, but
//! may reach outside of it (e.g. `$TURBO_ROOT$/tsconfig.json`, which expands
//! to `../../tsconfig.json`). Everything that consumes inputs — the git and
//! manual file hashers, and `--affected` task matching — needs to agree on
//! what a given input refers to. [`PackageInput::resolve`] is the single
//! place that decides this: it joins the input onto the package directory,
//! normalizes it with [`ValidatedGlob`] rules, and classifies the result.

use std::{borrow::Cow, str::FromStr};

use crate::{GlobError, ValidatedGlob, is_glob_pattern};

/// A task input glob resolved against its package directory.
#[derive(Clone, Debug)]
pub struct PackageInput {
    /// The repo-root-relative glob, normalized with [`ValidatedGlob`] rules.
    glob: ValidatedGlob,
    is_exclusion: bool,
    is_literal: bool,
    /// Byte offset into `glob` where the package-relative portion starts, or
    /// `None` when the glob is not contained in the package directory.
    package_relative_start: Option<usize>,
}

impl PackageInput {
    /// Resolves a raw input (optionally prefixed with `!` for exclusions)
    /// against `package_unix_path`, the repo-root-relative Unix path of the
    /// package (empty for the root package).
    pub fn resolve(package_unix_path: &str, raw: &str) -> Result<Self, GlobError> {
        let (is_exclusion, pattern) = raw
            .strip_prefix('!')
            .map_or((false, raw), |pattern| (true, pattern));

        let package = package_unix_path.trim_end_matches('/');
        let pattern_without_slash = pattern.trim_start_matches('/');
        let mut joined = String::with_capacity(package.len() + 1 + pattern_without_slash.len());
        joined.push_str(package);
        joined.push('/');
        joined.push_str(pattern_without_slash);
        let glob = strip_leading_slashes(ValidatedGlob::from_str(&joined)?);

        let package_relative_start = package_relative_start(glob.as_str(), package);

        Ok(Self {
            glob,
            is_exclusion,
            is_literal: !is_glob_pattern(pattern),
            package_relative_start,
        })
    }

    /// Whether the input was written as an exclusion (`!pattern`).
    pub fn is_exclusion(&self) -> bool {
        self.is_exclusion
    }

    /// Whether the input contains no glob metacharacters, i.e. it names a
    /// single file or directory.
    pub fn is_literal(&self) -> bool {
        self.is_literal
    }

    /// The resolved, repo-root-relative glob (without the `!` prefix).
    pub fn glob(&self) -> &ValidatedGlob {
        &self.glob
    }

    /// The resolved, repo-root-relative glob as a string.
    pub fn as_str(&self) -> &str {
        self.glob.as_str()
    }

    pub fn into_glob(self) -> ValidatedGlob {
        self.glob
    }

    /// Whether the glob resolves outside the repository root (e.g.
    /// `../../../x` from a package two levels deep). Such globs cannot match
    /// any repo-relative path.
    pub fn escapes_repo_root(&self) -> bool {
        let glob = self.glob.as_str();
        glob == ".." || glob.starts_with("../")
    }

    /// Whether the glob can match files outside the package directory.
    pub fn reaches_outside_package(&self) -> bool {
        self.package_relative_start.is_none()
    }

    /// The glob relative to the package directory, when it is contained in
    /// it. The package directory itself is returned as `"."`.
    pub fn package_relative(&self) -> Option<&str> {
        let start = self.package_relative_start?;
        let relative = &self.glob.as_str()[start..];
        Some(if relative.is_empty() { "." } else { relative })
    }
}

/// The root package joins as `/{input}`. `ValidatedGlob` strips that leading
/// slash on Unix but not on Windows, so strip it here to keep resolved globs
/// repo-root-relative on every platform. The walker trims leading slashes when
/// joining globs onto its base path, so walked files are unchanged.
fn strip_leading_slashes(glob: ValidatedGlob) -> ValidatedGlob {
    if !glob.inner.starts_with('/') {
        return glob;
    }
    ValidatedGlob {
        inner: glob.inner.trim_start_matches('/').to_owned(),
    }
}

/// Returns where the package-relative portion of a resolved glob starts, if
/// the glob is contained in the package directory.
fn package_relative_start(glob: &str, package: &str) -> Option<usize> {
    if glob == ".." || glob.starts_with("../") {
        return None;
    }
    if package.is_empty() {
        return Some(0);
    }

    // `ValidatedGlob` escapes `:` on Unix, so compare against the package path
    // as it appears inside the resolved glob.
    let package_in_glob: Cow<'_, str> = if package.contains(':') {
        Cow::Owned(package.replace(':', "\\:"))
    } else {
        Cow::Borrowed(package)
    };

    let rest = glob.strip_prefix(package_in_glob.as_ref())?;
    if rest.is_empty() {
        Some(glob.len())
    } else if rest.starts_with('/') {
        Some(package_in_glob.len() + 1)
    } else {
        None
    }
}

/// Resolves a normalized package-relative path (such as a file hash key like
/// `src/index.ts` or `../../tsconfig.json`) into a repo-root-relative path.
///
/// Returns `None` when the path escapes the repository root.
pub fn resolve_package_path<'a>(
    package_unix_path: &str,
    relative: &'a str,
) -> Option<Cow<'a, str>> {
    let package = package_unix_path.trim_end_matches('/');
    let relative = relative.strip_prefix("./").unwrap_or(relative);
    if !has_dot_segments(relative) {
        // Fast path for the common case of a plain in-package path.
        if package.is_empty() {
            return Some(Cow::Borrowed(relative));
        }
        return Some(Cow::Owned([package, "/", relative].concat()));
    }

    let mut segments: Vec<&str> = package.split('/').filter(|s| !s.is_empty()).collect();
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            segment => segments.push(segment),
        }
    }
    Some(Cow::Owned(segments.join("/")))
}

fn has_dot_segments(path: &str) -> bool {
    path.split('/')
        .any(|segment| segment == "." || segment == "..")
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    #[test_case("packages/a", "**/*.ts", "packages/a/**/*.ts", Some("**/*.ts") ; "package relative")]
    #[test_case("packages/a", "./src/**", "packages/a/src/**", Some("src/**") ; "dot slash")]
    #[test_case("packages/a", "/src/**", "packages/a/src/**", Some("src/**") ; "leading slash")]
    #[test_case("packages/a", "src/../lib/*.ts", "packages/a/lib/*.ts", Some("lib/*.ts") ; "inner dot dot")]
    #[test_case("packages/a", "../a/src/*.ts", "packages/a/src/*.ts", Some("src/*.ts") ; "traversal into own package")]
    #[test_case("packages/a", "../../tsconfig.json", "tsconfig.json", None ; "turbo root")]
    #[test_case("packages/a", "../b/**", "packages/b/**", None ; "sibling package")]
    #[test_case("packages/a", "../ab/**", "packages/ab/**", None ; "sibling sharing name prefix")]
    #[test_case("packages/a", "../a*/**", "packages/a*/**", None ; "glob over sibling names")]
    #[test_case("packages/a", ".", "packages/a", Some(".") ; "package dir")]
    #[test_case("", "./infra/**", "infra/**", Some("infra/**") ; "root package")]
    #[test_case("", "tsconfig.json", "tsconfig.json", Some("tsconfig.json") ; "root package literal")]
    fn resolves(package: &str, raw: &str, expected: &str, package_relative: Option<&str>) {
        let input = PackageInput::resolve(package, raw).unwrap();
        assert_eq!(input.as_str(), expected);
        assert_eq!(input.package_relative(), package_relative);
        assert_eq!(input.reaches_outside_package(), package_relative.is_none());
        assert!(!input.is_exclusion());
        assert!(!input.escapes_repo_root());
    }

    #[test]
    fn exclusion() {
        let input = PackageInput::resolve("packages/a", "!./src/generated/**").unwrap();
        assert!(input.is_exclusion());
        assert_eq!(input.as_str(), "packages/a/src/generated/**");
        assert_eq!(input.package_relative(), Some("src/generated/**"));
    }

    #[test]
    fn literal() {
        assert!(
            PackageInput::resolve("packages/a", "../../tsconfig.json")
                .unwrap()
                .is_literal()
        );
        assert!(
            !PackageInput::resolve("packages/a", "src/*.ts")
                .unwrap()
                .is_literal()
        );
    }

    #[test]
    fn escapes_repo_root() {
        let input = PackageInput::resolve("packages/a", "../../../outside.json").unwrap();
        assert!(input.escapes_repo_root());
        assert!(input.reaches_outside_package());
        assert_eq!(input.package_relative(), None);
    }

    #[test]
    fn strips_leading_slashes() {
        let glob = strip_leading_slashes(ValidatedGlob {
            inner: "//infra/**".to_owned(),
        });
        assert_eq!(glob.as_str(), "infra/**");
    }

    #[test]
    fn matches_legacy_hasher_join() {
        // The hashers previously built `{package}/{input}` and validated it.
        // Resolution must produce the same globs so hashes are stable. On
        // Windows the legacy root package globs kept a leading slash, which
        // the walker trims when joining onto its base path, so compare
        // without it.
        for (package, raw) in [
            ("packages/a", "src/**/*.ts"),
            ("packages/a", "../../tsconfig.json"),
            ("packages/a", "./src/../lib/**"),
            ("apps/nested/deep", "../../../jest.config.js"),
            ("", "infra/**"),
            ("", "./scripts/*.sh"),
        ] {
            let legacy =
                ValidatedGlob::from_str(&format!("{package}/{}", raw.trim_start_matches('/')))
                    .unwrap();
            let input = PackageInput::resolve(package, raw).unwrap();
            assert_eq!(
                input.as_str(),
                legacy.as_str().trim_start_matches('/'),
                "{package} + {raw}"
            );
        }
    }

    #[test_case("packages/a", "src/index.ts", Some("packages/a/src/index.ts") ; "in package")]
    #[test_case("packages/a", "../../tsconfig.json", Some("tsconfig.json") ; "turbo root")]
    #[test_case("packages/a", "../b/src/index.ts", Some("packages/b/src/index.ts") ; "sibling")]
    #[test_case("packages/a", "../../../x", None ; "escapes root")]
    #[test_case("", "src/index.ts", Some("src/index.ts") ; "root package")]
    fn resolves_package_paths(package: &str, relative: &str, expected: Option<&str>) {
        assert_eq!(resolve_package_path(package, relative).as_deref(), expected);
    }
}
