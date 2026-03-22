//! Glob matching for task `inputs` patterns against changed files.
//!
//! Shared between `turbo run --affected` (via `turborepo-lib`) and
//! `turbo query { affectedTasks }` (via `turborepo-query`).
//!
//! This is intentionally separate from the task hashing glob infrastructure
//! in `turborepo-task-hash`. That system walks the filesystem for cache
//! hashing; here we check a pre-computed set of changed file paths from SCM,
//! which only needs to know whether *any* changed file matches.
//!
//! # Glob precedence
//!
//! Exclusions are evaluated first. If any exclusion pattern matches a file,
//! the file is rejected regardless of inclusion patterns. Pattern ordering
//! in the `inputs` array does not affect matching behavior.

use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use wax::Program;

use crate::TaskInputs;

/// Pre-compiled glob patterns for efficient matching against many files.
///
/// Created via [`compile_globs`]. Exclusions take priority over inclusions
/// (see [`check_compiled_globs`] for precedence rules). When `default` is
/// true, all in-package files match unless excluded.
pub struct CompiledGlobs {
    inclusions: Vec<wax::Glob<'static>>,
    exclusions: Vec<wax::Glob<'static>>,
    /// True when `$TURBO_DEFAULT$` was present in the task's inputs,
    /// meaning all files within the package directory match by default.
    default: bool,
    /// True when any glob starts with `../`, indicating cross-package
    /// file references (from `$TURBO_ROOT$` expansion).
    has_traversal_globs: bool,
}

/// Pre-compiles a task's input globs for efficient matching against many files.
///
/// Invalid globs are logged at `warn` level and skipped.
///
/// When `inputs` has no globs and `default` is false (representing either
/// `inputs: []` or a missing `inputs` key), the compiled result will match
/// all files — see [`check_compiled_globs`] for details.
pub fn compile_globs(inputs: &TaskInputs) -> CompiledGlobs {
    let mut inclusions = Vec::new();
    let mut exclusions = Vec::new();
    let mut has_traversal_globs = false;

    for glob_str in &inputs.globs {
        if let Some(stripped) = glob_str.strip_prefix('!') {
            if stripped.starts_with("../") {
                has_traversal_globs = true;
            }
            match wax::Glob::new(stripped) {
                Ok(glob) => exclusions.push(glob.into_owned()),
                Err(e) => {
                    tracing::warn!(
                        glob = %stripped,
                        error = %e,
                        "invalid exclusion glob in task inputs; ignoring for affected detection"
                    );
                }
            }
        } else {
            if glob_str.starts_with("../") {
                has_traversal_globs = true;
            }
            match wax::Glob::new(glob_str) {
                Ok(glob) => inclusions.push(glob.into_owned()),
                Err(e) => {
                    tracing::warn!(
                        glob = %glob_str,
                        error = %e,
                        "invalid inclusion glob in task inputs; ignoring for affected detection"
                    );
                }
            }
        }
    }

    CompiledGlobs {
        inclusions,
        exclusions,
        default: inputs.default,
        has_traversal_globs,
    }
}

/// Checks whether a changed file matches pre-compiled task input globs.
///
/// Convenience wrapper over [`file_matches_compiled_inputs`] that converts
/// path types to strings. Prefer the `&str` overload in hot loops to avoid
/// repeated allocation.
pub fn file_matches_compiled_inputs_path(
    file: &AnchoredSystemPathBuf,
    package_unix_path: &RelativeUnixPathBuf,
    compiled: &CompiledGlobs,
) -> bool {
    let file_unix = file.to_unix().to_string();
    let pkg_str = package_unix_path.to_string();
    let pkg_prefix_slash = if pkg_str.is_empty() {
        String::new()
    } else {
        format!("{pkg_str}/")
    };
    file_matches_compiled_inputs(&file_unix, &pkg_str, &pkg_prefix_slash, compiled)
}

