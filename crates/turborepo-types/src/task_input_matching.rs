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
//! Each input mode mirrors how the git file hasher resolves it, so pattern
//! order never matters:
//!
//! - With `$TURBO_DEFAULT$`, package files are matched unless an exclusion
//!   matches them as written. Files outside the package also need an inclusion,
//!   as below.
//! - Otherwise, a file matches when it is a literal inclusion (the walker reads
//!   those with a single `stat`, and exclusions never remove them), or when it
//!   matches a glob inclusion or is inside a literal directory inclusion and no
//!   exclusion matches it or one of its parent directories.
//!
//! Startup and JIT inputs are then combined as a union.

use globwalk::PackageInput;
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use wax::Program;

use crate::TaskInputs;

/// Pre-compiled glob patterns for efficient matching against many files.
///
/// Created via [`compile_globs`] for a specific package. Globs are resolved
/// against the package directory into repo-root-relative patterns, so they are
/// only valid for matching files on behalf of that package. Each input mode
/// applies exclusions the way the git file hasher does (see the module docs
/// for precedence rules). When a mode's default is true, all in-package files
/// match unless excluded.
pub struct CompiledGlobs {
    startup: ModeGlobs,
    jit: ModeGlobs,
    eager: bool,
    /// The package's [`globwalk::PACKAGE_CONFIG_INPUTS`] when either input
    /// mode lists globs without its default files. The hashers add these files
    /// to such inputs, and exclusions do not remove them.
    config_inputs: Vec<wax::Glob<'static>>,
}

/// One input mode (startup or JIT), compiled the way the git file hasher
/// resolves it.
#[derive(Default)]
struct ModeGlobs {
    /// Inclusions resolved with a single `stat`. The walker never applies
    /// exclusions to them.
    literals: Vec<wax::Glob<'static>>,
    /// Glob inclusions, plus `<literal>/**` for literal inclusions the walker
    /// expands when they name a directory. Walked files are filtered by
    /// `walk_exclusions`.
    walked: Vec<wax::Glob<'static>>,
    /// Exclusions as written. With `$TURBO_DEFAULT$`, the hasher removes files
    /// matching these from the hash after walking.
    exclusions: Vec<wax::Glob<'static>>,
    /// Exclusions as the walker applies them: each also excludes everything
    /// under the paths it matches.
    walk_exclusions: Vec<wax::Glob<'static>>,
    /// True when `$TURBO_DEFAULT$` was present in the mode's inputs, meaning
    /// all files within the package directory match unless excluded.
    default: bool,
    /// True when any resolved inclusion can reach outside the package
    /// directory (e.g. from `$TURBO_ROOT$` expansion or `../` references).
    has_traversal_globs: bool,
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
/// all files in the package.
pub fn compile_globs(
    inputs: &TaskInputs,
    package_unix_path: &str,
) -> Result<CompiledGlobs, InvalidTaskInputGlob> {
    let startup = compile_patterns(&inputs.globs, inputs.default, package_unix_path)?;
    let jit = compile_patterns(&inputs.jit_globs, inputs.jit_default, package_unix_path)?;

    // Mirrors the hashers: a mode with explicit globs and no default files is
    // hashed from those globs plus the package's configuration files.
    let lists_files = |globs: &[String], default: bool| !globs.is_empty() && !default;
    let config_inputs = if lists_files(&inputs.globs, inputs.default)
        || lists_files(&inputs.jit_globs, inputs.jit_default)
    {
        globwalk::PACKAGE_CONFIG_INPUTS
            .iter()
            .map(|file| compile_glob(file, resolve_input(package_unix_path, file)?.as_str()))
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };

    Ok(CompiledGlobs {
        startup,
        jit,
        eager: inputs.eager,
        config_inputs,
    })
}

fn compile_glob(raw: &str, resolved: &str) -> Result<wax::Glob<'static>, InvalidTaskInputGlob> {
    wax::Glob::new(resolved)
        .map(wax::Glob::into_owned)
        .map_err(|error| InvalidTaskInputGlob {
            glob: raw.to_string(),
            error: Box::new(error),
        })
}

fn resolve_input(package_unix_path: &str, raw: &str) -> Result<PackageInput, InvalidTaskInputGlob> {
    PackageInput::resolve(package_unix_path, raw).map_err(|error| InvalidTaskInputGlob {
        glob: raw.to_string(),
        error: Box::new(error),
    })
}

