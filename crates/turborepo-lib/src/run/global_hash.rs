use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use either::Either;
use globwalk::{ValidatedGlob, WalkType};
use itertools::Itertools;
use thiserror::Error;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPathBuf};
use turborepo_env::{get_global_hashable_env_vars, DetailedMap, EnvironmentVariableMap};
use turborepo_lockfiles::Lockfile;
use turborepo_repository::{
    package_graph::PackageInfo,
    package_manager::{self, PackageManager},
};
use turborepo_scm::SCM;

use crate::{
    cli::EnvMode,
    hash::{GlobalHashable, TurboHash},
};

static DEFAULT_ENV_VARS: [&str; 1] = ["VERCEL_ANALYTICS_ID"];

const GLOBAL_CACHE_KEY: &str = "I can’t see ya, but I know you’re here";

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Env(#[from] turborepo_env::Error),
    #[error(transparent)]
    Globwalk(#[from] globwalk::WalkError),
    #[error("invalid glob for globwalking: {0}")]
    Glob(#[from] globwalk::GlobError),
    #[error(transparent)]
    Scm(#[from] turborepo_scm::Error),
    #[error(transparent)]
    PackageManager(#[from] turborepo_repository::package_manager::Error),
}

#[derive(Debug)]
pub struct GlobalHashableInputs<'a> {
    pub global_cache_key: &'static str,
    pub global_file_hash_map: HashMap<RelativeUnixPathBuf, String>,
    // This is `None` in single package mode
    pub root_external_dependencies_hash: Option<&'a str>,
    pub root_internal_dependencies_hash: Option<&'a str>,
    pub engines: Option<HashMap<&'a str, &'a str>>,
    pub env: &'a [String],
    // Only Option to allow #[derive(Default)]
    pub resolved_env_vars: Option<DetailedMap>,
    pub pass_through_env: Option<&'a [String]>,
    pub env_mode: EnvMode,
    pub framework_inference: bool,
    pub env_at_execution_start: &'a EnvironmentVariableMap,
}

#[allow(clippy::too_many_arguments)]
pub fn get_global_hash_inputs<'a, L: ?Sized + Lockfile>(
    root_external_dependencies_hash: Option<&'a str>,
    root_internal_dependencies_hash: Option<&'a str>,
    root_package: &'a PackageInfo,
    root_path: &AbsoluteSystemPath,
    package_manager: &PackageManager,
    lockfile: Option<&L>,
    global_file_dependencies: &'a [String],
    env_at_execution_start: &'a EnvironmentVariableMap,
    global_env: &'a [String],
    global_pass_through_env: Option<&'a [String]>,
    env_mode: EnvMode,
    framework_inference: bool,
    hasher: &SCM,
) -> Result<GlobalHashableInputs<'a>, Error> {
    let engines = root_package.package_json.engines();

    let global_hashable_env_vars =
        get_global_hashable_env_vars(env_at_execution_start, global_env)?;

    debug!(
        "global hash env vars {:?}",
        global_hashable_env_vars.all.names()
    );

    let mut global_deps =
        collect_global_deps(package_manager, root_path, global_file_dependencies)?;

    if lockfile.is_none() {
        global_deps.insert(root_path.join_component("package.json"));
        let lockfile_path = package_manager.lockfile_path(root_path);
        if lockfile_path.exists() {
            global_deps.insert(lockfile_path);
        }
    }

    let global_deps_paths = global_deps
        .iter()
        .map(|p| root_path.anchor(p).expect("path should be from root"))
        .collect::<Vec<_>>();

    let global_file_hash_map = hasher.get_hashes_for_files(root_path, &global_deps_paths, false)?;

    debug!(
        "external deps hash: {}",
        root_external_dependencies_hash.unwrap_or("no hash (single package)")
    );

    Ok(GlobalHashableInputs {
        global_cache_key: GLOBAL_CACHE_KEY,
        global_file_hash_map,
        root_external_dependencies_hash,
        root_internal_dependencies_hash,
        engines,
        env: global_env,
        resolved_env_vars: Some(global_hashable_env_vars),
        pass_through_env: global_pass_through_env,
        env_mode,
        framework_inference,
        env_at_execution_start,
    })
}

fn collect_global_deps(
    package_manager: &PackageManager,
    root_path: &AbsoluteSystemPath,
    global_file_dependencies: &[String],
) -> Result<HashSet<AbsoluteSystemPathBuf>, Error> {
    if global_file_dependencies.is_empty() {
        return Ok(HashSet::new());
    }
    let workspace_exclusions = match package_manager.get_workspace_globs(root_path) {
        Ok(globs) => globs.raw_exclusions,
        // If we hit a missing workspaces error, we could be in single package mode
        // so we should just use the default globs
        Err(package_manager::Error::Workspace(_)) => {
            package_manager.get_default_exclusions().collect()
        }
        Err(err) => {
            debug!("no workspace globs found");
            return Err(err.into());
        }
    };
    let (raw_inclusions, raw_exclusions): (Vec<_>, Vec<_>) = global_file_dependencies
        .iter()
        .partition_map(|glob| match glob.strip_prefix('!') {
            None => Either::Left(glob.as_str()),
            Some(exclusion) => Either::Right(exclusion),
        });
    let exclusions = workspace_exclusions
        .iter()
        .map(|s| s.as_str())
        .chain(raw_exclusions.iter().copied())
        .map(ValidatedGlob::from_str)
        .collect::<Result<Vec<_>, _>>()?;

    #[cfg(not(windows))]
    let inclusions = raw_inclusions
        .iter()
        .map(|i| ValidatedGlob::from_str(i))
        .collect::<Result<Vec<_>, _>>()?;
    // This is a bit of a hack to ensure that we don't crash
    // when given an absolute path on Windows. We don't support
    // absolute paths, but the ':' from the drive letter will also
    // fail to compile to a glob. We already know we aren't going to
    // get anything good, and we've already logged a warning, but we
    // can modify the glob so it compiles. This is similar to the old
    // behavior, which tacked it on to the end of the base path unmodified,
    // and then would produce no files.
    #[cfg(windows)]
    let inclusions: Vec<ValidatedGlob> = global_file_dependencies
        .iter()
        .map(|s| ValidatedGlob::from_str(&s.replace(":", "")))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(globwalk::globwalk(
        root_path,
        &inclusions,
        &exclusions,
        WalkType::Files,
    )?)
}

impl<'a> GlobalHashableInputs<'a> {
    pub fn calculate_global_hash(&self) -> String {
        let global_hashable = GlobalHashable {
            global_cache_key: self.global_cache_key,
            global_file_hash_map: &self.global_file_hash_map,
            root_external_dependencies_hash: self.root_external_dependencies_hash,
            root_internal_dependencies_hash: self.root_internal_dependencies_hash,
            engines: self.engines.clone().unwrap_or_default(),
            env: self.env,
            resolved_env_vars: self
                .resolved_env_vars
                .as_ref()
                .map(|evm| evm.all.to_hashable())
                .unwrap_or_default(),
            pass_through_env: self.pass_through_env.unwrap_or_default(),
            env_mode: self.env_mode,
            framework_inference: self.framework_inference,
        };

        global_hashable.hash()
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_env::EnvironmentVariableMap;
    use turborepo_lockfiles::Lockfile;
    use turborepo_repository::{package_graph::PackageInfo, package_manager::PackageManager};
    use turborepo_scm::SCM;

    use super::get_global_hash_inputs;
    use crate::{cli::EnvMode, run::global_hash::collect_global_deps};

    #[test]
    fn test_absolute_path() {
        // We don't technically support absolute paths in global deps,
        // but we shouldn't crash. We already print out a warning.
        // Send an absolute path through and verify that we don't crash.
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        // Always default included, so it has to exist
        root.join_component("package.json")
            .create_with_contents("{}")
            .unwrap();

        let env_var_map = EnvironmentVariableMap::default();
        let package_info = PackageInfo::default();
        let lockfile: Option<&dyn Lockfile> = None;
        #[cfg(windows)]
        let file_deps = ["C:\\some\\path".to_string()];
        #[cfg(not(windows))]
        let file_deps = ["/some/path".to_string()];
        let result = get_global_hash_inputs(
            None,
            None,
            &package_info,
            &root,
            &PackageManager::Pnpm,
            lockfile,
            &file_deps,
            &env_var_map,
            &[],
            None,
            EnvMode::Strict,
            false,
            &SCM::new(&root),
        );
        assert!(result.is_ok());
    }

    /// get_global_hash_inputs should not yield any folders when walking since
    /// turbo does not consider changes to folders when evaluating hashes,
    /// only to files
    #[test]
    fn test_collect_only_yields_files() {
        let tmp = tempfile::tempdir().unwrap();

        // add some files
        //   - package.json
        //   - src/index.js
        //   - src/index.test.js
        //   - empty-folder/

        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let src = root.join_component("src");

        root.join_component("package.json")
            .create_with_contents("{}")
            .unwrap();
        root.join_component("empty-folder")
            .create_dir_all()
            .unwrap();

        src.create_dir_all().unwrap();
        src.join_component("index.js")
            .create_with_contents("console.log('hello world');")
            .unwrap();
        src.join_component("index.test.js")
            .create_with_contents("")
            .unwrap();

        let global_file_dependencies = vec!["**".to_string()];
        let results =
            collect_global_deps(&PackageManager::Berry, &root, &global_file_dependencies).unwrap();

        // should not yield the root folder itself, src, or empty-folder
        assert_eq!(results.len(), 3, "{:?}", results);
    }
}