/// Checks whether a changed file matches pre-compiled task input globs.
///
/// The file path (repo-root-relative, Unix-style string) is first
/// relativized to the task's package directory. Files outside the package
/// match only if the task has traversal globs (from `$TURBO_ROOT$`
/// expansion). For the root package (empty prefix), all files are
/// considered in-package.
///
/// `pkg_prefix_slash` should be `"{pkg_str}/"` (or empty for the root
/// package) — pre-computed by the caller to avoid per-file allocation.
pub fn file_matches_compiled_inputs(
    file_unix: &str,
    pkg_str: &str,
    pkg_prefix_slash: &str,
    compiled: &CompiledGlobs,
) -> bool {
    let file_relative_to_pkg = if pkg_str.is_empty() {
        Some(file_unix)
    } else {
        file_unix.strip_prefix(pkg_prefix_slash)
    };

    // Files outside the package dir only match if there are traversal globs
    // (e.g. `../../jest.config.js` from a $TURBO_ROOT$ reference).
    let Some(relative_path) = file_relative_to_pkg else {
        if !compiled.has_traversal_globs {
            return false;
        }

        let depth = pkg_str.matches('/').count() + 1;
        let mut relative = String::with_capacity(depth * 3 + file_unix.len());
        for _ in 0..depth {
            relative.push_str("../");
        }
        relative.push_str(file_unix);

        // `default` (from $TURBO_DEFAULT$) only covers files *inside* the
        // package. For traversal paths (files outside the package, typically
        // from $TURBO_ROOT$), only explicit inclusion globs should match.
        return check_compiled_globs(&relative, &compiled.inclusions, &compiled.exclusions, false);
    };

    check_compiled_globs(
        relative_path,
        &compiled.inclusions,
        &compiled.exclusions,
        compiled.default,
    )
}

