use std::collections::HashMap;

use anyhow::Result;
use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};
use turborepo_env::{BySource, DetailedMap, EnvironmentVariableMap};
use turborepo_lockfiles::Lockfile;
use turborepo_ui::UI;

use crate::{cli::EnvMode, package_json::PackageJson, package_manager::PackageManager};

static DEFAULT_ENV_VARS: [&str; 1] = ["VERCEL_ANALYTICS_ID"];

#[derive(Default)]
pub struct GlobalHashableInputs {
    global_cache_key: &'static str,
    global_file_hash_map: HashMap<RelativeUnixPathBuf, String>,
    root_external_deps_hash: String,
    env: Vec<String>,
    // Only Option to allow #[derive(Default)]
    resolved_env_vars: Option<DetailedMap>,
    pass_through_env: Vec<String>,
    env_mode: EnvMode,
    framework_inference: bool,
    dot_env: Vec<RelativeUnixPathBuf>,
}

#[allow(clippy::too_many_arguments)]
pub fn get_global_hash_inputs<L: ?Sized + Lockfile>(
    _ui: &UI,
    _root_path: &AbsoluteSystemPath,
    _root_package_json: &PackageJson,
    _package_manager: &PackageManager,
    _lockfile: Option<&L>,
    _global_file_dependencies: Vec<String>,
    env_at_execution_start: &EnvironmentVariableMap,
    global_env: Vec<String>,
    _global_pass_through_env: Vec<String>,
    _env_mode: EnvMode,
    _framework_inference: bool,
    _dot_env: Vec<RelativeUnixPathBuf>,
) -> Result<GlobalHashableInputs> {
    let default_env_var_map = env_at_execution_start.from_wildcards(&DEFAULT_ENV_VARS[..])?;

    let user_env_var_set =
        env_at_execution_start.wildcard_map_from_wildcards_unresolved(&global_env)?;

    let mut all_env_var_map = EnvironmentVariableMap::default();
    all_env_var_map.union(&user_env_var_set.inclusions);
    all_env_var_map.union(&default_env_var_map);
    all_env_var_map.difference(&user_env_var_set.exclusions);

    let mut explicit_env_var_map = EnvironmentVariableMap::default();
    explicit_env_var_map.union(&user_env_var_set.inclusions);
    explicit_env_var_map.difference(&user_env_var_set.exclusions);

    let mut matching_env_var_map = EnvironmentVariableMap::default();
    matching_env_var_map.union(&default_env_var_map);
    matching_env_var_map.difference(&user_env_var_set.exclusions);

    let global_hashable_env_vars = DetailedMap {
        all: all_env_var_map,
        by_source: BySource {
            explicit: explicit_env_var_map,
            matching: matching_env_var_map,
        },
    };

    Ok(GlobalHashableInputs {
        resolved_env_vars: Some(global_hashable_env_vars),
        ..GlobalHashableInputs::default()
    })
}