fn compile_patterns(
    globs: &[String],
    default: bool,
    package_unix_path: &str,
) -> Result<ModeGlobs, InvalidTaskInputGlob> {
    let mut mode = ModeGlobs {
        default,
        ..Default::default()
    };

    for glob_str in globs {
        let input = resolve_input(package_unix_path, glob_str)?;
        let glob = compile_glob(glob_str, input.as_str())?;

        // Changed files are always repo-root-relative, so globs that escape
        // the repository root can never match.
        if input.escapes_repo_root() {
            continue;
        }

        let resolved = input.as_str();
        if input.is_exclusion() {
            // The walker excludes `X` and `X/**` (`globwalk`'s
            // `add_trailing_double_star`) on the absolute path, so the
            // repository root itself (`!.` in the root package) and everything
            // under it (`!**`) exclude `<root>/**`. Expanding the relative `**`
            // would produce `**/**`, which `wax` rejects.
            let mut walk_exclusions = Vec::with_capacity(2);
            match resolved {
                "" | "." | "**" => walk_exclusions.push("**".to_string()),
                _ => globwalk::add_trailing_double_star(&mut walk_exclusions, resolved),
            }
            for walk_exclusion in walk_exclusions {
                mode.walk_exclusions
                    .push(compile_glob(glob_str, &walk_exclusion)?);
            }
            mode.exclusions.push(glob);
            continue;
        }

        mode.has_traversal_globs |= input.reaches_outside_package();
        // The walker expands a literal inclusion that names a directory to
        // everything inside it, unless it contains a character `wax` may treat
        // as syntax (`globwalk`'s `add_doublestar_to_dir`). Match both forms,
        // since the path may be a directory in only one of the compared
        // revisions.
        if globwalk::expands_directory_inclusion(resolved) {
            let directory = match resolved.trim_end_matches('/') {
                "" | "." => "**".to_string(),
                literal => format!("{literal}/**"),
            };
            mode.walked.push(compile_glob(glob_str, &directory)?);
        }
        // The walker resolves inclusions without `wax` wildcards with a single
        // `stat` and does not apply exclusions to them. With
        // `$TURBO_DEFAULT$`, the hasher also stats inputs without glob
        // metacharacters itself, even when `wax` reads one (e.g. `$`) as a
        // wildcard.
        if glob.variance().is_invariant() || (default && input.is_literal()) {
            mode.literals.push(glob);
        } else {
            mode.walked.push(glob);
        }
    }

    Ok(mode)
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
        return (compiled.startup.has_traversal_globs
            && compiled.startup.matches(file_unix, false, false))
            || (compiled.jit.has_traversal_globs && compiled.jit.matches(file_unix, false, false));
    }

    compiled
        .config_inputs
        .iter()
        .any(|glob| glob.is_match(file_unix))
        || compiled.startup.matches(file_unix, true, compiled.eager)
        || compiled.jit.matches(file_unix, true, false)
}

