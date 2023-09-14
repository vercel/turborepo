use std::collections::{HashMap, HashSet};

use anyhow::Result;
use globwalk::WalkType;
use thiserror::Error;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};
use turborepo_env::{get_global_hashable_env_vars, DetailedMap, EnvironmentVariableMap};
use turborepo_lockfiles::Lockfile;
use turborepo_scm::SCM;

use crate::{
    cli::EnvMode,
    hash::{GlobalHashable, TurboHash},
    package_graph::WorkspaceInfo,
    package_manager::PackageManager,
};

static DEFAULT_ENV_VARS: [&str; 1] = ["VERCEL_ANALYTICS_ID"];

const GLOBAL_CACHE_KEY: &str = "You don't understand! I coulda had class. I coulda been a \
                                contender. I could've been somebody, instead of a bum, which is \
                                what I am.";

#[derive(Debug, Error)]
enum GlobalHashError {}

#[derive(Debug, Default)]
pub struct GlobalHashableInputs<'a> {
    global_cache_key: &'static str,
    global_file_hash_map: HashMap<RelativeUnixPathBuf, String>,
    root_external_dependencies_hash: String,
    env: &'a [String],
    // Only Option to allow #[derive(Default)]
    resolved_env_vars: Option<DetailedMap>,
    pass_through_env: &'a [String],
    env_mode: EnvMode,
    framework_inference: bool,
    dot_env: &'a [RelativeUnixPathBuf],
}

#[allow(clippy::too_many_arguments)]
pub fn get_global_hash_inputs<'a, L: ?Sized + Lockfile>(
    root_workspace: &WorkspaceInfo,
    root_path: &AbsoluteSystemPath,
    package_manager: &PackageManager,
    lockfile: Option<&L>,
    global_file_dependencies: &'a [String],
    env_at_execution_start: &EnvironmentVariableMap,
    global_env: &'a [String],
    global_pass_through_env: &'a [String],
    env_mode: EnvMode,
    framework_inference: bool,
    dot_env: &'a [RelativeUnixPathBuf],
) -> Result<GlobalHashableInputs<'a>> {
    let global_hashable_env_vars =
        get_global_hashable_env_vars(env_at_execution_start, global_env)?;

    debug!(
        "global hash env vars {:?}",
        global_hashable_env_vars.all.names()
    );

    let mut global_deps = HashSet::new();

    if !global_file_dependencies.is_empty() {
        let globs = package_manager.get_workspace_globs(root_path)?;

        let files = globwalk::globwalk(
            root_path,
            global_file_dependencies,
            &globs.raw_exclusions,
            WalkType::All,
        )?;

        global_deps.extend(files);
    }

    if lockfile.is_none() {
        global_deps.insert(root_path.join_component("package.json"));
        if let Ok(lockfile_path) = package_manager.lockfile_path(root_path) {
            if lockfile_path.exists() {
                global_deps.insert(lockfile_path);
            }
        }
    }

    let hasher = SCM::new(root_path);

    let global_deps_paths = global_deps
        .iter()
        .map(|p| root_path.anchor(p).expect("path should be from root"))
        .collect::<Vec<_>>();

    let mut global_file_hash_map =
        hasher.get_hashes_for_files(root_path, &global_deps_paths, false)?;

    if !dot_env.is_empty() {
        let system_dot_env = dot_env.iter().map(|p| p.to_anchored_system_path_buf());

        let dot_env_object = hasher.hash_existing_of(root_path, system_dot_env)?;

        global_file_hash_map.extend(dot_env_object);
    }

    let root_external_dependencies_hash = root_workspace.get_external_deps_hash();

    debug!(
        "rust external deps hash: {}",
        root_external_dependencies_hash
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
    })
}

impl<'a> GlobalHashableInputs<'a> {
    pub fn calculate_global_hash_from_inputs(mut self) -> String {
        match self.env_mode {
            EnvMode::Infer if !self.pass_through_env.is_empty() => {
                self.env_mode = EnvMode::Strict;
            }
            EnvMode::Loose => {
                self.pass_through_env = &[];
            }
            _ => {}
        }

        self.calculate_global_hash()
    }

    fn calculate_global_hash(self) -> String {
        let global_hashable = GlobalHashable {
            global_cache_key: self.global_cache_key,
            global_file_hash_map: self.global_file_hash_map,
            root_external_dependencies_hash: self.root_external_dependencies_hash,
            env: self.env,
            resolved_env_vars: self
                .resolved_env_vars
                .map(|evm| evm.all.to_hashable())
                .unwrap_or_default(),
            pass_through_env: self.pass_through_env,
            env_mode: self.env_mode,
            framework_inference: self.framework_inference,
            dot_env: self.dot_env,
        };

        global_hashable.hash()
    }
}