/// Checks whether a file path matches against compiled inclusion/exclusion
/// globs.
///
/// **Precedence**: Exclusions are evaluated first. If any exclusion pattern
/// matches, the file is rejected regardless of inclusion patterns. This means
/// pattern ordering in the `inputs` array does not affect matching behavior —
/// `["**/*.ts", "!generated.ts"]` and `["!generated.ts", "**/*.ts"]` are
/// equivalent.
fn check_compiled_globs(
    file_path: &str,
    inclusions: &[wax::Glob<'static>],
    exclusions: &[wax::Glob<'static>],
    default: bool,
) -> bool {
    for pattern in exclusions {
        if pattern.is_match(file_path) {
            return false;
        }
    }

    if default {
        return true;
    }

    // Both `inputs: []` (explicit empty) and a missing `inputs` key produce
    // TaskInputs { globs: [], default: false }. We treat both as "all files
    // are inputs" for affected detection, matching turbo's existing hashing
    // behavior.
    if inclusions.is_empty() && exclusions.is_empty() {
        return true;
    }

    for pattern in inclusions {
        if pattern.is_match(file_path) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};

    use super::*;
    use crate::TaskInputs;

    fn assert_match(file: &str, pkg: &str, inputs: &TaskInputs, expected: bool) {
        let compiled = compile_globs(inputs);
        let f = AnchoredSystemPathBuf::from_raw(file).unwrap();
        let p = RelativeUnixPathBuf::new(pkg.to_string()).unwrap();
        assert_eq!(
            file_matches_compiled_inputs_path(&f, &p, &compiled),
            expected,
            "file={file}, pkg={pkg}, expected={expected}"
        );
    }

    #[test]
    fn default_inputs_match_file_in_package() {
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &TaskInputs {
                globs: vec![],
                default: true,
            },
            true,
        );
    }

    #[test]
    fn default_inputs_do_not_match_file_outside_package() {
        assert_match(
            "packages/lib-b/src/index.ts",
            "packages/lib-a",
            &TaskInputs {
                globs: vec![],
                default: true,
            },
            false,
        );
    }

    #[test]
    fn explicit_glob_matches() {
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["src/**/*.ts".to_string()],
                default: false,
            },
            true,
        );
    }

    #[test]
    fn explicit_glob_does_not_match_other_files() {
        assert_match(
            "packages/lib-a/README.md",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["src/**/*.ts".to_string()],
                default: false,
            },
            false,
        );
    }

    #[test]
    fn exclusion_glob_overrides_default() {
        assert_match(
            "packages/lib-a/README.md",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["!**/*.md".to_string()],
                default: true,
            },
            false,
        );
    }

    #[test]
    fn exclusion_overrides_explicit_inclusion() {
        assert_match(
            "packages/lib-a/src/generated.ts",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["**/*.ts".to_string(), "!src/generated.ts".to_string()],
                default: false,
            },
            false,
        );
    }

    #[test]
    fn exclusion_ordering_is_irrelevant() {
        // Same as exclusion_overrides_explicit_inclusion but with reversed
        // glob ordering. Result should be identical: exclusions always win.
        assert_match(
            "packages/lib-a/src/generated.ts",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["!src/generated.ts".to_string(), "**/*.ts".to_string()],
                default: false,
            },
            false,
        );
    }

    #[test]
    fn multiple_exclusions_all_respected() {
        let inputs = TaskInputs {
            globs: vec!["!**/*.md".to_string(), "!**/*.test.ts".to_string()],
            default: true,
        };
        assert_match("packages/lib-a/README.md", "packages/lib-a", &inputs, false);
        assert_match(
            "packages/lib-a/foo.test.ts",
            "packages/lib-a",
            &inputs,
            false,
        );
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &inputs,
            true,
        );
    }

    #[test]
    fn turbo_root_glob_matches_root_file() {
        assert_match(
            "jest.config.js",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["../../jest.config.js".to_string()],
                default: true,
            },
            true,
        );
    }

    #[test]
    fn traversal_with_exclusion() {
        // Traversal globs with an exclusion: ../../* matches all root files,
        // but ../../jest.setup.js is excluded.
        let inputs = TaskInputs {
            globs: vec!["../../*".to_string(), "!../../jest.setup.js".to_string()],
            default: true,
        };
        assert_match("jest.config.js", "packages/lib-a", &inputs, true);
        assert_match("jest.setup.js", "packages/lib-a", &inputs, false);
    }

    #[test]
    fn no_inputs_config_matches_file_in_package() {
        assert_match(
            "packages/lib-a/anything.txt",
            "packages/lib-a",
            &TaskInputs::default(),
            true,
        );
    }

    #[test]
    fn explicit_empty_inputs_matches_all_in_package() {
        // inputs: [] → { globs: [], default: false }. Intentionally matches
        // all files to align with turbo's hashing behavior.
        let inputs = TaskInputs {
            globs: vec![],
            default: false,
        };
        assert_match(
            "packages/lib-a/anything.txt",
            "packages/lib-a",
            &inputs,
            true,
        );
        // But not files outside the package.
        assert_match(
            "packages/lib-b/anything.txt",
            "packages/lib-a",
            &inputs,
            false,
        );
    }

    #[test]
    fn root_package_matches() {
        assert_match(
            "scripts/check.sh",
            "",
            &TaskInputs {
                globs: vec![],
                default: true,
            },
            true,
        );
    }

    #[test]
    fn deeply_nested_traversal_glob() {
        assert_match(
            "jest.config.js",
            "apps/nested/deep/pkg",
            &TaskInputs {
                globs: vec!["../../../../jest.config.js".to_string()],
                default: true,
            },
            true,
        );
    }

    /// Regression test for https://github.com/vercel/turborepo/issues/12338
    ///
    /// When two tasks both use $TURBO_DEFAULT$ but have different $TURBO_ROOT$
    /// inputs, changing a root file that only one task references should NOT
    /// mark the other task as affected. The `default` flag from $TURBO_DEFAULT$
    /// must not apply to files outside the package directory.
    #[test]
    fn turbo_root_default_does_not_match_unrelated_root_file() {
        // Task "test" declares $TURBO_DEFAULT$ + $TURBO_ROOT$/test-config.txt
        // (resolved to ../../test-config.txt for packages/lib-a).
        // Changing build-config.txt at the root should NOT match this task.
        assert_match(
            "build-config.txt",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["../../test-config.txt".to_string()],
                default: true,
            },
            false,
        );
    }

    #[test]
    fn turbo_root_default_matches_declared_root_file() {
        // Same task shape, but this time the changed file IS the declared input.
        assert_match(
            "test-config.txt",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["../../test-config.txt".to_string()],
                default: true,
            },
            true,
        );
    }

    #[test]
    fn invalid_glob_is_skipped_gracefully() {
        // An invalid glob should be silently skipped, not panic.
        // The valid glob should still work.
        let inputs = TaskInputs {
            globs: vec!["[invalid".to_string(), "src/**/*.ts".to_string()],
            default: false,
        };
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &inputs,
            true,
        );
        assert_match("packages/lib-a/README.md", "packages/lib-a", &inputs, false);
    }
}
