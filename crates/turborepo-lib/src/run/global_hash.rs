use std::collections::{HashMap, HashSet};

use globwalk::WalkType;
use thiserror::Error;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};
use turborepo_env::{get_global_hashable_env_vars, DetailedMap, EnvironmentVariableMap};
use turborepo_lockfiles::Lockfile;
use turborepo_repository::package_manager::{self, PackageManager};
use turborepo_scm::SCM;

use crate::{
    cli::EnvMode,
    hash::{GlobalHashable, TurboHash},
};

static DEFAULT_ENV_VARS: [&str; 1] = ["VERCEL_ANALYTICS_ID"];

const GLOBAL_CACHE_KEY: &str = "HEY STELLLLLLLAAAAAAAAAAAAA";

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Env(#[from] turborepo_env::Error),
    #[error(transparent)]
    Globwalk(#[from] globwalk::WalkError),
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
    pub env: &'a [String],
    // Only Option to allow #[derive(Default)]
    pub resolved_env_vars: Option<DetailedMap>,
    pub pass_through_env: Option<&'a [String]>,
    pub env_mode: EnvMode,
    pub framework_inference: bool,
    pub dot_env: Option<&'a [RelativeUnixPathBuf]>,
    pub env_at_execution_start: &'a EnvironmentVariableMap,
}

#[allow(clippy::too_many_arguments)]
pub fn get_global_hash_inputs<'a, L: ?Sized + Lockfile>(
    root_external_dependencies_hash: Option<&'a str>,
    root_path: &AbsoluteSystemPath,
    package_manager: &PackageManager,
    lockfile: Option<&L>,
    global_file_dependencies: &'a [String],
    env_at_execution_start: &'a EnvironmentVariableMap,
    global_env: &'a [String],
    global_pass_through_env: Option<&'a [String]>,
    env_mode: EnvMode,
    framework_inference: bool,
    dot_env: Option<&'a [RelativeUnixPathBuf]>,
) -> Result<GlobalHashableInputs<'a>, Error> {
    let global_hashable_env_vars =
        get_global_hashable_env_vars(env_at_execution_start, global_env)?;

    debug!(
        "global hash env vars {:?}",
        global_hashable_env_vars.all.names()
    );

    let mut global_deps = HashSet::new();

    if !global_file_dependencies.is_empty() {
        let exclusions = match package_manager.get_workspace_globs(root_path) {
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

        let files = globwalk::globwalk(
            root_path,
            global_file_dependencies,
            &exclusions,
            WalkType::All,
        )?;

        global_deps.extend(files);
    }

    if lockfile.is_none() {
        global_deps.insert(root_path.join_component("package.json"));
        let lockfile_path = package_manager.lockfile_path(root_path);
        if lockfile_path.exists() {
            global_deps.insert(lockfile_path);
        }
    }

    let hasher = SCM::new(root_path);

    let global_deps_paths = global_deps
        .iter()
        .map(|p| root_path.anchor(p).expect("path should be from root"))
        .collect::<Vec<_>>();

    let mut global_file_hash_map =
        hasher.get_hashes_for_files(root_path, &global_deps_paths, false)?;

    if !dot_env.unwrap_or_default().is_empty() {
        let system_dot_env = dot_env
            .into_iter()
            .flatten()
            .map(|p| p.to_anchored_system_path_buf());

        let dot_env_object = hasher.hash_existing_of(root_path, system_dot_env)?;

        global_file_hash_map.extend(dot_env_object);
    }

    debug!(
        "external deps hash: {}",
        root_external_dependencies_hash.unwrap_or("no hash (single package)")
    );

    Ok(GlobalHashableInputs {
        global_cache_key: GLOBAL_CACHE_KEY,
        global_file_hash_map,
        root_external_dependencies_hash,
        env: global_env,
        resolved_env_vars: Some(global_hashable_env_vars),
        pass_through_env: global_pass_through_env,
        env_mode,
        framework_inference,
        dot_env,
        env_at_execution_start,
    })
}

impl<'a> GlobalHashableInputs<'a> {
    pub fn calculate_global_hash_from_inputs(&mut self) -> String {
        match self.env_mode {
            // In infer mode, if there is any pass_through config (even if it is an empty array)
            // we'll hash the whole object, so we can detect changes to that config
            // Further, resolve the envMode to the concrete value.
            EnvMode::Infer if self.pass_through_env.is_some() => {
                self.env_mode = EnvMode::Strict;
            }
            EnvMode::Loose => {
                // Remove the passthroughs from hash consideration if we're explicitly loose.
                self.pass_through_env = None;
            }
            _ => {}
        }

        self.calculate_global_hash()
    }

    fn calculate_global_hash(&self) -> String {
        let global_hashable = GlobalHashable {
            global_cache_key: self.global_cache_key,
            global_file_hash_map: &self.global_file_hash_map,
            root_external_dependencies_hash: self.root_external_dependencies_hash,
            env: self.env,
            resolved_env_vars: self
                .resolved_env_vars
                .as_ref()
                .map(|evm| evm.all.to_hashable())
                .unwrap_or_default(),
            pass_through_env: self.pass_through_env.unwrap_or_default(),
            env_mode: self.env_mode,
            framework_inference: self.framework_inference,
            dot_env: self.dot_env.unwrap_or_default(),
        };

        global_hashable.hash()
    }
}
