use std::{
    fmt,
    str::FromStr as _,
    sync::{Arc, Mutex},
};

use globwalk::{ValidatedGlob, fix_glob_pattern};
use itertools::Itertools as _;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError};
use wax::{Any, Glob, Program as _};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid workspace glob {fixed}: {err}")]
    Glob {
        fixed: String,
        #[source]
        err: Box<wax::BuildError>,
    },
    #[error("Invalid globwalk pattern {0}")]
    Globwalk(#[from] globwalk::GlobError),
    #[error(transparent)]
    WalkError(#[from] globwalk::WalkError),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("Workspace package resolves outside repository root: {0}")]
    WorkspacePackageOutsideRepo(String),
}

// WorkspaceGlobs is suitable for finding package.json files via globwalk
#[derive(Clone)]
pub struct WorkspaceGlobs {
    directory_inclusions: Any<'static>,
    directory_exclusions: Any<'static>,
    package_json_inclusions: Vec<ValidatedGlob>,
    pub raw_inclusions: Vec<String>,
    pub raw_exclusions: Vec<String>,
    validated_exclusions: Vec<ValidatedGlob>,
}

// Inference and discovery often compile the same pattern lists independently.
// Retain at most one successful result, keyed by exact constructor inputs, not
// repository paths or mtimes. Configuration is still read on every call and no
// discovered paths, symlink resolutions, or containment decisions are cached.
static LAST_WORKSPACE_GLOBS: Mutex<Option<Arc<WorkspaceGlobs>>> = Mutex::new(None);

impl Error {
    pub fn invalid_glob(fixed: String, err: wax::BuildError) -> Self {
        Self::Glob {
            fixed,
            err: Box::new(err),
        }
    }
}

impl fmt::Debug for WorkspaceGlobs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceGlobs")
            .field("inclusions", &self.raw_inclusions)
            .field("exclusions", &self.raw_exclusions)
            .finish()
    }
}

impl PartialEq for WorkspaceGlobs {
    fn eq(&self, other: &Self) -> bool {
        // Use the literals for comparison, not the compiled globs
        self.raw_inclusions == other.raw_inclusions && self.raw_exclusions == other.raw_exclusions
    }
}

impl Eq for WorkspaceGlobs {}

fn glob_with_contextual_error<S: AsRef<str>>(raw: S) -> Result<Glob<'static>, Error> {
    let raw = raw.as_ref();
    let fixed = fix_glob_pattern(raw);
    Glob::new(&fixed)
        .map(|g| g.into_owned())
        .map_err(|e| Error::invalid_glob(fixed.into_owned(), e))
}

fn any_with_contextual_error(
    precompiled: Vec<Glob<'static>>,
    text: Vec<String>,
) -> Result<wax::Any<'static>, Error> {
    wax::any(precompiled).map_err(|e| {
        let text = text.iter().join(",");
        Error::invalid_glob(text, e)
    })
}

fn compile_directory_globs(raw: &[String]) -> Result<Any<'static>, Error> {
    let fixed: Vec<_> = raw
        .iter()
        .map(|pattern| fix_glob_pattern(pattern))
        .collect();
    match wax::any(fixed.iter().map(|pattern| pattern.as_ref())) {
        Ok(combined) => Ok(combined.into_owned()),
        Err(_) => {
            // Preserve per-expression validation and error context on failure.
            let globs = raw
                .iter()
                .map(glob_with_contextual_error)
                .collect::<Result<Vec<_>, _>>()?;
            any_with_contextual_error(globs, raw.to_vec())
        }
    }
}

impl WorkspaceGlobs {
    pub fn new<S: Into<String>>(inclusions: Vec<S>, exclusions: Vec<S>) -> Result<Self, Error> {
        Self::compile_cached(
            inclusions.into_iter().map(Into::into).collect(),
            exclusions.into_iter().map(Into::into).collect(),
            &LAST_WORKSPACE_GLOBS,
        )
    }

