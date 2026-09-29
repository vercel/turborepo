//! Glob matching for task `inputs` patterns against changed files.
//!
//! Shared between `turbo run --affected` (via `turborepo-run`) and
//! `turbo query { affectedTasks }` (via `turborepo-query`).
//!
//! Matching is separate from the task hashing glob infrastructure in
//! `turborepo-scm`. That system walks the filesystem for cache hashing; here
//! we check a pre-computed set of changed file paths from SCM, which only
//! needs to know whether *any* changed file matches. Both resolve inputs
//! against the package directory with [`globwalk::PackageInput`], so they
//! agree on which files an input refers to.
//!
//! # Glob precedence
//!
//! Within each input mode, exclusions are evaluated first. If an exclusion
//! pattern matches a file, the file is rejected from that mode regardless of
//! inclusion patterns. Startup and JIT inputs are then combined as a union.

use globwalk::PackageInput;
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use wax::Program;

use crate::TaskInputs;

/// Pre-compiled glob patterns for efficient matching against many files.
///
/// Created via [`compile_globs`] for a specific package. Globs are resolved
/// against the package directory into repo-root-relative patterns, so they are
/// only valid for matching files on behalf of that package. Exclusions take
/// priority over inclusions within each input mode (see
/// [`check_compiled_globs`] for precedence rules). When a mode's default is
/// true, all in-package files match unless excluded.
pub struct CompiledGlobs {
    inclusions: Vec<wax::Glob<'static>>,
    exclusions: Vec<wax::Glob<'static>>,
    jit_inclusions: Vec<wax::Glob<'static>>,
    jit_exclusions: Vec<wax::Glob<'static>>,
    /// True when `$TURBO_DEFAULT$` was present in the task's inputs,
    /// meaning all files within the package directory match by default.
    default: bool,
    jit_default: bool,
    eager: bool,
    /// True when any resolved glob can reach outside the package directory
    /// (e.g. from `$TURBO_ROOT$` expansion or `../` references).
    has_traversal_globs: bool,
    jit_has_traversal_globs: bool,
}

#[derive(Debug)]
pub struct InvalidTaskInputGlob {
    pub glob: String,
    pub error: Box<dyn std::error::Error + Send + Sync + 'static>,
}

impl std::fmt::Display for InvalidTaskInputGlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid glob {:?}: {}", self.glob, self.error)
    }
}