fn any_match(globs: &[wax::Glob<'static>], file_path: &str) -> bool {
    globs.iter().any(|glob| glob.is_match(file_path))
}

impl ModeGlobs {
    /// Checks whether the hasher includes a file in this mode's hash.
    ///
    /// **Precedence**: pattern ordering in the `inputs` array does not
    /// affect matching — `["**/*.ts", "!generated.ts"]` and
    /// `["!generated.ts", "**/*.ts"]` are equivalent. See the module docs for
    /// how exclusions apply in each mode.
    fn matches(&self, file_path: &str, in_package: bool, fallback_to_all: bool) -> bool {
        if self.default {
            // `$TURBO_DEFAULT$` hashes every package file plus the walked
            // inclusions, then removes files matching an exclusion as written.
            return !any_match(&self.exclusions, file_path)
                && (in_package || self.walk_matches(file_path));
        }

        // Both `inputs: []` (explicit empty) and a missing `inputs` key
        // produce TaskInputs { globs: [], default: false }. We treat both as
        // "all files are inputs" for affected detection, matching turbo's
        // existing hashing behavior.
        if in_package
            && fallback_to_all
            && self.literals.is_empty()
            && self.walked.is_empty()
            && self.exclusions.is_empty()
        {
            return true;
        }

        self.walk_matches(file_path)
    }

    /// Whether the file walker returns the file for this mode's inclusions.
    fn walk_matches(&self, file_path: &str) -> bool {
        any_match(&self.literals, file_path)
            || (any_match(&self.walked, file_path) && !any_match(&self.walk_exclusions, file_path))
    }
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

    fn explicit(globs: &[&str]) -> TaskInputs {
        TaskInputs {
            globs: globs.iter().map(|glob| glob.to_string()).collect(),
            ..Default::default()
        }
    }

    /// The hashers add `package.json`, `turbo.json` and `turbo.jsonc` to
    /// explicit inputs, so changing them changes the task hash.
    #[test]
    fn explicit_inputs_match_package_config_files() {
        let inputs = explicit(&["src/**"]);
        for file in globwalk::PACKAGE_CONFIG_INPUTS {
            assert_match(
                &format!("packages/lib-a/{file}"),
                "packages/lib-a",
                &inputs,
                true,
            );
            assert_match(
                &format!("packages/lib-b/{file}"),
                "packages/lib-a",
                &inputs,
                false,
            );
        }
        assert_match(
            "packages/lib-a/src/package.json",
            "packages/lib-a",
            &inputs,
            true,
        );
        assert_match(
            "packages/lib-a/tsconfig.json",
            "packages/lib-a",
            &inputs,
            false,
        );
        assert_match("package.json", "", &explicit(&["scripts/**"]), true);
    }

    #[test]
    fn exclusions_do_not_remove_config_files_from_explicit_inputs() {
        let inputs = explicit(&["src/**", "!package.json", "!**/*.json"]);
        assert_match(
            "packages/lib-a/package.json",
            "packages/lib-a",
            &inputs,
            true,
        );
        assert_match("packages/lib-a/turbo.json", "packages/lib-a", &inputs, true);
        assert_match(
            "packages/lib-a/src/data.json",
            "packages/lib-a",
            &inputs,
            false,
        );
    }

    /// With `$TURBO_DEFAULT$`, an exclusion removes `package.json` from the
    /// hash, so it must not make the task affected either.
    #[test]
    fn default_inputs_respect_config_file_exclusions() {
        let inputs = TaskInputs {
            globs: vec!["!package.json".to_string()],
            default: true,
            ..Default::default()
        };
        assert_match(
            "packages/lib-a/package.json",
            "packages/lib-a",
            &inputs,
            false,
        );
        assert_match("packages/lib-a/turbo.json", "packages/lib-a", &inputs, true);
    }

    #[test]
    fn jit_inputs_match_package_config_files() {
        let inputs = TaskInputs {
            jit_globs: vec!["src/**".to_string()],
            eager: false,
            ..Default::default()
        };
        assert_match(
            "packages/lib-a/package.json",
            "packages/lib-a",
            &inputs,
            true,
        );
        assert_match("packages/lib-a/README.md", "packages/lib-a", &inputs, false);
    }

    #[test]
    fn literal_directory_inputs_match_files_inside() {
        for raw in ["src", "src/", "./src"] {
            let inputs = explicit(&[raw]);
            assert_match(
                "packages/lib-a/src/index.ts",
                "packages/lib-a",
                &inputs,
                true,
            );
            assert_match("packages/lib-a/src/a/b.ts", "packages/lib-a", &inputs, true);
            assert_match(
                "packages/lib-a/srcx/index.ts",
                "packages/lib-a",
                &inputs,
                false,
            );
        }
        let inputs = explicit(&["../../config"]);
        assert_match("config/app.json", "packages/lib-a", &inputs, true);
        assert_match("configs/app.json", "packages/lib-a", &inputs, false);
        assert_match("tools/x.ts", "", &explicit(&["tools"]), true);
        let jit = TaskInputs {
            jit_globs: vec!["generated".to_string()],
            eager: false,
            ..Default::default()
        };
        assert_match(
            "packages/lib-a/generated/a.ts",
            "packages/lib-a",
            &jit,
            true,
        );
    }

    /// With `$TURBO_DEFAULT$`, the hasher removes package files that match an
    /// exclusion as written, so excluding a directory keeps its contents.
    #[test]
    fn default_inputs_do_not_expand_exclusions() {
        let inputs = TaskInputs {
            globs: vec!["!src".to_string()],
            default: true,
            ..Default::default()
        };
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &inputs,
            true,
        );
    }

    /// Without `$TURBO_DEFAULT$`, the walker excludes `X` and `X/**`.
    #[test]
    fn explicit_exclusions_cover_directory_contents() {
        for globs in [
            ["src", "!src/gen"],
            ["src/**", "!src/gen"],
            ["src/**", "!**/gen"],
        ] {
            let inputs = explicit(&globs);
            assert_match(
                "packages/lib-a/src/gen/a.ts",
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
        assert_match(
            "packages/lib-a/src/index.ts",
            "packages/lib-a",
            &explicit(&["src", "!src"]),
            false,
        );
        let jit = TaskInputs {
            jit_globs: vec!["src".to_string(), "!src/gen".to_string()],
            eager: false,
            ..Default::default()
        };
        assert_match("packages/lib-a/src/gen/a.ts", "packages/lib-a", &jit, false);
    }

    /// The hashers walk inclusions outside the package with `X` and `X/**`
    /// exclusions even under `$TURBO_DEFAULT$`.
    #[test]
    fn default_inputs_expand_exclusions_for_walked_files_outside_package() {
        let inputs = TaskInputs {
            globs: vec!["../../shared".to_string(), "!../../shared/skip".to_string()],
            default: true,
            ..Default::default()
        };
        assert_match("shared/skip/a.txt", "packages/lib-a", &inputs, false);
        assert_match("shared/a.txt", "packages/lib-a", &inputs, true);
    }

    /// Literal file inclusions are resolved with a single `stat`, so walk
    /// exclusions never remove them. With `$TURBO_DEFAULT$`, exclusions as
    /// written still do.
    #[test]
    fn literal_file_inclusions_ignore_walk_exclusions() {
        for exclusion in ["!src", "!src/**", "!src/index.ts"] {
            assert_match(
                "packages/lib-a/src/index.ts",
                "packages/lib-a",
                &explicit(&["src/index.ts", exclusion]),
                true,
            );
        }
        // `wax` reads `$` as a wildcard, so the walker walks this input and
        // applies exclusions to it.
        assert_match(
            "packages/lib-a/src/$gen.ts",
            "packages/lib-a",
            &explicit(&["src/$gen.ts", "!src/$gen.ts"]),
            false,
        );
        let default = |exclusion: &str| TaskInputs {
            globs: vec!["../../shared/a.txt".to_string(), exclusion.to_string()],
            default: true,
            ..Default::default()
        };
        assert_match(
            "shared/a.txt",
            "packages/lib-a",
            &default("!../../shared"),
            true,
        );
        assert_match(
            "shared/a.txt",
            "packages/lib-a",
            &default("!../../shared/a.txt"),
            false,
        );
    }

    /// The walker excludes `<root>/**` for exclusions that resolve to the
    /// repository root or everything under it. Literal file inclusions and
    /// package configuration files bypass walk exclusions.
    #[test]
    fn exclusions_of_the_whole_repository() {
        for exclusion in ["!../../**", "!../..", "!../../"] {
            assert_match(
                "packages/lib-a/src/index.ts",
                "packages/lib-a",
                &explicit(&["src/index.ts", exclusion]),
                true,
            );
            let walked = explicit(&["src/**", exclusion]);
            assert_match("packages/lib-a/src/a.ts", "packages/lib-a", &walked, false);
            assert_match(
                "packages/lib-a/package.json",
                "packages/lib-a",
                &walked,
                true,
            );
        }
        for exclusion in ["!**", "!.", "!./", "!./**"] {
            let inputs = explicit(&["scripts/**", exclusion]);
            assert_match("scripts/a.sh", "", &inputs, false);
            assert_match("package.json", "", &inputs, true);
            assert_match("tools/a.sh", "", &explicit(&["tools", exclusion]), false);
        }
    }

    /// With `$TURBO_DEFAULT$`, the hasher stats an inclusion without glob
    /// metacharacters itself, even outside the package and when `wax` reads
    /// a character in it (`$`) as a wildcard, so walk exclusions do not
    /// remove it.
    #[test]
    fn default_inputs_stat_literal_inclusions_with_wax_syntax() {
        let inputs = TaskInputs {
            globs: vec![
                "../../shared/$a.txt".to_string(),
                "!../../shared".to_string(),
            ],
            default: true,
            ..Default::default()
        };
        assert_match("shared/$a.txt", "packages/lib-a", &inputs, true);
        assert_match("shared/b.txt", "packages/lib-a", &inputs, false);
    }

    /// The walker never expands an inclusion containing a character `wax`
    /// may treat as syntax, even when it names a directory.
    #[test]
    fn inclusions_with_glob_syntax_are_not_expanded_as_directories() {
        for raw in ["$gen", "gen:x"] {
            let inputs = explicit(&[raw]);
            assert_match(
                &format!("packages/lib-a/{raw}/a.ts"),
                "packages/lib-a",
                &inputs,
                false,
            );
            assert_match(
                &format!("packages/lib-a/{raw}"),
                "packages/lib-a",
                &inputs,
                true,
            );
        }
        assert_match(
            "packages/$lib/src/a.ts",
            "packages/$lib",
            &explicit(&["src"]),
            false,
        );
    }
}