    fn compile_cached(
        raw_inclusions: Vec<String>,
        raw_exclusions: Vec<String>,
        cache: &Mutex<Option<Arc<Self>>>,
    ) -> Result<Self, Error> {
        let hit = cache.lock().ok().and_then(|entry| {
            entry
                .as_ref()
                .filter(|globs| {
                    globs.raw_inclusions == raw_inclusions && globs.raw_exclusions == raw_exclusions
                })
                .cloned()
        });
        if let Some(globs) = hit {
            // Clone outside the lock. Owned pattern vectors isolate the cache
            // from caller mutations; regex programs themselves are shared by wax.
            return Ok(globs.as_ref().clone());
        }

        // Compile outside the lock: unrelated repositories must not block one
        // another on regex construction. Concurrent misses may compile twice.
        let globs = Self::compile(raw_inclusions, raw_exclusions)?;
        let stored = Arc::new(globs.clone());
        let evicted = cache.lock().ok().map(|mut entry| entry.replace(stored));
        // Release the mutex before destroying an evicted matcher. Poisoning
        // simply disables the optimization rather than failing discovery.
        drop(evicted);
        Ok(globs)
    }

    fn compile(raw_inclusions: Vec<String>, raw_exclusions: Vec<String>) -> Result<Self, Error> {
        let package_json_inclusions = raw_inclusions
            .iter()
            .map(|s| {
                let mut s: String = s.clone();
                if s.ends_with('/') {
                    s.push_str("package.json");
                } else {
                    s.push_str("/package.json");
                }
                ValidatedGlob::from_str(&s)
            })
            .collect::<Result<Vec<ValidatedGlob>, _>>()?;
        // `wax::any` accepts expressions directly: compiling each Glob first
        // builds regexes which the combinator immediately discards. Only compile
        // the combined matcher on the successful path.
        let directory_inclusions = compile_directory_globs(&raw_inclusions)?;
        let directory_exclusions = compile_directory_globs(&raw_exclusions)?;
        let validated_exclusions = raw_exclusions
            .iter()
            .map(|e| ValidatedGlob::from_str(e))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            directory_inclusions,
            directory_exclusions,
            package_json_inclusions,
            validated_exclusions,
            raw_exclusions,
            raw_inclusions,
        })
    }

    /// Checks if the given `target` matches this `WorkspaceGlobs`.
    ///
    /// Errors:
    /// This function returns an Err if `root` is not a valid anchor for
    /// `target`
    pub fn target_is_workspace(
        &self,
        root: &AbsoluteSystemPath,
        target: &AbsoluteSystemPath,
    ) -> Result<bool, PathError> {
        let search_value = root.anchor(target)?;

        let includes = self.directory_inclusions.is_match(&search_value);
        let excludes = self.directory_exclusions.is_match(&search_value);

        Ok(includes && !excludes)
    }

    pub fn get_package_jsons(
        &self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<impl Iterator<Item = AbsoluteSystemPathBuf> + use<>, Error> {
        let files = {
            let _span = tracing::info_span!("package_json_walk").entered();
            globwalk::globwalk_with_settings(
                repo_root,
                &self.package_json_inclusions,
                &self.validated_exclusions,
                globwalk::WalkType::Files,
                globwalk::Settings::default().follow_links(),
            )?
        };
        let _span = tracing::info_span!("package_json_realpath_check").entered();
        let repo_root = repo_root.to_realpath()?;
        for file in &files {
            let real_file = file.to_realpath()?;
            if !real_file.starts_with(&repo_root) {
                return Err(Error::WorkspacePackageOutsideRepo(file.to_string()));
            }
        }
        Ok(files.into_iter())
    }

    /// Finds manifests for another toolchain using the same validated
    /// workspace glob and repository-boundary semantics as JavaScript.
    pub fn get_manifests(
        &self,
        repo_root: &AbsoluteSystemPath,
        manifest_name: &str,
    ) -> Result<impl Iterator<Item = AbsoluteSystemPathBuf> + use<>, Error> {
        let inclusions = self
            .raw_inclusions
            .iter()
            .map(|pattern| {
                ValidatedGlob::from_str(&format!(
                    "{}/{}",
                    pattern.trim_end_matches('/'),
                    manifest_name
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let files = globwalk::globwalk_with_settings(
            repo_root,
            &inclusions,
            &self.validated_exclusions,
            globwalk::WalkType::Files,
            globwalk::Settings::default(),
        )?;
        let real_repo_root = repo_root.to_realpath()?;
        for file in &files {
            let real_file = file.to_realpath()?;
            if !real_file.starts_with(&real_repo_root) {
                return Err(Error::WorkspacePackageOutsideRepo(file.to_string()));
            }
        }
        Ok(files.into_iter())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn matcher_cache_preserves_exact_inputs_and_isolates_returned_clones() {
        let cache = Mutex::new(None);
        let inclusions = vec!["packages/*".to_string(), "apps/*".to_string()];
        let exclusions = vec!["packages/private".to_string()];
        let expected = WorkspaceGlobs::compile(inclusions.clone(), exclusions.clone()).unwrap();
        for _ in 0..2 {
            let mut cached =
                WorkspaceGlobs::compile_cached(inclusions.clone(), exclusions.clone(), &cache)
                    .unwrap();
            assert_eq!(cached, expected);
            for path in [
                "packages/web",
                "packages/private",
                "apps/site",
                "other/site",
            ] {
                assert_eq!(
                    cached.directory_inclusions.is_match(path),
                    expected.directory_inclusions.is_match(path)
                );
                assert_eq!(
                    cached.directory_exclusions.is_match(path),
                    expected.directory_exclusions.is_match(path)
                );
            }
            cached.raw_inclusions.clear();
            cached.raw_exclusions.push("apps/*".into());
        }
        let stored = cache.lock().unwrap().as_ref().unwrap().clone();
        WorkspaceGlobs::compile_cached(inclusions.clone(), exclusions.clone(), &cache).unwrap();
        assert!(Arc::ptr_eq(
            &stored,
            cache.lock().unwrap().as_ref().unwrap()
        ));

        // Exclusions and ordered raw spellings are part of the key, even if
        // two different inclusion lists happen to match the same directories.
        let changed = WorkspaceGlobs::compile_cached(inclusions, vec![], &cache).unwrap();
        assert!(!changed.directory_exclusions.is_match("packages/private"));
        let reordered = vec!["apps/*".into(), "packages/*".into(), "packages/*".into()];
        let changed = WorkspaceGlobs::compile_cached(reordered.clone(), vec![], &cache).unwrap();
        assert_eq!(changed.raw_inclusions, reordered);
        assert_eq!(cache.lock().unwrap().as_deref().unwrap(), &changed);
    }

    #[test]
    fn matcher_cache_does_not_store_errors() {
        let cache = Mutex::new(None);
        let valid =
            WorkspaceGlobs::compile_cached(vec!["packages/*".into()], vec![], &cache).unwrap();
        for pattern in ["[", "{a,"] {
            let raw = vec![pattern.to_string()];
            let cold = WorkspaceGlobs::compile(raw.clone(), vec![]).unwrap_err();
            let warm = WorkspaceGlobs::compile_cached(raw, vec![], &cache).unwrap_err();
            assert_eq!(cold.to_string(), warm.to_string());
            assert_eq!(cache.lock().unwrap().as_deref().unwrap(), &valid);
        }
    }

    #[test]
    fn matcher_cache_poisoning_falls_back_to_compilation() {
        let cache = Mutex::new(None);
        let _ = std::panic::catch_unwind(|| {
            let _guard = cache.lock().unwrap();
            panic!("poison test cache");
        });
        let globs =
            WorkspaceGlobs::compile_cached(vec!["packages/*".into()], vec![], &cache).unwrap();
        assert!(globs.directory_inclusions.is_match("packages/app"));
    }

    #[test]
    fn matcher_cache_handles_concurrent_different_keys() {
        let cache = Mutex::new(None);
        std::thread::scope(|scope| {
            for worker in 0..4 {
                let cache = &cache;
                scope.spawn(move || {
                    let pattern = format!("packages-{worker}/*");
                    let candidate = format!("packages-{worker}/app");
                    for _ in 0..8 {
                        let globs =
                            WorkspaceGlobs::compile_cached(vec![pattern.clone()], vec![], cache)
                                .unwrap();
                        assert_eq!(globs.raw_inclusions, vec![pattern.clone()]);
                        assert!(globs.directory_inclusions.is_match(candidate.as_str()));
                        assert!(!globs.directory_inclusions.is_match("other/app"));
                    }
                });
            }
        });
    }

    #[test]
    fn matcher_reuse_does_not_cache_workspace_configuration() {
        use crate::package_manager::PackageManager;
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let manifest = root.join_component("package.json");
        manifest
            .create_with_contents(r#"{"workspaces":["packages/*"]}"#)
            .unwrap();
        let first = PackageManager::Npm.get_workspace_globs(&root).unwrap();
        assert_eq!(first.raw_inclusions, vec!["packages/*"]);
        manifest
            .create_with_contents(r#"{"workspaces":["apps/*"]}"#)
            .unwrap();
        let second = PackageManager::Npm.get_workspace_globs(&root).unwrap();
        assert_eq!(second.raw_inclusions, vec!["apps/*"]);
        manifest.create_with_contents("{").unwrap();
        assert!(PackageManager::Npm.get_workspace_globs(&root).is_err());
        manifest.remove_file().unwrap();
        assert!(PackageManager::Npm.get_workspace_globs(&root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn matcher_reuse_still_rechecks_symlink_containment() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let packages = root.join_component("packages");
        packages.create_dir_all().unwrap();
        let inside = root.join_component("inside");
        inside.create_dir_all().unwrap();
        inside
            .join_component("package.json")
            .create_with_contents("{}")
            .unwrap();
        let alias = packages.join_component("app");
        std::os::unix::fs::symlink(inside.as_std_path(), alias.as_std_path()).unwrap();
        let globs = WorkspaceGlobs::new(vec!["packages/*"], vec![]).unwrap();
        assert_eq!(globs.get_package_jsons(&root).unwrap().count(), 1);
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("package.json"), "{}").unwrap();
        alias.remove_file().unwrap();
        std::os::unix::fs::symlink(outside.path(), alias.as_std_path()).unwrap();
        let reused = WorkspaceGlobs::new(vec!["packages/*"], vec![]).unwrap();
        assert!(matches!(
            reused.get_package_jsons(&root),
            Err(Error::WorkspacePackageOutsideRepo(_))
        ));
    }

    #[test]
    fn combined_directory_globs_match_precompiled_globs() {
        for patterns in [
            vec![],
            vec!["packages/*"],
            vec!["apps/*", "packages/**", "tools/{cli,web}"],
            vec!["**/node_modules/**", "**/.git", "**/.yarn"],
            vec!["пакеты/?", "apps/[a-z]*", "**foo"],
            vec![".", "packages/", "literal\\*"],
        ] {
            let raw: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
            let old = any_with_contextual_error(
                raw.iter()
                    .map(glob_with_contextual_error)
                    .collect::<Result<_, _>>()
                    .unwrap(),
                raw.clone(),
            )
            .unwrap();
            let new = compile_directory_globs(&raw).unwrap();
            for path in [
                "",
                ".",
                "packages",
                "packages/",
                "packages/web",
                "packages/a/b",
                "apps/cli",
                "tools/web",
                "пакеты/猫",
                "x/node_modules/pkg",
                "node_modules/",
                ".git",
                "foo",
                "afoo",
                "literal*",
                "apps/a\nb",
            ] {
                assert_eq!(
                    new.is_match(path),
                    old.is_match(path),
                    "{patterns:?}: {path:?}"
                );
            }
        }
    }

    #[test]
    fn combined_directory_globs_preserve_errors() {
        for pattern in ["[", "{a,", "packages/**/**", "<a:0,0>"] {
            let raw = vec!["packages/*".to_string(), pattern.to_string()];
            let old = raw
                .iter()
                .map(glob_with_contextual_error)
                .collect::<Result<Vec<_>, _>>()
                .and_then(|globs| any_with_contextual_error(globs, raw.clone()));
            let new = compile_directory_globs(&raw);
            assert_eq!(
                new.as_ref().err().map(ToString::to_string),
                old.as_ref().err().map(ToString::to_string),
                "{pattern}"
            );
        }
    }

    #[test]
    fn test_workspace_globs_trailing_slash() {
        let globs =
            WorkspaceGlobs::new(vec!["scripts/", "packages/**"], vec!["package/template"]).unwrap();
        assert_eq!(
            &globs
                .package_json_inclusions
                .iter()
                .map(|i| i.as_str())
                .collect::<Vec<_>>(),
            &["scripts/package.json", "packages/**/package.json"]
        );
    }

    // Regression tests for https://github.com/vercel/turborepo/issues/2517
    // Workspace packages behind symlinked directories must be discovered by
    // get_package_jsons().
    #[cfg(unix)]
    mod symlink_workspace_discovery {
        use std::collections::HashSet;

        use turbopath::AbsoluteSystemPathBuf;

        use super::*;

        #[test]
        fn discovers_package_behind_symlinked_directory() {
            let tmp = tempfile::TempDir::with_prefix("ws-symlink").unwrap();
            let root = tmp.path();

            // Real package
            std::fs::create_dir_all(root.join("apps/web")).unwrap();
            std::fs::write(root.join("apps/web/package.json"), r#"{"name": "web"}"#).unwrap();

            // Symlinked package: widgets/widget-a -> ../submodules/widget-a
            std::fs::create_dir_all(root.join("submodules/widget-a")).unwrap();
            std::fs::write(
                root.join("submodules/widget-a/package.json"),
                r#"{"name": "widget-a"}"#,
            )
            .unwrap();
            std::fs::create_dir_all(root.join("widgets")).unwrap();
            std::os::unix::fs::symlink("../submodules/widget-a", root.join("widgets/widget-a"))
                .unwrap();

            let repo_root = AbsoluteSystemPathBuf::try_from(root).unwrap();
            let globs = WorkspaceGlobs::new(vec!["apps/*", "widgets/*"], vec![]).unwrap();

            let package_jsons: HashSet<String> = globs
                .get_package_jsons(&repo_root)
                .unwrap()
                .map(|p| repo_root.anchor(p).unwrap().to_string())
                .collect();

            let expected: HashSet<String> = HashSet::from_iter([
                "apps/web/package.json".replace('/', std::path::MAIN_SEPARATOR_STR),
                "widgets/widget-a/package.json".replace('/', std::path::MAIN_SEPARATOR_STR),
            ]);

            assert_eq!(
                package_jsons, expected,
                "should discover packages behind symlinks"
            );
        }

        #[test]
        fn discovers_package_behind_symlink_with_doublestar_glob() {
            let tmp = tempfile::TempDir::with_prefix("ws-symlink-dstar").unwrap();
            let root = tmp.path();

            // Symlinked nested package
            std::fs::create_dir_all(root.join("external/nested/deep-pkg")).unwrap();
            std::fs::write(
                root.join("external/nested/deep-pkg/package.json"),
                r#"{"name": "deep-pkg"}"#,
            )
            .unwrap();
            std::fs::create_dir_all(root.join("packages")).unwrap();
            std::os::unix::fs::symlink("../external/nested", root.join("packages/nested")).unwrap();

            let repo_root = AbsoluteSystemPathBuf::try_from(root).unwrap();
            let globs = WorkspaceGlobs::new(vec!["packages/**"], vec![]).unwrap();

            let package_jsons: HashSet<String> = globs
                .get_package_jsons(&repo_root)
                .unwrap()
                .map(|p| repo_root.anchor(p).unwrap().to_string())
                .collect();

            assert!(
                package_jsons.contains(
                    &"packages/nested/deep-pkg/package.json"
                        .replace('/', std::path::MAIN_SEPARATOR_STR)
                ),
                "doublestar glob should find packages behind symlinks, got: {package_jsons:?}"
            );
        }

        #[test]
        fn rejects_package_behind_symlink_outside_repo_root() {
            let root_tmp = tempfile::TempDir::with_prefix("ws-symlink-outside-root").unwrap();
            let root = root_tmp.path();
            let outside_tmp = tempfile::TempDir::with_prefix("ws-symlink-outside-target").unwrap();
            let outside = outside_tmp.path();

            std::fs::create_dir_all(outside.join("widget-a")).unwrap();
            std::fs::write(
                outside.join("widget-a/package.json"),
                r#"{"name": "widget-a"}"#,
            )
            .unwrap();
            std::fs::create_dir_all(root.join("widgets")).unwrap();
            std::os::unix::fs::symlink(outside.join("widget-a"), root.join("widgets/widget-a"))
                .unwrap();

            let repo_root = AbsoluteSystemPathBuf::try_from(root).unwrap();
            let globs = WorkspaceGlobs::new(vec!["widgets/*"], vec![]).unwrap();

            let result = globs.get_package_jsons(&repo_root);

            assert!(
                result.is_err(),
                "workspace discovery should reject symlinked packages outside the repo root"
            );
        }
    }
}