impl std::error::Error for InvalidTaskInputGlob {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

/// Pre-compiles a task's input globs for efficient matching against many files.
///
/// `package_unix_path` is the repo-root-relative, Unix-style path of the
/// task's package (empty for the root package). Every glob is resolved
/// against it into a repo-root-relative pattern with
/// [`globwalk::PackageInput`], the same resolution the task hashers use. This
/// keeps package-relative globs like `**/*.ts` scoped to the package even when
/// the task also references files outside it via `$TURBO_ROOT$`.
///
/// When `inputs` has no globs and `default` is false (representing either
/// `inputs: []` or a missing `inputs` key), the compiled result will match
/// all files — see [`check_compiled_globs`] for details.
pub fn compile_globs(
    inputs: &TaskInputs,
    package_unix_path: &str,
) -> Result<CompiledGlobs, InvalidTaskInputGlob> {
    let (inclusions, exclusions, has_traversal_globs) =
        compile_patterns(&inputs.globs, package_unix_path)?;
    let (jit_inclusions, jit_exclusions, jit_has_traversal_globs) =
        compile_patterns(&inputs.jit_globs, package_unix_path)?;

    Ok(CompiledGlobs {
        inclusions,
        exclusions,
        jit_inclusions,
        jit_exclusions,
        default: inputs.default,
        jit_default: inputs.jit_default,
        eager: inputs.eager,
        has_traversal_globs,
        jit_has_traversal_globs,
    })
}

fn compile_patterns(
    globs: &[String],
    package_unix_path: &str,
) -> Result<(Vec<wax::Glob<'static>>, Vec<wax::Glob<'static>>, bool), InvalidTaskInputGlob> {
    let mut inclusions = Vec::new();
    let mut exclusions = Vec::new();
    let mut has_traversal_globs = false;

    for glob_str in globs {
        let invalid = |error: Box<dyn std::error::Error + Send + Sync>| InvalidTaskInputGlob {
            glob: glob_str.clone(),
            error,
        };
        let input = PackageInput::resolve(package_unix_path, glob_str)
            .map_err(|error| invalid(Box::new(error)))?;
        let glob = wax::Glob::new(input.as_str())
            .map_err(|error| invalid(Box::new(error)))?
            .into_owned();

        // Changed files are always repo-root-relative, so globs that escape
        // the repository root can never match.
        if input.escapes_repo_root() {
            continue;
        }

        if input.is_exclusion() {
            exclusions.push(glob);
        } else {
            has_traversal_globs |= input.reaches_outside_package();
            inclusions.push(glob);
        }
    }

    Ok((inclusions, exclusions, has_traversal_globs))
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
/// The file path must be repo-root-relative and Unix-style; `compiled` must
/// have been created for the same package. Files outside the package match
/// only through globs that reach outside the package directory (e.g. from
/// `$TURBO_ROOT$` expansion), and never through `$TURBO_DEFAULT$` or the
/// empty-inputs fallback. For the root package (empty prefix), all files are
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
    let in_package = pkg_str.is_empty() || file_unix.starts_with(pkg_prefix_slash);

    if !in_package {
        // Defaults only cover files inside the package. Keep startup and JIT
        // matching separate so exclusions in one mode do not affect the other.
        return (compiled.has_traversal_globs
            && check_compiled_globs(
                file_unix,
                &compiled.inclusions,
                &compiled.exclusions,
                false,
                false,
            ))
            || (compiled.jit_has_traversal_globs
                && check_compiled_globs(
                    file_unix,
                    &compiled.jit_inclusions,
                    &compiled.jit_exclusions,
                    false,
                    false,
                ));
    }

    check_compiled_globs(
        file_unix,
        &compiled.inclusions,
        &compiled.exclusions,
        compiled.default,
        compiled.eager,
    ) || check_compiled_globs(
        file_unix,
        &compiled.jit_inclusions,
        &compiled.jit_exclusions,
        compiled.jit_default,
        false,
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
    fallback_to_all: bool,
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
    if fallback_to_all && inclusions.is_empty() && exclusions.is_empty() {
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
    use crate::{DependencyOutputsInput, TaskInputs};

    fn assert_match(file: &str, pkg: &str, inputs: &TaskInputs, expected: bool) {
        let compiled = compile_globs(inputs, pkg).unwrap();
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
            },
            false,
        );
    }

    #[test]
    fn dot_slash_glob_matches_package_file() {
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["./src/**/*.ts".to_string()],
                default: false,
                ..Default::default()
            },
            true,
        );
    }

    #[test]
    fn dot_slash_glob_matches_root_package_file() {
        assert_match(
            "infra/config.txt",
            "",
            &TaskInputs {
                globs: vec!["./infra/**".to_string()],
                default: false,
                ..Default::default()
            },
            true,
        );
    }

    #[test]
    fn dot_slash_exclusion_glob_is_respected() {
        assert_match(
            "packages/lib-a/src/generated.ts",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["src/**/*.ts".to_string(), "!./src/generated.ts".to_string()],
                default: false,
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
            },
            false,
        );
    }

    #[test]
    fn multiple_exclusions_all_respected() {
        let inputs = TaskInputs {
            globs: vec!["!**/*.md".to_string(), "!**/*.test.ts".to_string()],
            default: true,
            ..Default::default()
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
                ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
            },
            true,
        );
    }

    #[test]
    fn invalid_glob_returns_error() {
        let inputs = TaskInputs {
            globs: vec!["[invalid".to_string(), "src/**/*.ts".to_string()],
            default: false,
            ..Default::default()
        };
        let error = match compile_globs(&inputs, "packages/lib-a") {
            Ok(_) => panic!("invalid glob compiled successfully"),
            Err(error) => error,
        };
        assert_eq!(error.glob, "[invalid");
    }

    #[test]
    fn jit_inputs_can_match_files_excluded_from_startup_inputs() {
        let inputs = TaskInputs {
            globs: vec!["!src/generated/**".to_string()],
            jit_globs: vec!["src/generated/**".to_string()],
            ..Default::default()
        };

        assert_match(
            "packages/lib-a/src/generated/client.ts",
            "packages/lib-a",
            &inputs,
            true,
        );
    }

    #[test]
    fn dot_slash_jit_glob_matches_package_file() {
        assert_match(
            "packages/lib-a/src/generated/client.ts",
            "packages/lib-a",
            &TaskInputs {
                jit_globs: vec!["./src/generated/**".to_string()],
                ..Default::default()
            },
            true,
        );
    }

    #[test]
    fn jit_inputs_respect_startup_exclusions_and_package_boundaries() {
        let inputs = TaskInputs {
            globs: vec!["!**/*.md".to_string()],
            default: true,
            jit_globs: vec!["src/gen/**".to_string()],
            ..Default::default()
        };

        assert_match("packages/lib-a/README.md", "packages/lib-a", &inputs, false);
        assert_match(
            "packages/lib-b/src/index.ts",
            "packages/lib-a",
            &inputs,
            false,
        );
    }

    #[test]
    fn jit_traversal_only_matches_declared_files() {
        let inputs = TaskInputs {
            jit_globs: vec!["../../schema.json".to_string()],
            eager: false,
            ..Default::default()
        };

        assert_match("other.json", "packages/lib-a", &inputs, false);
        assert_match("schema.json", "packages/lib-a", &inputs, true);
    }

    #[test]
    fn dependency_outputs_do_not_match_unrelated_files_like_jit() {
        let inputs = TaskInputs {
            globs: vec!["src/**/*.ts".to_string()],
            dependency_outputs: Some(DependencyOutputsInput {
                from: None,
                globs: vec![],
            }),
            ..Default::default()
        };

        assert_match("packages/lib-a/README.md", "packages/lib-a", &inputs, false);
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &inputs,
            true,
        );
    }

    /// Regression test for https://github.com/vercel/turborepo/issues/14340
    ///
    /// A package-relative `**/` glob must stay scoped to its package even
    /// when a `$TURBO_ROOT$` input makes the task consider files outside it.
    #[test]
    fn doublestar_glob_with_turbo_root_does_not_match_other_packages() {
        let inputs = TaskInputs {
            globs: vec!["**/*.ts".to_string(), "../../tsconfig.json".to_string()],
            default: false,
            ..Default::default()
        };
        assert_match("packages/b/src/index.ts", "packages/a", &inputs, false);
        assert_match("packages/a/src/index.ts", "packages/a", &inputs, true);
        assert_match("tsconfig.json", "packages/a", &inputs, true);
        assert_match("other.json", "packages/a", &inputs, false);
    }

    #[test]
    fn doublestar_jit_glob_with_turbo_root_does_not_match_other_packages() {
        let inputs = TaskInputs {
            jit_globs: vec!["**/*.ts".to_string(), "../../tsconfig.json".to_string()],
            ..Default::default()
        };
        assert_match("packages/b/src/index.ts", "packages/a", &inputs, false);
        assert_match("tsconfig.json", "packages/a", &inputs, true);
    }

    #[test]
    fn sibling_package_traversal_glob_is_normalized() {
        let inputs = TaskInputs {
            globs: vec![
                "../lib-b/src/**".to_string(),
                "!../../packages/lib-b/src/generated/**".to_string(),
            ],
            default: true,
            ..Default::default()
        };
        assert_match(
            "packages/lib-b/src/index.ts",
            "packages/lib-a",
            &inputs,
            true,
        );
        assert_match(
            "packages/lib-b/src/generated/client.ts",
            "packages/lib-a",
            &inputs,
            false,
        );
        assert_match(
            "packages/lib-c/src/index.ts",
            "packages/lib-a",
            &inputs,
            false,
        );
    }

    #[test]
    fn traversal_into_own_package_is_normalized() {
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["../lib-a/./src/*.ts".to_string()],
                default: false,
                ..Default::default()
            },
            true,
        );
    }

    #[test]
    fn prefix_sharing_sibling_package_is_outside_package() {
        // `packages/lib-ab` shares a string prefix with `packages/lib-a` but
        // is a different package.
        let inputs = TaskInputs {
            globs: vec!["**".to_string(), "../../tsconfig.json".to_string()],
            default: true,
            ..Default::default()
        };
        assert_match("packages/lib-ab/index.ts", "packages/lib-a", &inputs, false);
    }

    #[test]
    fn glob_escaping_repo_root_matches_nothing() {
        assert_match(
            "tsconfig.json",
            "packages/lib-a",
            &TaskInputs {
                globs: vec!["../../../tsconfig.json".to_string()],
                default: false,
                ..Default::default()
            },
            false,
        );
    }
}
