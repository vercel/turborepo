//! Glob matching for task `inputs` patterns against changed files.
//!
//! Shared between `turbo run --affected` (via `turborepo-lib`) and
//! `turbo query { affectedTasks }` (via `turborepo-query`).
//!
//! This is intentionally separate from the task hashing glob infrastructure
//! in `turborepo-task-hash`. That system walks the filesystem for cache
//! hashing; here we check a pre-computed set of changed file paths from SCM,
//! which only needs to know whether *any* changed file matches.

use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use wax::Program;

use crate::TaskInputs;

/// Pre-compiled glob patterns for efficient matching against many files.
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
/// The file path (repo-root-relative) is first relativized to the task's
/// package directory. Files outside the package match only if the task has
/// traversal globs (from `$TURBO_ROOT$` expansion). For the root package
/// (empty prefix), all files are considered in-package.
pub fn file_matches_compiled_inputs(
    file: &AnchoredSystemPathBuf,
    package_unix_path: &RelativeUnixPathBuf,
    compiled: &CompiledGlobs,
) -> bool {
    let file_unix = file.to_unix().to_string();
    let pkg_prefix = package_unix_path.to_string();

    let file_relative_to_pkg = if pkg_prefix.is_empty() {
        Some(file_unix.clone())
    } else {
        file_unix
            .strip_prefix(&format!("{pkg_prefix}/"))
            .map(|s| s.to_string())
    };

    // Files outside the package dir only match if there are traversal globs
    // (e.g. `../../jest.config.js` from a $TURBO_ROOT$ reference).
    let Some(relative_path) = file_relative_to_pkg else {
        if !compiled.has_traversal_globs {
            return false;
        }

        let depth = pkg_prefix.matches('/').count() + 1;
        let mut relative = String::new();
        for _ in 0..depth {
            relative.push_str("../");
        }
        relative.push_str(&file_unix);

        return check_compiled_globs(
            &relative,
            &compiled.inclusions,
            &compiled.exclusions,
            compiled.default,
        );
    };

    check_compiled_globs(
        &relative_path,
        &compiled.inclusions,
        &compiled.exclusions,
        compiled.default,
    )
}

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

    fn anchored(s: &str) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf::from_raw(s).unwrap()
    }

    fn pkg_path(s: &str) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf::new(s.to_string()).unwrap()
    }

    #[test]
    fn default_inputs_match_file_in_package() {
        let file = anchored("packages/lib-a/src/index.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec![],
            default: true,
        };
        let compiled = compile_globs(&inputs);
        assert!(file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn default_inputs_do_not_match_file_outside_package() {
        let file = anchored("packages/lib-b/src/index.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec![],
            default: true,
        };
        let compiled = compile_globs(&inputs);
        assert!(!file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn explicit_glob_matches() {
        let file = anchored("packages/lib-a/src/index.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["src/**/*.ts".to_string()],
            default: false,
        };
        let compiled = compile_globs(&inputs);
        assert!(file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn explicit_glob_does_not_match_other_files() {
        let file = anchored("packages/lib-a/README.md");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["src/**/*.ts".to_string()],
            default: false,
        };
        let compiled = compile_globs(&inputs);
        assert!(!file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn exclusion_glob_overrides_default() {
        let file = anchored("packages/lib-a/README.md");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["!**/*.md".to_string()],
            default: true,
        };
        let compiled = compile_globs(&inputs);
        assert!(!file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn exclusion_overrides_explicit_inclusion() {
        let file = anchored("packages/lib-a/src/generated.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["**/*.ts".to_string(), "!src/generated.ts".to_string()],
            default: false,
        };
        let compiled = compile_globs(&inputs);
        assert!(!file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn multiple_exclusions_all_respected() {
        let file_md = anchored("packages/lib-a/README.md");
        let file_test = anchored("packages/lib-a/foo.test.ts");
        let file_src = anchored("packages/lib-a/src/index.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["!**/*.md".to_string(), "!**/*.test.ts".to_string()],
            default: true,
        };
        let compiled = compile_globs(&inputs);
        assert!(!file_matches_compiled_inputs(&file_md, &pkg, &compiled));
        assert!(!file_matches_compiled_inputs(&file_test, &pkg, &compiled));
        assert!(file_matches_compiled_inputs(&file_src, &pkg, &compiled));
    }

    #[test]
    fn turbo_root_glob_matches_root_file() {
        let file = anchored("jest.config.js");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["../../jest.config.js".to_string()],
            default: true,
        };
        let compiled = compile_globs(&inputs);
        assert!(file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn no_inputs_config_matches_file_in_package() {
        let file = anchored("packages/lib-a/anything.txt");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs::default();
        let compiled = compile_globs(&inputs);
        assert!(file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn root_package_matches() {
        let file = anchored("scripts/check.sh");
        let pkg = pkg_path("");
        let inputs = TaskInputs {
            globs: vec![],
            default: true,
        };
        let compiled = compile_globs(&inputs);
        assert!(file_matches_compiled_inputs(&file, &pkg, &compiled));
    }

    #[test]
    fn deeply_nested_traversal_glob() {
        let file = anchored("jest.config.js");
        let pkg = pkg_path("apps/nested/deep/pkg");
        let inputs = TaskInputs {
            globs: vec!["../../../../jest.config.js".to_string()],
            default: true,
        };
        let compiled = compile_globs(&inputs);
        assert!(file_matches_compiled_inputs(&file, &pkg, &compiled));
    }
}
